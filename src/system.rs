//! Whole-machine readings: CPU (per core), memory, GPUs, disks, filesystems
//! and network, sampled straight from /proc and /sys.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn read(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn read_u64(path: impl AsRef<Path>) -> Option<u64> {
    read(path)?.trim().parse().ok()
}

#[derive(Clone, Copy, Default)]
struct CpuTimes {
    user: u64,
    nice: u64,
    system: u64,
    idle: u64,
    iowait: u64,
    irq: u64,
    softirq: u64,
    steal: u64,
}

impl CpuTimes {
    fn total(&self) -> u64 {
        self.user + self.nice + self.system + self.idle + self.iowait + self.irq + self.softirq + self.steal
    }
}

/// Percentages of one CPU over the last interval: user, nice, system, other
/// (irq/softirq/steal/iowait), as htop's meter colours them.
#[derive(Clone, Copy, Default)]
pub struct CpuShare {
    pub user: f64,
    pub nice: f64,
    pub system: f64,
    pub other: f64,
}

impl CpuShare {
    pub fn busy(&self) -> f64 {
        self.user + self.nice + self.system + self.other
    }
}

fn share(prev: &CpuTimes, cur: &CpuTimes) -> CpuShare {
    let dt = cur.total().saturating_sub(prev.total()) as f64;
    if dt <= 0.0 {
        return CpuShare::default();
    }
    let d = |a: u64, b: u64| a.saturating_sub(b) as f64 / dt * 100.0;
    CpuShare {
        user: d(cur.user, prev.user),
        nice: d(cur.nice, prev.nice),
        system: d(cur.system, prev.system),
        other: d(cur.irq + cur.softirq + cur.steal + cur.iowait, prev.irq + prev.softirq + prev.steal + prev.iowait),
    }
}

fn parse_cpu_line(line: &str) -> CpuTimes {
    let v: Vec<u64> = line.split_whitespace().skip(1).map(|x| x.parse().unwrap_or(0)).collect();
    let g = |i: usize| v.get(i).copied().unwrap_or(0);
    CpuTimes {
        user: g(0),
        nice: g(1),
        system: g(2),
        idle: g(3),
        iowait: g(4),
        irq: g(5),
        softirq: g(6),
        steal: g(7),
    }
}

#[derive(Default)]
pub struct Gpu {
    pub name: String,
    pub pdev: String,
    pub busy: f64,
    pub mem_busy: Option<f64>,
    pub vram_used: u64,
    pub vram_total: u64,
    pub gtt_used: u64,
    pub gtt_total: u64,
    pub temp_c: Option<f64>,
    pub temp_junction: Option<f64>,
    pub temp_mem: Option<f64>,
    pub power_w: Option<f64>,
    pub power_cap_w: Option<f64>,
    pub sclk_mhz: Option<f64>,
    pub mclk_mhz: Option<f64>,
    pub voltage_mv: Option<f64>,
    pub fan_rpm: Option<f64>,
    pub fan_max_rpm: Option<f64>,
    pub fan_pct: Option<f64>,
    pub link: String,
    pub perf_level: String,
    /// Drives a connected display (the primary GPU).
    pub display: bool,
}

struct AmdCard {
    /// DRM name ("card1"), for finding its display connectors.
    drm: String,
    device: PathBuf,
    hwmon: Option<PathBuf>,
    name: String,
    pdev: String,
    vram: u64,
}

/// Marketing name from lspci (pci.ids), once at startup.
fn lspci_name(pdev: &str) -> Option<String> {
    let out = std::process::Command::new("lspci").args(["-mm", "-s", pdev]).output().ok()?;
    let line = String::from_utf8_lossy(&out.stdout).to_string();
    // -mm fields are quoted: slot "class" "vendor" "device" ...
    let fields: Vec<&str> = line.split('"').filter(|f| !f.trim().is_empty()).collect();
    let dev = fields.get(3)?.to_string();
    // "Navi 31 [Radeon RX 7900 XT/7900 XTX/...]" -> prefer the bracketed part.
    let pretty = dev.split_once('[').and_then(|(_, r)| r.split(']').next()).map(|s| s.to_string()).unwrap_or(dev);
    Some(pretty)
}

fn amd_cards() -> Vec<AmdCard> {
    let mut cards = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/drm") else { return cards };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("card") && n[4..].chars().all(|c| c.is_ascii_digit()))
        .collect();
    names.sort();
    for n in names {
        let device = PathBuf::from("/sys/class/drm").join(&n).join("device");
        if !device.join("gpu_busy_percent").exists() {
            continue;
        }
        let hwmon = fs::read_dir(device.join("hwmon"))
            .ok()
            .and_then(|mut d| d.next())
            .and_then(|e| e.ok())
            .map(|e| e.path());
        let vram = read_u64(device.join("mem_info_vram_total")).unwrap_or(0);
        let pci = fs::read_link(&device)
            .ok()
            .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let short = pci.splitn(2, ':').nth(1).unwrap_or(&pci).to_string();
        let kind = if vram >= 2 << 30 { "AMD dGPU" } else { "AMD iGPU" };
        let name = read(device.join("product_name"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| lspci_name(&pci))
            .map(|n| if vram >= 2 << 30 { n } else { format!("{n} (integrated)") })
            .unwrap_or_else(|| format!("{kind} {short}"));
        cards.push(AmdCard { drm: n.clone(), device, hwmon, name, pdev: pci, vram });
    }
    // The card driving the displays first (the primary GPU), then by VRAM.
    cards.sort_by(|a, b| drives_display(&b.drm).cmp(&drives_display(&a.drm)).then(b.vram.cmp(&a.vram)));
    cards
}

/// Whether a DRM card has a connected, enabled display connector.
fn drives_display(drm: &str) -> bool {
    let prefix = format!("{drm}-");
    fs::read_dir("/sys/class/drm")
        .map(|d| {
            d.flatten().any(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.starts_with(&prefix)
                    && read(e.path().join("status")).map(|s| s.trim() == "connected").unwrap_or(false)
                    && read(e.path().join("enabled")).map(|s| s.trim() != "disabled").unwrap_or(true)
            })
        })
        .unwrap_or(false)
}

fn amd_sample(card: &AmdCard) -> Gpu {
    let d = &card.device;
    let hw = |f: &str| card.hwmon.as_ref().and_then(|h| read_u64(h.join(f)));
    // hwmon numbers temps by label (edge/junction/mem), not fixed slots.
    let temp = |label: &str| {
        (1..=4).find_map(|i| {
            let l = card.hwmon.as_ref().and_then(|h| read(h.join(format!("temp{i}_label"))))?;
            (l.trim() == label).then(|| hw(&format!("temp{i}_input")).map(|m| m as f64 / 1000.0)).flatten()
        })
    };
    let freq = |label: &str| {
        (1..=3).find_map(|i| {
            let l = card.hwmon.as_ref().and_then(|h| read(h.join(format!("freq{i}_label"))))?;
            (l.trim() == label).then(|| hw(&format!("freq{i}_input")).map(|hz| hz as f64 / 1e6)).flatten()
        })
    };
    let link = match (read(d.join("current_link_speed")), read(d.join("current_link_width"))) {
        (Some(s), Some(w)) => format!("PCIe {} x{}", s.trim().trim_end_matches(" PCIe"), w.trim()),
        _ => String::new(),
    };
    let pwm = hw("pwm1").zip(hw("pwm1_max").or(Some(255))).map(|(p, m)| p as f64 / m.max(1) as f64 * 100.0);
    Gpu {
        name: card.name.clone(),
        pdev: card.pdev.clone(),
        busy: read_u64(d.join("gpu_busy_percent")).unwrap_or(0) as f64,
        mem_busy: read_u64(d.join("mem_busy_percent")).map(|v| v as f64),
        vram_used: read_u64(d.join("mem_info_vram_used")).unwrap_or(0),
        vram_total: read_u64(d.join("mem_info_vram_total")).unwrap_or(0),
        gtt_used: read_u64(d.join("mem_info_gtt_used")).unwrap_or(0),
        gtt_total: read_u64(d.join("mem_info_gtt_total")).unwrap_or(0),
        temp_c: temp("edge").or_else(|| hw("temp1_input").map(|m| m as f64 / 1000.0)),
        temp_junction: temp("junction"),
        temp_mem: temp("mem"),
        power_w: hw("power1_average").or_else(|| hw("power1_input")).map(|u| u as f64 / 1e6),
        power_cap_w: hw("power1_cap").map(|u| u as f64 / 1e6),
        sclk_mhz: freq("sclk"),
        mclk_mhz: freq("mclk"),
        voltage_mv: hw("in0_input").map(|v| v as f64),
        fan_rpm: hw("fan1_input").map(|v| v as f64),
        fan_max_rpm: hw("fan1_max").map(|v| v as f64),
        fan_pct: pwm,
        link,
        perf_level: read(d.join("power_dpm_force_performance_level")).map(|s| s.trim().to_string()).unwrap_or_default(),
        display: drives_display(&card.drm),
    }
}

/// NVIDIA has no sysfs counters, so nvidia-smi is polled off the UI thread.
fn spawn_nvidia_poller() -> Option<Arc<Mutex<Vec<Gpu>>>> {
    let smi = ["/run/current-system/sw/bin/nvidia-smi", "/usr/bin/nvidia-smi"]
        .into_iter()
        .find(|p| Path::new(p).exists())?;
    let out = Arc::new(Mutex::new(Vec::new()));
    let shared = out.clone();
    std::thread::spawn(move || loop {
        let res = std::process::Command::new(smi)
            .args([
                "--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw",
                "--format=csv,noheader,nounits",
            ])
            .output();
        if let Ok(o) = res {
            let gpus = String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| {
                    let f: Vec<&str> = l.split(',').map(str::trim).collect();
                    let n = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok());
                    Gpu {
                        name: f.first().unwrap_or(&"NVIDIA").to_string(),
                        busy: n(1).unwrap_or(0.0),
                        vram_used: (n(2).unwrap_or(0.0) * 1048576.0) as u64,
                        vram_total: (n(3).unwrap_or(0.0) * 1048576.0) as u64,
                        temp_c: n(4),
                        power_w: n(5),
                        ..Default::default()
                    }
                })
                .collect();
            *shared.lock().unwrap() = gpus;
        }
        std::thread::sleep(std::time::Duration::from_millis(1500));
    });
    Some(out)
}

#[derive(Default, Clone, Copy)]
struct Counter {
    a: u64,
    b: u64,
}

pub struct System {
    prev_cpus: Vec<CpuTimes>,
    prev_total: CpuTimes,
    pub cores: Vec<CpuShare>,
    pub total: CpuShare,
    amd: Vec<AmdCard>,
    nvidia: Option<Arc<Mutex<Vec<Gpu>>>>,
    prev_disk: HashMap<String, Counter>,
    prev_net: HashMap<String, Counter>,
    last: Option<Instant>,
    pub interval: f64,
    pub snapshot: Value,
    pub mem_total: u64,
}

impl System {
    pub fn new() -> Self {
        System {
            prev_cpus: Vec::new(),
            prev_total: CpuTimes::default(),
            cores: Vec::new(),
            total: CpuShare::default(),
            amd: amd_cards(),
            nvidia: spawn_nvidia_poller(),
            prev_disk: HashMap::new(),
            prev_net: HashMap::new(),
            last: None,
            interval: 1.0,
            snapshot: Value::Null,
            mem_total: 1,
        }
    }

    pub fn sample(&mut self) {
        let now = Instant::now();
        let dt = self.last.map(|l| now.duration_since(l).as_secs_f64()).unwrap_or(1.0).max(0.05);
        self.interval = dt;
        self.last = Some(now);

        // CPU
        let stat = read("/proc/stat").unwrap_or_default();
        let mut cpus = Vec::new();
        let mut total = CpuTimes::default();
        let (mut ctxt, mut procs_running) = (0u64, 0u64);
        for line in stat.lines() {
            if line.starts_with("cpu ") {
                total = parse_cpu_line(line);
            } else if line.starts_with("cpu") {
                cpus.push(parse_cpu_line(line));
            } else if let Some(v) = line.strip_prefix("ctxt ") {
                ctxt = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("procs_running ") {
                procs_running = v.trim().parse().unwrap_or(0);
            }
        }
        let _ = ctxt;
        if self.prev_cpus.len() == cpus.len() {
            self.cores = cpus.iter().zip(&self.prev_cpus).map(|(c, p)| share(p, c)).collect();
            self.total = share(&self.prev_total, &total);
        } else {
            self.cores = vec![CpuShare::default(); cpus.len()];
        }
        self.prev_cpus = cpus;
        self.prev_total = total;

        let freqs: Vec<f64> = (0..self.cores.len())
            .map(|i| {
                read_u64(format!("/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq")).unwrap_or(0) as f64 / 1000.0
            })
            .collect();
        let cpu_temp = cpu_temperature();

        // Memory
        let meminfo = read("/proc/meminfo").unwrap_or_default();
        let mut mem: HashMap<&str, u64> = HashMap::new();
        for line in meminfo.lines() {
            let mut it = line.split_whitespace();
            if let (Some(k), Some(v)) = (it.next(), it.next()) {
                mem.insert(k.trim_end_matches(':'), v.parse::<u64>().unwrap_or(0) * 1024);
            }
        }
        let m = |k: &str| mem.get(k).copied().unwrap_or(0);
        let mem_total = m("MemTotal").max(1);
        self.mem_total = mem_total;
        let cached = m("Cached") + m("SReclaimable") - m("Shmem").min(m("Cached") + m("SReclaimable"));
        let buffers = m("Buffers");
        let used = mem_total.saturating_sub(m("MemFree") + cached + buffers);
        let swap_total = m("SwapTotal");
        let swap_used = swap_total.saturating_sub(m("SwapFree"));

        // GPUs
        let mut gpus: Vec<Gpu> = self.amd.iter().map(amd_sample).collect();
        if let Some(n) = &self.nvidia {
            gpus.extend(n.lock().unwrap().iter().cloned());
        }

        // Disks (whole devices only)
        let mut disks = Vec::new();
        let (mut rd_total, mut wr_total) = (0.0, 0.0);
        for line in read("/proc/diskstats").unwrap_or_default().lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 10 {
                continue;
            }
            let name = f[2];
            if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("zram") || name.starts_with("dm-") {
                continue;
            }
            if !Path::new("/sys/block").join(name).exists() {
                continue;
            }
            let cur = Counter { a: f[5].parse::<u64>().unwrap_or(0) * 512, b: f[9].parse::<u64>().unwrap_or(0) * 512 };
            let prev = self.prev_disk.insert(name.to_string(), cur).unwrap_or(cur);
            let r = cur.a.saturating_sub(prev.a) as f64 / dt;
            let w = cur.b.saturating_sub(prev.b) as f64 / dt;
            rd_total += r;
            wr_total += w;
            disks.push(json!({"name": name, "read": r, "write": w}));
        }

        // Filesystems
        let mut seen = std::collections::HashSet::new();
        let mut filesystems = Vec::new();
        for line in read("/proc/mounts").unwrap_or_default().lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 3 {
                continue;
            }
            let (dev, mnt, fstype) = (f[0], f[1].replace("\\040", " "), f[2]);
            if !matches!(fstype, "ext4" | "ext3" | "btrfs" | "xfs" | "vfat" | "exfat" | "f2fs" | "zfs" | "bcachefs" | "ntfs3" | "ntfs" | "fuseblk") {
                continue;
            }
            if mnt.starts_with("/nix/store") || mnt.starts_with("/var/lib/docker") || !seen.insert(dev.to_string()) {
                continue;
            }
            if let Some((used, total)) = statvfs(&mnt) {
                if total > 0 {
                    filesystems.push(json!({"mount": mnt, "device": dev, "type": fstype, "used": used, "total": total}));
                }
            }
        }

        // Network (physical interfaces count toward the totals)
        let mut ifaces = Vec::new();
        let (mut rx_total, mut tx_total) = (0.0, 0.0);
        for line in read("/proc/net/dev").unwrap_or_default().lines().skip(2) {
            let Some((name, rest)) = line.split_once(':') else { continue };
            let name = name.trim();
            if name == "lo" {
                continue;
            }
            let f: Vec<u64> = rest.split_whitespace().map(|x| x.parse().unwrap_or(0)).collect();
            if f.len() < 9 {
                continue;
            }
            let cur = Counter { a: f[0], b: f[8] };
            let prev = self.prev_net.insert(name.to_string(), cur).unwrap_or(cur);
            let rx = cur.a.saturating_sub(prev.a) as f64 / dt;
            let tx = cur.b.saturating_sub(prev.b) as f64 / dt;
            let physical = Path::new("/sys/class/net").join(name).join("device").exists()
                || name.starts_with("wg")
                || name.starts_with("tailscale");
            let up = read(format!("/sys/class/net/{name}/operstate")).map(|s| s.trim() == "up").unwrap_or(false);
            if physical {
                rx_total += rx;
                tx_total += tx;
            }
            if physical || rx > 0.0 || tx > 0.0 {
                ifaces.push(json!({"name": name, "rx": rx, "tx": tx, "physical": physical, "up": up}));
            }
        }

        let load: Vec<f64> = read("/proc/loadavg")
            .unwrap_or_default()
            .split_whitespace()
            .take(3)
            .map(|x| x.parse().unwrap_or(0.0))
            .collect();
        let uptime = read("/proc/uptime")
            .and_then(|s| s.split_whitespace().next().and_then(|x| x.parse::<f64>().ok()))
            .unwrap_or(0.0);

        self.snapshot = json!({
            "cpu": {
                "total": self.total.busy(),
                "user": self.total.user, "nice": self.total.nice, "system": self.total.system, "other": self.total.other,
                "cores": self.cores.iter().zip(&freqs).map(|(c, f)| json!([c.user, c.nice, c.system, c.other, f])).collect::<Vec<_>>(),
                "temp": cpu_temp,
                "running": procs_running,
            },
            "mem": {
                "total": mem_total, "used": used, "buffers": buffers, "cached": cached,
                "available": m("MemAvailable"),
                "swapTotal": swap_total, "swapUsed": swap_used,
            },
            "gpus": gpus.iter().map(|g| json!({
                "name": g.name, "pdev": g.pdev, "busy": g.busy, "memBusy": g.mem_busy,
                "vramUsed": g.vram_used, "vramTotal": g.vram_total, "gttUsed": g.gtt_used, "gttTotal": g.gtt_total,
                "temp": g.temp_c, "tempJunction": g.temp_junction, "tempMem": g.temp_mem,
                "power": g.power_w, "powerCap": g.power_cap_w, "sclk": g.sclk_mhz, "mclk": g.mclk_mhz,
                "voltage": g.voltage_mv, "fanRpm": g.fan_rpm, "fanMaxRpm": g.fan_max_rpm, "fanPct": g.fan_pct,
                "link": g.link, "perfLevel": g.perf_level, "display": g.display,
                "engines": [0.0, 0.0, 0.0, 0.0, 0.0],
            })).collect::<Vec<_>>(),
            "disk": {"read": rd_total, "write": wr_total, "devices": disks, "filesystems": filesystems},
            "net": {"rx": rx_total, "tx": tx_total, "interfaces": ifaces},
            "load": load,
            "uptime": uptime,
            "sensors": sensors(&self.amd.iter().map(|c| c.pdev.clone()).collect::<Vec<_>>()),
        });
    }
}

impl Clone for Gpu {
    fn clone(&self) -> Self {
        Gpu { name: self.name.clone(), pdev: self.pdev.clone(), link: self.link.clone(), perf_level: self.perf_level.clone(), ..*self }
    }
}

/// Every hwmon temperature and fan, with a friendly device name:
/// [{dev, label, kind: "temp"|"fan", value}] (°C or rpm).
fn sensors(gpu_order: &[String]) -> Vec<Value> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/hwmon") else { return out };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    dirs.sort();
    let (mut ram, mut nvme_n) = (0, 0);
    for h in dirs {
        let chip = read(h.join("name")).map(|s| s.trim().to_string()).unwrap_or_default();
        let dev = match chip.as_str() {
            "k10temp" | "coretemp" | "zenpower" | "cpu_thermal" => "CPU".to_string(),
            "amdgpu" | "nouveau" | "radeon" => {
                let pci = fs::canonicalize(h.join("device")).ok()
                    .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
                    .unwrap_or_default();
                match gpu_order.iter().position(|g| *g == pci) {
                    Some(i) => format!("GPU{}", i + 1),
                    None => "GPU".to_string(),
                }
            }
            "nvme" => {
                nvme_n += 1;
                read(h.join("device/model")).map(|m| m.trim().to_string()).filter(|m| !m.is_empty())
                    .unwrap_or_else(|| format!("NVMe {nvme_n}"))
            }
            "spd5118" | "jc42" => {
                ram += 1;
                format!("RAM {ram}")
            }
            c if c.starts_with("iwlwifi") || c.starts_with("mt7") || c.starts_with("ath") => "Wi-Fi".to_string(),
            c if c.starts_with("steam") => "Steam controller".to_string(),
            c => c.to_string(),
        };
        let Ok(files) = fs::read_dir(&h) else { continue };
        let mut names: Vec<String> = files.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort_by_key(|n| (n.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse::<u32>().unwrap_or(0), n.clone()));
        for n in names {
            let (kind, base) = if let Some(b) = n.strip_suffix("_input").filter(|b| b.starts_with("temp")) {
                ("temp", b.to_string())
            } else if let Some(b) = n.strip_suffix("_input").filter(|b| b.starts_with("fan")) {
                ("fan", b.to_string())
            } else {
                continue;
            };
            let Some(raw) = read_u64(h.join(&n)) else { continue };
            let label = read(h.join(format!("{base}_label"))).map(|l| l.trim().to_string()).unwrap_or(base.clone());
            // Generic "temp1"/"fan1" say nothing beyond the kind.
            let label = if label.trim_start_matches(kind).chars().all(|c| c.is_ascii_digit()) { kind.to_string() } else { label };
            let value = if kind == "temp" { raw as f64 / 1000.0 } else { raw as f64 };
            if kind == "temp" && (value <= 0.0 || value > 200.0) {
                continue;
            }
            out.push(json!({"dev": dev, "label": label, "kind": kind, "value": value}));
        }
    }
    // CPU, GPUs, RAM, drives, Wi-Fi, then anything else (stable within).
    let rank = |d: &str| {
        if d == "CPU" { 0 } else if d.starts_with("GPU") { 1 } else if d.starts_with("RAM") { 2 }
        else if d.contains("SSD") || d.starts_with("NVMe") || d.contains("GP-") { 3 } else if d == "Wi-Fi" { 4 } else { 5 }
    };
    out.sort_by(|a, b| {
        let (da, db) = (a["dev"].as_str().unwrap_or(""), b["dev"].as_str().unwrap_or(""));
        rank(da).cmp(&rank(db)).then(da.cmp(db))
    });
    out
}

fn cpu_temperature() -> Option<f64> {
    let entries = fs::read_dir("/sys/class/hwmon").ok()?;
    for e in entries.flatten() {
        let p = e.path();
        let name = read(p.join("name")).unwrap_or_default();
        if matches!(name.trim(), "k10temp" | "coretemp" | "zenpower" | "cpu_thermal") {
            if let Some(v) = read_u64(p.join("temp1_input")) {
                return Some(v as f64 / 1000.0);
            }
        }
    }
    None
}

fn statvfs(path: &str) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(path).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let bs = s.f_frsize as u64;
    let total = s.f_blocks as u64 * bs;
    let free = s.f_bfree as u64 * bs;
    Some((total.saturating_sub(free), total))
}
