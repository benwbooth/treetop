//! Container awareness: maps a process's cgroup to a Docker/Podman/containerd
//! container ID, and polls the engines' API sockets in the background for the
//! human names.

use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Container {
    pub name: String,
    pub image: String,
    pub engine: &'static str,
    pub status: String,
}

pub type Names = Arc<Mutex<HashMap<String, Container>>>;

/// Extracts the 64-hex container ID from a /proc/<pid>/cgroup body, along
/// with the engine that the cgroup path names.
pub fn container_id(cgroup: &str) -> Option<(String, &'static str)> {
    for line in cgroup.lines() {
        let path = line.rsplit(':').next().unwrap_or("");
        for seg in path.split('/').rev() {
            let (engine, rest) = if let Some(r) = seg.strip_prefix("docker-") {
                ("docker", r)
            } else if let Some(r) = seg.strip_prefix("libpod-") {
                ("podman", r)
            } else if let Some(r) = seg.strip_prefix("cri-containerd-") {
                ("containerd", r)
            } else if let Some(r) = seg.strip_prefix("crio-") {
                ("cri-o", r)
            } else {
                ("docker", seg)
            };
            let id = rest.strip_suffix(".scope").unwrap_or(rest);
            if id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
                // A bare 64-hex segment is cgroupfs-driver Docker (/docker/<id>).
                let engine = if engine == "docker" && path.contains("/libpod") { "podman" } else { engine };
                return Some((id.to_string(), engine));
            }
        }
    }
    None
}

fn http_get_unix(socket: &str, path: &str) -> Option<Value> {
    let mut s = UnixStream::connect(socket).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    write!(s, "GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n").ok()?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let body = text.split_once("\r\n\r\n")?.1;
    serde_json::from_str(body).ok()
}

fn sockets() -> Vec<(String, &'static str)> {
    let mut v = vec![("/var/run/docker.sock".to_string(), "docker"), ("/run/podman/podman.sock".to_string(), "podman")];
    if let Ok(rt) = std::env::var("XDG_RUNTIME_DIR") {
        v.push((format!("{rt}/podman/podman.sock"), "podman"));
        v.push((format!("{rt}/docker.sock"), "docker"));
    }
    v
}

pub fn spawn_poller() -> Names {
    let names: Names = Arc::new(Mutex::new(HashMap::new()));
    let shared = names.clone();
    std::thread::spawn(move || loop {
        let mut map = HashMap::new();
        for (sock, engine) in sockets() {
            if !std::path::Path::new(&sock).exists() {
                continue;
            }
            let Some(Value::Array(list)) = http_get_unix(&sock, "/containers/json") else { continue };
            for c in list {
                let id = c["Id"].as_str().unwrap_or_default().to_string();
                let name = c["Names"][0].as_str().unwrap_or(&id[..12.min(id.len())]).trim_start_matches('/').to_string();
                map.insert(
                    id,
                    Container {
                        name,
                        image: c["Image"].as_str().unwrap_or_default().to_string(),
                        engine,
                        status: c["Status"].as_str().unwrap_or_default().to_string(),
                    },
                );
            }
        }
        *shared.lock().unwrap() = map;
        std::thread::sleep(Duration::from_secs(3));
    });
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_systemd_driver() {
        let id = "a".repeat(64);
        let cg = format!("0::/system.slice/docker-{id}.scope");
        assert_eq!(container_id(&cg), Some((id, "docker")));
    }
    #[test]
    fn parses_cgroupfs_driver() {
        let id = "b".repeat(64);
        assert_eq!(container_id(&format!("0::/docker/{id}")), Some((id, "docker")));
    }
    #[test]
    fn host_process_has_none() {
        assert_eq!(container_id("0::/user.slice/user-1000.slice/session-2.scope"), None);
    }
}
