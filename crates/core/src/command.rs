//! Command lines: shortening /nix/store paths, and splitting a command into
//! coloured spans (program dir dim, program name bold, then each argument by
//! kind: flags, values, paths, numbers, URLs). Each front end draws the
//! spans its own way (HTML in the Qt app, styled cells in the terminal).

/// How /nix/store/<hash>- paths are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreStyle {
    Full,
    /// /nix/store/abcde…-name
    Hash,
    /// /n…/s…/abcde…-name
    Compact,
}

impl StoreStyle {
    pub fn parse(s: &str) -> StoreStyle {
        match s {
            "full" => StoreStyle::Full,
            "hash" => StoreStyle::Hash,
            _ => StoreStyle::Compact,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            StoreStyle::Full => "full",
            StoreStyle::Hash => "hash",
            StoreStyle::Compact => "compact",
        }
    }
}

fn nix32(c: u8) -> bool {
    c.is_ascii_digit() || (c.is_ascii_lowercase() && !matches!(c, b'e' | b'o' | b't' | b'u'))
}

/// /nix/store/<32-char hash>-name -> /nix/store/<first 5>…-name: short,
/// but different builds of the same package stay distinguishable.
pub fn tidy(s: &str, style: StoreStyle) -> String {
    if style == StoreStyle::Full {
        return s.to_string();
    }
    const P: &str = "/nix/store/";
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i..].starts_with(P) {
            let h = i + P.len();
            if h + 33 <= b.len() && b[h..h + 32].iter().all(|&c| nix32(c)) && b[h + 32] == b'-' {
                out.push_str(if style == StoreStyle::Compact { "/n…/s…/" } else { P });
                out.push_str(&s[h..h + 5]);
                out.push_str("…-");
                i = h + 33;
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// The program's directory, kernel-thread names.
    Dir,
    /// The program name (bold).
    Prog,
    Flag,
    Eq,
    Value,
    Path,
    /// The last component of a path.
    Leaf,
    Number,
    Url,
    Plain,
    /// Runs of spaces between tokens.
    Space,
    /// Line continuations in the pretty layout.
    Faint,
}

impl Kind {
    /// (colour, alpha over the text colour) for dark or light backgrounds;
    /// None = the normal text colour at that alpha.
    pub fn color(self, dark: bool) -> (Option<&'static str>, f64) {
        use Kind::*;
        match (self, dark) {
            (Dir | Eq, true) => (None, 0.45),
            (Dir | Eq, false) => (None, 0.5),
            (Faint, _) => (None, 0.18),
            (Prog, true) => (Some("#7fe08f"), 1.0),
            (Prog, false) => (Some("#17803a"), 1.0),
            (Flag, true) => (Some("#56b6e0"), 1.0),
            (Flag, false) => (Some("#16709a"), 1.0),
            (Value, true) => (Some("#e3c25a"), 1.0),
            (Value, false) => (Some("#8f6a00"), 1.0),
            (Path, true) => (Some("#7f97c9"), 1.0),
            (Path, false) => (Some("#3f5d99"), 1.0),
            (Leaf, true) => (Some("#a9c1f2"), 1.0),
            (Leaf, false) => (Some("#203f80"), 1.0),
            (Number, true) => (Some("#c98ae0"), 1.0),
            (Number, false) => (Some("#86379f"), 1.0),
            (Url, true) => (Some("#e8995a"), 1.0),
            (Url, false) => (Some("#a8541a"), 1.0),
            (Plain | Space, _) => (None, 1.0),
        }
    }
    pub fn bold(self) -> bool {
        self == Kind::Prog
    }
    pub fn name(self) -> &'static str {
        use Kind::*;
        match self {
            Dir => "dir",
            Prog => "prog",
            Flag => "flag",
            Eq => "eq",
            Value => "value",
            Path => "path",
            Leaf => "leaf",
            Number => "number",
            Url => "url",
            Plain => "plain",
            Space => "space",
            Faint => "faint",
        }
    }
    pub fn from_name(s: &str) -> Kind {
        use Kind::*;
        match s {
            "dir" => Dir,
            "prog" => Prog,
            "flag" => Flag,
            "eq" => Eq,
            "value" => Value,
            "path" => Path,
            "leaf" => Leaf,
            "number" => Number,
            "url" => Url,
            "space" => Space,
            "faint" => Faint,
            _ => Plain,
        }
    }
}

/// Spans as JSON: [[kind, text], ...].
pub fn spans_json(spans: &Spans) -> serde_json::Value {
    serde_json::Value::Array(spans.iter().map(|(k, t)| serde_json::json!([k.name(), t])).collect())
}

pub fn spans_from_json(v: &serde_json::Value) -> Spans {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|p| (Kind::from_name(p[0].as_str().unwrap_or("")), p[1].as_str().unwrap_or("").to_string()))
        .collect()
}

/// Spans as Qt rich text, coloured for a dark or light background over
/// the text colour `text` (#rrggbb).
pub fn html(spans: &Spans, dark: bool, text: &str) -> String {
    let base = crate::format::Rgb::hex(text);
    let mut out = String::new();
    for (k, t) in spans {
        if *k == Kind::Space {
            out.push_str(&t.replace(' ', "&nbsp;"));
            continue;
        }
        let esc = t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let (c, a) = k.color(dark);
        let color = match c {
            Some(c) => c.to_string(),
            None if a >= 1.0 => text.to_string(),
            // #AARRGGBB, as Qt.rgba() printed it.
            None => {
                let (r, g, b) = base.bytes();
                format!("#{:02x}{r:02x}{g:02x}{b:02x}", (a * 255.0).round() as u8)
            }
        };
        let body = if k.bold() { format!("<b>{esc}</b>") } else { esc };
        out.push_str(&format!("<font color='{color}'>{body}</font>"));
    }
    out
}

pub type Spans = Vec<(Kind, String)>;

fn push(out: &mut Spans, k: Kind, t: &str) {
    if !t.is_empty() {
        out.push((k, t.to_string()));
    }
}

fn path(out: &mut Spans, tok: &str) {
    match tok.rfind('/') {
        Some(i) if i + 1 < tok.len() => {
            push(out, Kind::Path, &tok[..=i]);
            push(out, Kind::Leaf, &tok[i + 1..]);
        }
        _ => push(out, Kind::Path, tok),
    }
}

/// -?[0-9][0-9.,:<extra>]*
fn numeric(t: &str, extra: &str) -> bool {
    let t = t.strip_prefix('-').unwrap_or(t);
    let mut cs = t.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_digit()) && cs.all(|c| c.is_ascii_digit() || ".,:".contains(c) || extra.contains(c))
}

fn value(out: &mut Spans, v: &str) {
    if v.find("://").is_some_and(|i| i > 0) {
        push(out, Kind::Url, v);
    } else if v.contains('/') {
        path(out, v);
    } else if numeric(v, "xXabcdefABCDEFkKmMgG") {
        push(out, Kind::Number, v);
    } else {
        push(out, Kind::Value, v);
    }
}

fn is_flag(tok: &str) -> bool {
    tok.starts_with('-') && tok.len() > 1 && !tok[1..].starts_with(|c: char| c.is_ascii_digit())
}

fn ident(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_alphabetic() || c == '_') && cs.all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
}

/// One argument's spans.
pub fn arg(out: &mut Spans, tok: &str) {
    if is_flag(tok) {
        match tok.find('=') {
            Some(eq) if eq > 0 => {
                push(out, Kind::Flag, &tok[..eq]);
                push(out, Kind::Eq, "=");
                value(out, &tok[eq + 1..]);
            }
            _ => push(out, Kind::Flag, tok),
        }
        return;
    }
    if let Some(eq) = tok.find('=') {
        if eq > 0 && ident(&tok[..eq]) {
            push(out, Kind::Flag, &tok[..eq]);
            push(out, Kind::Eq, "=");
            value(out, &tok[eq + 1..]);
            return;
        }
    }
    if tok.find("://").is_some_and(|i| i > 0) {
        push(out, Kind::Url, tok);
    } else if tok.contains('/') {
        path(out, tok);
    } else if numeric(tok, "") {
        push(out, Kind::Number, tok);
    } else if tok.starts_with('[') && tok.ends_with(']') {
        push(out, Kind::Dir, tok); // kernel threads
    } else {
        push(out, Kind::Plain, tok);
    }
}

/// In-row commands are capped (the tooltip has the whole thing).
pub const MAX_ROW_COMMAND: usize = 500;

/// The row version: shortened store paths, capped length, spaces kept.
pub fn highlight(command: &str, style: StoreStyle) -> Spans {
    let mut cmd = tidy(command, style);
    if cmd.chars().count() > MAX_ROW_COMMAND {
        cmd = cmd.chars().take(MAX_ROW_COMMAND).collect::<String>() + " …";
    }
    let mut out = Spans::new();
    // Split into tokens and runs of spaces, keeping both.
    let mut parts: Vec<&str> = Vec::new();
    let mut start = 0;
    let mut in_space = cmd.starts_with(' ');
    for (i, c) in cmd.char_indices() {
        if (c == ' ') != in_space {
            parts.push(&cmd[start..i]);
            start = i;
            in_space = c == ' ';
        }
    }
    parts.push(&cmd[start..]);
    // JS split(/( +)/) puts the program first even when empty.
    if cmd.starts_with(' ') {
        parts.insert(0, "");
    }
    let prog = parts.first().copied().unwrap_or("");
    let slash = prog.rfind('/').map(|i| i + 1).unwrap_or(0);
    push(&mut out, Kind::Dir, &prog[..slash]);
    push(&mut out, Kind::Prog, &prog[slash..]);
    for p in parts.iter().skip(1) {
        if p.chars().all(|c| c == ' ') {
            push(&mut out, Kind::Space, p);
        } else {
            arg(&mut out, p);
        }
    }
    out
}

/// The tooltip/details version, unshortened: the executable on the first
/// line (with the name it was run as, when different), then one argument
/// per line, keeping "--flag value" pairs together. `exe` ending in "?" is
/// a guess from PATH.
pub fn pretty(command: &str, exe: &str) -> Vec<Spans> {
    let toks: Vec<&str> = command.split(' ').filter(|t| !t.is_empty()).collect();
    if toks.is_empty() {
        return Vec::new();
    }
    let guessed = exe.ends_with('?');
    let path = if guessed { &exe[..exe.len() - 1] } else { exe };
    let prog = if path.is_empty() { toks[0] } else { path };
    let slash = prog.rfind('/').map(|i| i + 1).unwrap_or(0);
    let mut first = Spans::new();
    push(&mut first, Kind::Dir, &prog[..slash]);
    push(&mut first, Kind::Prog, &prog[slash..]);
    if guessed {
        push(&mut first, Kind::Dir, "  (? found on PATH; the real binary isn't readable without root)");
    }
    if !path.is_empty() && toks[0] != path {
        push(&mut first, Kind::Dir, &format!("  as {}", toks[0]));
    }
    let mut lines = vec![first];
    let mut i = 1;
    while i < toks.len() {
        let t = toks[i];
        let mut line: Spans = vec![(Kind::Space, "    ".into())];
        let flag = is_flag(t) && !t.contains('=');
        if flag && i + 1 < toks.len() && !toks[i + 1].starts_with('-') {
            arg(&mut line, t);
            line.push((Kind::Space, " ".into()));
            arg(&mut line, toks[i + 1]);
            i += 2;
        } else {
            arg(&mut line, t);
            i += 1;
        }
        lines.push(line);
    }
    // Shell-style line continuations, barely visible.
    let n = lines.len();
    for l in lines.iter_mut().take(n - 1) {
        l.push((Kind::Space, " ".into()));
        l.push((Kind::Faint, "\\".into()));
    }
    lines
}

/// An environment variable as lines of spans: NAME=value, with PATH-like
/// lists (a:b:c, not URLs) one entry per line, aligned under the value.
pub fn env_lines(name: &str, val: &str) -> Vec<Spans> {
    let parts: Vec<&str> = val.split(':').collect();
    let list = val.find(':').is_some_and(|i| i > 0) && parts.len() > 2 && !val.contains("://");
    let mut first = Spans::new();
    push(&mut first, Kind::Flag, name);
    push(&mut first, Kind::Eq, "=");
    if !list {
        value(&mut first, val);
        return vec![first];
    }
    value(&mut first, parts[0]);
    let mut lines = vec![first];
    let indent = " ".repeat(name.chars().count() + 1);
    for p in &parts[1..] {
        let mut l: Spans = vec![(Kind::Space, indent.clone())];
        value(&mut l, p);
        lines.push(l);
    }
    lines
}

/// Plain text of spans.
pub fn text(spans: &Spans) -> String {
    spans.iter().map(|(_, t)| t.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    const H: &str = "/nix/store/zsd35lbngm1hnknnbwa3nsfhw80sf03w-whisper.py";

    #[test]
    fn store_paths() {
        assert_eq!(tidy(H, StoreStyle::Compact), "/n…/s…/zsd35…-whisper.py");
        assert_eq!(tidy(H, StoreStyle::Hash), "/nix/store/zsd35…-whisper.py");
        assert_eq!(tidy(H, StoreStyle::Full), H);
        // 'e' isn't in Nix's base32: not a store hash, left alone.
        let not = "/nix/store/eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-x";
        assert_eq!(tidy(not, StoreStyle::Compact), not);
    }

    #[test]
    fn kinds() {
        let s = highlight("/usr/bin/python3 -u --port=9090 FOO=bar /srv/x.py 42 http://a [kworker]", StoreStyle::Full);
        let k: Vec<(Kind, &str)> = s.iter().filter(|(k, _)| *k != Kind::Space).map(|(k, t)| (*k, t.as_str())).collect();
        assert_eq!(k, vec![
            (Kind::Dir, "/usr/bin/"), (Kind::Prog, "python3"), (Kind::Flag, "-u"),
            (Kind::Flag, "--port"), (Kind::Eq, "="), (Kind::Number, "9090"),
            (Kind::Flag, "FOO"), (Kind::Eq, "="), (Kind::Value, "bar"),
            (Kind::Path, "/srv/"), (Kind::Leaf, "x.py"), (Kind::Number, "42"),
            (Kind::Url, "http://a"), (Kind::Dir, "[kworker]"),
        ]);
        assert_eq!(text(&s), "/usr/bin/python3 -u --port=9090 FOO=bar /srv/x.py 42 http://a [kworker]");
    }

    #[test]
    fn pretty_layout() {
        let l = pretty("python3 -u --model m.gguf -v", "/usr/bin/python3");
        let t: Vec<String> = l.iter().map(text).collect();
        assert_eq!(t, vec!["/usr/bin/python3  as python3 \\", "    -u \\", "    --model m.gguf \\", "    -v"]);
    }
}
