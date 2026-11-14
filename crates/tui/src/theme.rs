//! Colours: the Qt app's dark palette (Breeze-like base, htop accents),
//! drawn in true colour, or the nearest of the 256 when the terminal can't.

use ratatui::style::Color;
use std::sync::OnceLock;
use treetop_core::format::Rgb;

static TRUECOLOR: OnceLock<bool> = OnceLock::new();

pub fn truecolor() -> bool {
    *TRUECOLOR.get_or_init(|| {
        let ct = std::env::var("COLORTERM").unwrap_or_default().to_lowercase();
        ct.contains("truecolor") || ct.contains("24bit") || std::env::var("TERM").is_ok_and(|t| t.contains("direct") || t == "xterm-kitty")
            || std::env::var_os("KONSOLE_VERSION").is_some() || std::env::var_os("WT_SESSION").is_some()
    })
}

/// An RGB colour as the terminal can show it.
pub fn c(rgb: Rgb) -> Color {
    let (r, g, b) = rgb.bytes();
    if truecolor() {
        Color::Rgb(r, g, b)
    } else {
        Color::Indexed(nearest256(r, g, b))
    }
}

pub fn hex(s: &str) -> Color {
    c(Rgb::hex(s))
}

fn nearest256(r: u8, g: u8, b: u8) -> u8 {
    // The 6x6x6 cube or the grey ramp, whichever is closer.
    let q = |v: u8| -> u8 { if v < 48 { 0 } else if v < 115 { 1 } else { (v - 35) / 40 } };
    let (qr, qg, qb) = (q(r), q(g), q(b));
    let lv = |i: u8| if i == 0 { 0 } else { 55 + i as i32 * 40 };
    let cube = (lv(qr), lv(qg), lv(qb));
    let avg = (r as i32 + g as i32 + b as i32) / 3;
    let gi = ((avg - 8).max(0) / 10).min(23);
    let grey = 8 + gi * 10;
    let d = |a: (i32, i32, i32)| (a.0 - r as i32).pow(2) + (a.1 - g as i32).pow(2) + (a.2 - b as i32).pow(2);
    if d((grey, grey, grey)) < d(cube) { 232 + gi as u8 } else { 16 + 36 * qr + 6 * qg + qb }
}

// The window's colours (KDE Breeze dark) and htop's accents.
pub const BG: &str = "#202326";
pub const ALT: &str = "#24272b";
pub const TEXT: &str = "#fcfcfc";
pub const HIGHLIGHT: &str = "#3daee9";
pub const HIGHLIGHT_TEXT: &str = "#fcfcfc";
pub const GREEN: &str = "#43c96b";
pub const BLUE: &str = "#4f8fe0";
pub const RED: &str = "#e05a4f";
pub const YELLOW: &str = "#d6b23e";
pub const CYAN: &str = "#4fb3d9";
pub const MAGENTA: &str = "#c06fd8";
pub const ORANGE: &str = "#e0894f";
pub const PINK: &str = "#d070a8";
/// The process table's header bar.
pub const HEADER: &str = "#2e9e5b";
pub const HEADER_SORTED: &str = "#57c785";
pub const HEADER_HOVER: &str = "#3bb06b";
pub const HEADER_TEXT: &str = "#0d1a12";
pub const FOOTER_BG: &str = "#16261c";
pub const AGG: &str = "#56b6e0";
pub const TAG: &str = "#e0c14f";

pub fn bg() -> Color {
    hex(BG)
}
pub fn text() -> Color {
    hex(TEXT)
}
/// Text at `a` over the base (dim text, track colours).
pub fn text_a(a: f64) -> Color {
    c(Rgb::hex(TEXT).over(Rgb::hex(BG), a))
}
pub fn dim() -> Color {
    text_a(0.55)
}
/// `color` at `a` over the base.
pub fn over_bg(color: &str, a: f64) -> Color {
    c(Rgb::hex(color).over(Rgb::hex(BG), a))
}
pub fn over(color: Rgb, under: Rgb, a: f64) -> Color {
    c(color.over(under, a))
}
pub fn track() -> Color {
    text_a(0.07)
}
