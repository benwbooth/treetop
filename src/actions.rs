//! Signals and priority changes. Tries directly first; when the kernel says
//! EPERM (someone else's process, e.g. root's) the same action is re-run
//! through pkexec, which makes the desktop's polkit agent ask for a password.

use std::process::Command;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

pub struct Actions {
    tx: Mutex<Sender<String>>,
    rx: Mutex<Receiver<String>>,
    /// The polkit root helper, when root mode is on: used instead of a
    /// pkexec prompt per action.
    root: std::sync::Arc<Mutex<Option<crate::root::Helper>>>,
}

pub const SIGNALS: &[(&str, i32)] = &[
    ("TERM", libc::SIGTERM),
    ("KILL", libc::SIGKILL),
    ("HUP", libc::SIGHUP),
    ("INT", libc::SIGINT),
    ("QUIT", libc::SIGQUIT),
    ("STOP", libc::SIGSTOP),
    ("CONT", libc::SIGCONT),
    ("USR1", libc::SIGUSR1),
    ("USR2", libc::SIGUSR2),
    ("ABRT", libc::SIGABRT),
    ("ALRM", libc::SIGALRM),
    ("WINCH", libc::SIGWINCH),
];

fn signal_name(sig: i32) -> String {
    SIGNALS.iter().find(|(_, n)| *n == sig).map(|(s, _)| s.to_string()).unwrap_or_else(|| sig.to_string())
}

impl Actions {
    pub fn new(root: std::sync::Arc<Mutex<Option<crate::root::Helper>>>) -> Self {
        let (tx, rx) = channel();
        Actions { tx: Mutex::new(tx), rx: Mutex::new(rx), root }
    }

    /// A message for the status line.
    pub fn notify(&self, msg: String) {
        let _ = self.tx.lock().unwrap().send(msg);
    }

    /// Runs a signal/renice through the root helper if it's running.
    fn via_helper(&self, what: &str, request: String) -> Option<String> {
        let mut guard = self.root.lock().unwrap();
        let reply = guard.as_mut()?.request(&request)?;
        let ok = reply["ok"].as_u64().unwrap_or(0);
        let errors: Vec<String> = reply["errors"].as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(String::from)).collect()).unwrap_or_default();
        let mut s = format!("{what}: done as root on {ok}");
        if !errors.is_empty() {
            s += &format!("; {}", errors.join("; "));
        }
        Some(s)
    }

    /// Messages finished since the last call (results of elevated actions).
    pub fn drain(&self) -> Vec<String> {
        self.rx.lock().unwrap().try_iter().collect()
    }

    fn elevate(&self, what: String, args: Vec<String>) -> String {
        let tx = self.tx.lock().unwrap().clone();
        let pending = format!("{what}: asking for authorization…");
        std::thread::spawn(move || {
            let res = Command::new("pkexec").args(&args).output();
            let msg = match res {
                Ok(o) if o.status.success() => format!("{what}: done (as root)"),
                Ok(o) if o.status.code() == Some(126) => format!("{what}: authorization cancelled"),
                Ok(o) if o.status.code() == Some(127) => format!("{what}: not authorized"),
                Ok(o) => format!("{what}: failed: {}", String::from_utf8_lossy(&o.stderr).trim()),
                Err(e) => format!("{what}: could not run pkexec: {e}"),
            };
            let _ = tx.send(msg);
        });
        pending
    }

    pub fn send_signal(&self, pids: &[i32], sig: i32) -> String {
        let name = signal_name(sig);
        let mut denied = Vec::new();
        let mut errors = Vec::new();
        let mut ok = 0;
        for &pid in pids {
            if pid <= 1 && sig != libc::SIGCONT && sig != libc::SIGSTOP {
                errors.push(format!("refusing to signal PID {pid}"));
                continue;
            }
            if unsafe { libc::kill(pid, sig) } == 0 {
                ok += 1;
            } else {
                match std::io::Error::last_os_error().raw_os_error() {
                    Some(libc::EPERM) => denied.push(pid),
                    Some(libc::ESRCH) => errors.push(format!("{pid} already exited")),
                    _ => errors.push(format!("{pid}: {}", std::io::Error::last_os_error())),
                }
            }
        }
        let mut parts = Vec::new();
        if ok > 0 {
            parts.push(format!("SIG{name} sent to {ok} process{}", if ok == 1 { "" } else { "es" }));
        }
        let list_args = denied.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(" ");
        if !denied.is_empty() {
            if let Some(r) = self.via_helper(&format!("SIG{name}"), format!("signal {sig} {list_args}")) {
                parts.push(r);
                parts.extend(errors);
                return parts.join("; ");
            }
            let kill = which("kill");
            let mut args = vec![kill, "-s".into(), name.clone()];
            args.extend(denied.iter().map(|p| p.to_string()));
            let list = denied.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
            parts.push(self.elevate(format!("SIG{name} → {list}"), args));
        }
        parts.extend(errors);
        parts.join("; ")
    }

    pub fn renice(&self, pids: &[i32], nice: i32) -> String {
        let mut denied = Vec::new();
        let mut ok = 0;
        for &pid in pids {
            if unsafe { libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, nice) } == 0 {
                ok += 1;
            } else {
                denied.push(pid);
            }
        }
        let mut parts = Vec::new();
        if ok > 0 {
            parts.push(format!("nice {nice} set on {ok}"));
        }
        let list_args = denied.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(" ");
        if !denied.is_empty() {
            if let Some(r) = self.via_helper(&format!("renice {nice}"), format!("renice {nice} {list_args}")) {
                parts.push(r);
                return parts.join("; ");
            }
            let mut args = vec![which("renice"), "-n".into(), nice.to_string(), "-p".into()];
            args.extend(denied.iter().map(|p| p.to_string()));
            parts.push(self.elevate(format!("renice {nice}"), args));
        }
        parts.join("; ")
    }
}

/// pkexec needs an absolute path; resolve it from PATH ourselves.
fn which(cmd: &str) -> String {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .map(|d| std::path::Path::new(d).join(cmd))
        .find(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("/run/current-system/sw/bin/{cmd}"))
}
