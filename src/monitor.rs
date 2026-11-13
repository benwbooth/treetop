//! The QML-facing `Monitor` object.
//!
//! All sampling happens on a worker thread: it reads /proc and /sys, lays out
//! the process rows, diffs them against what the UI already has, and posts a
//! small JSON update to the GUI thread through `updated(json)`. The GUI
//! thread never touches /proc and only applies changed rows, so scrolling
//! and animations stay smooth. User actions (sort, filter, expand...) poke
//! the worker, which re-lays out rows immediately without re-sampling.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        type Monitor = super::MonitorRust;

        /// A sample: system readings plus a row diff
        /// `{count, changed: [[index, row], ...]}`.
        #[qsignal]
        fn updated(self: Pin<&mut Monitor>, json: QString);

        /// Result of inspect_as_root: {pid, environ, files} or {pid, error}.
        #[qsignal]
        fn inspected(self: Pin<&mut Monitor>, json: QString);

        /// Root mode through polkit: one prompt, then privileged readings.
        #[qinvokable]
        fn set_root_mode(self: &Monitor, on: bool);
        /// Reads a process's environment/open files as root (helper, or a
        /// one-time pkexec prompt); answers through inspected().
        #[qinvokable]
        fn inspect_as_root(self: &Monitor, pid: i32);

        #[qinvokable]
        fn set_interval(self: &Monitor, ms: i32);
        #[qinvokable]
        fn set_rows_enabled(self: &Monitor, enabled: bool);
        #[qinvokable]
        fn refresh(self: &Monitor);
        #[qinvokable]
        fn toggle(self: &Monitor, key: &QString);
        #[qinvokable]
        fn set_expanded_all(self: &Monitor, expanded: bool);
        /// Expand or collapse a process and everything below it.
        #[qinvokable]
        fn set_subtree_expanded(self: &Monitor, key: &QString, expanded: bool);
        #[qinvokable]
        fn set_view(self: &Monitor, view: &QString);
        #[qinvokable]
        fn set_sort(self: &Monitor, key: &QString, descending: bool);
        #[qinvokable]
        fn set_filter(self: &Monitor, text: &QString);
        #[qinvokable]
        fn set_show_kernel(self: &Monitor, show: bool);
        #[qinvokable]
        fn set_frozen(self: &Monitor, frozen: bool);
        /// Show only this process's tree (0 shows everything again).
        #[qinvokable]
        fn set_focus(self: &Monitor, pid: i32);
        #[qinvokable]
        fn send_signal(self: &Monitor, pids: &QString, signal: i32) -> QString;
        #[qinvokable]
        fn renice(self: &Monitor, pids: &QString, nice: i32) -> QString;
        #[qinvokable]
        fn signal_list(self: &Monitor) -> QString;
        #[qinvokable]
        fn details(self: &Monitor, pid: i32) -> QString;
        #[qinvokable]
        fn autostart_enabled(self: &Monitor) -> bool;
        #[qinvokable]
        fn tray_widget_present(self: &Monitor) -> bool;
        #[qinvokable]
        fn set_autostart(self: &Monitor, on: bool) -> QString;
    }

    impl cxx_qt::Threading for Monitor {}
    impl cxx_qt::Initialize for Monitor {}
}

use crate::{actions::Actions, processes::{Processes, View}, system::System};
use core::pin::Pin;
use cxx_qt::Threading;
use cxx_qt_lib::QString;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

struct State {
    system: System,
    procs: Processes,
    /// Rows as the UI currently has them (for diffing).
    shown: Vec<Value>,
    rows_enabled: bool,
    interval: Duration,
    /// Set by user actions: re-layout rows now, without a new sample.
    relayout: bool,
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

pub struct MonitorRust {
    shared: Arc<Shared>,
    actions: Arc<Actions>,
}

impl Default for MonitorRust {
    fn default() -> Self {
        let root = Arc::new(Mutex::new(None));
        MonitorRust {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    system: System::new(),
                    procs: { let mut p = Processes::new(); p.load_view(); p },
                    shown: Vec::new(),
                    rows_enabled: true,
                    interval: Duration::from_millis(1000),
                    relayout: false,
                }),
                wake: Condvar::new(),
                frozen: AtomicBool::new(false),
                root: root.clone(),
                root_starting: AtomicBool::new(false),
            }),
            actions: Arc::new(Actions::new(root)),
        }
    }
}

fn pid_list(s: &QString) -> Vec<i32> {
    s.to_string().split(',').filter_map(|p| p.trim().parse().ok()).collect()
}

/// Lays out rows and returns only the ones that differ from what the UI has.
fn row_diff(st: &mut State) -> Value {
    let rows = st.procs.rows();
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

impl cxx_qt::Initialize for qobject::Monitor {
    fn initialize(self: Pin<&mut Self>) {
        let qt = self.qt_thread();
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
                        for (i, e) in s.procs.gpu_engine_totals().into_iter().enumerate() {
                            out["gpus"][i]["engines"] = json!(e);
                        }
                        out["procs"] = s.procs.summary();
                        out["top"] = s.procs.top();
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
                    let text = out.to_string();
                    if qt.queue(move |obj| obj.updated(QString::from(&text))).is_err() {
                        break; // the QObject is gone
                    }
                }
            })
            .expect("spawn sampler thread");
    }
}

impl qobject::Monitor {
    fn with_state(&self, f: impl FnOnce(&mut State)) {
        let mut st = self.shared.state.lock().unwrap();
        f(&mut st);
        st.relayout = true;
        drop(st);
        self.shared.wake.notify_one();
    }

    fn set_interval(&self, ms: i32) {
        self.with_state(|s| s.interval = Duration::from_millis(ms.clamp(100, 60_000) as u64));
    }

    fn set_rows_enabled(&self, enabled: bool) {
        self.with_state(|s| s.rows_enabled = enabled);
    }

    fn refresh(&self) {
        self.with_state(|_| {});
    }

    fn toggle(&self, key: &QString) {
        let key = key.to_string();
        self.with_state(|s| {
            if !s.procs.collapsed.remove(&key) {
                s.procs.collapsed.insert(key);
            }
        });
    }

    fn set_expanded_all(&self, expanded: bool) {
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

    fn set_subtree_expanded(&self, key: &QString, expanded: bool) {
        let key = key.to_string();
        self.with_state(|s| {
            let p = &mut s.procs;
            let Some(root) = key.strip_prefix("p:").and_then(|k| k.parse::<i32>().ok()) else {
                // Container group rows: just that row.
                if expanded { p.collapsed.remove(&key); } else { p.collapsed.insert(key); }
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

    fn set_view(&self, view: &QString) {
        let v = match view.to_string().as_str() {
            "flat" => View::Flat,
            "containers" => View::Containers,
            "gpu" => View::Gpu,
            _ => View::Tree,
        };
        self.with_state(|s| s.procs.view = v);
    }

    fn set_sort(&self, key: &QString, descending: bool) {
        let key = key.to_string();
        self.with_state(|s| {
            s.procs.sort_key = key;
            s.procs.sort_desc = descending;
        });
    }

    fn set_filter(&self, text: &QString) {
        let t = text.to_string();
        self.with_state(|s| s.procs.filter = t);
    }

    fn set_show_kernel(&self, show: bool) {
        self.with_state(|s| s.procs.show_kernel = show);
    }

    fn set_focus(&self, pid: i32) {
        // Reset the row diff so the view rebuilds cleanly around the focus.
        self.with_state(|s| s.procs.focus = (pid > 0).then_some(pid));
    }

    fn set_frozen(&self, frozen: bool) {
        // No relayout needed: the next sample simply keeps the order.
        self.shared.frozen.store(frozen, Ordering::Relaxed);
    }

    fn set_root_mode(&self, on: bool) {
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
            match crate::root::Helper::start() {
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

    fn inspect_as_root(&self, pid: i32) {
        let qt = self.qt_thread();
        let root = self.shared.root.clone();
        std::thread::spawn(move || {
            let via_helper = root.lock().unwrap().as_mut().and_then(|h| h.request(&format!("inspect {pid}")));
            let mut v = match via_helper {
                Some(v) => v,
                None => crate::root::inspect_once(pid).unwrap_or_else(|e| json!({"error": e})),
            };
            v["pid"] = json!(pid);
            let text = v.to_string();
            let _ = qt.queue(move |obj| obj.inspected(QString::from(&text)));
        });
    }

    fn send_signal(&self, pids: &QString, signal: i32) -> QString {
        QString::from(&self.actions.send_signal(&pid_list(pids), signal))
    }

    fn renice(&self, pids: &QString, nice: i32) -> QString {
        QString::from(&self.actions.renice(&pid_list(pids), nice))
    }

    fn tray_widget_present(&self) -> bool {
        crate::autostart::tray_widget_present()
    }

    fn autostart_enabled(&self) -> bool {
        crate::autostart::enabled()
    }

    fn set_autostart(&self, on: bool) -> QString {
        QString::from(&crate::autostart::set(on).unwrap_or_else(|e| e))
    }

    fn signal_list(&self) -> QString {
        let list: Vec<_> = crate::actions::SIGNALS.iter().map(|(n, v)| json!({"name": n, "value": v})).collect();
        QString::from(&json!(list).to_string())
    }

    fn details(&self, pid: i32) -> QString {
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
        let v = json!({
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
        });
        QString::from(&v.to_string())
    }
}
