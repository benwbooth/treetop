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
}

pub struct MonitorRust {
    shared: Arc<Shared>,
    actions: Arc<Actions>,
}

impl Default for MonitorRust {
    fn default() -> Self {
        MonitorRust {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    system: System::new(),
                    procs: Processes::new(),
                    shown: Vec::new(),
                    rows_enabled: true,
                    interval: Duration::from_millis(1000),
                    relayout: false,
                }),
                wake: Condvar::new(),
            }),
            actions: Arc::new(Actions::new()),
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
                    let mut out = if st.relayout && Instant::now() < next {
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
                        out["notices"] = json!(actions.drain());
                        out
                    };
                    st.relayout = false;
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

    fn set_frozen(&self, frozen: bool) {
        // No relayout needed: the next sample simply keeps the order.
        self.shared.state.lock().unwrap().procs.frozen = frozen;
    }

    fn send_signal(&self, pids: &QString, signal: i32) -> QString {
        QString::from(&self.actions.send_signal(&pid_list(pids), signal))
    }

    fn renice(&self, pids: &QString, nice: i32) -> QString {
        QString::from(&self.actions.renice(&pid_list(pids), nice))
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
        });
        QString::from(&v.to_string())
    }
}
