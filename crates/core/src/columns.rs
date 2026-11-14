//! The process table's columns: what each is, how wide, how it's laid out
//! with your order/widths/hidden choices, and each cell's text and colour.
//! Shared by the Qt app and the terminal UI.

use crate::command::{self, StoreStyle};
use crate::format::{self, bytes, Rgb};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub key: &'static str,
    pub title: &'static str,
    /// Width in characters; 0 = the flexible Command column.
    pub chars: usize,
    pub right: bool,
    /// Only shown when some process is in a container.
    pub optional: bool,
    pub tip: &'static str,
}

const fn col(key: &'static str, title: &'static str, chars: usize, right: bool, tip: &'static str) -> Column {
    Column { key, title, chars, right, optional: false, tip }
}

pub const COLUMNS: &[Column] = &[
    col("pid", "PID", 8, true, "Process ID. The kernel's number for this process; signals and most tools refer to processes by it."),
    col("user", "USER", 10, false, "The account the process runs as (its real user). Inside containers this is the UID as seen from the host, so it may show as a bare number."),
    col("pri", "PRI", 4, true, "Kernel scheduling priority. For normal processes this is 20 + nice; lower means scheduled sooner. \"RT\" marks real-time processes, which always run before normal ones."),
    col("ni", "NI", 4, true, "Nice value, from -20 (greedy, highest priority) to 19 (polite, lowest). Changed with F7/F8; lowering it below the current value needs authorization."),
    col("virt", "VIRT", 7, true, "Virtual memory size: everything the process has mapped — code, libraries, heap, memory-mapped files and reserved-but-unused address space. Usually far bigger than what it really uses; RES is the better measure."),
    col("res", "RES", 7, true, "Resident memory: the part of the process that is actually in RAM right now, including shared libraries it uses. This is the number to look at for \"how much memory is it using\"."),
    col("shr", "SHR", 7, true, "Shared memory: the portion of RES backed by files or shared with other processes (libraries, shared buffers). Pages counted here may also be counted in other processes' RES."),
    col("state", "S", 2, true, "State. R running or runnable, S sleeping (waiting for something), D uninterruptible sleep (usually disk or driver I/O; can't be killed until it returns), T stopped, Z zombie (exited, waiting for its parent to collect it), I idle kernel thread."),
    col("cpu", "CPU%", 6, true, "Share of CPU time over the last update, where 100% is one full core. A process using several cores can go above 100%, up to 100% × the number of cores."),
    col("mem", "MEM%", 6, true, "Resident memory (RES) as a percentage of total physical RAM."),
    col("gpu", "GPU%", 6, true, "How busy this process keeps the GPU: the busiest of its GPU engines (graphics, compute, copy, video decode/encode) over the last update, from the kernel's DRM fdinfo — the same source amdgpu_top and nvtop use. Only processes you own are visible without root. ROCm compute work bypasses fdinfo, so for those processes it's the share of compute units their work occupies right now."),
    col("vram", "GMEM", 7, true, "Video memory (VRAM) this process has allocated on the GPU, as reported by the driver. Buffers it merely imported from other processes are not counted."),
    col("gpus", "GPU#", 5, false, "Which GPU(s) the process has open, numbered as in the GPU cards above (GPU1, GPU2, ...)."),
    col("engines", "GFX/CMP/DMA/DEC/ENC", 20, false, "Per-engine GPU usage in %: GFX (3D/graphics), CMP (compute: ML, OpenCL, ROCm), DMA (copy engine), DEC (video decode), ENC (video encode). \"ROCm\" marks a process holding ROCm compute queues (Ollama, PyTorch, …): its compute work isn't timed by the kernel, so CMP shows the compute units it occupies at this instant."),
    col("ior", "DISK R", 8, true, "Disk reads per second by this process, as the storage layer counts them (data actually fetched from disk, not page-cache hits). Blank for other users' processes: the kernel only shows their I/O counters to root."),
    col("iow", "DISK W", 8, true, "Disk writes per second by this process (data it caused to be written to storage). Blank for other users' processes, whose counters only root can read."),
    col("net", "NET ↓/↑", 15, true, "Network download/upload per second. Linux keeps no per-process network counters (tools like nethogs need root to capture packets), but it does count per network namespace — so this shows each container's traffic, on the container's first process and on its row in the Containers view. Host processes share the host's network and stay blank; the Network card above shows the host total."),
    col("time", "TIME+", 10, true, "Total CPU time the process has used since it started (user + system), in minutes:seconds.hundredths, or hours once it passes an hour. A busy process accumulates time quickly; a long-lived idle one barely moves."),
    Column { key: "container", title: "CONTAINER", chars: 16, right: false, optional: true, tip: "The Docker/Podman/containerd container the process belongs to, worked out from its cgroup. Container processes also get a coloured stripe, and the first process of each container a badge, in the Command column." },
    col("command", "Command", 0, false, "The full command line. In tree view the lines show which process started which; ▸/▾ collapse and expand children (or +/−). The program name is in bold."),
];

/// Hidden by default (the GUI's hiddenColumns default).
pub const DEFAULT_HIDDEN: &str = "pri,ni,shr,gpus,engines,container,time";

/// Columns with a background tint, and its colour.
pub fn tint(key: &str) -> Option<&'static str> {
    Some(match key {
        "cpu" | "time" => "#43c96b",
        "mem" | "res" | "virt" | "shr" => "#d6b23e",
        "gpu" => "#c06fd8",
        "vram" => "#d070a8",
        "ior" | "iow" => "#4fb3d9",
        "net" => "#4f8fe0",
        _ => return None,
    })
}

/// How strongly the tint shows (blue barely shows on a dark base).
pub fn tint_opacity(key: &str) -> f64 {
    if matches!(key, "ior" | "iow" | "net") { 0.14 } else { 0.08 }
}

/// A new sort column starts descending, except these.
pub fn sorts_ascending(key: &str) -> bool {
    matches!(key, "user" | "command" | "pid" | "container" | "gpus")
}

/// The sort key a header click means (Command sorts by PID, engines by GPU%).
pub fn sort_key_for(key: &str) -> &str {
    match key {
        "command" => "pid",
        "engines" => "gpu",
        k => k,
    }
}

pub struct Layout<'a> {
    /// Your column order (keys); others keep their place after these.
    pub order: &'a [String],
    pub widths: &'a HashMap<String, usize>,
    pub hidden: &'a [String],
    /// "tree", "flat", "containers" or "gpu".
    pub view: &'a str,
    /// Any process in a container (the container column is optional).
    pub any_containers: bool,
}

impl Layout<'_> {
    /// Every column in display order, with your widths applied.
    pub fn ordered(&self) -> Vec<Column> {
        let rank = |c: &Column, i: usize| -> f64 {
            if c.key == "command" {
                1e9
            } else if let Some(p) = self.order.iter().position(|k| k == c.key) {
                p as f64
            } else {
                1e6 + i as f64
            }
        };
        let mut v: Vec<(f64, Column)> = COLUMNS.iter().enumerate().map(|(i, c)| (rank(c, i), c.clone())).collect();
        v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        v.into_iter()
            .map(|(_, mut c)| {
                if c.chars > 0 {
                    if let Some(w) = self.widths.get(c.key) {
                        c.chars = (*w).clamp(2, 80);
                    }
                }
                c
            })
            .collect()
    }

    /// Columns forced on by the view (GPU view shows the GPU columns).
    pub fn forced(&self) -> &'static [&'static str] {
        if self.view == "gpu" { &["gpu", "vram", "gpus", "engines"] } else { &[] }
    }

    pub fn show_container_column(&self) -> bool {
        self.view != "containers" && self.any_containers
    }

    /// The columns shown, in order (Command last).
    pub fn visible(&self) -> Vec<Column> {
        let forced = self.forced();
        let show_c = self.show_container_column();
        self.ordered()
            .into_iter()
            .filter(|c| {
                c.key == "command"
                    || forced.contains(&c.key)
                    || ((!c.optional || show_c) && !self.hidden.iter().any(|h| h == c.key))
            })
            .collect()
    }
}

/// The order after moving `key` before `before` ("" = to the end).
pub fn move_column(current: &[Column], key: &str, before: &str) -> Vec<String> {
    if key == before || key == "command" {
        return current.iter().filter(|c| c.key != "command").map(|c| c.key.to_string()).collect();
    }
    let mut order: Vec<String> = current.iter().map(|c| c.key.to_string()).filter(|k| k != "command" && k != key).collect();
    match order.iter().position(|k| k == before) {
        Some(at) if !before.is_empty() => order.insert(at, key.to_string()),
        _ => order.push(key.to_string()),
    }
    order
}

/// Hiding or showing one column.
pub fn toggle_hidden(hidden: &[String], key: &str) -> Vec<String> {
    let mut h = hidden.to_vec();
    if let Some(i) = h.iter().position(|k| k == key) {
        h.remove(i);
    } else {
        h.push(key.to_string());
    }
    h
}

/// Width that fits every value (and the title), 2..60.
pub fn fit_width(col: &Column, rows: &[Value]) -> usize {
    let mut w = col.title.chars().count();
    for r in rows {
        w = w.max(cell_text(r, col.key).chars().count());
    }
    w.clamp(2, 60)
}

fn f(r: &Value, k: &str) -> f64 {
    r[k].as_f64().unwrap_or(0.0)
}
fn s<'a>(r: &'a Value, k: &str) -> &'a str {
    r[k].as_str().unwrap_or("")
}

/// A fixed column's text for a row. Blank instead of 0, so active
/// processes stand out.
pub fn cell_text(r: &Value, k: &str) -> String {
    let group = r["group"].as_bool().unwrap_or(false);
    let blank_group = |v: String| if group { String::new() } else { v };
    match k {
        "pid" => blank_group(r["pid"].as_i64().unwrap_or(0).to_string()),
        "user" => s(r, "user").to_string(),
        "pri" => blank_group({
            let p = r["pri"].as_i64().unwrap_or(0);
            if p <= -100 { "RT".into() } else { p.to_string() }
        }),
        "ni" => blank_group(r["ni"].as_i64().unwrap_or(0).to_string()),
        "virt" => blank_group(bytes(f(r, "virt"))),
        "res" => bytes(f(r, "res")),
        "shr" => blank_group(bytes(f(r, "shr"))),
        "state" => s(r, "state").to_string(),
        "cpu" => if f(r, "cpu") >= 0.05 { format::fixed(f(r, "cpu"), 1) } else { String::new() },
        "mem" => if f(r, "mem") >= 0.05 { format::fixed(f(r, "mem"), 1) } else { String::new() },
        "gpu" => if !s(r, "gpus").is_empty() || f(r, "gpu") > 0.0 { format::fixed(f(r, "gpu"), 1) } else { String::new() },
        "vram" => if f(r, "vram") > 0.0 { bytes(f(r, "vram")) } else { String::new() },
        "gpus" => s(r, "gpus").to_string(),
        "engines" => s(r, "gpuEngines").to_string(),
        "ior" => if f(r, "ioR") >= 1.0 { bytes(f(r, "ioR")) } else { String::new() },
        "iow" => if f(r, "ioW") >= 1.0 { bytes(f(r, "ioW")) } else { String::new() },
        "net" => {
            let (rx, tx) = (f(r, "netRx"), f(r, "netTx"));
            if rx >= 1.0 || tx >= 1.0 { format!("{}/{}", bytes(rx), bytes(tx)) } else { String::new() }
        }
        "time" => {
            if group {
                let n = r["count"].as_i64().unwrap_or(0);
                format!("{n} {}", if n == 1 { "proc" } else { "procs" })
            } else {
                format::cpu_time(f(r, "time"))
            }
        }
        "container" => blank_group(s(r, "container").to_string()),
        _ => String::new(),
    }
}

/// A cell's colour (hex), or None for the row's normal text colour. The
/// selected row's colour is the front end's business.
pub fn cell_color(r: &Value, k: &str) -> Option<String> {
    const GIB: f64 = 1073741824.0;
    const MIB: f64 = 1048576.0;
    let state = s(r, "state");
    if r["aggCount"].as_i64().unwrap_or(0) > 0 && matches!(k, "cpu" | "mem" | "res" | "gpu" | "vram" | "ior" | "iow" | "net") {
        return Some("#56b6e0".into());
    }
    let fixed = |c: &str| Some(c.to_string());
    match k {
        "state" if state == "R" => fixed("#43c96b"),
        "state" if state == "D" => fixed("#e05a4f"),
        "state" if state == "Z" || state == "T" => fixed("#d6a72e"),
        "cpu" if f(r, "cpu") >= 10.0 => Some(format::heat(f(r, "cpu") / 100.0).lighter(1.25).to_hex()),
        "gpu" if f(r, "gpu") >= 10.0 => Some(format::heat(f(r, "gpu") / 100.0).lighter(1.25).to_hex()),
        "mem" if f(r, "mem") >= 2.0 => Some(format::heat((f(r, "mem") / 100.0).sqrt()).lighter(1.25).to_hex()),
        "res" if f(r, "res") >= GIB => fixed("#4fb3d9"),
        "vram" if f(r, "vram") >= GIB => fixed("#c06fd8"),
        "ior" if f(r, "ioR") >= MIB => fixed("#4fb3d9"),
        "iow" if f(r, "ioW") >= MIB => fixed("#4fb3d9"),
        "net" if f(r, "netRx") + f(r, "netTx") >= MIB => fixed("#4f8fe0"),
        "user" if s(r, "user") == "root" => fixed("#e0794f"),
        "container" => format::container_color(s(r, "container")).map(Rgb::to_hex),
        _ => None,
    }
}

/// The usage bar behind CPU%/MEM%/GPU%: (fraction of the column, colour).
/// Memory uses a square-root scale: a few percent of RAM is already a lot.
pub fn usage_bar(r: &Value, k: &str) -> Option<(f64, Rgb)> {
    let v = match k {
        "cpu" | "mem" | "gpu" => f(r, k),
        _ => return None,
    };
    if v <= 0.05 {
        return None;
    }
    let frac = if k == "mem" { (v.max(0.0) / 100.0).sqrt() } else { v.max(0.0) / 100.0 };
    Some((frac.min(1.0), format::heat(frac)))
}

/// The fixed columns' position in a row's `cells`/`colors` arrays.
pub fn cell_index(key: &str) -> Option<usize> {
    COLUMNS.iter().filter(|c| c.chars > 0).position(|c| c.key == key)
}

/// Adds what a front end draws to a row: `cells` and `colors` (in
/// `cell_index` order; colour null = normal text), `cmd` (the command's
/// spans) and `cmdText` (the command as shown, store paths shortened).
pub fn decorate(r: &mut Value, style: StoreStyle) {
    let mut cells = Vec::new();
    let mut colors = Vec::new();
    for c in COLUMNS.iter().filter(|c| c.chars > 0) {
        cells.push(json!(cell_text(r, c.key)));
        colors.push(cell_color(r, c.key).map(Value::from).unwrap_or(Value::Null));
    }
    let command = r["command"].as_str().unwrap_or("").to_string();
    r["cells"] = Value::Array(cells);
    r["colors"] = Value::Array(colors);
    r["cmd"] = command::spans_json(&command::highlight(&command, style));
    r["cmdText"] = json!(command::tidy(&command, style));
}

/// The column table as JSON (for the GUI's header and menus); `cell` is
/// the column's index in a row's cells.
pub fn table_json() -> Value {
    json!(COLUMNS
        .iter()
        .map(|c| json!({"key": c.key, "title": c.title, "chars": c.chars, "right": c.right, "optional": c.optional,
                        "tip": c.tip, "cell": cell_index(c.key)}))
        .collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells() {
        let r = json!({"pid": 42, "user": "root", "pri": 20, "ni": 0, "virt": 1536.0, "res": 2.0 * 1073741824.0, "shr": 0.0,
            "state": "R", "cpu": 0.04, "mem": 12.0, "gpu": 0.0, "gpus": "", "vram": 0.0, "gpuEngines": "",
            "ioR": 0.5, "ioW": 2.0 * 1048576.0, "netRx": 0.0, "netTx": 0.0, "time": 75.5, "container": "", "count": 1, "group": false, "aggCount": 0});
        assert_eq!(cell_text(&r, "cpu"), "", "idle is blank");
        assert_eq!(cell_text(&r, "mem"), "12.0");
        assert_eq!(cell_text(&r, "virt"), "1.5K");
        assert_eq!(cell_text(&r, "ior"), "");
        assert_eq!(cell_text(&r, "iow"), "2.0M");
        assert_eq!(cell_text(&r, "time"), "1:15.50");
        assert_eq!(cell_color(&r, "state").as_deref(), Some("#43c96b"));
        assert_eq!(cell_color(&r, "res").as_deref(), Some("#4fb3d9"));
        assert_eq!(cell_color(&r, "user").as_deref(), Some("#e0794f"));
        assert_eq!(cell_color(&r, "cpu"), None);
        let g = json!({"group": true, "pid": 0, "count": 3, "time": 0.0, "res": 0.0});
        assert_eq!(cell_text(&g, "pid"), "");
        assert_eq!(cell_text(&g, "time"), "3 procs");
    }

    #[test]
    fn layout() {
        let order = vec!["cpu".to_string(), "pid".to_string()];
        let widths: HashMap<String, usize> = [("cpu".to_string(), 9)].into();
        let hidden: Vec<String> = DEFAULT_HIDDEN.split(',').map(String::from).collect();
        let l = Layout { order: &order, widths: &widths, hidden: &hidden, view: "tree", any_containers: true };
        let v = l.visible();
        assert_eq!(v[0].key, "cpu");
        assert_eq!(v[0].chars, 9);
        assert_eq!(v[1].key, "pid");
        assert_eq!(v.last().unwrap().key, "command");
        assert!(!v.iter().any(|c| c.key == "engines"));
        let g = Layout { view: "gpu", ..l };
        assert!(g.visible().iter().any(|c| c.key == "engines"), "forced in the GPU view");
        assert_eq!(move_column(&v, "pid", "cpu")[..2], ["pid".to_string(), "cpu".to_string()]);
    }
}
