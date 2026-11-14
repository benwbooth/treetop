//! treetop's settings file, shared by both front ends: the Qt app's QML
//! `Settings` write `~/.config/treetop/treetop.conf` (QSettings' INI form,
//! keys in `[General]`), and the terminal UI reads and writes the same keys
//! in the same encoding. Other groups (the window's `[MainWindow]`) are kept
//! as they are.

use std::path::PathBuf;
use std::time::SystemTime;

pub fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"));
    base.join("treetop").join("treetop.conf")
}

#[derive(Default, Debug, Clone)]
pub struct Settings {
    path: PathBuf,
    /// (group, [(raw key, raw value)]), in file order.
    groups: Vec<(String, Vec<(String, String)>)>,
    loaded_at: Option<SystemTime>,
}

/// Qt's INI escaping, read side: "quoted strings", \" \\ \n \t \r \0 \xHHHH.
pub fn decode(raw: &str) -> String {
    let raw = raw.trim();
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {}
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('0') => out.push('\0'),
                Some('x') => {
                    let mut hex = String::new();
                    while let Some(h) = chars.peek().copied().filter(|h| h.is_ascii_hexdigit()) {
                        hex.push(h);
                        chars.next();
                        if hex.len() == 4 {
                            break;
                        }
                    }
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                Some(o) => out.push(o),
                None => {}
            },
            _ => out.push(c),
        }
    }
    // "@@..." is an escaped leading '@' ("@Type(...)" values are Qt types).
    if let Some(rest) = out.strip_prefix("@@") {
        return format!("@{rest}");
    }
    out
}

/// Qt's INI escaping, write side: quoted when it holds anything that would
/// otherwise read back differently.
pub fn encode(v: &str) -> String {
    let v = if v.starts_with('@') { format!("@{v}") } else { v.to_string() };
    let needs = v.contains([',', ';', '=', '"', '\\', '#'])
        || v.starts_with(char::is_whitespace)
        || v.ends_with(char::is_whitespace)
        || v.chars().any(|c| c.is_control());
    let mut out = String::new();
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            c if c.is_control() => out.push_str(&format!("\\x{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    if needs { format!("\"{out}\"") } else { out }
}

impl Settings {
    /// The shared settings file (missing is fine: everything defaults).
    pub fn load() -> Settings {
        Self::load_from(path())
    }

    pub fn load_from(path: PathBuf) -> Settings {
        let mut s = Settings { path, ..Default::default() };
        s.reload();
        s
    }

    fn mtime(&self) -> Option<SystemTime> {
        std::fs::metadata(&self.path).and_then(|m| m.modified()).ok()
    }

    /// Re-read the file if someone else (the Qt app) changed it. True when
    /// it did.
    pub fn refresh(&mut self) -> bool {
        if self.mtime() != self.loaded_at {
            self.reload();
            return true;
        }
        false
    }

    fn reload(&mut self) {
        self.groups.clear();
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        self.parse(&text);
        self.loaded_at = self.mtime();
    }

    fn parse(&mut self, text: &str) {
        let mut cur: Option<usize> = None;
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with(';') || t.starts_with('#') {
                continue;
            }
            if t.starts_with('[') && t.ends_with(']') {
                self.groups.push((t[1..t.len() - 1].to_string(), Vec::new()));
                cur = Some(self.groups.len() - 1);
                continue;
            }
            let Some((k, v)) = t.split_once('=') else { continue };
            let idx = match cur {
                Some(i) => i,
                None => {
                    self.groups.push(("General".into(), Vec::new()));
                    let i = self.groups.len() - 1;
                    cur = Some(i);
                    i
                }
            };
            self.groups[idx].1.push((k.trim().to_string(), v.trim().to_string()));
        }
    }

    fn general(&self) -> Option<&Vec<(String, String)>> {
        self.groups.iter().find(|(g, _)| g == "General").map(|(_, v)| v)
    }

    /// A key's value as written in the file (still encoded).
    pub fn raw(&self, key: &str) -> Option<&str> {
        self.general()?.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// Sets an already-encoded value.
    pub fn set_raw(&mut self, key: &str, raw: &str) {
        if self.general().is_none() {
            self.groups.insert(0, ("General".into(), Vec::new()));
        }
        let g = &mut self.groups.iter_mut().find(|(g, _)| g == "General").unwrap().1;
        match g.iter_mut().find(|(k, _)| k == key) {
            Some(e) => e.1 = raw.to_string(),
            None => g.push((key.to_string(), raw.to_string())),
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.general()?.iter().find(|(k, _)| k == key).map(|(_, v)| decode(v))
    }

    pub fn str_or(&self, key: &str, default: &str) -> String {
        self.get(key).unwrap_or_else(|| default.to_string())
    }

    pub fn bool_or(&self, key: &str, default: bool) -> bool {
        match self.get(key).as_deref() {
            Some("true") => true,
            Some("false") => false,
            _ => default,
        }
    }

    pub fn f64_or(&self, key: &str, default: f64) -> f64 {
        self.get(key).and_then(|v| v.parse().ok()).unwrap_or(default)
    }

    /// A comma-separated list setting ("a,b,c").
    pub fn list(&self, key: &str, default: &str) -> Vec<String> {
        self.str_or(key, default).split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
    }

    /// A JSON-valued setting (columnWidths, columnOrder).
    pub fn json(&self, key: &str) -> Option<serde_json::Value> {
        self.get(key).and_then(|v| serde_json::from_str(&v).ok())
    }

    /// Sets a value (as Qt would write it); saved by `save`.
    pub fn set(&mut self, key: &str, value: &str) {
        let enc = encode(value);
        if self.general().is_none() {
            self.groups.insert(0, ("General".into(), Vec::new()));
        }
        let g = &mut self.groups.iter_mut().find(|(g, _)| g == "General").unwrap().1;
        match g.iter_mut().find(|(k, _)| k == key) {
            Some(e) => e.1 = enc,
            None => g.push((key.to_string(), enc)),
        }
    }

    pub fn set_bool(&mut self, key: &str, v: bool) {
        self.set(key, if v { "true" } else { "false" });
    }

    pub fn set_num(&mut self, key: &str, v: f64) {
        let s = if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{v}") };
        self.set(key, &s);
    }

    pub fn set_list(&mut self, key: &str, items: &[String]) {
        self.set(key, &items.join(","));
    }

    pub fn set_json(&mut self, key: &str, v: &serde_json::Value) {
        self.set(key, &v.to_string());
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (i, (g, kv)) in self.groups.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&format!("[{g}]\n"));
            for (k, v) in kv {
                out.push_str(&format!("{k}={v}\n"));
            }
        }
        out
    }

    /// Writes the file (atomically: a temp file renamed over it). To keep
    /// the other app's newer values, change settings through `update`.
    pub fn save(&mut self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("conf.tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(&tmp, &self.path)?;
        self.loaded_at = self.mtime();
        Ok(())
    }
}

/// A front end's private working copy of the shared settings, so changes
/// carry over only when it exits: `begin_session` copies the shared file to
/// `<name>.conf` (and a `<name>.base.conf` snapshot); the app reads and
/// writes only its copy while it runs; `end_session` merges the keys it
/// changed into the shared file. A session left by an unclean exit is
/// merged at the next start.
fn session_paths(name: &str) -> (PathBuf, PathBuf) {
    let p = path();
    (p.with_file_name(format!("{name}.conf")), p.with_file_name(format!("{name}.base.conf")))
}

pub fn begin_session(name: &str) -> PathBuf {
    end_session(name);
    let (private, base) = session_paths(name);
    if let Some(dir) = private.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = std::fs::read_to_string(path()).unwrap_or_default();
    let _ = std::fs::write(&base, &text);
    let _ = std::fs::write(&private, &text);
    private
}

pub fn end_session(name: &str) {
    let (private, base) = session_paths(name);
    if !private.exists() {
        let _ = std::fs::remove_file(&base);
        return;
    }
    let mine = Settings::load_from(private.clone());
    let before = Settings::load_from(base.clone());
    let changed: Vec<(String, String)> = mine
        .general()
        .map(|kv| kv.iter().filter(|(k, v)| before.raw(k) != Some(v.as_str())).cloned().collect())
        .unwrap_or_default();
    if !changed.is_empty() {
        let _ = update(|s| {
            for (k, v) in &changed {
                s.set_raw(k, v);
            }
        });
    }
    let _ = std::fs::remove_file(&private);
    let _ = std::fs::remove_file(&base);
}

/// The window's heartbeat (kept out of the settings file, which the Qt app
/// rewrites whole whenever it saves): {t: ms, visible}.
fn alive_path() -> PathBuf {
    path().with_file_name("alive.json")
}

pub fn mark_alive(visible: bool) {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0);
    let _ = std::fs::write(alive_path(), serde_json::json!({"t": t, "visible": visible}).to_string());
}

pub fn last_alive() -> serde_json::Value {
    std::fs::read_to_string(alive_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(serde_json::json!({"t": 0, "visible": true}))
}

/// A batch of changes applied onto the current file contents, so writing
/// one key never overwrites the other app's newer value of another.
pub fn update(f: impl FnOnce(&mut Settings)) -> std::io::Result<Settings> {
    let mut s = Settings::load();
    f(&mut s);
    s.save()?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Encodings as Qt wrote them in a real treetop.conf.
    const REAL: &str = r#"[General]
collapsedSections="sensors,storage"
columnOrder="[\"pid\",\"user\",\"cpu\"]"
columnWidths="{\"user\":16}"
filterText=
hiddenGpus=0000:59:00.0
interval=1000
lastAlive=1794615695911
showKernel=false

[MainWindow]
frameGeometry=@Rect(985 0 1969 1618)
maximized=false
"#;

    #[test]
    fn reads_qt_encodings() {
        let mut s = Settings::default();
        s.parse(REAL);
        assert_eq!(s.list("collapsedSections", ""), vec!["sensors", "storage"]);
        assert_eq!(s.json("columnOrder").unwrap(), serde_json::json!(["pid", "user", "cpu"]));
        assert_eq!(s.json("columnWidths").unwrap()["user"], 16);
        assert_eq!(s.str_or("filterText", "x"), "");
        assert_eq!(s.str_or("hiddenGpus", ""), "0000:59:00.0");
        assert_eq!(s.f64_or("interval", 0.0), 1000.0);
        assert!(!s.bool_or("showKernel", true));
        assert_eq!(s.get("frameGeometry"), None, "only [General] is ours");
    }

    #[test]
    fn sessions_merge_only_changes() {
        let dir = std::env::temp_dir().join(format!("treetop-settings-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        std::fs::create_dir_all(dir.join("treetop")).unwrap();
        std::fs::write(path(), "[General]\nview=tree\nsortKey=cpu\n").unwrap();
        let mine = begin_session("window");
        // The window changes the sort; meanwhile the terminal changes the view.
        std::fs::write(&mine, "[General]\nview=tree\nsortKey=mem\n").unwrap();
        update(|s| s.set("view", "flat")).unwrap();
        end_session("window");
        let s = Settings::load();
        assert_eq!(s.str_or("sortKey", ""), "mem", "the window's change merged");
        assert_eq!(s.str_or("view", ""), "flat", "the terminal's change kept");
        assert!(!mine.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writes_what_qt_reads() {
        let mut s = Settings::default();
        s.parse(REAL);
        s.set_list("collapsedSections", &["mem".into(), "gpu".into()]);
        s.set_json("columnOrder", &serde_json::json!(["cpu", "pid"]));
        s.set("filterText", "fire fox");
        s.set_bool("tintColumns", true);
        s.set_num("interval", 500.0);
        let t = s.to_text();
        assert!(t.contains("collapsedSections=\"mem,gpu\"\n"));
        assert!(t.contains("columnOrder=\"[\\\"cpu\\\",\\\"pid\\\"]\"\n"));
        assert!(t.contains("filterText=fire fox\n"));
        assert!(t.contains("tintColumns=true\n"));
        assert!(t.contains("interval=500\n"));
        assert!(t.contains("[MainWindow]\nframeGeometry=@Rect(985 0 1969 1618)\n"), "other groups kept");
        let mut back = Settings::default();
        back.parse(&t);
        assert_eq!(back.json("columnOrder").unwrap(), serde_json::json!(["cpu", "pid"]));
        assert_eq!(back.str_or("filterText", ""), "fire fox");
    }
}
