//! Number and text formatting shared by both front ends (was qml/Fmt.js),
//! plus the colour rules for table cells.

/// 1024-based size: "512B", "1.5K", "23M" (one decimal below 10).
pub fn bytes(n: f64) -> String {
    bytes_digits(n, None)
}

pub fn bytes_digits(n: f64, digits: Option<usize>) -> String {
    const U: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut n = if n.is_finite() { n } else { 0.0 };
    let mut i = 0;
    while n.abs() >= 1024.0 && i < U.len() - 1 {
        n /= 1024.0;
        i += 1;
    }
    if i == 0 {
        return format!("{}B", fixed(n, 0));
    }
    let d = digits.unwrap_or(if n < 10.0 { 1 } else { 0 });
    format!("{}{}", fixed(n, d), U[i])
}

pub fn rate(n: f64) -> String {
    format!("{}/s", bytes(n))
}

/// "12.3%", "100%" (no decimal from 99.95 up).
pub fn pct(n: f64) -> String {
    if n >= 99.95 {
        format!("{}%", fixed(n, 0))
    } else {
        format!("{}%", fixed(n, 1))
    }
}

/// JavaScript's toFixed: halves round away from zero (Rust's formatting
/// rounds them to even), so both front ends print the same digits.
pub fn fixed(x: f64, digits: usize) -> String {
    let m = 10f64.powi(digits as i32);
    let r = (x * m).round() / m;
    let s = format!("{r:.digits$}");
    if s == "-0" || s.starts_with("-0.") && s[1..].chars().all(|c| c == '0' || c == '.') {
        s[1..].to_string()
    } else {
        s
    }
}

/// htop's TIME+: m:ss.cc below an hour, then XhMM:SS, then XdHHh.
pub fn cpu_time(sec: f64) -> String {
    let sec = if sec.is_finite() { sec.max(0.0) } else { 0.0 };
    if sec < 3600.0 {
        let m = (sec / 60.0).floor();
        let s = sec - m * 60.0;
        let cs = ((s - s.floor()) * 100.0).floor();
        return format!("{}:{:02}.{:02}", m as u64, s.floor() as u64, cs as u64);
    }
    if sec < 86400.0 * 10.0 {
        let h = (sec / 3600.0).floor();
        return format!("{}h{:02}:{:02}", h as u64, ((sec % 3600.0) / 60.0).floor() as u64, (sec % 60.0).floor() as u64);
    }
    format!("{}d{:02}h", (sec / 86400.0).floor() as u64, ((sec % 86400.0) / 3600.0).floor() as u64)
}

/// "3 days, 04:17"
pub fn uptime(sec: f64) -> String {
    let sec = sec.max(0.0).floor() as u64;
    let (d, h, m) = (sec / 86400, sec % 86400 / 3600, sec % 3600 / 60);
    let days = match d {
        0 => String::new(),
        1 => "1 day, ".to_string(),
        d => format!("{d} days, "),
    };
    format!("{days}{h:02}:{m:02}")
}

/// Stable 0..1 hue per name (container colours); hashes UTF-16 units like
/// the QML did, so colours didn't change in the move.
pub fn hue(s: &str) -> f64 {
    let mut h: u32 = 0;
    for c in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(c as u32);
    }
    (h % 360) as f64 / 360.0
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub f64, pub f64, pub f64);

impl Rgb {
    pub fn hex(s: &str) -> Rgb {
        let s = s.trim_start_matches('#');
        let v = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
        Rgb(v(0), v(2), v(4))
    }
    pub fn to_hex(self) -> String {
        let c = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}", c(self.0), c(self.1), c(self.2))
    }
    pub fn bytes(self) -> (u8, u8, u8) {
        let c = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        (c(self.0), c(self.1), c(self.2))
    }
    /// `a` of `self` over `under`.
    pub fn over(self, under: Rgb, a: f64) -> Rgb {
        Rgb(under.0 + (self.0 - under.0) * a, under.1 + (self.1 - under.1) * a, under.2 + (self.2 - under.2) * a)
    }
    /// Qt's QColor::lighter(factor*100): scale HSV value, spilling into
    /// saturation when it saturates.
    pub fn lighter(self, factor: f64) -> Rgb {
        let (h, s, v) = rgb_to_hsv(self);
        let mut v = v * factor;
        let mut s = s;
        if v > 1.0 {
            s = (s - (v - 1.0)).max(0.0);
            v = 1.0;
        }
        hsv_to_rgb(h, s, v)
    }
    pub fn hsl(h: f64, s: f64, l: f64) -> Rgb {
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let hp = (h.rem_euclid(1.0)) * 6.0;
        let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
        let (r, g, b) = match hp as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = l - c / 2.0;
        Rgb(r + m, g + m, b + m)
    }
}

fn rgb_to_hsv(c: Rgb) -> (f64, f64, f64) {
    let (r, g, b) = (c.0, c.1, c.2);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> Rgb {
    let c = v * s;
    let hp = h.rem_euclid(1.0) * 6.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    Rgb(r + m, g + m, b + m)
}

/// Load colour for 0..1: green, then yellow, then red.
pub fn heat(f: f64) -> Rgb {
    let f = f.clamp(0.0, 1.0);
    if f < 0.5 {
        Rgb(0.26 + f * 1.3, 0.79, 0.42 - f * 0.4)
    } else {
        Rgb(0.91, 0.79 - (f - 0.5) * 1.1, 0.22 - (f - 0.5) * 0.2)
    }
}

/// Temperature colour: green, yellow from 60°, red from 80°.
pub fn temp_color(t: f64) -> &'static str {
    if t >= 80.0 {
        "#e05a4f"
    } else if t >= 60.0 {
        "#e3c25a"
    } else {
        "#43c96b"
    }
}

/// A container's colour: its name's hue (transparent for none / "Host").
pub fn container_color(name: &str) -> Option<Rgb> {
    (!name.is_empty() && name != "Host").then(|| Rgb::hsl(hue(name), 0.6, 0.55))
}

/// Cuts to `chars` with "…" and pads (right- or left-aligned), plus the
/// one-space column gap.
pub fn pad(text: &str, chars: usize, right: bool) -> String {
    let n = text.chars().count();
    let t: String = if n > chars {
        text.chars().take(chars.saturating_sub(1)).chain(std::iter::once('…')).collect()
    } else {
        text.to_string()
    };
    let fill = " ".repeat(chars.saturating_sub(t.chars().count()));
    if right {
        format!("{fill}{t} ")
    } else {
        format!("{t}{fill} ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_times() {
        assert_eq!(bytes(0.0), "0B");
        assert_eq!(bytes(1536.0), "1.5K");
        assert_eq!(bytes(23.0 * 1024.0 * 1024.0), "23M");
        assert_eq!(bytes(1023.0), "1023B");
        assert_eq!(pct(99.96), "100%");
        assert_eq!(pct(12.34), "12.3%");
        assert_eq!(fixed(0.25, 1), "0.3", "halves round up, like JS toFixed");
        assert_eq!(cpu_time(75.5), "1:15.50");
        assert_eq!(cpu_time(3725.0), "1h02:05");
        assert_eq!(cpu_time(86400.0 * 12.0 + 7200.0), "12d02h");
        assert_eq!(uptime(86400.0 + 3600.0 * 4.0 + 17.0 * 60.0), "1 day, 04:17");
        assert_eq!(pad("abcdef", 4, false), "abc… ");
        assert_eq!(pad("ab", 4, true), "  ab ");
    }

    #[test]
    fn colours() {
        assert_eq!(Rgb::hex("#43c96b").to_hex(), "#43c96b");
        assert_eq!(heat(0.0).to_hex(), "#42c96b");
        // Same hash as the QML's (h*31 + code unit) >>> 0.
        assert!((hue("seerr") - ((("seerr".encode_utf16().fold(0u32, |h, c| h.wrapping_mul(31).wrapping_add(c as u32))) % 360) as f64 / 360.0)).abs() < 1e-12);
    }
}
