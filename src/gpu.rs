//! Per-process GPU usage from DRM fdinfo (what amdgpu_top / nvtop read):
//! engine busy time per client, plus VRAM/GTT. Runs on its own thread because
//! walking every process's fd table is the expensive part; known GPU users
//! are re-read every tick, the whole process table every few seconds.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[allow(dead_code)]
pub const ENGINES: [&str; 5] = ["gfx", "compute", "dma", "dec", "enc"];

#[derive(Clone, Default, Debug)]
pub struct GpuUse {
    /// PCI address of the GPU (drm-pdev), e.g. "0000:03:00.0".
    pub pdev: String,
    pub vram: u64,
    pub gtt: u64,
    /// Busy % per engine, ENGINES order.
    pub engines: [f64; 5],
}

impl GpuUse {
    pub fn busiest(&self) -> f64 {
        self.engines.iter().cloned().fold(0.0, f64::max)
    }
}

pub type Shared = Arc<Mutex<HashMap<i32, Vec<GpuUse>>>>;

#[derive(Default)]
struct Client {
    pdev: String,
    vram: u64,
    gtt: u64,
    ns: [u64; 5],
}

fn parse_size(v: &str) -> u64 {
    let mut it = v.split_whitespace();
    let n: u64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    match it.next() {
        Some("KiB") => n * 1024,
        Some("MiB") => n * 1024 * 1024,
        Some("GiB") => n * 1024 * 1024 * 1024,
        _ => n,
    }
}

/// Reads every DRM client a process holds, keyed by (pdev, client-id) so
/// several fds on one client are counted once.
fn read_clients(pid: i32) -> Option<HashMap<(String, u64), Client>> {
    let dir = fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    let mut out: HashMap<(String, u64), Client> = HashMap::new();
    for e in dir.flatten() {
        let Ok(target) = fs::read_link(e.path()) else { continue };
        if !target.as_os_str().to_string_lossy().starts_with("/dev/dri/") {
            continue;
        }
        let fd = e.file_name();
        let Ok(info) = fs::read_to_string(format!("/proc/{pid}/fdinfo/{}", fd.to_string_lossy())) else { continue };
        let mut c = Client::default();
        let mut id = None;
        let (mut total_vram, mut req_vram, mut mem_vram, mut gtt) = (None, None, None, None);
        for line in info.lines() {
            let Some((k, v)) = line.split_once(':') else { continue };
            let v = v.trim();
            match k {
                "drm-client-id" => id = v.parse::<u64>().ok(),
                "drm-pdev" => c.pdev = v.to_string(),
                "amd-requested-vram" => req_vram = Some(parse_size(v)),
                "drm-total-vram" => total_vram = Some(parse_size(v)),
                "drm-memory-vram" => mem_vram = Some(parse_size(v)),
                "drm-memory-gtt" | "drm-total-gtt" if gtt.is_none() => gtt = Some(parse_size(v)),
                _ => {
                    if let Some(engine) = k.strip_prefix("drm-engine-") {
                        let ns: u64 = v.split_whitespace().next().and_then(|x| x.parse().ok()).unwrap_or(0);
                        let slot = match engine {
                            "gfx" | "render" | "rcs" => Some(0),
                            "compute" | "ccs" => Some(1),
                            "dma" | "copy" | "bcs" => Some(2),
                            "dec" | "video" | "vcs" => Some(3),
                            "enc" | "video-enhance" | "vecs" => Some(4),
                            _ => None,
                        };
                        if let Some(i) = slot {
                            c.ns[i] += ns;
                        }
                    }
                }
            }
        }
        let Some(id) = id else { continue };
        // amdgpu's "requested" is what this client allocated itself; the
        // resident figures also count buffers imported from others.
        c.vram = req_vram.or(total_vram).or(mem_vram).unwrap_or(0);
        c.gtt = gtt.unwrap_or(0);
        out.insert((c.pdev.clone(), id), c);
    }
    Some(out)
}

pub fn spawn_poller() -> Shared {
    let shared: Shared = Arc::new(Mutex::new(HashMap::new()));
    let out = shared.clone();
    std::thread::spawn(move || {
        let mut prev: HashMap<(i32, String, u64), ([u64; 5], Instant)> = HashMap::new();
        let mut users: HashSet<i32> = HashSet::new();
        let mut tick = 0u64;
        loop {
            let full = tick % 5 == 0;
            tick += 1;
            let pids: Vec<i32> = if full {
                fs::read_dir("/proc")
                    .map(|d| d.flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).collect())
                    .unwrap_or_default()
            } else {
                users.iter().copied().collect()
            };
            let now = Instant::now();
            let mut result: HashMap<i32, Vec<GpuUse>> = HashMap::new();
            let mut seen = HashSet::new();
            let mut new_users = HashSet::new();
            for pid in pids {
                let Some(clients) = read_clients(pid) else { continue };
                if clients.is_empty() {
                    continue;
                }
                new_users.insert(pid);
                let mut per_gpu: HashMap<String, GpuUse> = HashMap::new();
                for ((pdev, id), c) in clients {
                    let key = (pid, pdev.clone(), id);
                    let entry = per_gpu.entry(pdev.clone()).or_insert_with(|| GpuUse { pdev: pdev.clone(), ..Default::default() });
                    entry.vram += c.vram;
                    entry.gtt += c.gtt;
                    if let Some((old, at)) = prev.get(&key) {
                        let dt = now.duration_since(*at).as_secs_f64().max(0.05) * 1e9;
                        for i in 0..5 {
                            entry.engines[i] = (entry.engines[i] + c.ns[i].saturating_sub(old[i]) as f64 / dt * 100.0).min(100.0);
                        }
                    }
                    prev.insert(key.clone(), (c.ns, now));
                    seen.insert(key);
                }
                result.insert(pid, per_gpu.into_values().collect());
            }
            if full {
                users = new_users;
                prev.retain(|k, _| seen.contains(k));
            } else {
                // Keep entries for users found on the last full scan that
                // weren't re-read (exited); drop the dead ones.
                users.retain(|p| result.contains_key(p));
            }
            *out.lock().unwrap() = result;
            std::thread::sleep(Duration::from_millis(1000));
        }
    });
    shared
}
