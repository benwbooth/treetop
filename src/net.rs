//! Per-process network traffic, measured by the root helper the way nethogs
//! does: Linux keeps no per-process byte counters, so we read packet
//! *headers* (never payloads) from an AF_PACKET socket, add up bytes per local
//! (protocol, address, port), and attribute them to the process owning the
//! matching socket (socket tables + each process's open fds).

use std::collections::HashMap;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// (protocol 6/17, local address, local port)
type Key = (u8, IpAddr, u16);
type Counts = Arc<Mutex<HashMap<Key, (u64, u64)>>>;

pub struct NetAccounting {
    counts: Counts,
    inode_pid: HashMap<u64, i32>,
    last_scan: Option<Instant>,
}

fn parse_packet(buf: &[u8], outgoing: bool) -> Option<Key> {
    let version = buf.first()? >> 4;
    let (proto, src, dst, l4) = match version {
        4 => {
            let ihl = ((buf[0] & 0x0f) as usize) * 4;
            let proto = *buf.get(9)?;
            let src = IpAddr::V4(Ipv4Addr::new(*buf.get(12)?, buf[13], buf[14], buf[15]));
            let dst = IpAddr::V4(Ipv4Addr::new(*buf.get(16)?, buf[17], buf[18], buf[19]));
            (proto, src, dst, ihl)
        }
        6 => {
            let proto = *buf.get(6)?;
            let a: [u8; 16] = buf.get(8..24)?.try_into().ok()?;
            let b: [u8; 16] = buf.get(24..40)?.try_into().ok()?;
            (proto, IpAddr::V6(Ipv6Addr::from(a)), IpAddr::V6(Ipv6Addr::from(b)), 40)
        }
        _ => return None,
    };
    if proto != 6 && proto != 17 {
        return None;
    }
    let ports = buf.get(l4..l4 + 4)?;
    let sport = u16::from_be_bytes([ports[0], ports[1]]);
    let dport = u16::from_be_bytes([ports[2], ports[3]]);
    Some(if outgoing { (proto, src, sport) } else { (proto, dst, dport) })
}

fn capture(counts: Counts) {
    unsafe {
        let fd = libc::socket(libc::AF_PACKET, libc::SOCK_DGRAM, (libc::ETH_P_ALL as u16).to_be() as i32);
        if fd < 0 {
            return;
        }
        let lo = libc::if_nametoindex(c"lo".as_ptr()) as i32;
        // Headers only: MSG_TRUNC still reports the full packet length.
        let mut buf = [0u8; 128];
        loop {
            let mut sll: libc::sockaddr_ll = std::mem::zeroed();
            let mut len = std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t;
            let n = libc::recvfrom(
                fd,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                libc::MSG_TRUNC,
                &mut sll as *mut _ as *mut libc::sockaddr,
                &mut len,
            );
            if n <= 0 {
                continue;
            }
            if sll.sll_ifindex == lo {
                continue; // local traffic isn't network use
            }
            let outgoing = match sll.sll_pkttype {
                4 => true,             // PACKET_OUTGOING
                0 | 1 | 2 => false,    // to us, broadcast, multicast
                _ => continue,         // other hosts' packets (promiscuous)
            };
            let Some(key) = parse_packet(&buf[..(n as usize).min(buf.len())], outgoing) else { continue };
            let mut c = counts.lock().unwrap();
            let e = c.entry(key).or_insert((0, 0));
            if outgoing {
                e.1 += n as u64;
            } else {
                e.0 += n as u64;
            }
        }
    }
}

/// "0100007F:1F90" (or a 32-hex IPv6) -> address and port.
fn hex_addr(field: &str, v6: bool) -> Option<(IpAddr, u16)> {
    let (ip, port) = field.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    if !v6 {
        let n = u32::from_str_radix(ip, 16).ok()?;
        return Some((IpAddr::V4(Ipv4Addr::from(n.to_le_bytes())), port));
    }
    let mut bytes = [0u8; 16];
    for w in 0..4 {
        let word = u32::from_str_radix(ip.get(w * 8..w * 8 + 8)?, 16).ok()?;
        bytes[w * 4..w * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    let a = Ipv6Addr::from(bytes);
    Some((a.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(a)), port))
}

/// Host socket tables: (proto, local addr, port) -> socket inode.
fn socket_table() -> HashMap<Key, u64> {
    let mut map = HashMap::new();
    for (file, proto, v6) in [("tcp", 6u8, false), ("tcp6", 6, true), ("udp", 17, false), ("udp6", 17, true)] {
        let Ok(text) = fs::read_to_string(format!("/proc/net/{file}")) else { continue };
        for line in text.lines().skip(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 10 {
                continue;
            }
            let inode: u64 = f[9].parse().unwrap_or(0);
            if inode == 0 {
                continue;
            }
            if let Some((ip, port)) = hex_addr(f[1], v6) {
                map.entry((proto, ip, port)).or_insert(inode);
            }
        }
    }
    map
}

/// socket inode -> owning pid, from every process's fd table.
fn scan_inodes() -> HashMap<u64, i32> {
    let mut map = HashMap::new();
    let Ok(procs) = fs::read_dir("/proc") else { return map };
    for p in procs.flatten() {
        let Some(pid) = p.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else { continue };
        let Ok(fds) = fs::read_dir(p.path().join("fd")) else { continue };
        for fd in fds.flatten() {
            if let Ok(t) = fs::read_link(fd.path()) {
                let t = t.to_string_lossy();
                if let Some(inode) = t.strip_prefix("socket:[").and_then(|s| s.strip_suffix(']')) {
                    if let Ok(i) = inode.parse() {
                        map.entry(i).or_insert(pid);
                    }
                }
            }
        }
    }
    map
}

impl NetAccounting {
    pub fn start() -> NetAccounting {
        let counts: Counts = Arc::new(Mutex::new(HashMap::new()));
        let c = counts.clone();
        std::thread::Builder::new().name("treetop-capture".into()).spawn(move || capture(c)).ok();
        NetAccounting { counts, inode_pid: HashMap::new(), last_scan: None }
    }

    /// Bytes (rx, tx) per pid since the previous call.
    pub fn take(&mut self) -> HashMap<i32, (u64, u64)> {
        let counts: HashMap<Key, (u64, u64)> = std::mem::take(&mut *self.counts.lock().unwrap());
        let table = socket_table();
        // Rescan fd tables every 2 s, or sooner when traffic shows up on a
        // socket we haven't seen yet (a new connection).
        let unknown = table.values().any(|i| !self.inode_pid.contains_key(i));
        let age = self.last_scan.map(|t| t.elapsed()).unwrap_or(Duration::MAX);
        if age > Duration::from_secs(2) || (unknown && age > Duration::from_millis(500)) {
            self.inode_pid = scan_inodes();
            self.last_scan = Some(Instant::now());
        }
        let mut per_pid: HashMap<i32, (u64, u64)> = HashMap::new();
        for ((proto, ip, port), (rx, tx)) in counts {
            // Exact local address first, then a socket bound to any address.
            let any = if ip.is_ipv4() { IpAddr::V4(Ipv4Addr::UNSPECIFIED) } else { IpAddr::V6(Ipv6Addr::UNSPECIFIED) };
            let inode = table
                .get(&(proto, ip, port))
                .or_else(|| table.get(&(proto, any, port)))
                .or_else(|| table.get(&(proto, IpAddr::V6(Ipv6Addr::UNSPECIFIED), port)));
            let Some(pid) = inode.and_then(|i| self.inode_pid.get(i)) else { continue };
            let e = per_pid.entry(*pid).or_insert((0, 0));
            e.0 += rx;
            e.1 += tx;
        }
        per_pid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_tcp_outgoing_uses_source() {
        let mut p = [0u8; 40];
        p[0] = 0x45; // v4, ihl 5
        p[9] = 6; // TCP
        p[12..16].copy_from_slice(&[192, 168, 1, 2]);
        p[16..20].copy_from_slice(&[1, 1, 1, 1]);
        p[20..22].copy_from_slice(&50000u16.to_be_bytes());
        p[22..24].copy_from_slice(&443u16.to_be_bytes());
        assert_eq!(parse_packet(&p, true), Some((6, "192.168.1.2".parse().unwrap(), 50000)));
        assert_eq!(parse_packet(&p, false), Some((6, "1.1.1.1".parse().unwrap(), 443)));
    }

    #[test]
    fn ipv6_udp_incoming_uses_destination() {
        let mut p = [0u8; 48];
        p[0] = 0x60;
        p[6] = 17; // UDP
        let dst: Ipv6Addr = "2001:db8::5".parse().unwrap();
        p[24..40].copy_from_slice(&dst.octets());
        p[40..42].copy_from_slice(&443u16.to_be_bytes());
        p[42..44].copy_from_slice(&40000u16.to_be_bytes());
        assert_eq!(parse_packet(&p, false), Some((17, IpAddr::V6(dst), 40000)));
    }

    #[test]
    fn proc_net_addresses() {
        assert_eq!(hex_addr("0100007F:1F90", false), Some(("127.0.0.1".parse().unwrap(), 8080)));
        // IPv4-mapped IPv6 collapses to IPv4, so it matches IPv4 packets.
        assert_eq!(hex_addr("0000000000000000FFFF00000100007F:0050", true), Some(("127.0.0.1".parse().unwrap(), 80)));
    }

    #[test]
    fn own_sockets_are_found() {
        // The socket table and fd scan work for our own process unprivileged.
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let table = socket_table();
        let inode = table.get(&(6, "127.0.0.1".parse().unwrap(), port)).copied().expect("listener in /proc/net/tcp");
        assert_eq!(scan_inodes().get(&inode), Some(&(std::process::id() as i32)));
    }
}
