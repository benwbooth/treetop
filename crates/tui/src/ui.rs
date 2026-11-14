//! Draws the screen, mirroring the Qt window from top to bottom, and
//! records what's clickable where (`App::hits`) for input.rs.

use crate::app::{Act, App, Item, Popup};
use crate::theme::{self, hex};
use crate::widgets::{elide_middle, fill, fit, meter, sparkline, tint};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;
use serde_json::Value;
use treetop_core::columns::{self, Column};
use treetop_core::command::{self, Kind, Spans};
use treetop_core::format::{self, bytes, pct, rate, Rgb};
use treetop_core::keys::{self, FooterAction, Frontend};
use treetop_core::layout::{self, GpuEntry, PlanInput};
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Act(Act),
    /// Right-click menu for a GPU (by pdev).
    GpuCard(String),
    Row(usize),
    Expander(usize),
    /// The command cell of row i (its tooltip).
    Command(usize),
    /// A fixed cell (row, column key): tooltip when cut off.
    Cell(usize, &'static str),
    Sigma(usize),
    Header(&'static str),
    Gap(&'static str),
    Table,
    VBar,
    HBar,
    DetailsV,
    DetailsH,
    Menu(usize, usize),
    DetailsTab(usize),
    DetailsRoot,
    EnvFilter,
    Popup,
    Footer(FooterAction),
    Filter,
    /// Hover-only explanation.
    Tip(String),
}

pub struct Geometry {
    pub hits: Vec<(Rect, Hit)>,
    /// Columns as drawn: (key, x of its first cell before scrolling, width).
    pub cols: Vec<(&'static str, u16, u16)>,
    pub table: Rect,
    pub header_y: u16,
    /// Where the command column starts (before scrolling).
    pub cmd_x: u16,
    /// Scrollbars: (track, thumb start, thumb length) along the track.
    pub vbar: Option<(Rect, u16, u16)>,
    pub hbar: Option<(Rect, u16, u16)>,
    /// The details dialog's: bars, and (rows shown, lines, columns shown, widest).
    pub dvbar: Option<(Rect, u16, u16)>,
    pub dhbar: Option<(Rect, u16, u16)>,
    pub dsize: (usize, usize, usize, usize),
}

impl Geometry {
    pub fn at(&self, x: u16, y: u16) -> Option<&Hit> {
        // Last drawn wins (pop-ups are drawn on top).
        self.hits.iter().rev().find(|(r, _)| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height).map(|(_, h)| h)
    }
}

fn st(fg: Color) -> Style {
    Style::default().fg(fg)
}

fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style, max: u16) -> u16 {
    if max == 0 {
        return 0;
    }
    let (end, _) = buf.set_stringn(x, y, text, max as usize, style);
    end.saturating_sub(x)
}

fn n(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

pub fn draw(f: &mut Frame, app: &mut App) -> Geometry {
    let area = f.area();
    let buf = f.buffer_mut();
    fill(buf, area, theme::bg());
    let mut g = Geometry { hits: Vec::new(), cols: Vec::new(), table: Rect::default(), header_y: 0, cmd_x: 0, vbar: None, hbar: None, dvbar: None, dhbar: None, dsize: (0, 0, 0, 0) };
    if area.height < 6 || area.width < 30 {
        put(buf, 0, 0, "treetop: the terminal is too small", st(theme::text()), area.width);
        return g;
    }
    let mut y = 0u16;
    toolbar(buf, app, &mut g, area.width);
    y += 1;
    resources_header(buf, app, &mut g, y, area.width);
    y += 1;
    let footer_y = area.height - 1;
    if app.opts.show_resources {
        // Leave the table at least a third of the screen.
        let budget = footer_y.saturating_sub(y).saturating_sub((area.height / 3).max(6));
        let mut used = 0;
        if app.opts.show_cards && budget >= 5 {
            cards(buf, app, &mut g, Rect::new(0, y, area.width, 5));
            used += 5;
        }
        let rest = budget.saturating_sub(used);
        let r = Rect::new(1, y + used, area.width.saturating_sub(2), rest);
        let h = if app.view() == "gpu" { gpu_panels(buf, app, &mut g, r) } else if app.opts.show_meters { meters(buf, app, &mut g, r) } else { 0 };
        y += used + h;
    }
    if app.focus_pid() > 0 {
        focus_bar(buf, app, &mut g, y, area.width);
        y += 1;
    }
    table(buf, app, &mut g, Rect::new(0, y, area.width, footer_y - y));
    footer(buf, app, &mut g, footer_y, area.width);
    popups(buf, app, &mut g, area);
    tooltip(buf, app, &g, area);
    g
}

// ---- toolbar ----

fn toolbar(buf: &mut Buffer, app: &App, g: &mut Geometry, w: u16) {
    let y = 0;
    let mut x = 1;
    let item = |buf: &mut Buffer, g: &mut Geometry, x: &mut u16, text: &str, style: Style, hit: Hit| {
        let n = put(buf, *x, y, text, style, w.saturating_sub(*x));
        g.hits.push((Rect::new(*x, y, n, 1), hit));
        *x += n + 1;
    };
    for (label, v) in [("Tree", "tree"), ("List", "flat"), ("Containers", "containers"), ("GPU", "gpu")] {
        let on = app.view() == v;
        let style = if on { Style::default().bg(theme::over_bg(theme::HIGHLIGHT, 0.35)).fg(theme::text()).add_modifier(Modifier::BOLD) } else { st(theme::text()) };
        item(buf, g, &mut x, &format!(" {label} "), style, Hit::Act(Act::SetView(v)));
    }
    if matches!(app.view(), "tree" | "containers") {
        put(buf, x, y, "│", st(theme::dim()), 1);
        x += 2;
        let label = if app.collapsed_count > 0 { "⊞ Expand all" } else { "⊟ Collapse all" };
        item(buf, g, &mut x, label, st(theme::text()), Hit::Act(Act::ExpandOrCollapseAll));
    }
    put(buf, x, y, "│", st(theme::dim()), 1);
    x += 2;
    // The filter field.
    let fw: u16 = 32.min(w.saturating_sub(x + 40));
    if fw >= 8 {
        let editing = app.editing_filter;
        let bg = if editing { theme::over_bg(theme::HIGHLIGHT, 0.25) } else { theme::track() };
        tint(buf, x, y, fw, bg);
        let text = if app.opts.filter.is_empty() && !editing {
            put(buf, x + 1, y, &fit("Filter: name, command, user, container, PID (/)", fw as usize - 2), st(theme::dim()).bg(bg), fw - 2);
            String::new()
        } else {
            app.opts.filter.clone()
        };
        if !text.is_empty() || editing {
            let shown: String = text.chars().rev().take(fw as usize - 2).collect::<Vec<_>>().into_iter().rev().collect();
            let n = put(buf, x + 1, y, &shown, st(theme::text()).bg(bg), fw - 2);
            if editing {
                put(buf, x + 1 + n, y, "▏", st(theme::hex(theme::HIGHLIGHT)).bg(bg), 1);
            }
        }
        g.hits.push((Rect::new(x, y, fw, 1), Hit::Filter));
        x += fw + 2;
    }
    let k = if app.opts.show_kernel { "☑ Kernel threads" } else { "☐ Kernel threads" };
    item(buf, g, &mut x, k, st(theme::text()), Hit::Act(Act::ToggleKernel));
    // Right side: root, update interval, options.
    let iv = app.opts.interval;
    let ivs = if iv < 1000 { format!("{} s", iv as f64 / 1000.0) } else { format!("{} s", iv / 1000) };
    let root = if app.root_starting { "◌ root…" } else if app.root_on { "◉ root" } else { "○ root" };
    let right = [
        (root.to_string(), if app.root_on { st(hex(theme::ORANGE)).add_modifier(Modifier::BOLD) } else { st(theme::dim()) }, Hit::Act(Act::ToggleRoot)),
        (format!("Update {ivs} ▾"), st(theme::text()), Hit::Act(Act::ChooseInterval)),
        ("☰".to_string(), st(theme::text()).add_modifier(Modifier::BOLD), Hit::Act(Act::Options)),
    ];
    let total: u16 = right.iter().map(|(t, _, _)| t.chars().count() as u16 + 2).sum();
    let mut rx = w.saturating_sub(total).max(x);
    for (t, s, h) in right {
        let n = put(buf, rx, y, &t, s, w.saturating_sub(rx));
        g.hits.push((Rect::new(rx, y, n, 1), h));
        rx += n + 2;
    }
    // The root toggle explains itself on hover.
    let tip = "Root access: shows every process's disk I/O, GPU use, network traffic, environment and open files, and lets you signal and renice anyone's processes. Asks once (polkit, or sudo in a terminal without it).";
    if let Some((r, _)) = g.hits.iter().find(|(_, h)| *h == Hit::Act(Act::ToggleRoot)).cloned() {
        g.hits.push((r, Hit::Tip(tip.into())));
        g.hits.push((r, Hit::Act(Act::ToggleRoot)));
    }
}

fn resources_header(buf: &mut Buffer, app: &App, g: &mut Geometry, y: u16, w: u16) {
    let arrow = if app.opts.show_resources { "▾" } else { "▸" };
    let shown = app.shown_gpus();
    let summary = layout::summary(&app.snap, &shown).join(" · ");
    let n = put(buf, 1, y, &format!("{arrow} Resources "), st(theme::text()).add_modifier(Modifier::BOLD), w);
    put(buf, 1 + n, y, &summary, st(theme::dim()), w.saturating_sub(n + 4));
    put(buf, w - 2, y, "r", st(theme::dim()), 1);
    g.hits.push((Rect::new(0, y, w, 1), Hit::Act(Act::ToggleResources)));
}

// ---- cards ----

fn cards(buf: &mut Buffer, app: &App, g: &mut Geometry, r: Rect) {
    let s = &app.snap;
    let h = &app.hist;
    let shown = app.shown_gpus();
    struct Card<'a> {
        title: String,
        accent: &'static str,
        value: String,
        detail: String,
        series: Vec<(&'a [f64], &'static str)>,
        max: Option<f64>,
        mirror: bool,
        click: Option<Act>,
        gpu: Option<String>,
        hide: Option<String>,
    }
    let temp = n(&s["cpu"]["temp"]);
    let mut v = vec![Card {
        title: "CPU".into(),
        accent: theme::GREEN,
        value: pct(n(&s["cpu"]["total"])),
        detail: format!("{}load {}", if temp > 0.0 { format!("{}°C · ", format::fixed(temp, 0)) } else { String::new() }, layout::load(s)),
        series: vec![(&h.cpu, theme::GREEN)],
        max: Some(100.0),
        mirror: false,
        click: Some(Act::ToggleCpuCombined),
        gpu: None,
        hide: None,
    }];
    let mt = n(&s["mem"]["total"]);
    v.push(Card {
        title: "Memory".into(),
        accent: theme::YELLOW,
        value: if mt > 0.0 { pct(n(&s["mem"]["used"]) / mt * 100.0) } else { String::new() },
        detail: format!("{} / {}{}", bytes(n(&s["mem"]["used"])), bytes(mt), if n(&s["mem"]["swapUsed"]) > 0.0 { format!(" · swap {}", bytes(n(&s["mem"]["swapUsed"]))) } else { String::new() }),
        series: vec![(&h.mem, theme::YELLOW)],
        max: Some(100.0),
        mirror: false,
        click: None,
        gpu: None,
        hide: None,
    });
    for e in &shown {
        let gp = &s["gpus"][e.index];
        let empty: &[f64] = &[];
        v.push(Card {
            title: format!("▾ GPU {}", e.number),
            accent: theme::MAGENTA,
            value: format!("{}%", format::fixed(n(&gp["busy"]), 0)),
            detail: format!(
                "{}{}",
                if n(&gp["temp"]) > 0.0 { format!("{}°C", format::fixed(n(&gp["temp"]), 0)) } else { String::new() },
                if n(&gp["power"]) > 0.0 { format!(" · {}W", format::fixed(n(&gp["power"]), 0)) } else { String::new() }
            ),
            series: vec![(h.gpu.get(e.index).map(|x| x.as_slice()).unwrap_or(empty), theme::MAGENTA)],
            max: Some(100.0),
            mirror: false,
            click: Some(Act::ToggleView("gpu")),
            gpu: Some(e.pdev.clone()),
            hide: Some(e.pdev.clone()),
        });
        v.push(Card {
            title: format!("VRAM {}", e.number),
            accent: theme::PINK,
            value: format!("{}%", format::fixed(n(&gp["vramUsed"]) / n(&gp["vramTotal"]).max(1.0) * 100.0, 0)),
            detail: format!("{} / {}{}", bytes(n(&gp["vramUsed"])), bytes(n(&gp["vramTotal"])), if n(&gp["gttUsed"]) > 0.0 { format!(" · GTT {}", bytes(n(&gp["gttUsed"]))) } else { String::new() }),
            series: vec![(h.vram.get(e.index).map(|x| x.as_slice()).unwrap_or(empty), theme::PINK)],
            max: Some(100.0),
            mirror: false,
            click: Some(Act::ToggleView("gpu")),
            gpu: Some(e.pdev.clone()),
            hide: None,
        });
    }
    let (rd, wr) = (n(&s["disk"]["read"]), n(&s["disk"]["write"]));
    v.push(Card {
        title: "Disk".into(),
        accent: theme::CYAN,
        value: rate(rd + wr),
        detail: format!("R {} · W {}", rate(rd), rate(wr)),
        series: vec![(&h.rd, theme::CYAN), (&h.wr, theme::ORANGE)],
        max: None,
        mirror: true,
        click: None,
        gpu: None,
        hide: None,
    });
    let (rx, tx) = (n(&s["net"]["rx"]), n(&s["net"]["tx"]));
    v.push(Card {
        title: "Network".into(),
        accent: theme::BLUE,
        value: rate(rx + tx),
        detail: format!("↓ {} · ↑ {}", rate(rx), rate(tx)),
        series: vec![(&h.rx, theme::BLUE), (&h.tx, theme::RED)],
        max: None,
        mirror: true,
        click: None,
        gpu: None,
        hide: None,
    });
    let count = v.len() as u16;
    let cw = (r.width.saturating_sub(1)) / count.max(1);
    for (i, c) in v.iter().enumerate() {
        let x = r.x + 1 + i as u16 * cw;
        let w = cw.saturating_sub(1);
        if w < 8 {
            continue;
        }
        let card = Rect::new(x, r.y, w, r.height);
        let bg = theme::over_bg(c.accent, 0.08);
        fill(buf, card, bg);
        let border = theme::over_bg(c.accent, 0.35);
        // Rounded frame.
        for xx in x..x + w {
            buf[(xx, r.y)].set_char('─').set_fg(border);
            buf[(xx, r.y + r.height - 1)].set_char('─').set_fg(border);
        }
        for yy in r.y..r.y + r.height {
            buf[(x, yy)].set_char('│').set_fg(border);
            buf[(x + w - 1, yy)].set_char('│').set_fg(border);
        }
        buf[(x, r.y)].set_char('╭');
        buf[(x + w - 1, r.y)].set_char('╮');
        buf[(x, r.y + r.height - 1)].set_char('╰');
        buf[(x + w - 1, r.y + r.height - 1)].set_char('╯');
        let tn = put(buf, x + 1, r.y, &format!(" {} ", c.title), st(hex(c.accent)).bg(bg).add_modifier(Modifier::BOLD), w - 2);
        let vw = c.value.chars().count() as u16;
        if vw + tn + 3 < w {
            put(buf, x + w - 2 - vw, r.y, &c.value, st(theme::text()).bg(bg).add_modifier(Modifier::BOLD), vw);
        }
        put(buf, x + 1, r.y + 1, &fit(&c.detail, (w - 2) as usize), st(theme::dim()).bg(bg), w - 2);
        let series: Vec<(&[f64], Color)> = c.series.iter().map(|(s, col)| (*s, hex(col))).collect();
        sparkline(buf, Rect::new(x + 1, r.y + 2, w - 2, r.height - 3), &series, c.max, c.mirror);
        if let Some(p) = &c.hide {
            g.hits.push((Rect::new(x + 2, r.y, 1, 1), Hit::Act(Act::SetGpuShown(p.clone(), false))));
        }
        if let Some(p) = &c.gpu {
            let name = app.gpus().into_iter().find(|e| &e.pdev == p).map(|e| e.name).unwrap_or_default();
            g.hits.push((card, Hit::Tip(format!("{name}\nClick for the GPU view · right-click to hide"))));
            g.hits.push((card, Hit::GpuCard(p.clone())));
        }
        if let Some(a) = &c.click {
            g.hits.push((card, Hit::Act(a.clone())));
        }
        // The hide ▾ wins over the card click.
        if let Some(p) = &c.hide {
            g.hits.push((Rect::new(x + 2, r.y, 1, 1), Hit::Act(Act::SetGpuShown(p.clone(), false))));
        }
    }
}

// ---- meter sections ----

fn section_header(buf: &mut Buffer, g: &mut Geometry, x: u16, y: u16, w: u16, title: &str, open: bool, summary: &str, toggle: Act, action: Option<(String, Act, String)>) {
    let arrow = if open { "▾" } else { "▸" };
    let n = put(buf, x, y, &format!("{arrow} {title}"), st(hex(theme::CYAN)).add_modifier(Modifier::BOLD), w);
    let mut aw = 0;
    if let Some((text, act, tip)) = &action {
        aw = text.chars().count() as u16 + 1;
        let ax = x + w - aw + 1;
        put(buf, ax, y, text, st(hex(theme::MAGENTA)).add_modifier(Modifier::UNDERLINED), aw);
        g.hits.push((Rect::new(ax, y, aw, 1), Hit::Tip(tip.clone())));
        g.hits.push((Rect::new(ax, y, aw, 1), Hit::Act(act.clone())));
    }
    put(buf, x + n + 1, y, summary, st(theme::dim()), w.saturating_sub(n + 1 + aw));
    g.hits.insert(0, (Rect::new(x, y, w.saturating_sub(aw), 1), Hit::Act(toggle)));
}

fn cpu_segs(v: &Value) -> Vec<(f64, Color)> {
    // [user, nice, system, other] as nice (blue), user (green), system (red), other (orange).
    vec![(n(&v[1]), hex(theme::BLUE)), (n(&v[0]), hex(theme::GREEN)), (n(&v[2]), hex(theme::RED)), (n(&v[3]), hex(theme::ORANGE))]
}

/// Draws one section; returns the lines it took.
fn section(buf: &mut Buffer, app: &App, g: &mut Geometry, name: &str, x: u16, y: u16, w: u16, core_cols: usize, max_y: u16) -> u16 {
    let s = &app.snap;
    let shown = app.shown_gpus();
    let sum = layout::section_summary(name, s, &shown);
    let mut l = 0u16;
    let line = |l: u16| y + l < max_y;
    match name {
        "cpu" => {
            section_header(buf, g, x, y, w, "CPU", !app.opts.cpu_combined, &sum, Act::ToggleCpuCombined, None);
            l += 1;
            let c = &s["cpu"];
            if app.opts.cpu_combined {
                if line(l) {
                    let v = [c["user"].clone(), c["nice"].clone(), c["system"].clone(), c["other"].clone()];
                    let text = format!("usr {} sys {}  {}", pct(n(&c["user"])), pct(n(&c["system"])), pct(n(&c["total"])));
                    meter(buf, x, y + l, w, "all", 6, &cpu_segs(&Value::Array(v.to_vec())), 100.0, &text);
                }
                l += 1;
            } else {
                let cores = c["cores"].as_array().cloned().unwrap_or_default();
                let cols = core_cols.max(1);
                let rows = cores.len().div_ceil(cols);
                let cw = (w + 1) / cols as u16;
                for (i, core) in cores.iter().enumerate() {
                    let (col, row) = (i / rows.max(1), i % rows.max(1));
                    if !line(l + row as u16) {
                        continue;
                    }
                    let total = n(&core[0]) + n(&core[1]) + n(&core[2]) + n(&core[3]);
                    meter(buf, x + col as u16 * cw, y + l + row as u16, cw.saturating_sub(1), &i.to_string(), 3, &cpu_segs(core), 100.0, &pct(total));
                }
                l += rows as u16;
            }
        }
        "mem" => {
            let open = app.section_open("mem");
            section_header(buf, g, x, y, w, "Memory", open, &sum, Act::ToggleSection("mem"), None);
            l += 1;
            if open {
                let m = &s["mem"];
                let total = n(&m["total"]).max(1.0);
                if line(l) {
                    let segs = [(n(&m["used"]), hex(theme::GREEN)), (n(&m["buffers"]), hex(theme::BLUE)), (n(&m["cached"]), hex(theme::YELLOW))];
                    meter(buf, x, y + l, w, "Mem", 5, &segs, total, &format!("{}/{}", bytes(n(&m["used"])), bytes(total)));
                }
                l += 1;
                if n(&m["swapTotal"]) > 0.0 {
                    if line(l) {
                        meter(buf, x, y + l, w, "Swp", 5, &[(n(&m["swapUsed"]), hex(theme::RED))], n(&m["swapTotal"]), &format!("{}/{}", bytes(n(&m["swapUsed"])), bytes(n(&m["swapTotal"]))));
                    }
                    l += 1;
                }
            }
        }
        "gpu" => {
            let open = app.section_open("gpu");
            let hidden: Vec<GpuEntry> = app.gpus().into_iter().filter(|e| !e.shown).collect();
            let action = (!hidden.is_empty()).then(|| {
                let tip = hidden.iter().map(|e| format!("GPU{} {} ({})", e.number, e.name, e.reason)).collect::<Vec<_>>().join("\n") + "\nClick to choose which GPUs to show";
                (format!("+{} hidden", hidden.len()), Act::ChooseGpus, tip)
            });
            section_header(buf, g, x, y, w, "GPU", open, &sum, Act::ToggleSection("gpu"), action);
            l += 1;
            if open {
                for e in &shown {
                    let gp = &s["gpus"][e.index];
                    if line(l) {
                        let n1 = put(buf, x, y + l, &format!("▾ GPU{}  ", e.number), st(theme::text()), w);
                        put(buf, x + n1, y + l, &e.name, st(theme::dim()), w.saturating_sub(n1));
                        g.hits.push((Rect::new(x, y + l, w, 1), Hit::Tip(format!("{}\n▾ hides it · the GPU card or g opens the GPU view", e.name))));
                        g.hits.push((Rect::new(x, y + l, w, 1), Hit::Act(Act::SetGpuShown(e.pdev.clone(), false))));
                    }
                    l += 1;
                    if line(l) {
                        let text = format!(
                            "{}%{}{}",
                            format::fixed(n(&gp["busy"]), 0),
                            if n(&gp["temp"]) > 0.0 { format!(" {}°C", format::fixed(n(&gp["temp"]), 0)) } else { String::new() },
                            if n(&gp["power"]) > 0.0 { format!(" {}W", format::fixed(n(&gp["power"]), 0)) } else { String::new() }
                        );
                        meter(buf, x, y + l, w, "busy", 6, &[(n(&gp["busy"]), hex(theme::MAGENTA))], 100.0, &text);
                    }
                    l += 1;
                    if line(l) {
                        meter(buf, x, y + l, w, "VRAM", 6, &[(n(&gp["vramUsed"]), hex(theme::PINK))], n(&gp["vramTotal"]).max(1.0), &format!("{}/{}", bytes(n(&gp["vramUsed"])), bytes(n(&gp["vramTotal"]))));
                    }
                    l += 1;
                }
            }
        }
        "storage" => {
            let open = app.section_open("storage");
            section_header(buf, g, x, y, w, "Storage", open, &sum, Act::ToggleSection("storage"), None);
            l += 1;
            if open {
                let lw = (w / 3).clamp(6, 16);
                for fs in s["disk"]["filesystems"].as_array().cloned().unwrap_or_default() {
                    if line(l) {
                        let (used, total) = (n(&fs["used"]), n(&fs["total"]).max(1.0));
                        let color = if used / total > 0.9 { theme::RED } else { theme::CYAN };
                        let mount = fs["mount"].as_str().unwrap_or("");
                        meter(buf, x, y + l, w, &format!("{} ", elide_middle(mount, lw as usize - 1)), lw, &[(used, hex(color))], total, &format!("{}/{}", bytes(used), bytes(total)));
                        let tip = format!("{mount} · {} ({})", fs["device"].as_str().unwrap_or(""), fs["type"].as_str().unwrap_or(""));
                        g.hits.push((Rect::new(x, y + l, w, 1), Hit::Tip(tip)));
                    }
                    l += 1;
                }
            }
        }
        "system" => {
            let open = app.section_open("system");
            section_header(buf, g, x, y, w, "System", open, &sum, Act::ToggleSection("system"), None);
            l += 1;
            if open {
                let p = &s["procs"];
                let cyan = st(hex(theme::CYAN));
                let bold = st(theme::text()).add_modifier(Modifier::BOLD);
                if line(l) {
                    let mut xx = x;
                    let mut seg = |buf: &mut Buffer, t: &str, s: Style| {
                        xx += put(buf, xx, y + l, t, s, (x + w).saturating_sub(xx));
                    };
                    seg(buf, "Tasks: ", cyan);
                    seg(buf, &p["tasks"].as_i64().unwrap_or(0).to_string(), bold);
                    seg(buf, &format!(", {} thr, {} kthr; ", p["threads"].as_i64().unwrap_or(0), p["kthreads"].as_i64().unwrap_or(0)), st(theme::text()));
                    seg(buf, &format!("{} running", p["running"].as_i64().unwrap_or(0)), st(hex(theme::GREEN)).add_modifier(Modifier::BOLD));
                    let c = p["containers"].as_i64().unwrap_or(0);
                    if c > 0 {
                        seg(buf, "; ", st(theme::text()));
                        seg(buf, &format!("{c} containers"), st(hex(theme::MAGENTA)).add_modifier(Modifier::BOLD));
                    }
                }
                l += 1;
                if line(l) {
                    let mut xx = x;
                    for (t, s) in [("Load: ", cyan), (&layout::load(s)[..], bold), ("  Up: ", cyan), (&format::uptime(n(&s["uptime"]))[..], bold)] {
                        xx += put(buf, xx, y + l, t, s, (x + w).saturating_sub(xx));
                    }
                }
                l += 1;
                if line(l) {
                    let mut xx = x + put(buf, x, y + l, "Net: ", cyan, w);
                    for i in s["net"]["interfaces"].as_array().cloned().unwrap_or_default().iter().filter(|i| i["physical"].as_bool() == Some(true)) {
                        xx += put(buf, xx, y + l, i["name"].as_str().unwrap_or(""), bold, (x + w).saturating_sub(xx));
                        xx += put(buf, xx, y + l, &format!(" ↓{} ↑{}  ", rate(n(&i["rx"])), rate(n(&i["tx"]))), st(theme::text()), (x + w).saturating_sub(xx));
                    }
                }
                l += 1;
            }
        }
        "sensors" => {
            let open = app.section_open("sensors");
            section_header(buf, g, x, y, w, "Sensors", open, &sum, Act::ToggleSection("sensors"), None);
            l += 1;
            if open {
                for (dev, items) in layout::sensor_groups(s) {
                    // Device name, then each reading; wraps rather than cutting off.
                    let mut parts: Vec<(String, Style)> = vec![(format!("{dev} "), st(hex(theme::CYAN)).add_modifier(Modifier::BOLD))];
                    for it in &items {
                        let v = n(&it["value"]);
                        let (val, col) = if it["kind"] == "fan" {
                            (format!("{}rpm", format::fixed(v, 0)), hex(theme::BLUE))
                        } else {
                            (format!("{}°", format::fixed(v, 0)), hex(format::temp_color(v)))
                        };
                        parts.push((format!(" {} ", it["label"].as_str().unwrap_or("")), st(theme::dim())));
                        parts.push((val, st(col).add_modifier(Modifier::BOLD)));
                    }
                    let mut xx = x;
                    for (t, sty) in parts {
                        let tw = t.chars().count() as u16;
                        if xx + tw > x + w && xx > x {
                            l += 1;
                            xx = x + 2;
                        }
                        if line(l) {
                            xx += put(buf, xx, y + l, &t, sty, (x + w).saturating_sub(xx));
                        }
                    }
                    l += 1;
                }
            }
        }
        _ => {}
    }
    l
}

fn meters(buf: &mut Buffer, app: &App, g: &mut Geometry, r: Rect) -> u16 {
    if r.height == 0 || r.width < 20 {
        return 0;
    }
    let shown = app.shown_gpus();
    let input = PlanInput {
        width: r.width as f64,
        gap: 2.0,
        min_col: 38.0,
        core_col: 19.0,
        chars_per_unit: 1.0,
        snap: &app.snap,
        cpu_combined: app.opts.cpu_combined,
        collapsed: &app.opts.collapsed,
        shown_gpus: shown.len(),
    };
    let Some(plan) = input.plan() else { return 0 };
    let mut tallest = 0;
    for (ci, col) in plan.columns.iter().enumerate() {
        let x = r.x + ci as u16 * (plan.col_w as u16 + 2);
        let mut y = r.y;
        for name in col {
            if y >= r.y + r.height {
                break;
            }
            let used = section(buf, app, g, name, x, y, plan.col_w as u16, plan.core_cols, r.y + r.height);
            y += used;
        }
        tallest = tallest.max(y - r.y);
    }
    tallest.min(r.height)
}

// ---- GPU view ----

fn gpu_panels(buf: &mut Buffer, app: &App, g: &mut Geometry, r: Rect) -> u16 {
    let s = &app.snap;
    let mut y = r.y;
    let bottom = r.y + r.height;
    let names = ["GFX", "Compute", "DMA", "Decode", "Encode"];
    for e in app.shown_gpus() {
        if y + 3 > bottom {
            break;
        }
        let gp = &s["gpus"][e.index];
        let hn = put(buf, r.x, y, "▾ ", st(theme::text()), 2);
        g.hits.push((Rect::new(r.x, y, 2, 1), Hit::Act(Act::SetGpuShown(e.pdev.clone(), false))));
        let mut x = r.x + hn;
        x += put(buf, x, y, &format!("GPU{} ", e.number), st(hex(theme::MAGENTA)).add_modifier(Modifier::BOLD), r.width);
        x += put(buf, x, y, &format!("{}  ", e.name), st(theme::text()), r.width.saturating_sub(x - r.x));
        let mut info = gp["link"].as_str().unwrap_or("").to_string();
        if let Some(p) = gp["perfLevel"].as_str().filter(|p| !p.is_empty()) {
            info += &format!("  ·  dpm {p}");
        }
        info += &format!("  ·  {}", e.pdev);
        put(buf, x, y, &info, st(theme::dim()), (r.x + r.width).saturating_sub(x));
        g.hits.push((Rect::new(r.x, y, r.width, 1), Hit::GpuCard(e.pdev.clone())));
        y += 1;
        let lw = (r.width / 3).max(24);
        let kw = 28u16.min(r.width / 4);
        let gx = r.x + lw + 2;
        let sx = gx + kw + 2;
        let mut ly = y;
        let m = |buf: &mut Buffer, ly: &mut u16, label: &str, segs: &[(f64, Color)], max: f64, text: &str| {
            if *ly < bottom {
                meter(buf, r.x, *ly, lw, label, 9, segs, max, text);
            }
            *ly += 1;
        };
        let v = |x: &Value, unit: &str, d: usize| if x.is_null() { "—".to_string() } else { format!("{}{unit}", format::fixed(n(x), d)) };
        m(buf, &mut ly, "Busy", &[(n(&gp["busy"]), hex(theme::MAGENTA))], 100.0, &v(&gp["busy"], "%", 0));
        if !gp["memBusy"].is_null() {
            m(buf, &mut ly, "Mem bus", &[(n(&gp["memBusy"]), hex(theme::CYAN))], 100.0, &v(&gp["memBusy"], "%", 0));
        }
        for (i, name) in names.iter().enumerate() {
            let val = n(&gp["engines"][i]);
            let shade = Rgb::hex(theme::MAGENTA).lighter(1.0 + i as f64 * 0.12);
            m(buf, &mut ly, name, &[(val, theme::c(shade))], 100.0, &format!("{}%", format::fixed(val, 0)));
        }
        let un = n(&gp["unattributed"]);
        if un >= 20.0 && ly < bottom {
            let t = format!("⚠ {}% of the busy time isn't from any process treetop can see", format::fixed(un, 0));
            put(buf, r.x, ly, &t, st(hex(theme::RED)), lw + kw);
            g.hits.push((Rect::new(r.x, ly, lw, 1), Hit::Tip("ROCm compute work (processes marked ROCm in the engines column), other users' processes without root access, or a stuck GPU job.".into())));
            ly += 1;
        }
        m(buf, &mut ly, "VRAM", &[(n(&gp["vramUsed"]), hex(theme::PINK))], n(&gp["vramTotal"]).max(1.0), &format!("{}/{}", bytes(n(&gp["vramUsed"])), bytes(n(&gp["vramTotal"]))));
        if n(&gp["gttTotal"]) > 0.0 {
            m(buf, &mut ly, "GTT", &[(n(&gp["gttUsed"]), hex(theme::CYAN))], n(&gp["gttTotal"]), &format!("{}/{}", bytes(n(&gp["gttUsed"])), bytes(n(&gp["gttTotal"]))));
        }
        // The label/value grid.
        let fan = if gp["fanRpm"].is_null() {
            "—".to_string()
        } else {
            format!("{}{}", v(&gp["fanRpm"], " rpm", 0), if gp["fanPct"].is_null() { String::new() } else { format!(" ({})", v(&gp["fanPct"], "%", 0)) })
        };
        let pairs = [
            ("Shader clock", v(&gp["sclk"], " MHz", 0)),
            ("Memory clock", v(&gp["mclk"], " MHz", 0)),
            ("Edge temp", v(&gp["temp"], "°C", 0)),
            ("Junction", v(&gp["tempJunction"], "°C", 0)),
            ("Memory temp", v(&gp["tempMem"], "°C", 0)),
            ("Power", if gp["power"].is_null() { "—".into() } else { format!("{} / {} W", format::fixed(n(&gp["power"]), 0), format::fixed(n(&gp["powerCap"]), 0)) }),
            ("Voltage", v(&gp["voltage"], " mV", 0)),
            ("Fan", fan),
        ];
        for (i, (k, val)) in pairs.iter().enumerate() {
            let yy = y + i as u16;
            if yy < bottom && kw > 10 {
                put(buf, gx, yy, k, st(theme::dim()), kw);
                let vw = val.chars().count() as u16;
                put(buf, gx + kw.saturating_sub(vw), yy, val, st(theme::text()), vw);
            }
        }
        let h = (ly - y).max(pairs.len() as u16).min(bottom.saturating_sub(y));
        let sw = (r.x + r.width).saturating_sub(sx);
        if sw > 8 && h > 1 {
            let empty: Vec<f64> = Vec::new();
            let busy = app.hist.gpu.get(e.index).unwrap_or(&empty);
            let vram = app.hist.vram.get(e.index).unwrap_or(&empty);
            sparkline(buf, Rect::new(sx, y, sw, h), &[(busy, hex(theme::MAGENTA)), (vram, hex(theme::CYAN))], Some(100.0), false);
        }
        y += h + 1;
    }
    // Hidden or idle GPUs leave a way back.
    for e in app.gpus().into_iter().filter(|e| !e.shown) {
        if y >= bottom {
            break;
        }
        let t = format!("▸ GPU{} ({}) is {}  + show", e.number, e.name, e.reason);
        let n1 = put(buf, r.x, y, &t, st(theme::dim()), r.width);
        g.hits.push((Rect::new(r.x, y, n1, 1), Hit::Act(Act::SetGpuShown(e.pdev.clone(), true))));
        y += 1;
    }
    y - r.y
}

// ---- focus bar ----

fn focus_bar(buf: &mut Buffer, app: &App, g: &mut Geometry, y: u16, w: u16) {
    let bg = theme::over_bg(theme::CYAN, 0.14);
    tint(buf, 0, y, w, bg);
    let f = &app.snap["procs"]["focus"];
    let mut x = 1;
    let seg = |buf: &mut Buffer, g: &mut Geometry, x: &mut u16, t: &str, s: Style, hit: Option<Hit>| {
        let n1 = put(buf, *x, y, t, s.bg(bg), w.saturating_sub(*x));
        if let Some(h) = hit {
            g.hits.push((Rect::new(*x, y, n1, 1), h));
        }
        *x += n1;
    };
    seg(buf, g, &mut x, "⌂ All", st(hex(theme::CYAN)).add_modifier(Modifier::UNDERLINED), Some(Hit::Act(Act::Focus(0))));
    let chain: Vec<Value> = f["chain"].as_array().cloned().unwrap_or_default();
    for a in chain.iter().rev() {
        seg(buf, g, &mut x, " ▸ ", st(theme::dim()), None);
        let pid = a["pid"].as_i64().unwrap_or(0) as i32;
        seg(buf, g, &mut x, a["name"].as_str().unwrap_or(""), st(hex(theme::CYAN)).add_modifier(Modifier::UNDERLINED), Some(Hit::Act(Act::Focus(pid))));
    }
    seg(buf, g, &mut x, " ▸ ", st(theme::dim()), None);
    let me = format!("{} (PID {}, {} processes)", f["name"].as_str().unwrap_or(""), f["pid"].as_i64().unwrap_or(0), f["count"].as_i64().unwrap_or(0));
    seg(buf, g, &mut x, &me, st(theme::text()).add_modifier(Modifier::BOLD), None);
    let exit = "✕ Show everything (Esc)";
    let ex = w.saturating_sub(exit.chars().count() as u16 + 1);
    if ex > x {
        put(buf, ex, y, exit, st(theme::text()).bg(bg), w - ex);
        g.hits.push((Rect::new(ex, y, w - ex, 1), Hit::Act(Act::Focus(0))));
    }
}

// ---- the process table ----

struct Line {
    cells: Vec<(char, Style)>,
}

impl Line {
    fn push(&mut self, t: &str, s: Style) {
        for c in t.chars() {
            self.cells.push((c, s));
            // Wide characters take two cells.
            if c.width().unwrap_or(1) == 2 {
                self.cells.push(('\0', s));
            }
        }
    }
    fn len(&self) -> usize {
        self.cells.len()
    }
}

fn kind_style(k: Kind, bg: Rgb) -> Style {
    let (c, a) = k.color(true);
    let fg = match c {
        Some(c) => hex(c),
        None => theme::over(Rgb::hex(theme::TEXT), bg, a),
    };
    let s = st(fg);
    if k.bold() { s.add_modifier(Modifier::BOLD) } else { s }
}

fn table(buf: &mut Buffer, app: &mut App, g: &mut Geometry, r: Rect) {
    if r.height < 2 {
        return;
    }
    let cols = app.visible_columns();
    let fixed: Vec<&Column> = cols.iter().filter(|c| c.chars > 0).collect();
    // Gutter (tag marker), then the fixed columns.
    let mut x = 1u16;
    for c in &fixed {
        g.cols.push((c.key, x, c.chars as u16));
        x += c.chars as u16 + 1;
    }
    g.cmd_x = x;
    g.header_y = r.y;
    let hs = app.hscroll as u16;
    // Header.
    let hy = r.y;
    tint(buf, 0, hy, r.width, hex(theme::HEADER));
    let hover_key = app.mouse.and_then(|(mx, my)| (my == hy).then_some(mx)).and_then(|mx| {
        let cx = mx + hs;
        g.cols.iter().find(|(_, x, w)| cx >= *x && cx < x + w + 1).map(|(k, _, _)| *k)
    });
    let mut header = Line { cells: vec![(' ', Style::default())] };
    let mut spans: Vec<(&'static str, usize, usize)> = Vec::new();
    for c in &cols {
        let start = header.len();
        let sorted = c.key == app.opts.sort_key;
        let arrow = if sorted { if app.opts.sort_desc { "▾" } else { "▴" } } else { "" };
        let title = format!("{}{arrow}", c.title);
        let bg = if sorted { hex(theme::HEADER_SORTED) } else if Some(c.key) == hover_key { hex(theme::HEADER_HOVER) } else { hex(theme::HEADER) };
        let s = Style::default().fg(hex(theme::HEADER_TEXT)).bg(bg).add_modifier(Modifier::BOLD);
        if c.chars > 0 {
            header.push(&format::pad(&title, c.chars, c.right), s);
        } else {
            header.push(&title, s);
        }
        spans.push((c.key, start, header.len()));
    }
    for (i, (ch, s)) in header.cells.iter().enumerate().skip(hs as usize) {
        let sx = i as u16 - hs;
        if sx >= r.width {
            break;
        }
        if *ch != '\0' {
            buf[(sx, hy)].set_char(*ch).set_style(*s);
        }
    }
    for (k, a, b) in &spans {
        let (a, b) = ((*a as u16).saturating_sub(hs), (*b as u16).saturating_sub(hs));
        if b > a {
            g.hits.push((Rect::new(a, hy, (b - a).min(r.width.saturating_sub(a)), 1), Hit::Header(k)));
        }
    }
    for (k, x, w) in &g.cols {
        let gx = (x + w).saturating_sub(hs);
        if gx < r.width {
            g.hits.push((Rect::new(gx, hy, 1, 1), Hit::Gap(k)));
        }
    }
    let hidden = app.opts.hidden_cols.len();
    let btn = if hidden > 0 { format!(" ⋯ columns ({hidden} hidden) ") } else { " ⋯ columns ".to_string() };
    let bw = btn.chars().count() as u16;
    let bx = r.width.saturating_sub(bw);
    put(buf, bx, hy, &btn, Style::default().fg(hex(theme::HEADER_TEXT)).bg(hex(theme::HEADER)).add_modifier(Modifier::BOLD), bw);
    g.hits.push((Rect::new(bx, hy, bw, 1), Hit::Act(Act::ColumnMenu)));
    // Where a dragged column would land.
    if let Some(crate::app::Drag::Column(_, mx)) = &app.drag {
        let cx = mx + hs;
        let at = g.cols.iter().find(|(_, x, w)| cx < x + w / 2 + 1).map(|(_, x, _)| *x).unwrap_or(g.cmd_x);
        let sx = at.saturating_sub(1).saturating_sub(hs);
        if sx < r.width {
            buf[(sx, hy)].set_char('▏').set_fg(hex(theme::TAG));
        }
    }

    // Rows, with a vertical scrollbar at the right and, when lines are
    // wider than the screen, a horizontal one along the bottom.
    let full = Rect::new(r.x, r.y + 1, r.width, r.height - 1);
    // Widest line: the fixed columns, then tree lines, ▸, badge, Σ, command.
    let widest = app
        .rows
        .iter()
        .map(|row| {
            let badge = row["container"].as_str().map(|c| if c.is_empty() { 0 } else { c.chars().count() + 6 }).unwrap_or(0);
            g.cmd_x as usize + 1 + row["prefix"].as_str().map(|p| p.chars().count()).unwrap_or(0) + 2 + badge + 6
                + row["cmdText"].as_str().map(|t| t.chars().count().min(command::MAX_ROW_COMMAND + 2)).unwrap_or(0)
        })
        .max()
        .unwrap_or(0);
    let view_w = full.width.saturating_sub(1);
    let need_h = widest > view_w as usize && full.height > 2;
    let body = Rect::new(full.x, full.y, view_w, full.height - need_h as u16);
    app.hmax = widest.saturating_sub(view_w as usize);
    app.view_w = view_w as usize;
    app.hscroll = app.hscroll.min(app.hmax);
    g.table = body;
    app.page = body.height as usize;
    app.table_y = body.y;
    app.clamp_scroll();
    g.hits.push((body, Hit::Table));
    let r = Rect::new(r.x, r.y, view_w, r.height);
    let bar_track = theme::text_a(0.06);
    let bar_thumb = theme::text_a(0.35);
    // Vertical.
    let track = Rect::new(view_w, body.y, 1, body.height);
    let total = app.rows.len().max(1);
    let (vstart, vlen) = thumb(track.height, body.height as usize, total, app.top);
    for yy in 0..track.height {
        let on = yy >= vstart && yy < vstart + vlen;
        buf[(track.x, track.y + yy)].set_char(if on { '┃' } else { '│' }).set_fg(if on { bar_thumb } else { bar_track }).set_bg(theme::bg());
    }
    g.vbar = Some((track, vstart, vlen));
    g.hits.push((track, Hit::VBar));
    // Horizontal.
    if need_h {
        let track = Rect::new(0, body.y + body.height, view_w, 1);
        let (hstart, hlen) = thumb(track.width, view_w as usize, widest, app.hscroll);
        for xx in 0..track.width {
            let on = xx >= hstart && xx < hstart + hlen;
            buf[(track.x + xx, track.y)].set_char(if on { '━' } else { '─' }).set_fg(if on { bar_thumb } else { bar_track }).set_bg(theme::bg());
        }
        g.hbar = Some((track, hstart, hlen));
        g.hits.push((track, Hit::HBar));
    }
    let base = Rgb::hex(theme::BG);
    let alt = Rgb::hex(theme::ALT);
    let view = app.view().to_string();
    for (vi, i) in (app.top..app.rows.len()).enumerate() {
        if vi >= body.height as usize {
            break;
        }
        let y = body.y + vi as u16;
        let row = &app.rows[i];
        let current = row["key"].as_str() == Some(app.cur_key.as_str());
        let group = row["group"].as_bool().unwrap_or(false);
        let kernel = row["kernel"].as_bool().unwrap_or(false);
        let container = row["container"].as_str().unwrap_or("");
        let chip = format::container_color(container);
        let row_bg: Rgb = if current {
            Rgb::hex(theme::HIGHLIGHT)
        } else if group {
            chip.map(|c| c.over(base, 0.16)).unwrap_or(base)
        } else {
            let b = if i % 2 == 1 { alt } else { base };
            match chip {
                Some(c) if !container.is_empty() => c.over(b, 0.06),
                _ => b,
            }
        };
        let fg = if current { hex(theme::HIGHLIGHT_TEXT) } else if kernel { theme::dim() } else { theme::text() };
        let mut line = Line { cells: Vec::new() };
        let bgc = theme::c(row_bg);
        // Tag marker.
        let tagged = app.tagged.contains(&(row["pid"].as_i64().unwrap_or(0) as i32));
        line.push(if tagged { "▌" } else { " " }, st(hex(theme::TAG)).bg(bgc));
        let cells = row["cells"].as_array();
        let colors = row["colors"].as_array();
        let mut cut: Vec<(&'static str, u16, u16)> = Vec::new();
        for c in &fixed {
            let idx = columns::cell_index(c.key).unwrap_or(0);
            let text = cells.and_then(|a| a.get(idx)).and_then(|v| v.as_str()).unwrap_or("");
            let color = if current {
                fg
            } else {
                colors.and_then(|a| a.get(idx)).and_then(|v| v.as_str()).map(hex).unwrap_or(fg)
            };
            if text.chars().count() > c.chars {
                cut.push((c.key, line.len() as u16, c.chars as u16));
            }
            line.push(&format::pad(text, c.chars, c.right), st(color));
        }
        // Command column: container stripe, tree lines, ▸/▾, badge, Σ, command.
        let cmd_start = line.len();
        line.push(if chip.is_some() && !group { "▎" } else { " " }, st(chip.map(theme::c).unwrap_or(fg)));
        let mut prefix = row["prefix"].as_str().unwrap_or("").to_string();
        if view == "containers" && row["depth"].as_i64().unwrap_or(0) > 0 && prefix.is_empty() {
            prefix = "  ".into();
        }
        line.push(&prefix, st(if current { fg } else { theme::dim() }));
        let mut exp = None;
        let has_kids = row["hasChildren"].as_bool().unwrap_or(false) && view != "flat" && view != "gpu";
        if has_kids {
            let at = line.len();
            line.push(if row["expanded"].as_bool().unwrap_or(true) { "▾ " } else { "▸ " }, st(fg));
            exp = Some(at);
        }
        let show_badge = chip.is_some() && (group || (view != "containers" && (row["containerRoot"].as_bool().unwrap_or(false) || view == "flat" || view == "gpu")));
        if show_badge {
            let engine = row["engine"].as_str().unwrap_or("");
            let label = if engine.is_empty() || engine == "docker" { format!(" ⬢ {container} ") } else { format!(" ⬢ {container} · {engine} ") };
            line.push(&label, Style::default().fg(hex("#101418")).bg(theme::c(chip.unwrap())).add_modifier(Modifier::BOLD));
            line.push(" ", st(fg));
        }
        let agg = row["aggCount"].as_i64().unwrap_or(0);
        let mut sigma = None;
        if agg > 0 {
            sigma = Some(line.len());
            line.push(&format!("Σ{agg} "), st(hex(theme::AGG)).add_modifier(Modifier::BOLD));
        }
        let cmd_text = row["cmdText"].as_str().unwrap_or("");
        if group {
            line.push(cmd_text, st(theme::dim()));
        } else if current {
            let t: String = cmd_text.chars().take(command::MAX_ROW_COMMAND).collect();
            line.push(&t, st(fg));
        } else {
            let spans: Spans = command::spans_from_json(&row["cmd"]);
            for (k, t) in &spans {
                line.push(t, kind_style(*k, row_bg));
            }
        }
        // Backgrounds: row, column tints, usage bars.
        let mut bgs: Vec<Rgb> = vec![row_bg; line.len().max(r.width as usize + app.hscroll)];
        if !current {
            for (k, x, w) in &g.cols {
                if app.opts.tint {
                    if let Some(tc) = columns::tint(k) {
                        let c = Rgb::hex(tc).over(row_bg, columns::tint_opacity(k));
                        for b in bgs.iter_mut().skip(*x as usize).take(*w as usize + 1) {
                            *b = c;
                        }
                    }
                }
            }
        }
        for (k, x, w) in &g.cols {
            if let Some((frac, color)) = columns::usage_bar(row, k) {
                let cells_n = ((frac * *w as f64).round() as usize).max(1);
                let a = if current { 0.55 } else { 0.32 };
                for b in bgs.iter_mut().skip(*x as usize).take(cells_n) {
                    *b = color.over(*b, a);
                }
            }
        }
        for sx in 0..r.width {
            let li = sx as usize + app.hscroll;
            let bg = theme::c(bgs.get(li).copied().unwrap_or(row_bg));
            let cell = &mut buf[(sx, y)];
            cell.set_bg(bg);
            if let Some((ch, s)) = line.cells.get(li) {
                if *ch != '\0' {
                    cell.set_char(*ch);
                    cell.set_fg(s.fg.unwrap_or(fg));
                    cell.modifier = s.add_modifier;
                    if let Some(b) = s.bg {
                        cell.set_bg(b);
                    }
                }
            }
        }
        g.hits.push((Rect::new(0, y, r.width, 1), Hit::Row(i)));
        let to_screen = |at: usize, w: usize| -> Option<Rect> {
            let a = at as i64 - app.hscroll as i64;
            if a < 0 || a >= r.width as i64 {
                return None;
            }
            Some(Rect::new(a as u16, y, (w as u16).min(r.width - a as u16), 1))
        };
        if let Some(rr) = to_screen(cmd_start, line.len() - cmd_start) {
            g.hits.push((Rect::new(rr.x, y, r.width - rr.x, 1), Hit::Command(i)));
        }
        for (k, at, w) in cut {
            if let Some(rr) = to_screen(at as usize, w as usize) {
                g.hits.push((rr, Hit::Cell(i, k)));
            }
        }
        if let Some(at) = sigma {
            if let Some(rr) = to_screen(at, format!("Σ{agg}").chars().count()) {
                g.hits.push((rr, Hit::Sigma(i)));
            }
        }
        if let Some(at) = exp {
            // The tree lines and the arrow both fold.
            let from = cmd_start + 1;
            if let Some(rr) = to_screen(from, at + 2 - from) {
                g.hits.push((rr, Hit::Expander(i)));
            }
        }
    }
}

/// A scrollbar thumb: (start, length) on a track `len` cells long showing
/// `view` of `total` items from `pos`.
pub fn thumb(len: u16, view: usize, total: usize, pos: usize) -> (u16, u16) {
    if total <= view || len == 0 {
        return (0, len);
    }
    let l = ((view as f64 / total as f64 * len as f64).round() as u16).clamp(1, len);
    let max_pos = (total - view) as f64;
    let start = ((pos as f64 / max_pos).min(1.0) * (len - l) as f64).round() as u16;
    (start, l)
}

/// The scroll position for a thumb dragged to `start`.
pub fn thumb_pos(len: u16, thumb_len: u16, view: usize, total: usize, start: i32) -> usize {
    if total <= view || len <= thumb_len {
        return 0;
    }
    let f = (start.max(0) as f64 / (len - thumb_len) as f64).min(1.0);
    (f * (total - view) as f64).round() as usize
}

// ---- footer ----

fn footer(buf: &mut Buffer, app: &App, g: &mut Geometry, y: u16, w: u16) {
    tint(buf, 0, y, w, hex(theme::FOOTER_BG));
    let mut x = 0;
    for (k, label, act) in keys::footer(Frontend::Terminal, app.view()) {
        let kn = put(buf, x, y, k, st(theme::text()).bg(hex(theme::FOOTER_BG)).add_modifier(Modifier::BOLD), w.saturating_sub(x));
        let ln = put(buf, x + kn, y, &format!(" {label} "), Style::default().fg(hex(theme::HEADER_TEXT)).bg(hex(theme::HEADER)), w.saturating_sub(x + kn));
        g.hits.push((Rect::new(x, y, kn + ln, 1), Hit::Footer(act)));
        x += kn + ln;
    }
    let mut right = String::new();
    if app.frozen {
        right.push_str("‖ order held while the pointer is over the list   ");
    }
    if let Some(s) = app.status_text() {
        right.push_str(s);
    }
    if !right.is_empty() {
        let max = (w.saturating_sub(x + 1)) as usize;
        let n1 = right.chars().count();
        // Left-elided: the end of a message matters most.
        let t: String = if n1 > max { std::iter::once('…').chain(right.chars().skip(n1 - max + 1)).collect() } else { right };
        let tw = t.chars().count() as u16;
        put(buf, w - tw, y, &t, st(hex(theme::YELLOW)).bg(hex(theme::FOOTER_BG)), tw);
    }
}

// ---- pop-ups ----

fn boxed(buf: &mut Buffer, r: Rect, title: &str) {
    fill(buf, r, hex("#2a2e32"));
    let border = theme::dim();
    for x in r.x..r.x + r.width {
        buf[(x, r.y)].set_char('─').set_fg(border);
        buf[(x, r.y + r.height - 1)].set_char('─').set_fg(border);
    }
    for y in r.y..r.y + r.height {
        buf[(r.x, y)].set_char('│').set_fg(border);
        buf[(r.x + r.width - 1, y)].set_char('│').set_fg(border);
    }
    buf[(r.x, r.y)].set_char('╭');
    buf[(r.x + r.width - 1, r.y)].set_char('╮');
    buf[(r.x, r.y + r.height - 1)].set_char('╰');
    buf[(r.x + r.width - 1, r.y + r.height - 1)].set_char('╯');
    if !title.is_empty() {
        put(buf, r.x + 2, r.y, &format!(" {title} "), st(theme::text()).add_modifier(Modifier::BOLD).bg(hex("#2a2e32")), r.width.saturating_sub(4));
    }
}

fn popups(buf: &mut Buffer, app: &mut App, g: &mut Geometry, area: Rect) {
    let Some(p) = &mut app.popup else { return };
    match p {
        Popup::Menus(stack) => {
            let mut px = 0u16;
            let mut py = 0u16;
            for (level, m) in stack.iter_mut().enumerate() {
                let w = m.items.iter().map(|i| i.label().chars().count() + if matches!(i, Item::Sub(..)) { 3 } else { 0 }).max().unwrap_or(10) as u16 + 4;
                let h = m.items.len() as u16 + 2;
                let w = w.min(area.width);
                let h = h.min(area.height);
                // Anchors: MAX = right edge, MAX-1 = a third across (F9 menu).
                let mut x = if level > 0 { px } else if m.x == u16::MAX { area.width.saturating_sub(w + 1) } else if m.x == u16::MAX - 1 { area.width / 3 } else { m.x };
                let mut y = if level > 0 { py } else { m.y };
                if x + w > area.width {
                    x = area.width.saturating_sub(w);
                }
                if y + h > area.height {
                    y = area.height.saturating_sub(h);
                }
                m.x = x;
                m.y = y;
                let r = Rect::new(x, y, w, h);
                boxed(buf, r, "");
                g.hits.push((r, Hit::Popup));
                for (i, it) in m.items.iter().enumerate() {
                    let iy = y + 1 + i as u16;
                    if iy >= y + h - 1 {
                        break;
                    }
                    let sel = i == m.sel;
                    let bg = if sel && it.selectable() { hex(theme::HIGHLIGHT) } else { hex("#2a2e32") };
                    tint(buf, x + 1, iy, w - 2, bg);
                    match it {
                        Item::Sep => {
                            for xx in x + 1..x + w - 1 {
                                buf[(xx, iy)].set_char('─').set_fg(theme::text_a(0.2));
                            }
                        }
                        Item::Title(t) => {
                            put(buf, x + 2, iy, t, st(theme::dim()).bg(bg), w - 3);
                        }
                        _ => {
                            let fgc = if it.selectable() { theme::text() } else { theme::dim() };
                            put(buf, x + 2, iy, &it.label(), st(fgc).bg(bg), w - 4);
                            if matches!(it, Item::Sub(..)) {
                                put(buf, x + w - 3, iy, "▸", st(fgc).bg(bg), 1);
                            }
                            g.hits.push((Rect::new(x + 1, iy, w - 2, 1), Hit::Menu(level, i)));
                        }
                    }
                }
                px = x + w - 1;
                py = y + 1 + m.sel as u16;
            }
        }
        Popup::Help(scroll) => {
            let text = keys::help_text(Frontend::Terminal);
            let lines: Vec<&str> = text.lines().collect();
            let w = (lines.iter().map(|l| l.chars().count()).max().unwrap_or(40) as u16 + 4).min(area.width);
            let h = (lines.len() as u16 + 2).min(area.height);
            let r = Rect::new((area.width - w) / 2, (area.height - h) / 2, w, h);
            boxed(buf, r, "treetop keys");
            g.hits.push((r, Hit::Popup));
            *scroll = (*scroll).min(lines.len().saturating_sub(h as usize - 2));
            for (i, l) in lines.iter().skip(*scroll).enumerate() {
                if i as u16 >= h - 2 {
                    break;
                }
                put(buf, r.x + 2, r.y + 1 + i as u16, l, st(theme::text()).bg(hex("#2a2e32")), w - 4);
            }
        }
        Popup::Details(d) => details(buf, d, g, area),
    }
}

fn details(buf: &mut Buffer, d: &mut crate::app::Details, g: &mut Geometry, area: Rect) {
    let w = area.width.saturating_sub(8).min(120).max(40.min(area.width));
    let h = area.height.saturating_sub(4).min(44).max(10.min(area.height));
    let r = Rect::new((area.width - w) / 2, (area.height - h) / 2, w, h);
    let info = &d.info;
    boxed(buf, r, &format!("PID {} — {}", d.pid, info["name"].as_str().unwrap_or("")));
    g.hits.push((r, Hit::Popup));
    let bg = hex("#2a2e32");
    let env = info["environ"].as_array().cloned();
    let files = info["files"].as_array().cloned();
    let tabs = [
        "Overview".to_string(),
        env.as_ref().map(|e| format!("Environment ({})", e.len())).unwrap_or("Environment".into()),
        files.as_ref().map(|f| format!("Open files ({})", f.len())).unwrap_or("Open files".into()),
    ];
    let mut x = r.x + 2;
    for (i, t) in tabs.iter().enumerate() {
        let on = i == d.tab;
        let s = if on { Style::default().fg(theme::text()).bg(theme::over_bg(theme::HIGHLIGHT, 0.4)).add_modifier(Modifier::BOLD) } else { st(theme::dim()).bg(bg) };
        let n1 = put(buf, x, r.y + 1, &format!(" {t} "), s, r.width.saturating_sub(x - r.x + 1));
        g.hits.push((Rect::new(x, r.y + 1, n1, 1), Hit::DetailsTab(i)));
        x += n1 + 1;
    }
    put(buf, r.x + r.width.saturating_sub(18), r.y + 1, "Esc closes · ←→ tabs", st(theme::dim()).bg(bg), 17);
    let body = Rect::new(r.x + 2, r.y + 3, r.width - 4, r.height - 4);
    let mut lines: Vec<Spans> = Vec::new();
    let s = |t: &str| t.to_string();
    let kv = |lines: &mut Vec<Spans>, k: &str, v: String| {
        lines.push(vec![(Kind::Dir, format!("{k:<18}")), (Kind::Plain, v)]);
    };
    match d.tab {
        0 => {
            let cmd = command::pretty(info["command"].as_str().unwrap_or(""), info["exe"].as_str().unwrap_or(""));
            for (i, l) in cmd.into_iter().enumerate() {
                let mut v = vec![(Kind::Dir, if i == 0 { format!("{:<18}", "Command") } else { " ".repeat(18) })];
                v.extend(l);
                lines.push(v);
            }
            kv(&mut lines, "Executable", s(info["exe"].as_str().unwrap_or("")));
            kv(&mut lines, "Working dir", s(info["cwd"].as_str().unwrap_or("")));
            kv(&mut lines, "State", s(info["state"].as_str().unwrap_or("")));
            kv(&mut lines, "Parent PID", s(info["ppid"].as_str().unwrap_or("")));
            kv(&mut lines, "Threads", s(info["threads"].as_str().unwrap_or("")));
            let fds = info["fds"].as_i64().unwrap_or(-1);
            let of = match (&files, fds) {
                (Some(f), _) => format!("{} files", f.len()),
                (None, n1) if n1 >= 0 => format!("{n1} descriptors"),
                _ => "(needs root)".into(),
            };
            kv(&mut lines, "Open files", of);
            kv(&mut lines, "UIDs", info["uid"].as_str().unwrap_or("").split_whitespace().collect::<Vec<_>>().join("  "));
            kv(&mut lines, "Peak VM / swap", format!("{} / {}", info["vmPeak"].as_str().unwrap_or(""), info["vmSwap"].as_str().unwrap_or("")));
            kv(&mut lines, "Disk read / write", if info["readBytes"].is_null() { "(needs root)".into() } else { format!("{} / {}", bytes(n(&info["readBytes"])), bytes(n(&info["writeBytes"]))) });
            kv(&mut lines, "Context switches", format!("{} voluntary, {} forced", info["ctxtVoluntary"].as_str().unwrap_or(""), info["ctxtInvoluntary"].as_str().unwrap_or("")));
            kv(&mut lines, "cgroup", s(info["cgroup"].as_str().unwrap_or("")));
            let cname = info["containerName"].as_str().map(|c| format!("{c} ({})", info["engine"].as_str().unwrap_or(""))).unwrap_or("—".into());
            kv(&mut lines, "Container", cname);
            kv(&mut lines, "Image", s(info["containerImage"].as_str().unwrap_or("—")));
            kv(&mut lines, "Container ID", info["containerId"].as_str().map(|c| c.chars().take(12).collect()).unwrap_or("—".into()));
            let eng = info["gpuEngines"].as_array().map(|a| a.iter().map(|v| format!("{}%", format::fixed(n(v), 0))).collect::<Vec<_>>().join(" / ")).unwrap_or_default();
            let gpus = info["gpus"].as_str().unwrap_or("");
            kv(&mut lines, "GPU", if gpus.is_empty() { "—".into() } else { format!("GPU{gpus} · VRAM {} · GTT {} · GFX/CMP/DMA/DEC/ENC {eng}", bytes(n(&info["vram"])), bytes(n(&info["gtt"]))) });
        }
        1 => match &env {
            Some(e) => {
                // The filter line.
                let fy = body.y;
                let fbg = if d.editing_filter { theme::over_bg(theme::HIGHLIGHT, 0.25) } else { theme::track() };
                tint(buf, body.x, fy, body.width, fbg);
                let ft = if d.env_filter.is_empty() && !d.editing_filter { "Filter (/)".to_string() } else { format!("{}{}", d.env_filter, if d.editing_filter { "▏" } else { "" }) };
                put(buf, body.x + 1, fy, &ft, st(if d.env_filter.is_empty() { theme::dim() } else { theme::text() }).bg(fbg), body.width - 2);
                g.hits.push((Rect::new(body.x, fy, body.width, 1), Hit::EnvFilter));
                let f = d.env_filter.to_lowercase();
                for kv in e {
                    let (k, v) = (kv[0].as_str().unwrap_or(""), kv[1].as_str().unwrap_or(""));
                    if !f.is_empty() && !format!("{k}={v}").to_lowercase().contains(&f) {
                        continue;
                    }
                    lines.extend(command::env_lines(k, v));
                }
            }
            None => needs_root(&mut lines),
        },
        _ => match &files {
            Some(f) => {
                for x in f {
                    let kind = x["kind"].as_str().unwrap_or("");
                    let k = match kind {
                        "socket" => Kind::Flag,
                        "pipe" => Kind::Value,
                        "device" => Kind::Number,
                        "anon" | "memfd" => Kind::Dir,
                        _ => Kind::Prog,
                    };
                    lines.push(vec![(Kind::Dir, format!("{:>5}  ", x["fd"].as_i64().map(|v| v.to_string()).unwrap_or_default())), (k, format!("{kind:<8} ")), (Kind::Plain, x["target"].as_str().unwrap_or("").to_string())]);
                }
            }
            None => needs_root(&mut lines),
        },
    }
    // The list (below the filter on the Environment tab), with scrollbars.
    let top = if d.tab == 1 && env.is_some() { 1 } else { 0 };
    let widest = lines.iter().map(|l| l.iter().map(|(_, t)| t.chars().count()).sum::<usize>()).max().unwrap_or(0);
    let lw = body.width.saturating_sub(1);
    let need_h = widest > lw as usize;
    let list = Rect::new(body.x, body.y + top, lw, body.height.saturating_sub(top + need_h as u16));
    let rows = list.height as usize;
    let scroll = &mut d.scroll[d.tab];
    *scroll = (*scroll).min(lines.len().saturating_sub(rows));
    let hs = &mut d.hscroll[d.tab];
    *hs = (*hs).min(widest.saturating_sub(lw as usize));
    let (scroll, hs) = (*scroll, *hs);
    g.dsize = (rows, lines.len(), lw as usize, widest);
    let panel = Rgb::hex("#2a2e32");
    for (i, l) in lines.iter().skip(scroll).enumerate() {
        if i >= rows {
            break;
        }
        let y = list.y + i as u16;
        // Cells of the line, then the visible window of them.
        let mut cells: Vec<(char, Style)> = Vec::new();
        for (k, t) in l {
            let mut sty = kind_style(*k, panel).bg(bg);
            if *k == Kind::Plain {
                sty = sty.fg(theme::text());
            }
            cells.extend(t.chars().map(|c| (c, sty)));
        }
        let shown: Vec<(char, Style)> = cells.into_iter().skip(hs).take(lw as usize).collect();
        let mut xx = list.x;
        for (c, sty) in shown {
            xx += put(buf, xx, y, &c.to_string(), sty, 1);
        }
        // The "Read as root…" button.
        if l.first().is_some_and(|(_, t)| t == "[ Read as root… ]") {
            if d.waiting_root {
                put(buf, list.x, y, "[ Waiting for authorization… ]", st(theme::dim()).bg(bg), 30);
            }
            g.hits.push((Rect::new(list.x, y, 30, 1), Hit::DetailsRoot));
        }
    }
    let track_c = theme::text_a(0.10);
    let thumb_c = theme::text_a(0.40);
    let vt = Rect::new(list.x + lw, list.y, 1, list.height);
    let (vs, vl) = thumb(vt.height, rows, lines.len(), scroll);
    for yy in 0..vt.height {
        let on = yy >= vs && yy < vs + vl;
        buf[(vt.x, vt.y + yy)].set_char(if on { '┃' } else { '│' }).set_fg(if on { thumb_c } else { track_c });
    }
    g.dvbar = Some((vt, vs, vl));
    g.hits.push((vt, Hit::DetailsV));
    if need_h {
        let ht = Rect::new(list.x, list.y + list.height, lw, 1);
        let (hs0, hl) = thumb(ht.width, lw as usize, widest, hs);
        for xx in 0..ht.width {
            let on = xx >= hs0 && xx < hs0 + hl;
            buf[(ht.x + xx, ht.y)].set_char(if on { '━' } else { '─' }).set_fg(if on { thumb_c } else { track_c });
        }
        g.dhbar = Some((ht, hs0, hl));
        g.hits.push((ht, Hit::DetailsH));
    }
}

fn needs_root(lines: &mut Vec<Spans>) {
    lines.push(vec![(Kind::Plain, "The kernel only shows this to the process's owner (or root).".into())]);
    lines.push(Vec::new());
    lines.push(vec![(Kind::Flag, "[ Read as root… ]".into())]);
    lines.push(Vec::new());
    lines.push(vec![(Kind::Dir, "Or turn on root access (◉ root in the toolbar) to see it for every process.".into())]);
}

// ---- tooltips ----

fn wrap(text: &str, w: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split(' ') {
            if line.chars().count() + word.chars().count() + 1 > w && !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
    }
    out
}

fn tooltip(buf: &mut Buffer, app: &App, g: &Geometry, area: Rect) {
    if app.popup.is_some() || app.drag.is_some() {
        return;
    }
    let Some((mx, my)) = app.mouse else { return };
    let since = app.mouse_moved.elapsed().as_millis();
    let Some(hit) = g.at(mx, my) else { return };
    // Tooltips sit under other hits (a card's click): find the tip-bearing one.
    let tip_hit = g.hits.iter().rev().filter(|(r, _)| mx >= r.x && mx < r.x + r.width && my >= r.y && my < r.y + r.height).map(|(_, h)| h).find(|h| {
        matches!(h, Hit::Tip(_) | Hit::Header(_) | Hit::Gap(_) | Hit::Cell(..) | Hit::Sigma(_) | Hit::Command(_) | Hit::Expander(_))
    });
    let Some(h) = tip_hit.or(Some(hit)) else { return };
    let (delay, lines): (u128, Vec<(String, Style)>) = match h {
        Hit::Tip(t) => (600, wrap(t, 60).into_iter().map(|l| (l, st(theme::text()))).collect()),
        Hit::Header(k) => {
            let c = columns::COLUMNS.iter().find(|c| c.key == *k).unwrap();
            let mut v = vec![(c.title.to_string(), st(theme::text()).add_modifier(Modifier::BOLD))];
            v.extend(wrap(c.tip, 60).into_iter().map(|l| (l, st(theme::text()))));
            v.push((String::new(), st(theme::text())));
            v.push(("Click to sort, drag to move, right-click to choose columns.".into(), st(theme::dim())));
            (600, v)
        }
        Hit::Gap(_) => (600, vec![("Drag to resize · double-click to fit all columns".into(), st(theme::text()))]),
        Hit::Expander(_) => (600, vec![("Collapse/Expand · Shift+click: everything below · keys: +/− ←/→, Ctrl+→/← everything below".into(), st(theme::text()))]),
        Hit::Sigma(i) => {
            let n1 = app.rows.get(*i).and_then(|r| r["aggCount"].as_i64()).unwrap_or(0);
            (500, wrap(&format!("Σ: the values on this folded row are totals for {n1} processes (it and everything below it). Expand it to see each one."), 60).into_iter().map(|l| (l, st(theme::text()))).collect())
        }
        Hit::Cell(i, k) => {
            let c = columns::COLUMNS.iter().find(|c| c.key == *k).unwrap();
            let text = app.rows.get(*i).map(|r| columns::cell_text(r, k)).unwrap_or_default();
            (400, vec![(format!("{}: {text}", c.title), st(theme::text()))])
        }
        Hit::Command(i) => {
            if app.rows.get(*i).is_none_or(|r| r["group"].as_bool() == Some(true)) {
                return;
            }
            return draw_command_tip(buf, app, *i, mx, my, area, since);
        }
        _ => return,
    };
    if since < delay {
        return;
    }
    draw_tip_box(buf, &lines, mx, my, area);
}

fn draw_tip_box(buf: &mut Buffer, lines: &[(String, Style)], mx: u16, my: u16, area: Rect) {
    let w = (lines.iter().map(|(l, _)| l.chars().count()).max().unwrap_or(0) as u16 + 4).min(area.width);
    let h = (lines.len() as u16 + 2).min(area.height);
    let mut x = mx + 2;
    let mut y = my + 1;
    if x + w > area.width {
        x = area.width.saturating_sub(w);
    }
    if y + h > area.height {
        y = my.saturating_sub(h);
    }
    let r = Rect::new(x, y, w, h);
    boxed(buf, r, "");
    for (i, (l, s)) in lines.iter().enumerate() {
        if i as u16 >= h - 2 {
            break;
        }
        put(buf, x + 2, y + 1 + i as u16, l, s.bg(hex("#2a2e32")), w - 4);
    }
}

fn draw_command_tip(buf: &mut Buffer, app: &App, i: usize, mx: u16, my: u16, area: Rect, since: u128) {
    if since < 700 {
        return;
    }
    let r = &app.rows[i];
    let lines = command::pretty(r["command"].as_str().unwrap_or(""), r["exe"].as_str().unwrap_or(""));
    let mut who = format!("PID {} · {}", r["pid"], r["user"].as_str().unwrap_or(""));
    if let Some(c) = r["container"].as_str().filter(|c| !c.is_empty() && *c != "Host") {
        who += &format!(" · ⬢ {c}");
    }
    let maxw = lines.iter().map(|l| command::text(l).chars().count()).max().unwrap_or(0).max(who.chars().count());
    let w = (maxw as u16 + 4).min(area.width);
    let h = (lines.len() as u16 + 4).min(area.height);
    let mut x = mx + 2;
    let mut y = my + 1;
    if x + w > area.width {
        x = area.width.saturating_sub(w);
    }
    if y + h > area.height {
        y = my.saturating_sub(h);
    }
    let rr = Rect::new(x, y, w, h);
    boxed(buf, rr, "");
    let panel = Rgb::hex("#2a2e32");
    for (li, l) in lines.iter().enumerate() {
        if li as u16 >= h - 4 {
            break;
        }
        let mut xx = x + 2;
        for (k, t) in l {
            xx += put(buf, xx, y + 1 + li as u16, t, kind_style(*k, panel).bg(hex("#2a2e32")), (x + w - 2).saturating_sub(xx));
        }
    }
    put(buf, x + 2, y + h - 2, &who, st(theme::dim()).bg(hex("#2a2e32")), w - 4);
}

#[cfg(test)]
mod tests {
    use crate::app::{App, Msg, Popup};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use serde_json::json;

    fn fixture() -> serde_json::Value {
        let mut row = json!({"key": "p:1", "pid": 1, "ppid": 0, "depth": 0, "prefix": "", "hasChildren": true, "expanded": true,
            "group": false, "user": "root", "uid": 0, "pri": 20, "ni": 0, "virt": 1e8, "res": 2e7, "shr": 1e6, "state": "S",
            "cpu": 12.5, "mem": 0.4, "time": 75.5, "threads": 1, "container": "", "engine": "", "image": "", "name": "systemd",
            "command": "/usr/lib/systemd/systemd --system", "exe": "/usr/lib/systemd/systemd", "kernel": false, "count": 1,
            "containerRoot": false, "gpu": 0.0, "vram": 0.0, "gtt": 0.0, "gpus": "", "gpuEngines": "", "computeQueues": 0,
            "ioR": 0.0, "ioW": 0.0, "aggCount": 3, "netRx": 0.0, "netTx": 0.0});
        treetop_core::columns::decorate(&mut row, treetop_core::command::StoreStyle::Compact);
        json!({
            "cpu": {"total": 12.5, "user": 10.0, "nice": 0.0, "system": 2.5, "other": 0.0, "cores": [[10, 0, 2, 0, 3000], [11, 0, 3, 0, 3000]], "temp": 50.0},
            "mem": {"total": 64e9, "used": 16e9, "buffers": 1e9, "cached": 8e9, "available": 40e9, "swapTotal": 0, "swapUsed": 0},
            "gpus": [{"name": "Test GPU", "pdev": "0000:03:00.0", "busy": 40.0, "vramUsed": 4e9, "vramTotal": 16e9, "display": true, "engines": [40, 0, 0, 0, 0]}],
            "disk": {"read": 1e6, "write": 2e6, "devices": [], "filesystems": [{"mount": "/", "device": "nvme0n1p2", "type": "ext4", "used": 1e11, "total": 5e11}]},
            "net": {"rx": 1e5, "tx": 2e5, "interfaces": [{"name": "eth0", "rx": 1e5, "tx": 2e5, "physical": true, "up": true}]},
            "load": [0.5, 0.6, 0.7], "uptime": 3600.0,
            "sensors": [{"dev": "CPU", "label": "Tctl", "kind": "temp", "value": 50.0}],
            "procs": {"tasks": 1, "threads": 1, "kthreads": 0, "running": 1, "containers": 0, "focus": null},
            "notices": [], "root": false, "rootStarting": false, "collapsedCount": 0,
            "rows": {"count": 1, "changed": [[0, row]]},
        })
    }

    fn render(app: &mut App) -> String {
        let mut t = Terminal::new(TestBackend::new(160, 40)).unwrap();
        t.draw(|f| {
            super::draw(f, app);
        })
        .unwrap();
        let buf = t.backend().buffer().clone();
        (0..40).map(|y| (0..160).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "\n").collect()
    }

    #[test]
    fn draws_the_main_screen_menus_and_details() {
        // Hermetic: our own empty settings.
        let dir = std::env::temp_dir().join(format!("treetop-tui-test-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(treetop_core::Engine::new(), tx);
        app.on_msg(Msg::Update(fixture()));
        let s = render(&mut app);
        for want in ["Tree", "Resources", "CPU", "Memory", "GPU 1", "VRAM 1", "Disk", "Network", "PID", "systemd", "Σ3", "F9", "Test GPU"] {
            assert!(s.contains(want), "{want:?} missing from:\n{s}");
        }
        app.run(crate::app::Act::SignalMenu);
        let s = render(&mut app);
        assert!(s.contains("Terminate (15)") && s.contains("Send signal"), "signal menu:\n{s}");
        app.popup = None;
        app.run(crate::app::Act::Help);
        assert!(render(&mut app).contains("treetop keys"));
        app.popup = None;
        app.set_view("gpu");
        let s = render(&mut app);
        assert!(s.contains("Shader clock") && s.contains("Compute"), "GPU view:\n{s}");
        assert!(matches!(app.popup, None | Some(Popup::Help(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
