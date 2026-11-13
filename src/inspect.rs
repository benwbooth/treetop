//! Per-process detail for the details dialog: environment variables and
//! open file descriptors, with sockets resolved to their endpoints.
//! Both need the same access as ptrace: your own processes, or root.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;

/// Environment as sorted [name, value] pairs; None if unreadable.
pub fn environ(pid: i32) -> Option<Value> {
    let raw = fs::read(format!("/proc/{pid}/environ")).ok()?;
    let mut vars: Vec<(String, String)> = raw
        .split(|b| *b == 0)
        .filter(|e| !e.is_empty())
        .map(|e| {
            let s = String::from_utf8_lossy(e);
            match s.split_once('=') {
                Some((k, v)) => (k.to_string(), v.to_string()),
                None => (s.to_string(), String::new()),
            }
        })
        .collect();
    vars.sort();
    Some(json!(vars.into_iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>()))
}

fn hex_port(h: &str) -> u16 {
    u16::from_str_radix(h, 16).unwrap_or(0)
}

/// "0100007F:1F90" -> "127.0.0.1:8080"; IPv6 is four little-endian words.
fn addr(field: &str, v6: bool) -> String {
    let Some((ip, port)) = field.split_once(':') else { return field.to_string() };
    let port = hex_port(port);
    if !v6 {
        let n = u32::from_str_radix(ip, 16).unwrap_or(0);
        let b = n.to_le_bytes();
        return format!("{}.{}.{}.{}:{port}", b[0], b[1], b[2], b[3]);
    }
    let mut bytes = [0u8; 16];
    for w in 0..4 {
        let word = u32::from_str_radix(ip.get(w * 8..w * 8 + 8).unwrap_or("0"), 16).unwrap_or(0);
        bytes[w * 4..w * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    let a = std::net::Ipv6Addr::from(bytes);
    match a.to_ipv4_mapped() {
        Some(v4) => format!("{v4}:{port}"),
        None => format!("[{a}]:{port}"),
    }
}

fn tcp_state(code: &str) -> &'static str {
    match code {
        "01" => "ESTABLISHED",
        "02" => "SYN_SENT",
        "03" => "SYN_RECV",
        "04" => "FIN_WAIT1",
        "05" => "FIN_WAIT2",
        "06" => "TIME_WAIT",
        "07" => "CLOSE",
        "08" => "CLOSE_WAIT",
        "09" => "LAST_ACK",
        "0A" => "LISTEN",
        "0B" => "CLOSING",
        _ => "",
    }
}

/// Socket inode -> description, from the process's own network namespace.
fn sockets(pid: i32) -> HashMap<u64, String> {
    let mut map = HashMap::new();
    for (file, proto, v6) in [("tcp", "TCP", false), ("tcp6", "TCP", true), ("udp", "UDP", false), ("udp6", "UDP", true)] {
        let Ok(text) = fs::read_to_string(format!("/proc/{pid}/net/{file}")) else { continue };
        for line in text.lines().skip(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 10 {
                continue;
            }
            let inode: u64 = f[9].parse().unwrap_or(0);
            let local = addr(f[1], v6);
            let remote = addr(f[2], v6);
            let desc = if proto == "TCP" && f[3] == "0A" {
                format!("{proto} {local} LISTEN")
            } else if remote.ends_with(":0") && (remote.starts_with("0.0.0.0") || remote.starts_with("[::]")) {
                format!("{proto} {local}")
            } else {
                let st = if proto == "TCP" { tcp_state(f[3]) } else { "" };
                format!("{proto} {local} → {remote} {st}").trim_end().to_string()
            };
            map.insert(inode, desc);
        }
    }
    // Netlink: sockets for talking to the kernel (network config, udev, ...).
    if let Ok(text) = fs::read_to_string(format!("/proc/{pid}/net/netlink")) {
        for line in text.lines().skip(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 10 {
                continue;
            }
            let inode: u64 = f[9].parse().unwrap_or(0);
            let kind = match f[1] {
                "0" => "route (network config)",
                "4" => "sock_diag",
                "9" => "audit",
                "15" => "uevent (udev/devices)",
                "16" => "generic",
                p => p,
            };
            map.insert(inode, format!("netlink {kind}"));
        }
    }
    for (file, proto) in [("raw", "RAW"), ("raw6", "RAW6"), ("udplite", "UDPLITE")] {
        let Ok(text) = fs::read_to_string(format!("/proc/{pid}/net/{file}")) else { continue };
        for line in text.lines().skip(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() > 9 {
                map.insert(f[9].parse().unwrap_or(0), format!("{proto} {}", addr(f[1], file.ends_with('6'))));
            }
        }
    }
    if let Ok(text) = fs::read_to_string(format!("/proc/{pid}/net/unix")) {
        for line in text.lines().skip(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 7 {
                continue;
            }
            let inode: u64 = f[6].parse().unwrap_or(0);
            let path = f.get(7).map(|p| p.to_string()).unwrap_or_else(|| "(unnamed)".into());
            map.insert(inode, format!("unix {path}"));
        }
    }
    map
}

/// Open file descriptors as {fd, kind, target}; None if unreadable.
pub fn files(pid: i32) -> Option<Value> {
    let dir = fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    let mut fds: Vec<(i64, String)> = dir
        .flatten()
        .filter_map(|e| {
            let fd: i64 = e.file_name().to_str()?.parse().ok()?;
            let target = fs::read_link(e.path()).ok()?.display().to_string();
            Some((fd, target))
        })
        .collect();
    fds.sort();
    let socks = if fds.iter().any(|(_, t)| t.starts_with("socket:")) { sockets(pid) } else { HashMap::new() };
    let out: Vec<Value> = fds
        .into_iter()
        .map(|(fd, target)| {
            let (kind, shown) = if let Some(inode) = target.strip_prefix("socket:[").and_then(|s| s.strip_suffix(']')) {
                let desc = inode.parse::<u64>().ok().and_then(|i| socks.get(&i).cloned());
                ("socket", desc.unwrap_or_else(|| format!("socket (inode {inode})")))
            } else if target.starts_with("pipe:") {
                ("pipe", target.clone())
            } else if target.starts_with("anon_inode:") {
                ("anon", target.trim_start_matches("anon_inode:").to_string())
            } else if target.starts_with("/dev/") {
                ("device", target.clone())
            } else if target.starts_with("/memfd:") {
                ("memfd", target.clone())
            } else {
                ("file", target.clone())
            };
            json!({"fd": fd, "kind": kind, "target": shown})
        })
        .collect();
    Some(json!(out))
}
