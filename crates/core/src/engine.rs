//! The sampling engine, shared by the Qt app and the terminal UI.
//!
//! All sampling happens on a worker thread: it reads /proc and /sys, lays out
//! the process rows, diffs them against what the front end already has, and
//! hands a JSON update to `on_update`. The front end never touches /proc and
//! only applies changed rows. User actions (sort, filter, expand...) poke the
//! worker, which re-lays out rows at once without re-sampling.
//!
//! An update is the system snapshot (see `system.rs`) plus `procs`, `top`,
//! `notices`, `root`, `rootStarting`, `collapsedCount` and a row diff
//! `rows: {count, changed: [[index, row], ...]}`; a re-layout without a new
//! sample is `{rowsOnly: true, collapsedCount, rows}`.

use crate::{actions::Actions, processes::{Processes, View}, system::System};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

struct State {
    system: System,
    cooling: crate::cooling::Shared,
    procs: Processes,
    /// Rows as the front end currently has them (for diffing).
    shown: Vec<Value>,
    rows_enabled: bool,
    interval: Duration,
    /// Set by user actions: re-layout rows now, without a new sample.
    relayout: bool,
    /// How /nix/store paths show in rows.
    store_style: crate::command::StoreStyle,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    /// Pointer over the list: keep the order (atomic so hovering never
    /// waits on the sampler's lock).
    frozen: AtomicBool,
    root: Arc<Mutex<Option<crate::root::Helper>>>,
    root_starting: AtomicBool,
}

#[derive(Clone)]
pub struct Engine {
    shared: Arc<Shared>,
    actions: Arc<Actions>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// Lays out rows and returns only the ones that differ from what the UI has.
fn row_diff(st: &mut State) -> Value {
    let mut rows = st.procs.rows();
    for r in rows.iter_mut() {
        crate::columns::decorate(r, st.store_style);
    }
    let mut changed = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        if st.shown.get(i) != Some(r) {
            changed.push(json!([i, r]));
        }
    }
    let count = rows.len();
    st.shown = rows;
    json!({"count": count, "changed": changed})
}

/// "12,34" -> [12, 34]
pub fn pid_list(s: &str) -> Vec<i32> {
    s.split(',').filter_map(|p| p.trim().parse().ok()).collect()
}

impl Engine {
    pub fn new() -> Self {
        let root = Arc::new(Mutex::new(None));
        Engine {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    system: System::new(),
                    cooling: crate::cooling::spawn_poller(),
                    procs: {
                        let mut p = Processes::new();
                        p.load_view();
                        p
                    },
                    shown: Vec::new(),
                    rows_enabled: true,
                    interval: Duration::from_millis(1000),
                    relayout: false,
                    store_style: crate::command::StoreStyle::Compact,
                }),
                wake: Condvar::new(),
                frozen: AtomicBool::new(false),
                root: root.clone(),
                root_starting: AtomicBool::new(false),
            }),
            actions: Arc::new(Actions::new(root)),
        }
    }

    /// Starts the sampler thread. `on_update` gets every update; returning
    /// false stops the thread (the front end is gone).
    pub fn start(&self, mut on_update: impl FnMut(Value) -> bool + Send + 'static) {
        let shared = self.shared.clone();
        let actions = self.actions.clone();
        std::thread::Builder::new()
            .name("treetop-sampler".into())
            .spawn(move || {
                let mut next = Instant::now();
                loop {
                    let mut st = shared.state.lock().unwrap();
                    // Sleep until the next sample is due or a user action wakes us.
                    while !st.relayout && Instant::now() < next {
                        let wait = next.saturating_duration_since(Instant::now());
                        st = shared.wake.wait_timeout(st, wait).unwrap().0;
                    }
                    let sampling = !(st.relayout && Instant::now() < next);
                    if sampling {
                        // Ask the root helper (if any) without holding the
                        // state lock, so the UI never waits on it.
                        drop(st);
                        let reply = shared.root.lock().unwrap().as_mut().map(|h| h.request("sample"));
                        st = shared.state.lock().unwrap();
                        match reply {
                            Some(Some(v)) => st.procs.root = Some(crate::root::RootData::from_json(&v)),
                            Some(None) => {
                                *shared.root.lock().unwrap() = None;
                                st.procs.root = None;
                                actions.notify("Root access ended".into());
                            }
                            None => st.procs.root = None,
                        }
                    }
                    st.procs.frozen = shared.frozen.load(Ordering::Relaxed);
                    let mut out = if !sampling {
                        // Re-layout only (sort/filter/expand changed).
                        json!({"rowsOnly": true})
                    } else {
                        next = Instant::now() + st.interval;
                        let s = &mut *st;
                        s.system.sample();
                        s.procs.gpu_order = s.system.snapshot["gpus"]
                            .as_array()
                            .map(|a| a.iter().map(|g| g["pdev"].as_str().unwrap_or("").to_string()).collect())
                            .unwrap_or_default();
                        s.procs.sample(s.system.interval, s.system.mem_total);
                        let mut out = s.system.snapshot.clone();
                        let mut unattributed: f64 = 0.0;
                        for (i, e) in s.procs.gpu_engine_totals().into_iter().enumerate() {
                            // Busy the hardware reports that no process's
                            // engine time explains (ROCm compute, a stuck
                            // queue, or other users' work without root).
                            let busy = out["gpus"][i]["busy"].as_f64().unwrap_or(0.0);
                            let un = (busy - e.iter().cloned().fold(0.0, f64::max)).max(0.0);
                            out["gpus"][i]["unattributed"] = json!(un);
                            unattributed = unattributed.max(un);
                            out["gpus"][i]["engines"] = json!(e);
                        }
                        // liquidctl fans/pumps/liquid temps: the root helper's
                        // (sees every device) when root access is on, else ours.
                        let cooling = match s.procs.root.as_ref().map(|r| &r.cooling) {
                            Some(c) if !c.is_empty() => c.clone(),
                            _ => s.cooling.lock().unwrap().clone(),
                        };
                        if let Some(list) = out["sensors"].as_array_mut() {
                            list.extend(cooling);
                            list.extend(crate::cooling::lights_out());
                        }
                        out["procs"] = s.procs.summary();
                        out["top"] = s.procs.top();
                        out["top"]["gpuUnattributed"] = json!(unattributed);
                        let mut notices = actions.drain();
                        if let Some(f) = s.procs.focus {
                            if !s.procs.procs.contains_key(&f) {
                                s.procs.focus = None;
                                notices.push(format!("PID {f} exited; showing everything again"));
                            }
                        }
                        out["notices"] = json!(notices);
                        out["root"] = json!(s.procs.root.is_some());
                        out["rootStarting"] = json!(shared.root_starting.load(Ordering::Relaxed));
                        out
                    };
                    st.relayout = false;
                    st.procs.save_view();
                    out["collapsedCount"] = json!(st.procs.collapsed.len());
                    if st.rows_enabled {
                        out["rows"] = row_diff(&mut st);
                    }
                    drop(st);
                    if !on_update(out) {
                        break; // the front end is gone
                    }
                }
            })
            .expect("spawn sampler thread");
    }

    fn with_state(&self, f: impl FnOnce(&mut State)) {
        let mut st = self.shared.state.lock().unwrap();
        f(&mut st);
        st.relayout = true;
        drop(st);
        self.shared.wake.notify_one();
    }

    /// How /nix/store paths show: "full", "hash" or "compact".
    pub fn set_store_style(&self, style: &str) {
        let st = crate::command::StoreStyle::parse(style);
        self.with_state(|s| s.store_style = st);
    }

    /// Send every row again with the next update (the front end's drawing
    /// of them changed, e.g. its colours).
    pub fn resend(&self) {
        self.with_state(|s| s.shown.clear());
    }

    pub fn set_interval(&self, ms: i32) {
        self.with_state(|s| s.interval = Duration::from_millis(ms.clamp(100, 60_000) as u64));
    }

    pub fn set_rows_enabled(&self, enabled: bool) {
        self.with_state(|s| s.rows_enabled = enabled);
    }

    /// Re-lay out rows now.
    pub fn refresh(&self) {
        self.with_state(|_| {});
    }

    pub fn toggle(&self, key: &str) {
        let key = key.to_string();
        self.with_state(|s| {
            if !s.procs.collapsed.remove(&key) {
                s.procs.collapsed.insert(key);
            }
        });
    }

    pub fn set_expanded_all(&self, expanded: bool) {
        self.with_state(|s| {
            let p = &mut s.procs;
            if expanded {
                p.collapsed.clear();
            } else {
                // Collapse below the top level; collapsing init would hide everything.
                let parents: Vec<String> = p
                    .procs
                    .values()
                    .filter(|x| p.procs.contains_key(&x.ppid) && x.ppid > 1)
                    .map(|x| format!("p:{}", x.ppid))
                    .collect();
                p.collapsed.extend(parents);
            }
        });
    }

    /// Expand or collapse a process and everything below it.
    pub fn set_subtree_expanded(&self, key: &str, expanded: bool) {
        let key = key.to_string();
        self.with_state(|s| {
            let p = &mut s.procs;
            let Some(root) = key.strip_prefix("p:").and_then(|k| k.parse::<i32>().ok()) else {
                // Container group rows: just that row.
                if expanded {
                    p.collapsed.remove(&key);
                } else {
                    p.collapsed.insert(key);
                }
                return;
            };
            let mut children: std::collections::HashMap<i32, Vec<i32>> = std::collections::HashMap::new();
            for x in p.procs.values() {
                children.entry(x.ppid).or_default().push(x.pid);
            }
            let mut stack = vec![root];
            while let Some(pid) = stack.pop() {
                let k = format!("p:{pid}");
                let kids = children.get(&pid).cloned().unwrap_or_default();
                if expanded {
                    p.collapsed.remove(&k);
                } else if !kids.is_empty() {
                    p.collapsed.insert(k);
                }
                stack.extend(kids);
            }
        });
    }

    pub fn set_view(&self, view: &str) {
        let v = match view {
            "flat" => View::Flat,
            "containers" => View::Containers,
            "gpu" => View::Gpu,
            _ => View::Tree,
        };
        self.with_state(|s| s.procs.view = v);
    }

    pub fn set_sort(&self, key: &str, descending: bool) {
        let key = key.to_string();
        self.with_state(|s| {
            s.procs.sort_key = key;
            s.procs.sort_desc = descending;
        });
    }

    pub fn set_filter(&self, text: &str) {
        let t = text.to_string();
        self.with_state(|s| s.procs.filter = t);
    }

    pub fn set_show_kernel(&self, show: bool) {
        self.with_state(|s| s.procs.show_kernel = show);
    }

    /// Show only this process's tree (0 shows everything again).
    pub fn set_focus(&self, pid: i32) {
        self.with_state(|s| s.procs.focus = (pid > 0).then_some(pid));
    }

    pub fn set_frozen(&self, frozen: bool) {
        // No relayout needed: the next sample simply keeps the order.
        self.shared.frozen.store(frozen, Ordering::Relaxed);
    }

    /// Root mode: one authorization, then privileged readings. `launcher`
    /// says how to become root (polkit, or sudo in a terminal).
    pub fn set_root_mode(&self, on: bool) {
        self.set_root_mode_with(on, crate::root::Launcher::Pkexec);
    }

    pub fn set_root_mode_with(&self, on: bool, launcher: crate::root::Launcher) {
        if !on {
            // Dropping the helper closes its stdin; it exits.
            *self.shared.root.lock().unwrap() = None;
            self.actions.notify("Root access off".into());
            self.refresh();
            return;
        }
        if self.shared.root.lock().unwrap().is_some() || self.shared.root_starting.swap(true, Ordering::SeqCst) {
            return;
        }
        let shared = self.shared.clone();
        let actions = self.actions.clone();
        std::thread::spawn(move || {
            // Blocks while polkit asks for the password.
            match crate::root::Helper::start_with(launcher) {
                Ok(h) => {
                    *shared.root.lock().unwrap() = Some(h);
                    actions.notify("Root access on: showing every process's I/O, GPU, paths and files".into());
                }
                Err(e) => actions.notify(e),
            }
            shared.root_starting.store(false, Ordering::SeqCst);
            shared.state.lock().unwrap().relayout = true;
            shared.wake.notify_one();
        });
    }

    pub fn root_on(&self) -> bool {
        self.shared.root.lock().unwrap().is_some()
    }

    /// Reads a process's environment and open files as root (the helper,
    /// or a one-time prompt); answers through `done` on another thread with
    /// {pid, environ, files} or {pid, error}.
    pub fn inspect_as_root(&self, pid: i32, done: impl FnOnce(Value) + Send + 'static) {
        let root = self.shared.root.clone();
        std::thread::spawn(move || {
            let via_helper = root.lock().unwrap().as_mut().and_then(|h| h.request(&format!("inspect {pid}")));
            let mut v = match via_helper {
                Some(v) => v,
                None => crate::root::inspect_once(pid).unwrap_or_else(|e| json!({"error": e})),
            };
            v["pid"] = json!(pid);
            done(v);
        });
    }

    pub fn send_signal(&self, pids: &[i32], signal: i32) -> String {
        self.actions.send_signal(pids, signal)
    }

    pub fn renice(&self, pids: &[i32], nice: i32) -> String {
        self.actions.renice(pids, nice)
    }

    /// [{name, value}] for the "Send signal" menu.
    pub fn signal_list(&self) -> Value {
        json!(crate::actions::SIGNALS.iter().map(|(n, v)| json!({"name": n, "value": v})).collect::<Vec<_>>())
    }

    /// Everything the details dialog shows about one process.
    pub fn details(&self, pid: i32) -> Value {
        let base = format!("/proc/{pid}");
        let read = |f: &str| std::fs::read_to_string(format!("{base}/{f}")).unwrap_or_default();
        let status = read("status");
        let field = |k: &str| {
            status.lines().find_map(|l| l.strip_prefix(k).map(|v| v.trim_start_matches(':').trim().to_string())).unwrap_or_default()
        };
        let cwd = std::fs::read_link(format!("{base}/cwd")).map(|p| p.display().to_string()).unwrap_or_default();
        let exe = std::fs::read_link(format!("{base}/exe")).map(|p| p.display().to_string()).unwrap_or_default();
        let fds = std::fs::read_dir(format!("{base}/fd")).map(|d| d.count() as i64).unwrap_or(-1);
        let io = read("io");
        let io_field = |k: &str| io.lines().find_map(|l| l.strip_prefix(k).and_then(|v| v.trim_start_matches(':').trim().parse::<u64>().ok()));
        let cgroup = read("cgroup").lines().next().unwrap_or("").rsplit(':').next().unwrap_or("").to_string();
        let mut environ = crate::inspect::environ(pid);
        let mut files = crate::inspect::files(pid);
        if environ.is_none() || files.is_none() {
            // Root mode: the helper can read them without another prompt.
            if let Some(v) = self.shared.root.lock().unwrap().as_mut().and_then(|h| h.request(&format!("inspect {pid}"))) {
                environ = environ.or_else(|| (!v["environ"].is_null()).then(|| v["environ"].clone()));
                files = files.or_else(|| (!v["files"].is_null()).then(|| v["files"].clone()));
            }
        }
        let st = self.shared.state.lock().unwrap();
        let p = st.procs.procs.get(&pid);
        let container = p.and_then(|p| p.container.clone());
        let cinfo = container.as_ref().and_then(|id| st.procs.names.lock().unwrap().get(id).cloned());
        json!({
            "pid": pid, "name": p.map(|p| p.name.clone()), "command": p.map(|p| p.cmd.clone()),
            "exe": exe, "cwd": cwd, "fds": fds, "cgroup": cgroup,
            "state": field("State"), "threads": field("Threads"), "ppid": field("PPid"),
            "uid": field("Uid"), "vmPeak": field("VmPeak"), "vmSwap": field("VmSwap"),
            "ctxtVoluntary": field("voluntary_ctxt_switches"), "ctxtInvoluntary": field("nonvoluntary_ctxt_switches"),
            "readBytes": io_field("read_bytes"), "writeBytes": io_field("write_bytes"),
            "containerId": container,
            "containerName": cinfo.as_ref().map(|c| c.name.clone()),
            "containerImage": cinfo.as_ref().map(|c| c.image.clone()),
            "containerStatus": cinfo.as_ref().map(|c| c.status.clone()),
            "engine": cinfo.as_ref().map(|c| c.engine),
            "gpus": p.map(|p| p.gpus.clone()),
            "vram": p.map(|p| p.vram), "gtt": p.map(|p| p.gtt),
            "gpuEngines": p.map(|p| p.gpu_engines.to_vec()),
            // null when the kernel won't show them (other users' processes).
            "environ": environ,
            "files": files,
        })
    }
}
