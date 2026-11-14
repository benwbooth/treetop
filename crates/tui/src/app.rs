//! The terminal UI's state and everything it can do. Drawing is in ui.rs,
//! key and mouse handling in input.rs; both call into here.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use treetop_core::columns::{self, Column, Layout};
use treetop_core::layout::{self, GpuEntry, Histories};
use treetop_core::settings::{self, Settings};
use treetop_core::Engine;

pub enum Msg {
    Update(Value),
    Inspected(Value),
}

/// Everything a click, key or menu item can do.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    SetView(&'static str),
    ToggleView(&'static str),
    ExpandOrCollapseAll,
    ExpandAll,
    CollapseAll,
    EditFilter,
    ToggleKernel,
    ToggleRoot,
    SetInterval(i64),
    ChooseInterval,
    Options,
    ToggleResources,
    ToggleCards,
    ToggleMeters,
    ToggleCpuCombined,
    ToggleTint,
    ToggleSection(&'static str),
    SetGpuShown(String, bool),
    ChooseGpus,
    Signal(i32),
    SignalMenu,
    Renice(i32),
    ToggleFocusCurrent,
    Focus(i32),
    Details,
    ToggleCurrent,
    ClearTags,
    ToggleColumn(&'static str),
    ShowAllColumns,
    ColumnMenu,
    StoreStyle(&'static str),
    Help,
    Quit,
    Nothing,
}

#[derive(Clone, Debug)]
pub enum Item {
    Title(String),
    Act(String, Act),
    Check(String, bool, Act),
    Sub(String, Vec<Item>),
    Sep,
}

impl Item {
    pub fn label(&self) -> String {
        match self {
            Item::Title(t) | Item::Act(t, _) | Item::Sub(t, _) => t.clone(),
            Item::Check(t, on, _) => format!("{} {t}", if *on { "☑" } else { "☐" }),
            Item::Sep => String::new(),
        }
    }
    pub fn selectable(&self) -> bool {
        !matches!(self, Item::Title(_) | Item::Sep | Item::Act(_, Act::Nothing))
    }
}

#[derive(Clone, Debug)]
pub struct Menu {
    pub items: Vec<Item>,
    pub sel: usize,
    pub x: u16,
    pub y: u16,
}

impl Menu {
    pub fn new(items: Vec<Item>, x: u16, y: u16) -> Menu {
        let sel = items.iter().position(Item::selectable).unwrap_or(0);
        Menu { items, sel, x, y }
    }
    pub fn step(&mut self, d: i32) {
        let n = self.items.len() as i32;
        if n == 0 {
            return;
        }
        let mut i = self.sel as i32;
        for _ in 0..n {
            i = (i + d).rem_euclid(n);
            if self.items[i as usize].selectable() {
                self.sel = i as usize;
                return;
            }
        }
    }
}

pub struct Details {
    pub pid: i32,
    pub info: Value,
    pub tab: usize,
    pub scroll: [usize; 3],
    pub env_filter: String,
    pub editing_filter: bool,
    pub waiting_root: bool,
}

pub enum Popup {
    Menus(Vec<Menu>),
    Help(usize),
    Details(Box<Details>),
}

/// The options shared with the Qt app (treetop.conf [General]).
pub struct Opts {
    pub view: String,
    pub sort_key: String,
    pub sort_desc: bool,
    pub filter: String,
    pub show_kernel: bool,
    pub show_resources: bool,
    pub show_cards: bool,
    pub show_meters: bool,
    pub cpu_combined: bool,
    pub tint: bool,
    pub store_style: String,
    pub shorten: bool,
    pub hidden_cols: Vec<String>,
    pub widths: HashMap<String, usize>,
    pub order: Vec<String>,
    pub hidden_gpus: Vec<String>,
    pub pinned_gpus: Vec<String>,
    pub collapsed: Vec<String>,
    pub interval: i64,
    pub root_mode: bool,
}

impl Opts {
    pub fn load(s: &Settings) -> Opts {
        let mut hidden = s.list("hiddenColumns", columns::DEFAULT_HIDDEN);
        // The Qt app's one-time migration: TIME+ hidden by default.
        if s.f64_or("columnsVersion", 0.0) < 1.0 && !hidden.iter().any(|h| h == "time") {
            hidden.push("time".into());
        }
        let widths = s
            .json("columnWidths")
            .and_then(|v| v.as_object().cloned())
            .map(|m| m.into_iter().filter_map(|(k, v)| v.as_u64().map(|w| (k, w as usize))).collect())
            .unwrap_or_default();
        let order = s
            .json("columnOrder")
            .and_then(|v| v.as_array().cloned())
            .map(|a| a.into_iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        Opts {
            view: s.str_or("view", "tree"),
            sort_key: s.str_or("sortKey", "cpu"),
            sort_desc: s.bool_or("sortDesc", true),
            filter: s.str_or("filterText", ""),
            show_kernel: s.bool_or("showKernel", false),
            show_resources: s.bool_or("showResources", true),
            show_cards: s.bool_or("showCards", true),
            show_meters: s.bool_or("showMeters", true),
            cpu_combined: s.bool_or("cpuCombined", false),
            tint: s.bool_or("tintColumns", true),
            store_style: s.str_or("storeStyle", "compact"),
            shorten: s.bool_or("shortenStorePaths", true),
            hidden_cols: hidden,
            widths,
            order,
            hidden_gpus: s.list("hiddenGpus", ""),
            pinned_gpus: s.list("pinnedGpus", ""),
            collapsed: s.list("collapsedSections", ""),
            interval: s.f64_or("interval", 1000.0) as i64,
            root_mode: s.bool_or("rootMode", false),
        }
    }

    pub fn effective_store_style(&self) -> &str {
        if self.shorten { &self.store_style } else { "full" }
    }
}

pub enum Drag {
    /// Moving a column header: its key, and where the pointer is.
    Column(&'static str, u16),
    /// Resizing a column: its key, the pointer's start x, the start width.
    Resize(&'static str, u16, usize),
}

pub struct App {
    pub engine: Engine,
    pub tx: Sender<Msg>,
    pub opts: Opts,
    dirty: HashSet<&'static str>,
    pub snap: Value,
    pub hist: Histories,
    pub rows: Vec<Value>,
    pub keys: Vec<String>,
    pub collapsed_count: usize,
    pub cur_key: String,
    pub cur_pid: i32,
    /// First row shown, and how far rows are scrolled sideways.
    pub top: usize,
    pub hscroll: usize,
    /// Rows the table shows, and the screen row of its first (last draw).
    pub page: usize,
    pub table_y: u16,
    pub tagged: HashSet<i32>,
    pub status: Option<(String, Instant)>,
    pub editing_filter: bool,
    pub popup: Option<Popup>,
    pub frozen: bool,
    pub drag: Option<Drag>,
    pub mouse: Option<(u16, u16)>,
    pub mouse_moved: Instant,
    pub last_click: Option<(Instant, u16, u16)>,
    pub quit: bool,
    pub root_on: bool,
    pub root_starting: bool,
    pub signals: Vec<(String, i32)>,
    /// Asked for the sudo password: the main loop does that on the terminal.
    pub want_sudo: bool,
    restored: bool,
}

pub fn launcher() -> treetop_core::root::Launcher {
    // polkit needs an agent: a graphical session has one; SSH or a bare
    // console doesn't, so use sudo there (asked on this terminal).
    let graphical = std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some();
    let ssh = std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some();
    if graphical && !ssh {
        treetop_core::root::Launcher::Pkexec
    } else {
        treetop_core::root::Launcher::Sudo
    }
}

impl App {
    pub fn new(engine: Engine, tx: Sender<Msg>) -> App {
        let s = Settings::load();
        let opts = Opts::load(&s);
        let cur_key = s.str_or("selectedKey", "");
        let signals = engine
            .signal_list()
            .as_array()
            .map(|a| a.iter().map(|x| (x["name"].as_str().unwrap_or("").to_string(), x["value"].as_i64().unwrap_or(0) as i32)).collect())
            .unwrap_or_default();
        let app = App {
            engine,
            tx,
            opts,
            dirty: HashSet::new(),
            snap: json!({}),
            hist: Histories::default(),
            rows: Vec::new(),
            keys: Vec::new(),
            collapsed_count: 0,
            cur_key,
            cur_pid: 0,
            top: 0,
            hscroll: 0,
            page: 20,
            table_y: 0,
            tagged: HashSet::new(),
            status: None,
            editing_filter: false,
            popup: None,
            frozen: false,
            drag: None,
            mouse: None,
            mouse_moved: Instant::now(),
            last_click: None,
            quit: false,
            root_on: false,
            root_starting: false,
            signals,
            want_sudo: false,
            restored: false,
        };
        // The engine starts with the shared settings.
        let o = &app.opts;
        app.engine.set_view(&o.view);
        app.engine.set_sort(&o.sort_key, o.sort_desc);
        app.engine.set_filter(&o.filter);
        app.engine.set_show_kernel(o.show_kernel);
        app.engine.set_interval(o.interval as i32);
        app.engine.set_store_style(o.effective_store_style());
        app
    }

    /// Writes the options changed since the last save, merged onto the
    /// file as it is now (so the Qt app's other changes survive).
    pub fn save(&mut self) {
        if self.dirty.is_empty() {
            return;
        }
        let dirty = std::mem::take(&mut self.dirty);
        let o = &self.opts;
        let cur = self.cur_key.clone();
        let _ = settings::update(|s| {
            for k in dirty {
                match k {
                    "view" => s.set("view", &o.view),
                    "sortKey" => s.set("sortKey", &o.sort_key),
                    "sortDesc" => s.set_bool("sortDesc", o.sort_desc),
                    "filterText" => s.set("filterText", &o.filter),
                    "showKernel" => s.set_bool("showKernel", o.show_kernel),
                    "showResources" => s.set_bool("showResources", o.show_resources),
                    "showCards" => s.set_bool("showCards", o.show_cards),
                    "showMeters" => s.set_bool("showMeters", o.show_meters),
                    "cpuCombined" => s.set_bool("cpuCombined", o.cpu_combined),
                    "tintColumns" => s.set_bool("tintColumns", o.tint),
                    "storeStyle" => s.set("storeStyle", &o.store_style),
                    "hiddenColumns" => {
                        s.set_list("hiddenColumns", &o.hidden_cols);
                        s.set_num("columnsVersion", 1.0);
                    }
                    "columnWidths" => s.set_json("columnWidths", &json!(o.widths)),
                    "columnOrder" => s.set_json("columnOrder", &json!(o.order)),
                    "hiddenGpus" => s.set_list("hiddenGpus", &o.hidden_gpus),
                    "pinnedGpus" => s.set_list("pinnedGpus", &o.pinned_gpus),
                    "collapsedSections" => s.set_list("collapsedSections", &o.collapsed),
                    "interval" => s.set_num("interval", o.interval as f64),
                    "rootMode" => s.set_bool("rootMode", o.root_mode),
                    "selectedKey" => s.set("selectedKey", &cur),
                    _ => {}
                }
            }
        });
    }

    fn mark(&mut self, k: &'static str) {
        self.dirty.insert(k);
    }

    pub fn notify(&mut self, text: impl Into<String>) {
        let t = text.into();
        if !t.is_empty() {
            self.status = Some((t, Instant::now()));
        }
    }

    pub fn status_text(&self) -> Option<&str> {
        self.status.as_ref().filter(|(_, at)| at.elapsed() < Duration::from_secs(6)).map(|(t, _)| t.as_str())
    }

    // ---- updates from the engine ----

    pub fn on_msg(&mut self, m: Msg) {
        match m {
            Msg::Update(v) => self.apply(v),
            Msg::Inspected(v) => {
                if let Some(Popup::Details(d)) = &mut self.popup {
                    if v["pid"].as_i64() == Some(d.pid as i64) {
                        d.waiting_root = false;
                        if let Some(e) = v["error"].as_str() {
                            self.status = Some((e.to_string(), Instant::now()));
                        } else {
                            if d.info["environ"].is_null() {
                                d.info["environ"] = v["environ"].clone();
                            }
                            if d.info["files"].is_null() {
                                d.info["files"] = v["files"].clone();
                            }
                        }
                    }
                }
            }
        }
    }

    fn apply(&mut self, mut v: Value) {
        if v["rowsOnly"].as_bool() != Some(true) {
            self.hist.push(&v);
            for n in v["notices"].as_array().cloned().unwrap_or_default() {
                if let Some(t) = n.as_str() {
                    self.notify(t);
                }
            }
            self.root_on = v["root"].as_bool().unwrap_or(false);
            self.root_starting = v["rootStarting"].as_bool().unwrap_or(false);
            let rows = v.as_object_mut().and_then(|o| o.remove("rows"));
            self.snap = v.clone();
            if let Some(r) = rows {
                v["rows"] = r;
            }
        }
        if let Some(c) = v["collapsedCount"].as_u64() {
            self.collapsed_count = c as usize;
        }
        if let Some(d) = v.get("rows") {
            self.apply_rows(d);
        }
    }

    /// Patches rows in place; keeps what you're looking at on screen: the
    /// selected row if visible, else the top row, at the same offset.
    fn apply_rows(&mut self, d: &Value) {
        let count = d["count"].as_u64().unwrap_or(0) as usize;
        let at_top = self.top == 0;
        let sel = self.keys.iter().position(|k| *k == self.cur_key);
        let anchor = if at_top {
            None
        } else if let Some(s) = sel.filter(|s| *s >= self.top && *s < self.top + self.page) {
            Some((self.cur_key.clone(), s - self.top))
        } else {
            self.keys.get(self.top).map(|k| (k.clone(), 0))
        };
        for pair in d["changed"].as_array().into_iter().flatten() {
            let i = pair[0].as_u64().unwrap_or(0) as usize;
            let row = pair[1].clone();
            if i < self.rows.len() {
                self.rows[i] = row;
            } else {
                self.rows.push(row);
            }
        }
        self.rows.truncate(count);
        self.keys = self.rows.iter().map(|r| r["key"].as_str().unwrap_or("").to_string()).collect();
        if let Some((k, off)) = anchor {
            if let Some(j) = self.keys.iter().position(|x| *x == k) {
                self.top = j.saturating_sub(off);
            }
        }
        // The selected process went away: select what's now at its place.
        if !self.keys.iter().any(|k| *k == self.cur_key) {
            if let Some(s) = sel.or(if self.restored { None } else { Some(0) }) {
                if let Some(k) = self.keys.get(s.min(self.keys.len().saturating_sub(1))) {
                    self.cur_key = k.clone();
                }
            }
        }
        if !self.restored && !self.rows.is_empty() {
            // First rows: bring the saved selection into view.
            self.restored = true;
            if let Some(i) = self.cur_index() {
                self.top = i.saturating_sub(self.page / 3);
            }
        }
        self.cur_pid = self.cur_row().and_then(|r| r["pid"].as_i64()).unwrap_or(0) as i32;
        self.clamp_scroll();
    }

    // ---- derived ----

    pub fn view(&self) -> &str {
        &self.opts.view
    }

    pub fn gpus(&self) -> Vec<GpuEntry> {
        layout::gpus(&self.snap, &self.hist.gpu, &self.opts.hidden_gpus, &self.opts.pinned_gpus)
    }

    pub fn shown_gpus(&self) -> Vec<GpuEntry> {
        self.gpus().into_iter().filter(|g| g.shown).collect()
    }

    pub fn any_containers(&self) -> bool {
        self.snap["procs"]["containers"].as_u64().unwrap_or(0) > 0
    }

    pub fn visible_columns(&self) -> Vec<Column> {
        Layout {
            order: &self.opts.order,
            widths: &self.opts.widths,
            hidden: &self.opts.hidden_cols,
            view: &self.opts.view,
            any_containers: self.any_containers(),
        }
        .visible()
    }

    pub fn cur_index(&self) -> Option<usize> {
        self.keys.iter().position(|k| *k == self.cur_key)
    }

    pub fn cur_row(&self) -> Option<&Value> {
        self.cur_index().map(|i| &self.rows[i])
    }

    pub fn focus_pid(&self) -> i32 {
        self.snap["procs"]["focus"]["pid"].as_i64().unwrap_or(0) as i32
    }

    pub fn section_open(&self, s: &str) -> bool {
        !self.opts.collapsed.iter().any(|c| c == s)
    }

    /// Tagged processes, or the selected one.
    pub fn selected_pids(&self) -> Vec<i32> {
        if !self.tagged.is_empty() {
            let mut v: Vec<i32> = self.tagged.iter().copied().collect();
            v.sort();
            v
        } else if self.cur_pid > 0 {
            vec![self.cur_pid]
        } else {
            Vec::new()
        }
    }

    // ---- selection and scrolling ----

    pub fn select(&mut self, i: usize) {
        if self.rows.is_empty() {
            return;
        }
        let i = i.min(self.rows.len() - 1);
        self.cur_key = self.keys[i].clone();
        self.cur_pid = self.rows[i]["pid"].as_i64().unwrap_or(0) as i32;
        // Scroll only as far as needed to show it.
        if i < self.top {
            self.top = i;
        } else if i >= self.top + self.page.max(1) {
            self.top = i + 1 - self.page.max(1);
        }
        self.mark("selectedKey");
    }

    pub fn move_sel(&mut self, d: i64) {
        let i = self.cur_index().unwrap_or(0) as i64;
        let n = self.rows.len() as i64;
        if n > 0 {
            self.select((i + d).clamp(0, n - 1) as usize);
        }
    }

    pub fn scroll(&mut self, d: i64) {
        self.top = (self.top as i64 + d).max(0) as usize;
        self.clamp_scroll();
    }

    pub fn clamp_scroll(&mut self) {
        let max = self.rows.len().saturating_sub(self.page.max(1));
        self.top = self.top.min(max);
    }

    pub fn hscroll_by(&mut self, d: i64) {
        self.hscroll = (self.hscroll as i64 + d).clamp(0, 2000) as usize;
    }

    // ---- actions ----

    pub fn set_view(&mut self, v: &str) {
        self.opts.view = v.to_string();
        self.engine.set_view(v);
        self.hscroll = 0;
        self.mark("view");
        if v == "gpu" && self.opts.sort_key != "gpu" {
            self.opts.sort_key = String::new();
            self.sort_by("gpu");
        }
    }

    /// Clicking the sorted column reverses it; a new column starts in its
    /// natural direction.
    pub fn sort_by(&mut self, key: &str) {
        let key = columns::sort_key_for(key).to_string();
        if self.opts.sort_key == key {
            self.opts.sort_desc = !self.opts.sort_desc;
        } else {
            self.opts.sort_desc = !columns::sorts_ascending(&key);
            self.opts.sort_key = key;
        }
        self.engine.set_sort(&self.opts.sort_key, self.opts.sort_desc);
        self.mark("sortKey");
        self.mark("sortDesc");
    }

    /// P / M / T: always descending.
    pub fn sort_desc_by(&mut self, key: &str) {
        self.opts.sort_key = String::new();
        self.sort_by(key);
        if !self.opts.sort_desc {
            self.sort_by(key);
        }
    }

    pub fn set_filter(&mut self, f: String) {
        self.opts.filter = f;
        self.engine.set_filter(&self.opts.filter);
        self.mark("filterText");
    }

    pub fn toggle_current(&mut self) {
        if let Some(r) = self.cur_row() {
            if r["hasChildren"].as_bool() == Some(true) {
                let k = r["key"].as_str().unwrap_or("").to_string();
                self.engine.toggle(&k);
            }
        }
    }

    /// Expand (false) or collapse (true) the selected row.
    pub fn set_collapsed(&mut self, collapse: bool) {
        if let Some(r) = self.cur_row() {
            let expanded = r["expanded"].as_bool().unwrap_or(true);
            if r["hasChildren"].as_bool() == Some(true) && expanded == collapse {
                let k = r["key"].as_str().unwrap_or("").to_string();
                self.engine.toggle(&k);
            }
        }
    }

    pub fn toggle_focus(&mut self, pid: i32) {
        self.engine.set_focus(if pid > 0 && pid != self.focus_pid() { pid } else { 0 });
    }

    pub fn toggle_tag(&mut self, pid: i32) {
        if pid > 0 && !self.tagged.remove(&pid) {
            self.tagged.insert(pid);
        }
    }

    pub fn renice(&mut self, delta: i32) {
        let pids = self.selected_pids();
        if pids.is_empty() {
            return;
        }
        let ni = self.cur_row().and_then(|r| r["ni"].as_i64()).unwrap_or(0) as i32;
        let msg = self.engine.renice(&pids, (ni + delta).clamp(-20, 19));
        self.notify(msg);
    }

    pub fn signal(&mut self, sig: i32) {
        let pids = self.selected_pids();
        if pids.is_empty() {
            return;
        }
        let msg = self.engine.send_signal(&pids, sig);
        self.tagged.clear();
        self.notify(msg);
    }

    pub fn set_gpu_shown(&mut self, pdev: &str, show: bool) {
        let idle = self.gpus().iter().any(|g| g.pdev == pdev && g.idle);
        let (h, p) = layout::set_gpu_shown(&self.opts.hidden_gpus, &self.opts.pinned_gpus, pdev, show, idle);
        self.opts.hidden_gpus = h;
        self.opts.pinned_gpus = p;
        self.mark("hiddenGpus");
        self.mark("pinnedGpus");
    }

    pub fn toggle_section(&mut self, s: &str) {
        if let Some(i) = self.opts.collapsed.iter().position(|c| c == s) {
            self.opts.collapsed.remove(i);
        } else {
            self.opts.collapsed.push(s.to_string());
        }
        self.mark("collapsedSections");
    }

    pub fn set_root(&mut self, on: bool) {
        self.opts.root_mode = on;
        self.mark("rootMode");
        if on && launcher() == treetop_core::root::Launcher::Sudo {
            // The password is asked on this terminal, outside the screen.
            self.want_sudo = true;
        } else {
            self.engine.set_root_mode_with(on, launcher());
        }
    }

    pub fn set_column_width(&mut self, key: &str, chars: usize) {
        self.opts.widths.insert(key.to_string(), chars.clamp(2, 80));
        self.mark("columnWidths");
    }

    pub fn fit_all_columns(&mut self) {
        for c in self.visible_columns().into_iter().filter(|c| c.chars > 0) {
            let w = columns::fit_width(&c, &self.rows);
            self.opts.widths.insert(c.key.to_string(), w);
        }
        self.mark("columnWidths");
    }

    pub fn move_column(&mut self, key: &str, before: &str) {
        let all = Layout {
            order: &self.opts.order,
            widths: &self.opts.widths,
            hidden: &self.opts.hidden_cols,
            view: &self.opts.view,
            any_containers: self.any_containers(),
        }
        .ordered();
        self.opts.order = columns::move_column(&all, key, before);
        self.mark("columnOrder");
    }

    pub fn open_details(&mut self, pid: i32) {
        if pid <= 0 {
            return;
        }
        let info = self.engine.details(pid);
        self.popup = Some(Popup::Details(Box::new(Details {
            pid,
            info,
            tab: 0,
            scroll: [0; 3],
            env_filter: String::new(),
            editing_filter: false,
            waiting_root: false,
        })));
    }

    pub fn inspect_as_root(&mut self) {
        if let Some(Popup::Details(d)) = &mut self.popup {
            d.waiting_root = true;
            let tx = self.tx.clone();
            self.engine.inspect_as_root(d.pid, move |v| {
                let _ = tx.send(Msg::Inspected(v));
            });
        }
    }

    pub fn open_menu(&mut self, items: Vec<Item>, x: u16, y: u16) {
        self.popup = Some(Popup::Menus(vec![Menu::new(items, x, y)]));
    }

    // ---- menus (the same items as the Qt app's) ----

    pub fn signal_menu(&self) -> Vec<Item> {
        let pids = self.selected_pids();
        let title = if self.tagged.len() > 1 || (!self.tagged.is_empty() && !self.tagged.contains(&self.cur_pid)) {
            format!("{} tagged processes", self.tagged.len())
        } else if let Some(p) = pids.first() {
            format!("PID {p}")
        } else {
            "No process".to_string()
        };
        let focused = self.cur_pid > 0 && self.cur_pid == self.focus_pid();
        let sigs: Vec<Item> = self.signals.iter().map(|(n, v)| Item::Act(format!("{n} ({v})"), Act::Signal(*v))).collect();
        vec![
            Item::Title(title),
            Item::Act(if focused { "Show all processes (f)".into() } else { "Focus on this process tree (f)".into() }, Act::ToggleFocusCurrent),
            Item::Sep,
            Item::Act("Terminate (15)".into(), Act::Signal(15)),
            Item::Act("Kill (9)".into(), Act::Signal(9)),
            Item::Act("Stop (19)".into(), Act::Signal(19)),
            Item::Act("Continue (18)".into(), Act::Signal(18)),
            Item::Sub("Send signal".into(), sigs),
            Item::Sep,
            Item::Act("Nice +1".into(), Act::Renice(1)),
            Item::Act("Nice −1".into(), Act::Renice(-1)),
            Item::Sep,
            Item::Act("Details… (Enter)".into(), Act::Details),
            Item::Act("Collapse / expand".into(), Act::ToggleCurrent),
            Item::Act("Expand all".into(), Act::ExpandAll),
            Item::Act("Collapse all".into(), Act::CollapseAll),
            Item::Sep,
            if self.tagged.is_empty() { Item::Act("Clear tags".into(), Act::Nothing) } else { Item::Act("Clear tags".into(), Act::ClearTags) },
        ]
    }

    pub fn column_menu(&self) -> Vec<Item> {
        let mut v: Vec<Item> = columns::COLUMNS
            .iter()
            .filter(|c| c.key != "command")
            .map(|c| Item::Check(c.title.to_string(), !self.opts.hidden_cols.iter().any(|h| h == c.key), Act::ToggleColumn(c.key)))
            .collect();
        v.push(Item::Sep);
        let st = self.opts.effective_store_style().to_string();
        v.push(Item::Sub(
            "/nix/store paths".into(),
            vec![
                Item::Check("Full".into(), st == "full", Act::StoreStyle("full")),
                Item::Check("Short hash (/nix/store/abcde…-)".into(), st == "hash", Act::StoreStyle("hash")),
                Item::Check("Compact (/n…/s…/abcde…-)".into(), st == "compact", Act::StoreStyle("compact")),
            ],
        ));
        v.push(Item::Act("Show all columns".into(), Act::ShowAllColumns));
        v
    }

    pub fn gpu_items(&self) -> Vec<Item> {
        self.gpus()
            .into_iter()
            .map(|g| {
                let name = format!("GPU{} — {}{}", g.number, g.name, if g.idle { "  (idle)" } else { "" });
                Item::Check(name, g.shown, Act::SetGpuShown(g.pdev.clone(), !g.shown))
            })
            .collect()
    }

    pub fn options_menu(&self) -> Vec<Item> {
        let o = &self.opts;
        let root = if self.root_starting { "Root access (waiting for authorization…)" } else { "Root access" };
        vec![
            Item::Check(root.into(), self.root_on, if self.root_starting { Act::Nothing } else { Act::ToggleRoot }),
            Item::Sub("GPUs".into(), self.gpu_items()),
            Item::Sep,
            Item::Check("Show resources (r)".into(), o.show_resources, Act::ToggleResources),
            Item::Check("Graphs".into(), o.show_cards, Act::ToggleCards),
            Item::Check("Meters".into(), o.show_meters, Act::ToggleMeters),
            Item::Check("Combine CPU cores (1)".into(), o.cpu_combined, Act::ToggleCpuCombined),
            Item::Check("Tint resource columns".into(), o.tint, Act::ToggleTint),
            Item::Check("Kernel threads (K)".into(), o.show_kernel, Act::ToggleKernel),
            Item::Sub("Update".into(), self.interval_items()),
            Item::Sep,
            Item::Act("Keys… (h)".into(), Act::Help),
            Item::Act("Quit (q)".into(), Act::Quit),
        ]
    }

    pub fn interval_items(&self) -> Vec<Item> {
        [250, 500, 1000, 2000, 5000]
            .into_iter()
            .map(|ms| {
                let label = if ms < 1000 { format!("{} s", ms as f64 / 1000.0) } else { format!("{} s", ms / 1000) };
                Item::Check(label, self.opts.interval == ms, Act::SetInterval(ms))
            })
            .collect()
    }

    pub fn resources_menu(&self) -> Vec<Item> {
        let o = &self.opts;
        let mut v = vec![
            Item::Check("Graphs".into(), o.show_cards, Act::ToggleCards),
            Item::Check("Meters".into(), o.show_meters, Act::ToggleMeters),
            Item::Check("Combine CPU cores (1)".into(), o.cpu_combined, Act::ToggleCpuCombined),
            Item::Sep,
        ];
        for g in self.gpus() {
            let shown = !o.hidden_gpus.contains(&g.pdev);
            v.push(Item::Check(format!("Show GPU{} — {}", g.number, g.name), shown, Act::SetGpuShown(g.pdev.clone(), !shown)));
        }
        v
    }

    pub fn gpu_menu(&self, g: &GpuEntry) -> Vec<Item> {
        vec![
            Item::Act(format!("Hide GPU {}", g.number), Act::SetGpuShown(g.pdev.clone(), false)),
            Item::Act("GPU view (g)".into(), Act::ToggleView("gpu")),
        ]
    }

    /// Runs an action (from a key, a click or a menu).
    pub fn run(&mut self, a: Act) {
        match a {
            Act::SetView(v) => self.set_view(v),
            Act::ToggleView(v) => {
                let next = if self.opts.view == v { "tree" } else { v };
                self.set_view(next);
            }
            Act::ExpandOrCollapseAll => self.engine.set_expanded_all(self.collapsed_count > 0),
            Act::ExpandAll => self.engine.set_expanded_all(true),
            Act::CollapseAll => self.engine.set_expanded_all(false),
            Act::EditFilter => {
                self.popup = None;
                self.editing_filter = true;
            }
            Act::ToggleKernel => {
                self.opts.show_kernel = !self.opts.show_kernel;
                self.engine.set_show_kernel(self.opts.show_kernel);
                self.engine.refresh();
                self.mark("showKernel");
            }
            Act::ToggleRoot => {
                let on = !self.root_on;
                self.set_root(on);
            }
            Act::SetInterval(ms) => {
                self.opts.interval = ms;
                self.engine.set_interval(ms as i32);
                self.mark("interval");
            }
            Act::ChooseInterval => {
                let items = self.interval_items();
                self.open_menu(items, 0, 1);
            }
            Act::Options => {
                let items = self.options_menu();
                self.open_menu(items, u16::MAX, 1);
            }
            Act::ToggleResources => {
                self.opts.show_resources = !self.opts.show_resources;
                self.mark("showResources");
            }
            Act::ToggleCards => {
                self.opts.show_cards = !self.opts.show_cards;
                self.mark("showCards");
            }
            Act::ToggleMeters => {
                self.opts.show_meters = !self.opts.show_meters;
                self.mark("showMeters");
            }
            Act::ToggleCpuCombined => {
                self.opts.cpu_combined = !self.opts.cpu_combined;
                self.mark("cpuCombined");
            }
            Act::ToggleTint => {
                self.opts.tint = !self.opts.tint;
                self.mark("tintColumns");
            }
            Act::ToggleSection(s) => self.toggle_section(s),
            Act::SetGpuShown(p, show) => self.set_gpu_shown(&p, show),
            Act::ChooseGpus => {
                let mut items = vec![Item::Title("GPUs to show".into())];
                items.extend(self.gpu_items());
                self.open_menu(items, 2, 6);
            }
            Act::Signal(s) => self.signal(s),
            Act::SignalMenu => {
                let items = self.signal_menu();
                // At a third of the width, just below the selected row.
                let y = self.table_y + self.cur_index().map(|i| i.saturating_sub(self.top)).unwrap_or(0) as u16 + 1;
                self.open_menu(items, u16::MAX - 1, y);
            }
            Act::Renice(d) => self.renice(d),
            Act::ToggleFocusCurrent => {
                let p = self.cur_pid;
                self.toggle_focus(p);
            }
            Act::Focus(p) => self.engine.set_focus(p),
            Act::Details => {
                let p = self.cur_pid;
                self.open_details(p);
            }
            Act::ToggleCurrent => self.toggle_current(),
            Act::ClearTags => self.tagged.clear(),
            Act::ToggleColumn(k) => {
                self.opts.hidden_cols = columns::toggle_hidden(&self.opts.hidden_cols, k);
                self.mark("hiddenColumns");
            }
            Act::ShowAllColumns => {
                self.opts.hidden_cols.clear();
                self.mark("hiddenColumns");
            }
            Act::ColumnMenu => {
                let items = self.column_menu();
                self.open_menu(items, u16::MAX, 3);
            }
            Act::StoreStyle(s) => {
                self.opts.store_style = s.to_string();
                self.opts.shorten = true;
                self.engine.set_store_style(s);
                self.hscroll = 0;
                self.mark("storeStyle");
            }
            Act::Help => self.popup = Some(Popup::Help(0)),
            Act::Quit => self.quit = true,
            Act::Nothing => {}
        }
    }
}
