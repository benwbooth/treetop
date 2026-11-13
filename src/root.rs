//! Root access through polkit. `treetop --root-helper`, started once with
//! pkexec, answers line-based requests on stdin/stdout:
//!
//!   sample               -> {"io": {pid: [read, write]}, "exe": {pid: path},
//!                            "gpu": {pid: [{pdev, vram, gtt, engines}]}}
//!   inspect <pid>        -> {"environ": ..., "files": ...}
//!   signal <sig> <pid>.. -> {"ok": n, "errors": [...]}
//!   renice <n> <pid>..   -> {"ok": n, "errors": [...]}
//!
//! It only reads /proc and sends signals/priorities, and exits when its
//! stdin closes (treetop quit or turned root mode off).

use crate::{gpu, inspect};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// Privileged readings merged into the normal sample.
#[derive(Default)]
pub struct RootData {
    pub io: HashMap<i32, (u64, u64)>,
    pub exe: HashMap<i32, String>,
    pub gpu: HashMap<i32, Vec<gpu::GpuUse>>,
}

impl RootData {
    pub fn from_json(v: &Value) -> RootData {
        let pid = |k: &String| k.parse::<i32>().ok();
        let mut d = RootData::default();
        if let Some(m) = v["io"].as_object() {
            for (k, x) in m {
                if let (Some(p), Some(r), Some(w)) = (pid(k), x[0].as_u64(), x[1].as_u64()) {
                    d.io.insert(p, (r, w));
                }
            }
        }
        if let Some(m) = v["exe"].as_object() {
            for (k, x) in m {
                if let (Some(p), Some(s)) = (pid(k), x.as_str()) {
                    d.exe.insert(p, s.to_string());
                }
            }
        }
        if let Some(m) = v["gpu"].as_object() {
            for (k, list) in m {
                let Some(p) = pid(k) else { continue };
                let uses = list
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|u| {
                                let mut engines = [0.0; 5];
                                for (i, e) in engines.iter_mut().enumerate() {
                                    *e = u["engines"][i].as_f64().unwrap_or(0.0);
                                }
                                gpu::GpuUse {
                                    pdev: u["pdev"].as_str().unwrap_or("").to_string(),
                                    vram: u["vram"].as_u64().unwrap_or(0),
                                    gtt: u["gtt"].as_u64().unwrap_or(0),
                                    engines,
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                d.gpu.insert(p, uses);
            }
        }
        d
    }
}

fn pids() -> Vec<i32> {
    std::fs::read_dir("/proc")
        .map(|d| d.flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).collect())
        .unwrap_or_default()
}

fn sample(gpu: &gpu::Shared) -> Value {
    let mut io = Map::new();
    let mut exe = Map::new();
    for pid in pids() {
        if let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/io")) {
            let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)?.trim().parse::<u64>().ok()).unwrap_or(0);
            io.insert(pid.to_string(), json!([field("read_bytes:"), field("write_bytes:")]));
        }
        if let Ok(p) = std::fs::read_link(format!("/proc/{pid}/exe")) {
            exe.insert(pid.to_string(), json!(p.display().to_string()));
        }
    }
    let gpu: Map<String, Value> = gpu
        .lock()
        .unwrap()
        .iter()
        .map(|(pid, uses)| {
            let list: Vec<Value> = uses
                .iter()
                .map(|u| json!({"pdev": u.pdev, "vram": u.vram, "gtt": u.gtt, "engines": u.engines}))
                .collect();
            (pid.to_string(), json!(list))
        })
        .collect();
    json!({"io": io, "exe": exe, "gpu": gpu})
}

fn act(cmd: &str, rest: &[&str]) -> Value {
    let mut ok = 0;
    let mut errors = Vec::new();
    let Some(n) = rest.first().and_then(|s| s.parse::<i32>().ok()) else {
        return json!({"ok": 0, "errors": ["bad request"]});
    };
    for p in rest[1..].iter().filter_map(|s| s.parse::<i32>().ok()) {
        if p <= 1 && cmd == "signal" && n != libc::SIGSTOP && n != libc::SIGCONT {
            errors.push(format!("refusing to signal PID {p}"));
            continue;
        }
        let r = if cmd == "signal" {
            unsafe { libc::kill(p, n) }
        } else {
            unsafe { libc::setpriority(libc::PRIO_PROCESS, p as libc::id_t, n) }
        };
        if r == 0 {
            ok += 1;
        } else {
            errors.push(format!("{p}: {}", std::io::Error::last_os_error()));
        }
    }
    json!({"ok": ok, "errors": errors})
}

/// `treetop --root-helper`: the loop that runs as root.
pub fn helper_main() {
    let gpu = gpu::spawn_poller();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(out, "ready");
    let _ = out.flush();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let parts: Vec<&str> = line.split_whitespace().collect();
        let reply = match parts.first().copied() {
            Some("sample") => sample(&gpu),
            Some("inspect") => {
                let pid = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
                json!({"environ": inspect::environ(pid), "files": inspect::files(pid)})
            }
            Some(cmd @ ("signal" | "renice")) => act(cmd, &parts[1..]),
            Some("quit") | None => break,
            Some(_) => json!({"error": "unknown request"}),
        };
        let _ = writeln!(out, "{reply}");
        let _ = out.flush();
    }
}

/// `treetop --inspect <pid>` (one-shot, run through pkexec).
pub fn inspect_main(pid: i32) {
    println!("{}", json!({"environ": inspect::environ(pid), "files": inspect::files(pid)}));
}

fn self_exe() -> Result<String, String> {
    std::env::current_exe().map(|p| p.display().to_string()).map_err(|e| e.to_string())
}

/// The running helper, as seen from the unprivileged side.
pub struct Helper {
    _child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
}

impl Helper {
    /// Starts the helper through pkexec; blocks until polkit has answered.
    pub fn start() -> Result<Helper, String> {
        let mut child = Command::new("pkexec")
            .arg(self_exe()?)
            .arg("--root-helper")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("couldn't run pkexec: {e}"))?;
        let stdin = child.stdin.take().unwrap();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut first = String::new();
        let _ = out.read_line(&mut first);
        if first.trim() != "ready" {
            let _ = child.wait();
            return Err("root access wasn't granted".into());
        }
        Ok(Helper { _child: child, stdin, out })
    }

    pub fn request(&mut self, line: &str) -> Option<Value> {
        writeln!(self.stdin, "{line}").ok()?;
        self.stdin.flush().ok()?;
        let mut reply = String::new();
        if self.out.read_line(&mut reply).ok()? == 0 {
            return None;
        }
        serde_json::from_str(&reply).ok()
    }
}

/// One-time privileged read of a process's environment and open files.
pub fn inspect_once(pid: i32) -> Result<Value, String> {
    let out = Command::new("pkexec")
        .arg(self_exe()?)
        .arg("--inspect")
        .arg(pid.to_string())
        .output()
        .map_err(|e| format!("couldn't run pkexec: {e}"))?;
    if !out.status.success() {
        return Err("root access wasn't granted".into());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}
