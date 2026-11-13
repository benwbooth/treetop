//! Root access through polkit. `treetop --root-helper`, started once with
//! pkexec, answers line-based requests on stdin/stdout:
//!
//!   sample               -> {"io": {pid: [read, write]}, "exe": {pid: path},
//!                            "gpu": {pid: [{pdev, vram, gtt, engines}]},
//!                            "net": {pid: [rx, tx]} (bytes since last sample),
//!                            "netSecs": seconds covered}
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
    /// Per-process network rates (rx, tx) in bytes/s, from packet headers.
    pub net: HashMap<i32, (f64, f64)>,
    /// liquidctl sensors read as root (reaches root-only coolers).
    pub cooling: Vec<Value>,
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
        d.cooling = v["cooling"].as_array().cloned().unwrap_or_default();
        let secs = v["netSecs"].as_f64().unwrap_or(1.0).max(0.05);
        if let Some(m) = v["net"].as_object() {
            for (k, x) in m {
                if let (Some(p), Some(r), Some(t)) = (pid(k), x[0].as_u64(), x[1].as_u64()) {
                    d.net.insert(p, (r as f64 / secs, t as f64 / secs));
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

fn sample(gpu: &gpu::Shared, net: &mut crate::net::NetAccounting, last: &mut std::time::Instant) -> Value {
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
    let secs = last.elapsed().as_secs_f64();
    *last = std::time::Instant::now();
    let netmap: Map<String, Value> = net.take().into_iter().map(|(p, (r, t))| (p.to_string(), json!([r, t]))).collect();
    json!({"io": io, "exe": exe, "gpu": gpu, "net": netmap, "netSecs": secs})
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

/// How long the helper waits for treetop to come back after it exits, so a
/// restart or rebuild reconnects without another password prompt.
const LINGER: std::time::Duration = std::time::Duration::from_secs(600);

/// The helper's socket, in the authorizing user's private runtime directory
/// (only they can reach it; connections are also checked by uid).
fn socket_path(uid: u32) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("/run/user/{uid}/treetop-root.sock"))
}

struct HelperState {
    gpu: gpu::Shared,
    cooling: crate::cooling::Shared,
    net: crate::net::NetAccounting,
    last: std::time::Instant,
}

/// Answers one client's requests until it disconnects.
fn serve(input: impl BufRead, mut out: impl Write, st: &mut HelperState) {
    let _ = writeln!(out, "ready");
    let _ = out.flush();
    for line in input.lines() {
        let Ok(line) = line else { break };
        let parts: Vec<&str> = line.split_whitespace().collect();
        let reply = match parts.first().copied() {
            Some("sample") => {
                let mut v = sample(&st.gpu, &mut st.net, &mut st.last);
                v["cooling"] = json!(*st.cooling.lock().unwrap());
                v
            }
            Some("inspect") => {
                let pid = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
                json!({"environ": inspect::environ(pid), "files": inspect::files(pid)})
            }
            Some(cmd @ ("signal" | "renice")) => act(cmd, &parts[1..]),
            Some("quit") | None => break,
            Some(_) => json!({"error": "unknown request"}),
        };
        if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() {
            break;
        }
    }
}

/// The connecting process's uid (SO_PEERCRED).
fn peer_uid(stream: &std::os::unix::net::UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let r = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    (r == 0).then_some(cred.uid)
}

/// `treetop --root-helper`, run as root through pkexec. Serves treetop over
/// stdin/stdout; when treetop exits it waits LINGER on a socket only the
/// authorizing user can use, so a restart reconnects without a prompt.
pub fn helper_main() {
    let mut st = HelperState {
        gpu: gpu::spawn_poller(),
        cooling: crate::cooling::spawn_poller(),
        net: crate::net::NetAccounting::start(),
        last: std::time::Instant::now(),
    };
    serve(std::io::stdin().lock(), std::io::stdout().lock(), &mut st);

    let Some(uid) = std::env::var("PKEXEC_UID").ok().and_then(|u| u.parse::<u32>().ok()) else { return };
    let path = socket_path(uid);
    let _ = std::fs::remove_file(&path);
    let Ok(listener) = std::os::unix::net::UnixListener::bind(&path) else { return };
    if let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) {
        unsafe {
            libc::chmod(c.as_ptr(), 0o600);
            libc::chown(c.as_ptr(), uid, u32::MAX);
        }
    }
    let _ = listener.set_nonblocking(true);
    let mut deadline = std::time::Instant::now() + LINGER;
    while std::time::Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _)) => {
                if peer_uid(&stream) != Some(uid) {
                    continue; // only the user who authorized us
                }
                let _ = stream.set_nonblocking(false);
                if let Ok(reader) = stream.try_clone() {
                    serve(BufReader::new(reader), stream, &mut st);
                }
                deadline = std::time::Instant::now() + LINGER;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    }
    let _ = std::fs::remove_file(&path);
}

/// `treetop --inspect <pid>` (one-shot, run through pkexec).
pub fn inspect_main(pid: i32) {
    println!("{}", json!({"environ": inspect::environ(pid), "files": inspect::files(pid)}));
}

/// A pkexec command that works from any environment: the setuid wrapper by
/// full path (a dev shell's PATH may hold a non-setuid copy), and no $SHELL
/// (pkexec refuses to run when $SHELL isn't listed in /etc/shells, as with
/// a Nix dev shell's bash).
pub fn pkexec() -> Command {
    let path = ["/run/wrappers/bin/pkexec", "/usr/bin/pkexec", "/bin/pkexec"]
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
        .unwrap_or("pkexec");
    let mut c = Command::new(path);
    c.env_remove("SHELL");
    c
}

/// pkexec's own explanation, for the status line.
pub fn pkexec_error(code: Option<i32>, stderr: &str) -> String {
    match code {
        Some(126) => "Root access: authorization was cancelled".into(),
        Some(127) => "Root access: not authorized".into(),
        _ => {
            let msg = stderr.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("").trim();
            if msg.is_empty() { "Root access failed".into() } else { format!("Root access failed: {msg}") }
        }
    }
}

fn self_exe() -> Result<String, String> {
    std::env::current_exe().map(|p| p.display().to_string()).map_err(|e| e.to_string())
}

enum Transport {
    Child { _child: Child, stdin: ChildStdin, out: BufReader<ChildStdout> },
    Socket { stream: std::os::unix::net::UnixStream, out: BufReader<std::os::unix::net::UnixStream> },
}

/// The running helper, as seen from the unprivileged side.
pub struct Helper {
    t: Transport,
}

impl Helper {
    /// Reconnects to a helper still waiting from a previous run (no prompt),
    /// else starts one through pkexec (blocks while polkit asks).
    pub fn start() -> Result<Helper, String> {
        if let Some(h) = Self::reconnect() {
            return Ok(h);
        }
        Self::spawn()
    }

    fn reconnect() -> Option<Helper> {
        let path = socket_path(unsafe { libc::getuid() });
        let stream = std::os::unix::net::UnixStream::connect(&path).ok()?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(3))).ok()?;
        let mut out = BufReader::new(stream.try_clone().ok()?);
        let mut first = String::new();
        out.read_line(&mut first).ok()?;
        if first.trim() != "ready" {
            return None;
        }
        stream.set_read_timeout(None).ok()?;
        Some(Helper { t: Transport::Socket { stream, out } })
    }

    fn spawn() -> Result<Helper, String> {
        let mut child = pkexec()
            .arg(self_exe()?)
            .arg("--root-helper")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("couldn't run pkexec: {e}"))?;
        let stdin = child.stdin.take().unwrap();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut first = String::new();
        let _ = out.read_line(&mut first);
        if first.trim() != "ready" {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = std::io::Read::read_to_string(&mut e, &mut err);
            }
            let code = child.wait().ok().and_then(|s| s.code());
            return Err(pkexec_error(code, &err));
        }
        Ok(Helper { t: Transport::Child { _child: child, stdin, out } })
    }

    pub fn request(&mut self, line: &str) -> Option<Value> {
        let mut reply = String::new();
        let n = match &mut self.t {
            Transport::Child { stdin, out, .. } => {
                writeln!(stdin, "{line}").ok()?;
                stdin.flush().ok()?;
                out.read_line(&mut reply).ok()?
            }
            Transport::Socket { stream, out } => {
                writeln!(stream, "{line}").ok()?;
                stream.flush().ok()?;
                out.read_line(&mut reply).ok()?
            }
        };
        if n == 0 {
            return None;
        }
        serde_json::from_str(&reply).ok()
    }
}

/// One-time privileged read of a process's environment and open files.
pub fn inspect_once(pid: i32) -> Result<Value, String> {
    let out = pkexec()
        .arg(self_exe()?)
        .arg("--inspect")
        .arg(pid.to_string())
        .output()
        .map_err(|e| format!("couldn't run pkexec: {e}"))?;
    if !out.status.success() {
        return Err(pkexec_error(out.status.code(), &String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}
