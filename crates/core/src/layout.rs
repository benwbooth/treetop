//! What to show where, shared by both front ends: which GPUs are shown,
//! how the meter sections pack into columns, and the summary texts.

use crate::format::{self, bytes, pct, rate};
use serde_json::Value;

/// How many samples of history the graphs keep.
pub const HISTORY: usize = 90;

/// The graphs' histories, fed every sample.
#[derive(Default, Clone, Debug)]
pub struct Histories {
    pub cpu: Vec<f64>,
    pub mem: Vec<f64>,
    pub rx: Vec<f64>,
    pub tx: Vec<f64>,
    pub rd: Vec<f64>,
    pub wr: Vec<f64>,
    /// Per GPU: busy %, VRAM %.
    pub gpu: Vec<Vec<f64>>,
    pub vram: Vec<Vec<f64>>,
}

fn push(v: &mut Vec<f64>, x: f64) {
    if v.len() >= HISTORY {
        v.remove(0);
    }
    v.push(if x.is_finite() { x } else { 0.0 });
}

impl Histories {
    pub fn push(&mut self, s: &Value) {
        let n = |v: &Value| v.as_f64().unwrap_or(0.0);
        push(&mut self.cpu, n(&s["cpu"]["total"]));
        push(&mut self.mem, n(&s["mem"]["used"]) / n(&s["mem"]["total"]).max(1.0) * 100.0);
        push(&mut self.rx, n(&s["net"]["rx"]));
        push(&mut self.tx, n(&s["net"]["tx"]));
        push(&mut self.rd, n(&s["disk"]["read"]));
        push(&mut self.wr, n(&s["disk"]["write"]));
        let gpus = s["gpus"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
        self.gpu.resize(gpus.len(), Vec::new());
        self.vram.resize(gpus.len(), Vec::new());
        for (i, g) in gpus.iter().enumerate() {
            push(&mut self.gpu[i], n(&g["busy"]));
            push(&mut self.vram[i], n(&g["vramUsed"]) / n(&g["vramTotal"]).max(1.0) * 100.0);
        }
    }
}

/// A GPU that did nothing for the last minute (and doesn't drive a
/// display) folds away on its own; it comes back when it gets busy.
pub fn gpu_idle(gpu: &Value, busy_history: &[f64]) -> bool {
    if gpu["display"].as_bool().unwrap_or(false) || busy_history.len() < 20 {
        return false;
    }
    let recent = &busy_history[busy_history.len().saturating_sub(60)..];
    let total = gpu["vramTotal"].as_f64().unwrap_or(0.0);
    let vram = if total > 0.0 { gpu["vramUsed"].as_f64().unwrap_or(0.0) / total * 100.0 } else { 0.0 };
    recent.iter().cloned().fold(f64::MIN, f64::max) < 3.0 && vram < 10.0
}

#[derive(Clone, Debug, PartialEq)]
pub struct GpuEntry {
    /// Index into the snapshot's gpus.
    pub index: usize,
    /// GPU1, GPU2... (display-driving first, as the snapshot orders them).
    pub number: usize,
    pub pdev: String,
    pub name: String,
    pub idle: bool,
    pub shown: bool,
    /// Why it isn't shown: "hidden" (by you) or "idle".
    pub reason: &'static str,
}

/// Every GPU with whether it's shown. `histories[i]` is GPU i's busy%.
pub fn gpus(snap: &Value, histories: &[Vec<f64>], hidden: &[String], pinned: &[String]) -> Vec<GpuEntry> {
    let empty = Vec::new();
    snap["gpus"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let pdev = g["pdev"].as_str().unwrap_or("").to_string();
            let idle = gpu_idle(g, histories.get(i).map(|v| v.as_slice()).unwrap_or(&[]));
            let is_hidden = hidden.contains(&pdev);
            let shown = !is_hidden && (!idle || pinned.contains(&pdev));
            GpuEntry {
                index: i,
                number: i + 1,
                name: g["name"].as_str().unwrap_or("").to_string(),
                idle,
                shown,
                reason: if is_hidden { "hidden" } else if !shown { "idle" } else { "" },
                pdev,
            }
        })
        .collect()
}

/// Show or hide a GPU: (hidden, pinned) afterwards. Showing an idle GPU
/// pins it, so it stays shown while idle.
pub fn set_gpu_shown(hidden: &[String], pinned: &[String], pdev: &str, show: bool, idle: bool) -> (Vec<String>, Vec<String>) {
    let mut h: Vec<String> = hidden.iter().filter(|p| *p != pdev).cloned().collect();
    let mut p: Vec<String> = pinned.to_vec();
    if show {
        if idle && !p.iter().any(|x| x == pdev) {
            p.push(pdev.to_string());
        }
    } else {
        h.push(pdev.to_string());
        p.retain(|x| x != pdev);
    }
    (h, p)
}

/// Sensors grouped by device, in order: [(dev, [sensor...])].
pub fn sensor_groups(snap: &Value) -> Vec<(String, Vec<Value>)> {
    let mut g: Vec<(String, Vec<Value>)> = Vec::new();
    for x in snap["sensors"].as_array().into_iter().flatten() {
        let dev = x["dev"].as_str().unwrap_or("").to_string();
        if g.last().map(|(d, _)| d != &dev).unwrap_or(true) {
            g.push((dev, Vec::new()));
        }
        g.last_mut().unwrap().1.push(x.clone());
    }
    g
}

/// The inputs to section packing. Widths are in the front end's own units
/// (pixels for the Qt app, characters for the terminal).
pub struct PlanInput<'a> {
    pub width: f64,
    pub gap: f64,
    /// Narrowest a column may be when there's more than one.
    pub min_col: f64,
    /// Width of one per-core CPU meter column.
    pub core_col: f64,
    /// Characters per width unit (for wrapping the sensor lines).
    pub chars_per_unit: f64,
    pub snap: &'a Value,
    pub cpu_combined: bool,
    /// Collapsed sections ("mem", "gpu", "storage", "system", "sensors").
    pub collapsed: &'a [String],
    pub shown_gpus: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub cols: usize,
    pub col_w: f64,
    /// Per-core CPU meters across.
    pub core_cols: usize,
    /// Section names per column.
    pub columns: Vec<Vec<&'static str>>,
}

impl PlanInput<'_> {
    fn open(&self, s: &str) -> bool {
        !self.collapsed.iter().any(|c| c == s)
    }
    fn core_cols(&self, col_w: f64) -> usize {
        ((col_w / self.core_col).floor() as usize).clamp(1, 4)
    }
    /// Lines a section takes in a column this wide.
    pub fn lines(&self, name: &str, col_w: f64) -> usize {
        let s = self.snap;
        let n_cores = s["cpu"]["cores"].as_array().map(|a| a.len()).unwrap_or(0);
        let n_gpus = s["gpus"].as_array().map(|a| a.len()).unwrap_or(0);
        let n_fs = s["disk"]["filesystems"].as_array().map(|a| a.len()).unwrap_or(0);
        let n_sensors = s["sensors"].as_array().map(|a| a.len()).unwrap_or(0);
        match name {
            "cpu" => {
                if self.cpu_combined { 2 } else { 1 + n_cores.div_ceil(self.core_cols(col_w)) }
            }
            "mem" => 1 + if self.open("mem") { if s["mem"]["swapTotal"].as_f64().unwrap_or(0.0) > 0.0 { 2 } else { 1 } } else { 0 },
            "gpu" => if n_gpus == 0 { 0 } else { 1 + if self.open("gpu") { 3 * self.shown_gpus } else { 0 } },
            "storage" => 1 + if self.open("storage") { n_fs } else { 0 },
            "system" => 1 + if self.open("system") { 3 } else { 0 },
            "sensors" => {
                if n_sensors == 0 {
                    0
                } else if !self.open("sensors") {
                    1
                } else {
                    // Each device line, plus wrapped continuation lines.
                    let per_line = ((col_w * self.chars_per_unit).floor() as usize).max(10);
                    1 + sensor_groups(s)
                        .iter()
                        .map(|(dev, items)| {
                            let n = dev.chars().count()
                                + items.iter().map(|x| x["label"].as_str().unwrap_or("").chars().count() + 7).sum::<usize>();
                            n.div_ceil(per_line)
                        })
                        .sum::<usize>()
                }
            }
            _ => 1,
        }
    }

    /// Tries 1..5 columns and keeps the arrangement with the fewest lines
    /// (ties: fewer columns). Columns left empty give their width back.
    pub fn plan(&self) -> Option<Plan> {
        if self.width <= 0.0 {
            return None;
        }
        let has_gpus = self.snap["gpus"].as_array().is_some_and(|a| !a.is_empty());
        let has_sensors = self.snap["sensors"].as_array().is_some_and(|a| !a.is_empty());
        let names: Vec<&'static str> = ["cpu", "mem", "gpu", "storage", "system", "sensors"]
            .into_iter()
            .filter(|n| (*n != "gpu" || has_gpus) && (*n != "sensors" || has_sensors))
            .collect();
        let mut best: Option<(f64, Plan)> = None;
        for cols in 1..=names.len().min(5) {
            let col_w = (self.width - self.gap * (cols as f64 - 1.0)) / cols as f64;
            if cols > 1 && col_w < self.min_col {
                break;
            }
            let mut heights = vec![0.0f64; cols];
            let mut columns: Vec<Vec<&'static str>> = vec![Vec::new(); cols];
            for n in &names {
                let mut c = 0;
                for i in 1..cols {
                    if heights[i] < heights[c] {
                        c = i;
                    }
                }
                columns[c].push(n);
                heights[c] += self.lines(n, col_w) as f64 + 0.3;
            }
            let h = heights.iter().cloned().fold(0.0, f64::max);
            if best.as_ref().map(|(bh, _)| h < bh - 0.01).unwrap_or(true) {
                let columns: Vec<_> = columns.into_iter().filter(|c| !c.is_empty()).collect();
                best = Some((h, Plan { cols, col_w: col_w.floor(), core_cols: self.core_cols(col_w), columns }));
            }
        }
        let (_, mut p) = best?;
        if p.columns.len() < p.cols {
            p.cols = p.columns.len();
            p.col_w = ((self.width - self.gap * (p.cols as f64 - 1.0)) / p.cols as f64).floor();
            p.core_cols = self.core_cols(p.col_w);
        }
        Some(p)
    }
}

fn arr(v: &Value) -> &[Value] {
    v.as_array().map(|a| a.as_slice()).unwrap_or(&[])
}
fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

/// The Resources line (and tray tooltip): CPU · MEM · GPUs, net, disk.
pub fn summary(snap: &Value, shown: &[GpuEntry]) -> Vec<String> {
    let mem_total = num(&snap["mem"]["total"]);
    if mem_total <= 0.0 {
        return vec!["treetop".into()];
    }
    let mut t = format!("CPU {} · MEM {}", pct(num(&snap["cpu"]["total"])), pct(num(&snap["mem"]["used"]) / mem_total * 100.0));
    for g in shown {
        t += &format!(" · GPU{} {}%", g.number, format::fixed(num(&snap["gpus"][g.index]["busy"]), 0));
    }
    vec![
        t,
        format!("↓ {}  ↑ {}", rate(num(&snap["net"]["rx"])), rate(num(&snap["net"]["tx"]))),
        format!("Disk R {}  W {}", rate(num(&snap["disk"]["read"])), rate(num(&snap["disk"]["write"]))),
    ]
}

/// Each meter section's header summary.
pub fn section_summary(name: &str, snap: &Value, shown: &[GpuEntry]) -> String {
    match name {
        "cpu" => {
            let temp = num(&snap["cpu"]["temp"]);
            format!(
                "{} · {} threads{}",
                pct(num(&snap["cpu"]["total"])),
                arr(&snap["cpu"]["cores"]).len(),
                if temp > 0.0 { format!(" · {}°C", format::fixed(temp, 0)) } else { String::new() }
            )
        }
        "mem" => {
            let m = &snap["mem"];
            format!(
                "{} / {}{}",
                bytes(num(&m["used"])),
                bytes(num(&m["total"])),
                if num(&m["swapTotal"]) > 0.0 { format!(" · swap {}", bytes(num(&m["swapUsed"]))) } else { String::new() }
            )
        }
        "gpu" => shown
            .iter()
            .map(|g| format!("GPU{} {}%", g.number, format::fixed(num(&snap["gpus"][g.index]["busy"]), 0)))
            .collect::<Vec<_>>()
            .join(" · "),
        "storage" => {
            let fs = arr(&snap["disk"]["filesystems"]);
            format!(
                "{} / {} · R {} W {}",
                bytes(fs.iter().map(|f| num(&f["used"])).sum()),
                bytes(fs.iter().map(|f| num(&f["total"])).sum()),
                rate(num(&snap["disk"]["read"])),
                rate(num(&snap["disk"]["write"]))
            )
        }
        "sensors" => {
            let all = arr(&snap["sensors"]);
            let temps: Vec<f64> = all.iter().filter(|x| x["kind"] == "temp").map(|x| num(&x["value"])).collect();
            let fans: Vec<f64> = all.iter().filter(|x| x["kind"] == "fan").map(|x| num(&x["value"])).collect();
            let mut s = String::new();
            if !temps.is_empty() {
                s += &format!("hottest {}°C", format::fixed(temps.iter().cloned().fold(f64::MIN, f64::max), 0));
            }
            if !fans.is_empty() {
                s += &format!(
                    " · {} {}{} rpm",
                    fans.len(),
                    if fans.len() == 1 { "fan " } else { "fans " },
                    fans.iter().map(|f| format::fixed(*f, 0)).collect::<Vec<_>>().join("/")
                );
            }
            s
        }
        "system" => format!("{} tasks · load {}", snap["procs"]["tasks"].as_i64().unwrap_or(0), load(snap)),
        _ => String::new(),
    }
}

/// "0.52 0.61 0.70"
pub fn load(snap: &Value) -> String {
    arr(&snap["load"]).iter().map(|v| format::fixed(num(v), 2)).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn gpu_rules() {
        let busy = json!({"display": false, "vramUsed": 1.0, "vramTotal": 100.0});
        assert!(!gpu_idle(&busy, &[0.0; 10]), "too little history");
        assert!(gpu_idle(&busy, &[0.0; 30]));
        assert!(!gpu_idle(&json!({"display": true}), &[0.0; 30]), "drives a display");
        let (h, p) = set_gpu_shown(&["a".into()], &[], "a", true, true);
        assert!(h.is_empty() && p == vec!["a".to_string()]);
        let (h, p) = set_gpu_shown(&[], &["a".into()], "a", false, true);
        assert!(h == vec!["a".to_string()] && p.is_empty());
    }

    #[test]
    fn packing() {
        let snap = json!({"cpu": {"cores": vec![[0,0,0,0]; 24]}, "mem": {"swapTotal": 1.0}, "gpus": [{}],
            "disk": {"filesystems": [{}, {}]}, "sensors": [{"dev": "CPU", "label": "Tctl", "kind": "temp", "value": 50.0}]});
        let input = PlanInput { width: 1600.0, gap: 18.0, min_col: 300.0, core_col: 150.0, chars_per_unit: 0.1,
            snap: &snap, cpu_combined: false, collapsed: &[], shown_gpus: 1 };
        let p = input.plan().unwrap();
        assert!(p.cols >= 2 && p.cols <= 5);
        assert_eq!(p.columns.iter().map(|c| c.len()).sum::<usize>(), 6, "every section placed once");
        let narrow = PlanInput { width: 400.0, ..input };
        assert_eq!(narrow.plan().unwrap().cols, 1);
    }
}
