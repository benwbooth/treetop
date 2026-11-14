//! Small drawing helpers: braille sparklines and htop-style meters.

use crate::theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

/// Writes `text` at (x, y), clipped to `max` cells; returns cells used.
pub fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style, max: u16) -> u16 {
    let area = *buf.area();
    if y < area.y || y >= area.y + area.height {
        return 0;
    }
    let mut n = 0u16;
    for ch in text.chars() {
        if n >= max || x + n >= area.x + area.width {
            break;
        }
        let cell = &mut buf[(x + n, y)];
        cell.set_char(ch);
        cell.set_style(style);
        n += 1;
    }
    n
}

/// Fills a rectangle's background.
pub fn fill(buf: &mut Buffer, r: Rect, bg: Color) {
    let r = r.intersection(*buf.area());
    for y in r.y..r.y + r.height {
        for x in r.x..r.x + r.width {
            let cell = &mut buf[(x, y)];
            cell.set_char(' ');
            cell.set_style(Style::default().bg(bg).fg(theme::text()));
        }
    }
}

/// Sets only the background over a span of cells.
pub fn tint(buf: &mut Buffer, x: u16, y: u16, w: u16, bg: Color) {
    let area = *buf.area();
    for i in 0..w {
        if x + i < area.x + area.width && y < area.y + area.height && y >= area.y {
            buf[(x + i, y)].set_bg(bg);
        }
    }
}

/// A bar graph of recent samples, one per cell across, rising in eighth
/// blocks. Several series overlap (the shorter drawn over the taller).
/// `mirror`: the first series rises above the middle, the second hangs
/// below it in half blocks (disk read/write, network down/up).
pub fn sparkline(buf: &mut Buffer, r: Rect, series: &[(&[f64], Color)], max: Option<f64>, mirror: bool) {
    if r.width == 0 || r.height == 0 {
        return;
    }
    let n = r.width as usize;
    let take = |v: &[f64]| -> Vec<f64> {
        let s = &v[v.len().saturating_sub(n)..];
        let mut out = vec![f64::NAN; n - s.len()];
        out.extend_from_slice(s);
        out
    };
    let peak = |v: &[f64]| v.iter().cloned().filter(|x| x.is_finite()).fold(0.0, f64::max);
    if mirror && series.len() >= 2 {
        let a = take(series[0].0);
        let b = take(series[1].0);
        let m = max.unwrap_or_else(|| (peak(&a).max(peak(&b)) * 1.15).max(1.0));
        let up_rows = r.height.div_ceil(2);
        let down_rows = r.height - up_rows;
        bars(buf, Rect { height: up_rows, ..r }, &[(a, series[0].1)], m);
        if down_rows > 0 {
            hang(buf, Rect { y: r.y + up_rows, height: down_rows, ..r }, &b, m, series[1].1);
        }
        return;
    }
    let cols: Vec<(Vec<f64>, Color)> = series.iter().map(|(v, c)| (take(v), *c)).collect();
    let m = max.unwrap_or_else(|| (cols.iter().map(|(v, _)| peak(v)).fold(0.0, f64::max) * 1.15).max(1.0));
    bars(buf, r, &cols, m);
}

const EIGHTHS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Bars rising from the bottom of `r`.
fn bars(buf: &mut Buffer, r: Rect, series: &[(Vec<f64>, Color)], max: f64) {
    let rows = r.height as usize;
    for cx in 0..r.width as usize {
        // Heights in eighths of a cell, tallest drawn first.
        let mut hs: Vec<(usize, Color)> = series
            .iter()
            .filter_map(|(v, c)| {
                let x = v.get(cx).copied().unwrap_or(f64::NAN);
                if !x.is_finite() || x <= 0.0 {
                    return None;
                }
                let h = ((x / max).clamp(0.0, 1.0) * (rows * 8) as f64).round() as usize;
                Some((h.max(1), *c))
            })
            .collect();
        hs.sort_by(|a, b| b.0.cmp(&a.0));
        let x = r.x + cx as u16;
        let mut under: Vec<Option<Color>> = vec![None; rows];
        for (h, color) in hs {
            for row in 0..rows {
                let from_bottom = rows - 1 - row;
                let fill = h.saturating_sub(from_bottom * 8).min(8);
                if fill == 0 {
                    continue;
                }
                let cell = &mut buf[(x, r.y + row as u16)];
                cell.set_char(EIGHTHS[fill]);
                cell.set_fg(color);
                // A shorter bar's top over a taller one's full cell.
                if fill < 8 {
                    if let Some(b) = under[row] {
                        cell.set_bg(b);
                    }
                } else {
                    under[row] = Some(color);
                }
            }
        }
    }
}

/// Bars hanging down from the top of `r`, in half blocks.
fn hang(buf: &mut Buffer, r: Rect, v: &[f64], max: f64, color: Color) {
    let rows = r.height as usize;
    for (cx, x) in v.iter().enumerate().take(r.width as usize) {
        if !x.is_finite() || *x <= 0.0 {
            continue;
        }
        let h = (((x / max).clamp(0.0, 1.0) * (rows * 2) as f64).round() as usize).max(1);
        for row in 0..rows {
            let fill = h.saturating_sub(row * 2).min(2);
            if fill == 0 {
                break;
            }
            let cell = &mut buf[(r.x + cx as u16, r.y + row as u16)];
            cell.set_char(if fill == 2 { '█' } else { '▀' });
            cell.set_fg(color);
        }
    }
}

/// An htop-style meter: label, then a bar of coloured segments over the
/// track, with `text` right-aligned inside it.
pub fn meter(buf: &mut Buffer, x: u16, y: u16, w: u16, label: &str, label_w: u16, segs: &[(f64, Color)], max: f64, text: &str) {
    let lw = put(buf, x, y, &fit(label, label_w as usize), Style::default().fg(theme::hex(theme::CYAN)), label_w);
    let bx = x + label_w.max(lw);
    let bw = w.saturating_sub(label_w);
    if bw == 0 {
        return;
    }
    tint(buf, bx, y, bw, theme::track());
    // Segments, with eighth-blocks for the last partial cell.
    let mut acc = 0.0;
    for (v, color) in segs {
        let start = (acc / max * bw as f64).clamp(0.0, bw as f64);
        acc += v.max(0.0);
        let end = (acc / max * bw as f64).clamp(0.0, bw as f64);
        let (s, e) = (start.round() as u16, end.round() as u16);
        tint(buf, bx + s, y, e.saturating_sub(s), *color);
    }
    let t: String = text.chars().take(bw as usize).collect();
    let tx = bx + bw - t.chars().count() as u16;
    for (i, ch) in t.chars().enumerate() {
        let cell = &mut buf[(tx + i as u16, y)];
        cell.set_char(ch);
        cell.set_fg(theme::text());
        cell.modifier.insert(Modifier::BOLD);
    }
}

/// Cuts a string to `w` cells with "…".
pub fn fit(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n <= w {
        s.to_string()
    } else if w == 0 {
        String::new()
    } else {
        s.chars().take(w - 1).chain(std::iter::once('…')).collect()
    }
}

/// Middle-elides (mount points, labels): "/home/…/media".
pub fn elide_middle(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n <= w || w < 3 {
        return fit(s, w);
    }
    let left = (w - 1) / 2;
    let right = w - 1 - left;
    let a: String = s.chars().take(left).collect();
    let b: String = s.chars().skip(n - right).collect();
    format!("{a}…{b}")
}
