//! The QML-facing `Monitor` object, over `treetop_core::Engine`.
//!
//! The engine samples on its own thread and hands each update here; it is
//! posted to the GUI thread through `updated(json)`, so the GUI thread never
//! touches /proc and only applies changed rows.

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

        // ---- shared presentation logic (treetop_core) ----
        /// Colours for command lines: dark background?, text colour.
        #[qinvokable]
        fn set_palette(self: &Monitor, dark: bool, text: &QString);
        /// "full", "hash" or "compact".
        #[qinvokable]
        fn set_store_style(self: &Monitor, style: &QString);
        /// The column table (JSON).
        #[qinvokable]
        fn columns(self: &Monitor) -> QString;
        /// A command laid out for the tooltip/details (rich text).
        #[qinvokable]
        fn pretty_command(self: &Monitor, command: &QString, exe: &QString) -> QString;
        /// Every GPU with whether it's shown (JSON).
        #[qinvokable]
        fn gpu_state(self: &Monitor, hidden: &QString, pinned: &QString) -> QString;
        /// Show/hide a GPU: {hidden, pinned} (comma lists) afterwards.
        #[qinvokable]
        fn set_gpu_shown(self: &Monitor, hidden: &QString, pinned: &QString, pdev: &QString, show: bool) -> QString;
        /// Meter sections packed into columns (JSON).
        #[qinvokable]
        fn plan(self: &Monitor, width: f64, gap: f64, point_size: f64, cpu_combined: bool, collapsed: &QString, shown_gpus: i32) -> QString;
        /// The Resources summary and each section's summary (JSON).
        #[qinvokable]
        fn summaries(self: &Monitor, hidden: &QString, pinned: &QString) -> QString;
        #[qinvokable]
        fn help_text(self: &Monitor) -> QString;
        /// An environment variable as rich text.
        #[qinvokable]
        fn env_html(self: &Monitor, name: &QString, value: &QString) -> QString;
    }

    impl cxx_qt::Threading for Monitor {}
    impl cxx_qt::Initialize for Monitor {}
}

use core::pin::Pin;
use cxx_qt::Threading;
use cxx_qt_lib::QString;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use treetop_core::command;
use treetop_core::engine::{pid_list, Engine};
use treetop_core::layout::{self, Histories};

/// What the update thread needs to turn rows into rich text.
struct Palette {
    dark: bool,
    text: String,
}

#[derive(Default)]
struct Latest {
    snap: Value,
    hist: Histories,
}

/// The QML-facing object: a thin wrapper over the shared `Engine`.
pub struct MonitorRust {
    engine: Engine,
    palette: Arc<Mutex<Palette>>,
    latest: Arc<Mutex<Latest>>,
}

impl Default for MonitorRust {
    fn default() -> Self {
        MonitorRust {
            engine: Engine::new(),
            palette: Arc::new(Mutex::new(Palette { dark: true, text: "#ffffff".into() })),
            latest: Arc::new(Mutex::new(Latest::default())),
        }
    }
}

fn list(s: &QString) -> Vec<String> {
    s.to_string().split(',').map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect()
}

/// Rows as the QML table takes them: cells and colours as \u{1} -joined
/// strings (one role each, cheap for ListModel), the command as rich text.
fn for_qml(row: &mut Value, p: &Palette) {
    let join = |v: &Value| {
        v.as_array().map(|a| a.iter().map(|x| x.as_str().unwrap_or("")).collect::<Vec<_>>().join("\u{1}")).unwrap_or_default()
    };
    let cells = join(&row["cells"]);
    let colors = join(&row["colors"]);
    let html = command::html(&command::spans_from_json(&row["cmd"]), p.dark, &p.text);
    let o = row.as_object_mut().unwrap();
    o.remove("cmd");
    o.insert("cells".into(), json!(cells));
    o.insert("colors".into(), json!(colors));
    o.insert("cmdHtml".into(), json!(html));
}

impl cxx_qt::Initialize for qobject::Monitor {
    fn initialize(self: Pin<&mut Self>) {
        let qt = self.qt_thread();
        let palette = self.palette.clone();
        let latest = self.latest.clone();
        self.engine.start(move |mut out| {
            if out["rowsOnly"].is_null() {
                let mut l = latest.lock().unwrap();
                l.hist.push(&out);
                let mut snap = out.clone();
                snap.as_object_mut().unwrap().remove("rows");
                l.snap = snap;
            }
            // (get_mut: indexing a missing key would insert it.)
            if let Some(changed) = out.get_mut("rows").and_then(|r| r.get_mut("changed")).and_then(Value::as_array_mut) {
                let p = palette.lock().unwrap();
                for pair in changed.iter_mut() {
                    for_qml(&mut pair[1], &p);
                }
            }
            let text = out.to_string();
            // Fails once the QObject is gone: stop sampling.
            qt.queue(move |obj| obj.updated(QString::from(&text))).is_ok()
        });
    }
}

impl qobject::Monitor {
    fn set_palette(&self, dark: bool, text: &QString) {
        let text = text.to_string();
        // Qt prints colours with alpha as #AARRGGBB; keep #RRGGBB.
        let text = if text.len() == 9 { format!("#{}", &text[3..]) } else { text };
        {
            let mut p = self.palette.lock().unwrap();
            if p.dark == dark && p.text == text {
                return;
            }
            *p = Palette { dark, text };
        }
        self.engine.resend();
    }

    fn set_store_style(&self, style: &QString) {
        self.engine.set_store_style(&style.to_string());
    }

    fn columns(&self) -> QString {
        QString::from(&treetop_core::columns::table_json().to_string())
    }

    fn pretty_command(&self, command: &QString, exe: &QString) -> QString {
        let p = self.palette.lock().unwrap();
        let lines = command::pretty(&command.to_string(), &exe.to_string());
        let html: Vec<String> = lines.iter().map(|l| command::html(l, p.dark, &p.text)).collect();
        QString::from(&format!("<span style='white-space:pre'>{}</span>", html.join("<br>")))
    }

    fn gpu_entries(&self, hidden: &QString, pinned: &QString) -> Vec<layout::GpuEntry> {
        let l = self.latest.lock().unwrap();
        layout::gpus(&l.snap, &l.hist.gpu, &list(hidden), &list(pinned))
    }

    fn gpu_state(&self, hidden: &QString, pinned: &QString) -> QString {
        let v: Vec<Value> = self
            .gpu_entries(hidden, pinned)
            .into_iter()
            .map(|g| json!({"index": g.index, "number": g.number, "pdev": g.pdev, "name": g.name, "idle": g.idle, "shown": g.shown, "reason": g.reason}))
            .collect();
        QString::from(&json!(v).to_string())
    }

    fn set_gpu_shown(&self, hidden: &QString, pinned: &QString, pdev: &QString, show: bool) -> QString {
        let pdev = pdev.to_string();
        let idle = self.gpu_entries(hidden, pinned).iter().any(|g| g.pdev == pdev && g.idle);
        let (h, p) = layout::set_gpu_shown(&list(hidden), &list(pinned), &pdev, show, idle);
        QString::from(&json!({"hidden": h.join(","), "pinned": p.join(",")}).to_string())
    }

    fn plan(&self, width: f64, gap: f64, point_size: f64, cpu_combined: bool, collapsed: &QString, shown_gpus: i32) -> QString {
        let l = self.latest.lock().unwrap();
        let collapsed = list(collapsed);
        let input = layout::PlanInput {
            width,
            gap,
            min_col: 300.0,
            core_col: 150.0,
            chars_per_unit: 1.0 / (point_size * 0.8).max(1.0),
            snap: &l.snap,
            cpu_combined,
            collapsed: &collapsed,
            shown_gpus: shown_gpus.max(0) as usize,
        };
        let v = match input.plan() {
            Some(p) => json!({"cols": p.cols, "colW": p.col_w, "coreCols": p.core_cols, "columns": p.columns}),
            None => Value::Null,
        };
        QString::from(&v.to_string())
    }

    fn summaries(&self, hidden: &QString, pinned: &QString) -> QString {
        let shown: Vec<_> = self.gpu_entries(hidden, pinned).into_iter().filter(|g| g.shown).collect();
        let l = self.latest.lock().unwrap();
        let mut v = json!({"summary": layout::summary(&l.snap, &shown).join("\n")});
        for s in ["cpu", "mem", "gpu", "storage", "sensors", "system"] {
            v[s] = json!(layout::section_summary(s, &l.snap, &shown));
        }
        QString::from(&v.to_string())
    }

    fn env_html(&self, name: &QString, value: &QString) -> QString {
        let p = self.palette.lock().unwrap();
        let lines = command::env_lines(&name.to_string(), &value.to_string());
        QString::from(&lines.iter().map(|l| command::html(l, p.dark, &p.text)).collect::<Vec<_>>().join("<br>"))
    }

    fn help_text(&self) -> QString {
        QString::from(&treetop_core::keys::help_text(treetop_core::keys::Frontend::Window))
    }

    fn set_interval(&self, ms: i32) {
        self.engine.set_interval(ms);
    }

    fn set_rows_enabled(&self, enabled: bool) {
        self.engine.set_rows_enabled(enabled);
    }

    fn refresh(&self) {
        self.engine.refresh();
    }

    fn toggle(&self, key: &QString) {
        self.engine.toggle(&key.to_string());
    }

    fn set_expanded_all(&self, expanded: bool) {
        self.engine.set_expanded_all(expanded);
    }

    fn set_subtree_expanded(&self, key: &QString, expanded: bool) {
        self.engine.set_subtree_expanded(&key.to_string(), expanded);
    }

    fn set_view(&self, view: &QString) {
        self.engine.set_view(&view.to_string());
    }

    fn set_sort(&self, key: &QString, descending: bool) {
        self.engine.set_sort(&key.to_string(), descending);
    }

    fn set_filter(&self, text: &QString) {
        self.engine.set_filter(&text.to_string());
    }

    fn set_show_kernel(&self, show: bool) {
        self.engine.set_show_kernel(show);
    }

    fn set_focus(&self, pid: i32) {
        self.engine.set_focus(pid);
    }

    fn set_frozen(&self, frozen: bool) {
        self.engine.set_frozen(frozen);
    }

    fn set_root_mode(&self, on: bool) {
        self.engine.set_root_mode(on);
    }

    fn inspect_as_root(&self, pid: i32) {
        let qt = self.qt_thread();
        self.engine.inspect_as_root(pid, move |v| {
            let text = v.to_string();
            let _ = qt.queue(move |obj| obj.inspected(QString::from(&text)));
        });
    }

    fn send_signal(&self, pids: &QString, signal: i32) -> QString {
        QString::from(&self.engine.send_signal(&pid_list(&pids.to_string()), signal))
    }

    fn renice(&self, pids: &QString, nice: i32) -> QString {
        QString::from(&self.engine.renice(&pid_list(&pids.to_string()), nice))
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
        QString::from(&self.engine.signal_list().to_string())
    }

    fn details(&self, pid: i32) -> QString {
        QString::from(&self.engine.details(pid).to_string())
    }
}
