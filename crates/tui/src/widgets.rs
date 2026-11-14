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

/// A braille sparkline: 2 samples per cell across, 4 dots per row up.
/// `mirror`: the second series grows downwards from the middle.
pub fn sparkline(buf: &mut Buffer, r: Rect, series: &[(&[f64], Color)], max: Option<f64>, mirror: bool) {
    if r.width == 0 || r.height == 0 {
        return;
    }
    let samples = r.width as usize * 2;
    let take = |v: &[f64]| -> Vec<f64> {
        let s = &v[v.len().saturating_sub(samples)..];
        let mut out = vec![f64::NAN; samples - s.len()];
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
        draw_dots(buf, Rect { height: up_rows, ..r }, &a, m, series[0].1, false);
        if down_rows > 0 {
            draw_dots(buf, Rect { y: r.y + up_rows, height: down_rows, ..r }, &b, m, series[1].1, true);
        }
        return;
    }
    for (v, color) in series {
        let s = take(v);
        let m = max.unwrap_or_else(|| (peak(&s) * 1.15).max(1.0));
        draw_dots(buf, r, &s, m, *color, false);
    }
}

fn draw_dots(buf: &mut Buffer, r: Rect, samples: &[f64], max: f64, color: Color, down: bool) {
    let rows = r.height as usize;
    let levels = rows * 4;
    // Braille dot bits: left column 1,2,3,7 (top to bottom), right 4,5,6,8.
    const LEFT: [u32; 4] = [0x01, 0x02, 0x04, 0x40];
    const RIGHT: [u32; 4] = [0x08, 0x10, 0x20, 0x80];
    for cx in 0..r.width as usize {
        for row in 0..rows {
            let mut bits = 0u32;
            for (side, idx) in [(0usize, cx * 2), (1, cx * 2 + 1)] {
                let v = samples.get(idx).copied().unwrap_or(f64::NAN);
                if !v.is_finite() {
                    continue;
                }
                let h = ((v / max).clamp(0.0, 1.0) * levels as f64).round() as usize;
                let h = if v > 0.0 { h.max(1) } else { 0 };
                // Dots filled in this cell row (counted from the baseline).
                let from_base = if down { row } else { rows - 1 - row };
                let filled = h.saturating_sub(from_base * 4).min(4);
                for d in 0..filled {
                    let dot = if down { d } else { 3 - d };
                    bits |= if side == 0 { LEFT[dot] } else { RIGHT[dot] };
                }
            }
            if bits != 0 {
                let x = r.x + cx as u16;
                let y = r.y + row as u16;
                let cell = &mut buf[(x, y)];
                cell.set_char(char::from_u32(0x2800 + bits).unwrap_or(' '));
                cell.set_fg(color);
            }
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
