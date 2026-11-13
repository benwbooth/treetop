//! Process table: samples /proc, computes htop-style CPU%/MEM%, and lays the
//! processes out as rows for a tree, a flat list, or grouped by container.

use crate::containers::{self, Names};
use crate::gpu;
use crate::root::RootData;
use std::cell::RefCell;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;

#[derive(Clone)]
struct Static {
    start: u64,
    uid: u32,
    cmd: String,
    /// Full path of the executable ("" if unknown; "?"-suffixed if guessed).
    exe: String,
    container: Option<(String, &'static str)>,
}

/// The executable's full path: /proc/<pid>/exe when readable (our own
/// processes, or as root); otherwise argv[0] resolved through PATH, marked
/// with a trailing "?" since it's a guess.
fn resolve_exe(base: &std::path::Path, argv0: &str) -> String {
    if let Ok(p) = fs::read_link(base.join("exe")) {
        return p.display().to_string();
    }
    if argv0.is_empty() {
        return String::new();
    }
    if argv0.starts_with('/') {
        return argv0.to_string();
    }
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .map(|d| std::path::Path::new(d).join(argv0))
        .find(|p| p.is_file())
        .and_then(|p| fs::canonicalize(p).ok())
        .map(|p| format!("{}?", p.display()))
        .unwrap_or_default()
}

#[derive(Clone)]
pub struct Proc {
    pub pid: i32,
    pub ppid: i32,
    pub uid: u32,
    pub name: String,
    pub cmd: String,
    pub exe: String,
    pub state: char,
    pub pri: i64,
    pub ni: i64,
    pub threads: u64,
    pub virt: u64,
    pub res: u64,
    pub shr: u64,
    pub ticks: u64,
    pub cpu: f64,
    pub mem: f64,
    pub kernel: bool,
    pub container: Option<String>,
    pub engine: &'static str,
    /// Smoothed CPU% used for ordering, so rows don't jitter.
    pub cpu_avg: f64,
    pub gpu: f64,
    pub vram: u64,
    pub gtt: u64,
    pub gpu_engines: [f64; 5],
    pub gpus: String,
    /// Disk read/write in bytes/s from /proc/<pid>/io; -1 when unreadable
    /// (other users' processes without root).
    pub io_r: f64,
    pub io_w: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Tree,
    Flat,
    Containers,
    Gpu,
}

pub struct Processes {
    statics: HashMap<i32, Static>,
    prev_ticks: HashMap<i32, u64>,
    prev_io: HashMap<i32, (u64, u64)>,
    /// Container network: previous (rx, tx) per container and the current rates.
    prev_cnet: HashMap<String, (u64, u64)>,
    pub cnet: HashMap<String, (f64, f64)>,
    users: HashMap<u32, String>,
    clk_tck: f64,
    page: u64,
    pub procs: HashMap<i32, Proc>,
    pub view: View,
    pub sort_key: String,
    pub sort_desc: bool,
    pub filter: String,
    pub show_kernel: bool,
    pub collapsed: HashSet<String>,
    pub names: Names,
    pub gpu: gpu::Shared,
    /// PCI addresses of the GPUs in display order (GPU1, GPU2, ...).
    pub gpu_order: Vec<String>,
    /// While set (pointer over the list), sibling order is kept exactly.
    pub frozen: bool,
    /// Focus mode: only this process and its descendants are shown.
    pub focus: Option<i32>,
    /// Readings from the polkit root helper, when root mode is on.
    pub root: Option<RootData>,
    prev_rank: RefCell<HashMap<i32, usize>>,
}

fn load_users() -> HashMap<u32, String> {
    let mut m = HashMap::new();
    for line in fs::read_to_string("/etc/passwd").unwrap_or_default().lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() > 2 {
            if let Ok(uid) = f[2].parse() {
                m.insert(uid, f[0].to_string());
            }
        }
    }
    m
}

impl Processes {
    pub fn new() -> Self {
        Processes {
            statics: HashMap::new(),
            prev_ticks: HashMap::new(),
            prev_io: HashMap::new(),
            prev_cnet: HashMap::new(),
            cnet: HashMap::new(),
            users: load_users(),
            clk_tck: unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64,
            page: unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64,
            procs: HashMap::new(),
            view: View::Tree,
            sort_key: "cpu".into(),
            sort_desc: true,
            filter: String::new(),
            show_kernel: false,
            collapsed: HashSet::new(),
            names: containers::spawn_poller(),
            gpu: gpu::spawn_poller(),
            gpu_order: Vec::new(),
            frozen: false,
            focus: None,
            root: None,
            prev_rank: RefCell::new(HashMap::new()),
        }
    }

    pub fn user(&self, uid: u32) -> String {
        self.users.get(&uid).cloned().unwrap_or_else(|| uid.to_string())
    }

    pub fn sample(&mut self, interval: f64, mem_total: u64) {
        let mut procs = HashMap::new();
        let mut ticks_now = HashMap::new();
        let mut io_now = HashMap::new();
        let mut cnet_pid: HashMap<String, i32> = HashMap::new();
        let Ok(dir) = fs::read_dir("/proc") else { return };
        for e in dir.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else { continue };
            let base = e.path();
            let Ok(stat) = fs::read_to_string(base.join("stat")) else { continue };
            let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else { continue };
            let name = stat[open + 1..close].to_string();
            let f: Vec<&str> = stat[close + 2..].split_whitespace().collect();
            if f.len() < 22 {
                continue;
            }
            let n = |i: usize| f[i].parse::<i64>().unwrap_or(0);
            let ppid = n(1) as i32;
            let start = n(19) as u64;
            let ticks = (n(11) + n(12)) as u64;
            let kernel = pid == 2 || ppid == 2;

            let st = match self.statics.get_mut(&pid) {
                Some(s) if s.start == start => {
                    // Root mode can now replace a guessed executable path.
                    if s.exe.ends_with('?') {
                        if let Some(real) = self.root.as_ref().and_then(|r| r.exe.get(&pid)) {
                            s.exe = real.clone();
                        }
                    }
                    s.clone()
                }
                _ => {
                    let uid = fs::metadata(&base).map(|m| m.uid()).unwrap_or(0);
                    let raw = fs::read(base.join("cmdline")).unwrap_or_default();
                    let cmd = String::from_utf8_lossy(&raw).replace('\0', " ").trim().to_string();
                    let container = if kernel {
                        None
                    } else {
                        fs::read_to_string(base.join("cgroup")).ok().and_then(|c| containers::container_id(&c))
                    };
                    let exe = if kernel {
                        String::new()
                    } else {
                        let argv0 = raw.split(|b| *b == 0).next().map(|a| String::from_utf8_lossy(a).into_owned()).unwrap_or_default();
                        match self.root.as_ref().and_then(|r| r.exe.get(&pid)) {
                            Some(real) => real.clone(),
                            None => resolve_exe(&base, &argv0),
                        }
                    };
                    let s = Static { start, uid, cmd, exe, container };
                    self.statics.insert(pid, s.clone());
                    s
                }
            };

            let statm = fs::read_to_string(base.join("statm")).unwrap_or_default();
            let sm: Vec<u64> = statm.split_whitespace().map(|x| x.parse().unwrap_or(0)).collect();
            let res = sm.get(1).copied().unwrap_or(0) * self.page;
            let shr = sm.get(2).copied().unwrap_or(0) * self.page;

            // Disk I/O (storage-layer bytes, so page-cache hits don't count).
            let (io_r, io_w) = match fs::read_to_string(base.join("io")) {
                Ok(io) => {
                    let field = |k: &str| io.lines().find_map(|l| l.strip_prefix(k)?.trim().parse::<u64>().ok()).unwrap_or(0);
                    let cur = (field("read_bytes:"), field("write_bytes:"));
                    let old = self.prev_io.get(&pid).copied().unwrap_or(cur);
                    io_now.insert(pid, cur);
                    (cur.0.saturating_sub(old.0) as f64 / interval, cur.1.saturating_sub(old.1) as f64 / interval)
                }
                Err(_) => match self.root.as_ref().and_then(|r| r.io.get(&pid)).copied() {
                    // Root mode: the helper read the counters for us.
                    Some(cur) => {
                        let old = self.prev_io.get(&pid).copied().unwrap_or(cur);
                        io_now.insert(pid, cur);
                        (cur.0.saturating_sub(old.0) as f64 / interval, cur.1.saturating_sub(old.1) as f64 / interval)
                    }
                    None => (-1.0, -1.0),
                },
            };

            let prev = self.prev_ticks.get(&pid).copied().unwrap_or(ticks);
            let cpu = ticks.saturating_sub(prev) as f64 / (interval * self.clk_tck) * 100.0;
            ticks_now.insert(pid, ticks);

            let (container, engine) = match st.container {
                Some((id, engine)) => (Some(id), engine),
                None => (None, ""),
            };
            if let Some(id) = &container {
                // Any process of the container sees its network namespace.
                cnet_pid.entry(id.clone()).or_insert(pid);
            }
            let cpu_avg = match self.procs.get(&pid) {
                Some(old) => old.cpu_avg * 0.5 + cpu * 0.5,
                None => cpu,
            };
            procs.insert(
                pid,
                Proc {
                    cpu_avg,
                    gpu: 0.0,
                    vram: 0,
                    gtt: 0,
                    gpu_engines: [0.0; 5],
                    gpus: String::new(),
                    io_r,
                    io_w,
                    pid,
                    ppid,
                    uid: st.uid,
                    exe: st.exe,
                    cmd: if st.cmd.is_empty() { format!("[{name}]") } else { st.cmd },
                    name,
                    state: f[0].chars().next().unwrap_or('?'),
                    pri: n(15),
                    ni: n(16),
                    threads: n(17) as u64,
                    virt: n(20) as u64,
                    res,
                    shr,
                    ticks,
                    cpu,
                    mem: res as f64 / mem_total as f64 * 100.0,
                    kernel,
                    container,
                    engine,
                },
            );
        }
        self.statics.retain(|pid, _| procs.contains_key(pid));
        self.prev_ticks = ticks_now;
        self.prev_io = io_now;
        // Per-container network: /proc/<pid>/net/dev shows that process's
        // network namespace, and it's readable for any process.
        let mut cnet = HashMap::new();
        let mut prev_cnet = HashMap::new();
        for (id, pid) in &cnet_pid {
            let Ok(dev) = fs::read_to_string(format!("/proc/{pid}/net/dev")) else { continue };
            let (mut rx, mut tx) = (0u64, 0u64);
            for line in dev.lines().skip(2) {
                let Some((name, rest)) = line.split_once(':') else { continue };
                if name.trim() == "lo" {
                    continue;
                }
                let f: Vec<u64> = rest.split_whitespace().map(|x| x.parse().unwrap_or(0)).collect();
                if f.len() > 8 {
                    rx += f[0];
                    tx += f[8];
                }
            }
            let old = self.prev_cnet.get(id).copied().unwrap_or((rx, tx));
            cnet.insert(id.clone(), (rx.saturating_sub(old.0) as f64 / interval, tx.saturating_sub(old.1) as f64 / interval));
            prev_cnet.insert(id.clone(), (rx, tx));
        }
        self.cnet = cnet;
        self.prev_cnet = prev_cnet;
        for (pid, uses) in self.gpu_uses().iter() {
            if let Some(p) = procs.get_mut(pid) {
                let mut ids = Vec::new();
                for u in uses {
                    p.vram += u.vram;
                    p.gtt += u.gtt;
                    p.gpu = p.gpu.max(u.busiest());
                    for i in 0..5 {
                        p.gpu_engines[i] += u.engines[i];
                    }
                    if let Some(i) = self.gpu_order.iter().position(|d| *d == u.pdev) {
                        ids.push((i + 1).to_string());
                    }
                }
                ids.sort();
                p.gpus = ids.join(",");
            }
        }
        self.procs = procs;
    }

    /// Per-process GPU use: the root helper's (every process) in root mode,
    /// else our own poller's (our processes).
    fn gpu_uses(&self) -> HashMap<i32, Vec<gpu::GpuUse>> {
        match &self.root {
            Some(r) if !r.gpu.is_empty() => r.gpu.clone(),
            _ => self.gpu.lock().unwrap().clone(),
        }
    }

    /// Engine busy % per GPU (summed over processes), in gpu_order.
    pub fn gpu_engine_totals(&self) -> Vec<[f64; 5]> {
        let mut t = vec![[0.0; 5]; self.gpu_order.len()];
        for uses in self.gpu_uses().values() {
            for u in uses {
                if let Some(i) = self.gpu_order.iter().position(|d| *d == u.pdev) {
                    for e in 0..5 {
                        t[i][e] = (t[i][e] + u.engines[e]).min(100.0);
                    }
                }
            }
        }
        t
    }

    fn visible(&self, p: &Proc) -> bool {
        self.show_kernel || !p.kernel
    }

    fn matches(&self, p: &Proc, names: &HashMap<String, containers::Container>) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        let f = self.filter.to_lowercase();
        p.cmd.to_lowercase().contains(&f)
            || p.name.to_lowercase().contains(&f)
            || p.pid.to_string() == f
            || self.user(p.uid).to_lowercase().contains(&f)
            || p.container.as_ref().and_then(|id| names.get(id)).map(|c| c.name.to_lowercase().contains(&f)).unwrap_or(false)
    }

    /// Bucketed value for "live" sort keys: coarse log-scale buckets on a
    /// smoothed value, so processes only change places when they differ by a
    /// real margin. None for static keys (pid, user, command...).
    fn bucket(&self, p: &Proc) -> Option<i64> {
        let (v, floor) = match self.sort_key.as_str() {
            "cpu" => (p.cpu_avg, 0.5),
            "mem" | "res" => (p.res as f64, 4.0 * 1048576.0),
            "virt" => (p.virt as f64, 16.0 * 1048576.0),
            "shr" => (p.shr as f64, 4.0 * 1048576.0),
            "time" => (p.ticks as f64 / self.clk_tck, 1.0),
            "gpu" => (p.gpu, 0.5),
            "vram" => (p.vram as f64, 4.0 * 1048576.0),
            "ior" => (p.io_r.max(0.0), 16.0 * 1024.0),
            "net" => (self.net_of(p), 4.0 * 1024.0),
            "iow" => (p.io_w.max(0.0), 16.0 * 1024.0),
            _ => return None,
        };
        Some(if v < floor { 0 } else { 1 + ((v / floor).ln() / 1.35f64.ln()) as i64 })
    }

    /// Orders siblings. Live keys keep the previous on-screen order inside a
    /// bucket (and entirely while frozen); new processes go last.
    fn order_pids(&self, pids: &mut [i32]) {
        let ranks = self.prev_rank.borrow();
        let live = pids.first().and_then(|p| self.procs.get(p)).map(|p| self.bucket(p).is_some()).unwrap_or(false);
        if pids.len() < 2 || ranks.is_empty() || (!live && !self.frozen) {
            drop(ranks);
            self.sort_pids(pids);
            return;
        }
        let rank = |p: &i32| ranks.get(p).copied().unwrap_or(usize::MAX);
        if self.frozen {
            pids.sort_by_key(|p| (rank(p), *p));
            return;
        }
        let desc = self.sort_desc;
        pids.sort_by_key(|p| {
            let b = self.bucket(&self.procs[p]).unwrap_or(0);
            (if desc { -b } else { b }, rank(p), *p)
        });
    }

    /// Network rate of the process's container (0 for host processes).
    fn net_of(&self, p: &Proc) -> f64 {
        p.container.as_ref().and_then(|id| self.cnet.get(id)).map(|v| v.0 + v.1).unwrap_or(0.0)
    }

    fn sort_pids(&self, pids: &mut [i32]) {
        let key = self.sort_key.as_str();
        let procs = &self.procs;
        pids.sort_by(|a, b| {
            let (pa, pb) = (&procs[a], &procs[b]);
            let ord = match key {
                "pid" => pa.pid.cmp(&pb.pid),
                "user" => self.user(pa.uid).cmp(&self.user(pb.uid)),
                "pri" => pa.pri.cmp(&pb.pri),
                "ni" => pa.ni.cmp(&pb.ni),
                "virt" => pa.virt.cmp(&pb.virt),
                "res" | "mem" => pa.res.cmp(&pb.res),
                "shr" => pa.shr.cmp(&pb.shr),
                "state" => pa.state.cmp(&pb.state),
                "time" => pa.ticks.cmp(&pb.ticks),
                "threads" => pa.threads.cmp(&pb.threads),
                "command" => pa.cmd.cmp(&pb.cmd),
                "container" => pa.container.cmp(&pb.container),
                "gpu" => pa.gpu.partial_cmp(&pb.gpu).unwrap_or(std::cmp::Ordering::Equal),
                "vram" => pa.vram.cmp(&pb.vram),
                "ior" => pa.io_r.partial_cmp(&pb.io_r).unwrap_or(std::cmp::Ordering::Equal),
                "net" => self.net_of(pa).partial_cmp(&self.net_of(pb)).unwrap_or(std::cmp::Ordering::Equal),
                "iow" => pa.io_w.partial_cmp(&pb.io_w).unwrap_or(std::cmp::Ordering::Equal),
                _ => pa.cpu.partial_cmp(&pb.cpu).unwrap_or(std::cmp::Ordering::Equal),
            };
            let ord = if self.sort_desc { ord.reverse() } else { ord };
            ord.then(pa.pid.cmp(&pb.pid))
        });
    }

    fn row(&self, p: &Proc, depth: usize, prefix: &str, has_children: bool, names: &HashMap<String, containers::Container>) -> Value {
        let c = p.container.as_ref().and_then(|id| names.get(id));
        let key = format!("p:{}", p.pid);
        // First process inside a container (its parent is outside it):
        // where the tree view puts the container badge.
        let container_root = p.container.is_some()
            && self.procs.get(&p.ppid).map(|pp| pp.container != p.container).unwrap_or(true);
        json!({
            "key": key, "pid": p.pid, "ppid": p.ppid, "depth": depth, "prefix": prefix,
            "hasChildren": has_children, "expanded": !self.collapsed.contains(&key), "group": false,
            "user": self.user(p.uid), "uid": p.uid, "pri": p.pri, "ni": p.ni,
            "virt": p.virt as f64, "res": p.res as f64, "shr": p.shr as f64,
            "state": p.state.to_string(), "cpu": p.cpu, "mem": p.mem,
            "time": p.ticks as f64 / self.clk_tck, "threads": p.threads,
            "container": c.map(|c| c.name.clone()).or_else(|| p.container.as_ref().map(|id| id[..12].to_string())).unwrap_or_default(),
            "engine": p.engine, "image": c.map(|c| c.image.clone()).unwrap_or_default(),
            "name": p.name, "command": p.cmd, "exe": p.exe, "kernel": p.kernel, "count": 1,
            "containerRoot": container_root,
            "gpu": p.gpu, "vram": p.vram as f64, "gtt": p.gtt as f64, "gpus": p.gpus,
            "gpuEngines": p.gpu_engines.iter().map(|v| format!("{v:.0}")).collect::<Vec<_>>().join("/"),
            "ioR": p.io_r, "ioW": p.io_w,
            // Network is only known per container (network namespace); shown
            // on the container's first process.
            "netRx": if container_root { p.container.as_ref().and_then(|id| self.cnet.get(id)).map(|v| v.0).unwrap_or(-1.0) } else { -1.0 },
            "netTx": if container_root { p.container.as_ref().and_then(|id| self.cnet.get(id)).map(|v| v.1).unwrap_or(-1.0) } else { -1.0 },
        })
    }

    /// Depth-first walk emitting htop-style tree lines.
    #[allow(clippy::too_many_arguments)]
    fn walk(
        &self,
        pid: i32,
        children: &HashMap<i32, Vec<i32>>,
        keep: &Option<HashSet<i32>>,
        depth: usize,
        lines: &str,
        last: bool,
        out: &mut Vec<Value>,
        names: &HashMap<String, containers::Container>,
    ) {
        let kids: Vec<i32> = children
            .get(&pid)
            .map(|v| v.iter().copied().filter(|c| keep.as_ref().map(|k| k.contains(c)).unwrap_or(true)).collect())
            .unwrap_or_default();
        let prefix = if depth == 0 { String::new() } else { format!("{lines}{}", if last { "└─ " } else { "├─ " }) };
        out.push(self.row(&self.procs[&pid], depth, &prefix, !kids.is_empty(), names));
        if self.collapsed.contains(&format!("p:{pid}")) {
            return;
        }
        let next_lines = if depth == 0 { String::new() } else { format!("{lines}{}", if last { "   " } else { "│  " }) };
        for (i, k) in kids.iter().enumerate() {
            self.walk(*k, children, keep, depth + 1, &next_lines, i + 1 == kids.len(), out, names);
        }
    }

    /// Builds a forest out of `pids` (parents outside the set make roots).
    fn forest(&self, pids: &[i32]) -> (Vec<i32>, HashMap<i32, Vec<i32>>) {
        let set: HashSet<i32> = pids.iter().copied().collect();
        let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
        let mut roots = Vec::new();
        for &pid in pids {
            let pp = self.procs[&pid].ppid;
            if pp != pid && set.contains(&pp) {
                children.entry(pp).or_default().push(pid);
            } else {
                roots.push(pid);
            }
        }
        for v in children.values_mut() {
            self.order_pids(v);
        }
        self.order_pids(&mut roots);
        (roots, children)
    }

    /// Matching processes plus every ancestor, so a filtered tree keeps its shape.
    fn keep_set(&self, pids: &[i32], names: &HashMap<String, containers::Container>) -> Option<HashSet<i32>> {
        if self.filter.is_empty() {
            return None;
        }
        let mut keep = HashSet::new();
        for &pid in pids {
            if self.matches(&self.procs[&pid], names) {
                let mut cur = pid;
                while keep.insert(cur) {
                    match self.procs.get(&cur) {
                        Some(p) if p.ppid != cur && self.procs.contains_key(&p.ppid) => cur = p.ppid,
                        _ => break,
                    }
                }
            }
        }
        Some(keep)
    }

    /// The focused process and all its descendants.
    fn subtree(&self, root: i32) -> HashSet<i32> {
        let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
        for p in self.procs.values() {
            if p.ppid != p.pid {
                children.entry(p.ppid).or_default().push(p.pid);
            }
        }
        let mut set = HashSet::new();
        let mut stack = vec![root];
        while let Some(pid) = stack.pop() {
            if set.insert(pid) {
                if let Some(c) = children.get(&pid) {
                    stack.extend(c);
                }
            }
        }
        set
    }

    pub fn rows(&self) -> Vec<Value> {
        let names = self.names.lock().unwrap().clone();
        let mut pids: Vec<i32> = self.procs.values().filter(|p| self.visible(p)).map(|p| p.pid).collect();
        if let Some(f) = self.focus.filter(|f| self.procs.contains_key(f)) {
            let keep = self.subtree(f);
            pids.retain(|p| keep.contains(p));
        }
        let mut out = Vec::new();
        match self.view {
            View::Flat | View::Gpu => {
                pids.retain(|p| self.matches(&self.procs[p], &names));
                if self.view == View::Gpu {
                    pids.retain(|p| !self.procs[p].gpus.is_empty());
                }
                self.order_pids(&mut pids);
                for pid in pids {
                    out.push(self.row(&self.procs[&pid], 0, "", false, &names));
                }
            }
            View::Tree => {
                let keep = self.keep_set(&pids, &names);
                let (roots, children) = self.forest(&pids);
                for r in roots.iter().filter(|r| keep.as_ref().map(|k| k.contains(r)).unwrap_or(true)) {
                    self.walk(*r, &children, &keep, 0, "", true, &mut out, &names);
                }
            }
            View::Containers => {
                let keep = self.keep_set(&pids, &names);
                let mut groups: HashMap<Option<String>, Vec<i32>> = HashMap::new();
                for &pid in &pids {
                    groups.entry(self.procs[&pid].container.clone()).or_default().push(pid);
                }
                let mut order: Vec<Option<String>> = groups.keys().cloned().collect();
                let agg = |g: &Option<String>| -> (f64, f64, u64) {
                    groups[g].iter().fold((0.0, 0.0, 0), |(c, m, r), p| {
                        let p = &self.procs[p];
                        (c + p.cpu, m + p.mem, r + p.res)
                    })
                };
                order.sort_by(|a, b| {
                    // Containers first, the host last.
                    match (a.is_none(), b.is_none()) {
                        (true, false) => return std::cmp::Ordering::Greater,
                        (false, true) => return std::cmp::Ordering::Less,
                        _ => {}
                    }
                    let (ca, ma, _) = agg(a);
                    let (cb, mb, _) = agg(b);
                    let ord = if self.sort_key == "mem" || self.sort_key == "res" { ma.partial_cmp(&mb) } else { ca.partial_cmp(&cb) };
                    let ord = ord.unwrap_or(std::cmp::Ordering::Equal);
                    if self.sort_desc { ord.reverse() } else { ord }
                });
                for g in order {
                    let members: Vec<i32> = groups[&g]
                        .iter()
                        .copied()
                        .filter(|p| keep.as_ref().map(|k| k.contains(p)).unwrap_or(true))
                        .collect();
                    if members.is_empty() {
                        continue;
                    }
                    let (cpu, mem, res) = agg(&g);
                    let key = format!("c:{}", g.as_deref().unwrap_or("host"));
                    let c = g.as_ref().and_then(|id| names.get(id));
                    let title = match (&g, c) {
                        (None, _) => "Host".to_string(),
                        (Some(_), Some(c)) => c.name.clone(),
                        (Some(id), None) => id[..12].to_string(),
                    };
                    let engine = g.as_ref().and_then(|_| groups[&g].first().map(|p| self.procs[p].engine.to_string()));
                    out.push(json!({
                        "key": key, "pid": 0, "ppid": 0, "depth": 0, "prefix": "", "hasChildren": true,
                        "expanded": !self.collapsed.contains(&key), "group": true,
                        "user": "", "uid": -1, "pri": 0, "ni": 0, "virt": 0.0, "res": res as f64, "shr": 0.0,
                        "state": "", "cpu": cpu, "mem": mem, "time": 0.0, "threads": 0,
                        "container": title, "engine": engine.unwrap_or_default(),
                        "image": c.map(|c| c.image.clone()).unwrap_or_default(),
                        "name": title, "command": c.map(|c| format!("{}  ·  {}", c.image, c.status)).unwrap_or_else(|| "processes outside any container".into()),
                        "kernel": false, "count": members.len(), "exe": "",
                        "containerRoot": false,
                        "gpu": members.iter().map(|p| self.procs[p].gpu).fold(0.0, f64::max),
                        "vram": members.iter().map(|p| self.procs[p].vram).sum::<u64>() as f64,
                        "gtt": 0.0, "gpus": "", "gpuEngines": "",
                        "ioR": members.iter().map(|p| self.procs[p].io_r.max(0.0)).sum::<f64>(),
                        "ioW": members.iter().map(|p| self.procs[p].io_w.max(0.0)).sum::<f64>(),
                        "netRx": g.as_ref().and_then(|id| self.cnet.get(id)).map(|v| v.0).unwrap_or(-1.0),
                        "netTx": g.as_ref().and_then(|id| self.cnet.get(id)).map(|v| v.1).unwrap_or(-1.0),
                    }));
                    if self.collapsed.contains(&key) {
                        continue;
                    }
                    let (roots, children) = self.forest(&members);
                    for (i, r) in roots.iter().enumerate() {
                        self.walk(*r, &children, &keep, 1, "", i + 1 == roots.len(), &mut out, &names);
                    }
                }
            }
        }
        let mut ranks = self.prev_rank.borrow_mut();
        ranks.clear();
        for (i, r) in out.iter().enumerate() {
            if let Some(pid) = r["pid"].as_i64().filter(|p| *p > 0) {
                ranks.insert(pid as i32, i);
            }
        }
        out
    }

    pub fn summary(&self) -> Value {
        let tasks = self.procs.values().filter(|p| !p.kernel).count();
        let kthreads = self.procs.values().filter(|p| p.kernel).count();
        let threads: u64 = self.procs.values().filter(|p| !p.kernel).map(|p| p.threads).sum();
        let running = self.procs.values().filter(|p| p.state == 'R').count();
        let containers = self.names.lock().unwrap().len();
        let gpu_procs = self.procs.values().filter(|p| !p.gpus.is_empty()).count();
        // Focus breadcrumb: the focused process's ancestors, outermost first.
        let focus = self.focus.and_then(|f| self.procs.get(&f)).map(|p| {
            let mut chain = Vec::new();
            let mut cur = self.procs.get(&p.ppid);
            while let Some(a) = cur {
                chain.push(json!({"pid": a.pid, "name": a.name}));
                if a.ppid == a.pid || chain.len() > 64 {
                    break;
                }
                cur = self.procs.get(&a.ppid);
            }
            chain.reverse();
            json!({"pid": p.pid, "name": p.name, "chain": chain, "count": self.subtree(p.pid).len()})
        });
        json!({"tasks": tasks, "threads": threads, "kthreads": kthreads, "running": running, "containers": containers, "gpuProcs": gpu_procs, "focus": focus})
    }
}
