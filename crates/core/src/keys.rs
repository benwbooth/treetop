//! The keys both front ends share: the help text and the footer bar.

/// Which front end the text is for: a few keys differ (the window hides to
/// the tray; the terminal quits).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Frontend {
    Window,
    Terminal,
}

/// (keys, what they do), as the help shows them.
pub fn help(fe: Frontend) -> Vec<(&'static str, &'static str)> {
    let fold = if fe == Frontend::Window { ("+ / − / ← →", "expand / collapse subtree") } else { ("+ / −", "expand / collapse subtree") };
    let sideways = if fe == Frontend::Window {
        ("Shift+← →", "scroll sideways (Shift+wheel too)")
    } else {
        ("← → / < >", "scroll sideways (Alt or Ctrl+wheel too, or the scrollbar)")
    };
    let mut v = vec![
        ("↑ ↓ PgUp PgDn Home End", "move"),
        ("Space", "tag process (Ctrl+click too)"),
        ("U", "untag all"),
        fold,
        sideways,
        ("* / _", "expand all / collapse all"),
        ("Ctrl+→ / Ctrl+←", "expand / collapse everything below"),
        ("Click ▸/▾", "fold (or the tree lines); Shift+click: whole subtree"),
        ("Enter", "details"),
        ("f / double-click", "show only this process tree (again or Esc: all)"),
        ("F9 / k", "signal menu (right-click too)"),
        ("F7 / ]", "nice − (higher priority)"),
        ("F8 / [", "nice + (lower priority)"),
        ("F5 / t", "tree ⇄ list"),
        ("c", "group by container"),
        ("g", "GPU view (amdgpu_top-style)"),
        ("1", "per-core ⇄ combined CPU meter"),
        ("r", "collapse / expand resources"),
        ("K", "show kernel threads"),
        ("P / M / T", "sort by CPU / memory / time"),
        ("F3 / /", "filter"),
    ];
    match fe {
        Frontend::Window => {
            v.push(("Esc / F10 / q", "hide to tray     Ctrl+Q quit"));
            v.push(("Ctrl+= / Ctrl+- / Ctrl+0", "font size"));
        }
        Frontend::Terminal => {
            v.push(("Scrollbars", "drag the thumb, click the track to page"));
            v.push(("Esc", "clear the filter, leave focus"));
            v.push(("F10 / q / Ctrl+Q", "quit"));
            v.push(("F2", "options menu"));
        }
    }
    v.push(("Right-click the header", "choose columns"));
    v
}

pub const HELP_FOOTNOTE: &str = "Signalling someone else's process (e.g. root's)\nasks for authorization through polkit.";

/// What a footer entry does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FooterAction {
    Help,
    Search,
    TreeList,
    Containers,
    Gpu,
    NiceDown,
    NiceUp,
    Kill,
    HideOrQuit,
}

/// The footer bar: (key, label, action). `view` picks F5's label.
pub fn footer(fe: Frontend, view: &str) -> Vec<(&'static str, &'static str, FooterAction)> {
    vec![
        ("F1", "Help", FooterAction::Help),
        ("F3", "Search", FooterAction::Search),
        ("F5", if view == "tree" { "List" } else { "Tree" }, FooterAction::TreeList),
        ("c", "Containers", FooterAction::Containers),
        ("g", "GPU", FooterAction::Gpu),
        ("F7", "Nice-", FooterAction::NiceDown),
        ("F8", "Nice+", FooterAction::NiceUp),
        ("F9", "Kill", FooterAction::Kill),
        ("F10", if fe == Frontend::Window { "Hide" } else { "Quit" }, FooterAction::HideOrQuit),
    ]
}

/// The help as one text (keys padded to a column).
pub fn help_text(fe: Frontend) -> String {
    let rows = help(fe);
    let w = rows.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0).min(24);
    let mut s = String::new();
    for (k, d) in rows {
        let pad = w.saturating_sub(k.chars().count()) + 2;
        s += &format!("{k}{}{d}\n", " ".repeat(pad));
    }
    s + "\n" + HELP_FOOTNOTE
}
