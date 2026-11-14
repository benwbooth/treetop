//! Keys and mouse, matching the Qt app's bindings.

use crate::app::{Act, App, Drag, Item, Menu, Popup};
use crate::ui::{Geometry, Hit};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::time::{Duration, Instant};
use treetop_core::keys::FooterAction;

fn footer_act(a: FooterAction, view: &str) -> Act {
    match a {
        FooterAction::Help => Act::Help,
        FooterAction::Search => Act::EditFilter,
        FooterAction::TreeList => Act::SetView(if view == "tree" { "flat" } else { "tree" }),
        FooterAction::Containers => Act::ToggleView("containers"),
        FooterAction::Gpu => Act::ToggleView("gpu"),
        FooterAction::NiceDown => Act::Renice(-1),
        FooterAction::NiceUp => Act::Renice(1),
        FooterAction::Kill => Act::SignalMenu,
        FooterAction::HideOrQuit => Act::Quit,
    }
}

/// Activates menu item `i` of menu `level`: runs it, or opens its submenu.
fn activate(app: &mut App, level: usize, i: usize) {
    let Some(Popup::Menus(stack)) = &mut app.popup else { return };
    stack.truncate(level + 1);
    let Some(m) = stack.get_mut(level) else { return };
    m.sel = i;
    match m.items.get(i).cloned() {
        Some(Item::Sub(_, items)) => stack.push(Menu::new(items, 0, 0)),
        Some(Item::Act(_, a)) | Some(Item::Check(_, _, a)) => {
            if a != Act::Nothing {
                app.popup = None;
                app.run(a);
            }
        }
        _ => {}
    }
}

pub fn key(app: &mut App, k: KeyEvent) {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    if ctrl && matches!(k.code, KeyCode::Char('q') | KeyCode::Char('c')) {
        app.quit = true;
        return;
    }
    // Pop-ups first.
    match &mut app.popup {
        Some(Popup::Menus(stack)) => {
            let level = stack.len() - 1;
            let m = stack.last_mut().unwrap();
            match k.code {
                KeyCode::Up | KeyCode::Char('k') => m.step(-1),
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => m.step(1),
                KeyCode::Enter | KeyCode::Right | KeyCode::Char(' ') => {
                    let sel = m.sel;
                    activate(app, level, sel);
                }
                KeyCode::Left | KeyCode::Esc => {
                    if stack.len() > 1 && k.code == KeyCode::Left {
                        stack.pop();
                    } else {
                        app.popup = None;
                    }
                }
                _ => {}
            }
            return;
        }
        Some(Popup::Help(scroll)) => {
            match k.code {
                KeyCode::Down | KeyCode::Char('j') => *scroll += 1,
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                _ => app.popup = None,
            }
            return;
        }
        Some(Popup::Details(d)) => {
            if d.editing_filter {
                match k.code {
                    KeyCode::Char(c) => d.env_filter.push(c),
                    KeyCode::Backspace => {
                        d.env_filter.pop();
                    }
                    KeyCode::Esc => {
                        d.env_filter.clear();
                        d.editing_filter = false;
                    }
                    KeyCode::Enter | KeyCode::Down => d.editing_filter = false,
                    _ => {}
                }
                d.scroll[1] = 0;
                return;
            }
            let page = 10;
            match k.code {
                KeyCode::Esc | KeyCode::Char('q') => app.popup = None,
                KeyCode::Left | KeyCode::BackTab => d.tab = (d.tab + 2) % 3,
                KeyCode::Right | KeyCode::Tab => d.tab = (d.tab + 1) % 3,
                KeyCode::Char('1') => d.tab = 0,
                KeyCode::Char('2') => d.tab = 1,
                KeyCode::Char('3') => d.tab = 2,
                KeyCode::Down | KeyCode::Char('j') => d.scroll[d.tab] += 1,
                KeyCode::Up | KeyCode::Char('k') => d.scroll[d.tab] = d.scroll[d.tab].saturating_sub(1),
                KeyCode::PageDown | KeyCode::Char(' ') => d.scroll[d.tab] += page,
                KeyCode::PageUp => d.scroll[d.tab] = d.scroll[d.tab].saturating_sub(page),
                KeyCode::Home => d.scroll[d.tab] = 0,
                KeyCode::End => d.scroll[d.tab] = usize::MAX / 2,
                KeyCode::Char('/') if d.tab == 1 => d.editing_filter = true,
                KeyCode::Char('r') => app.inspect_as_root(),
                _ => {}
            }
            return;
        }
        None => {}
    }
    // The filter box.
    if app.editing_filter {
        let mut f = app.opts.filter.clone();
        match k.code {
            KeyCode::Char('u') if ctrl => f.clear(),
            KeyCode::Char(c) => f.push(c),
            KeyCode::Backspace => {
                f.pop();
            }
            KeyCode::Esc => {
                f.clear();
                app.editing_filter = false;
            }
            KeyCode::Enter | KeyCode::Down | KeyCode::Tab => {
                app.editing_filter = false;
                return;
            }
            _ => return,
        }
        app.set_filter(f);
        return;
    }
    match k.code {
        KeyCode::F(1) | KeyCode::Char('h') | KeyCode::Char('?') => app.run(Act::Help),
        KeyCode::F(2) => app.run(Act::Options),
        KeyCode::F(3) | KeyCode::Char('/') => app.run(Act::EditFilter),
        KeyCode::Char('f') if ctrl => app.run(Act::EditFilter),
        KeyCode::F(5) | KeyCode::Char('t') => {
            let v = if app.view() == "tree" { "flat" } else { "tree" };
            app.set_view(v);
        }
        KeyCode::Char('c') => app.run(Act::ToggleView("containers")),
        KeyCode::Char('g') => app.run(Act::ToggleView("gpu")),
        KeyCode::Char('K') => app.run(Act::ToggleKernel),
        KeyCode::Char('P') => app.sort_desc_by("cpu"),
        KeyCode::Char('M') => app.sort_desc_by("mem"),
        KeyCode::Char('T') => app.sort_desc_by("time"),
        KeyCode::Char('1') => app.run(Act::ToggleCpuCombined),
        KeyCode::Char('r') => app.run(Act::ToggleResources),
        KeyCode::Char('q') | KeyCode::F(10) => app.quit = true,
        KeyCode::Esc => {
            // Clear the filter, else leave focus mode.
            if !app.opts.filter.is_empty() {
                app.set_filter(String::new());
            } else if app.focus_pid() > 0 {
                app.engine.set_focus(0);
            }
        }
        // The process list.
        KeyCode::Char(' ') => {
            let p = app.cur_pid;
            app.toggle_tag(p);
            app.move_sel(1);
        }
        // Sideways: Shift or Alt+←/→ (terminals often keep Shift+arrows
        // for themselves), < and >, and plain ←/→ where nothing folds.
        KeyCode::Right if shift || alt => app.hscroll_by(8),
        KeyCode::Left if shift || alt => app.hscroll_by(-8),
        KeyCode::Char('>') => app.hscroll_by(8),
        KeyCode::Char('<') => app.hscroll_by(-8),
        KeyCode::Right if app.arrows_scroll() => app.hscroll_by(8),
        KeyCode::Left if app.arrows_scroll() => app.hscroll_by(-8),
        KeyCode::Right | KeyCode::Left if ctrl => {
            if let Some(key) = app.cur_row().and_then(|r| r["key"].as_str().map(String::from)) {
                app.engine.set_subtree_expanded(&key, k.code == KeyCode::Right);
            }
        }
        KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Right => app.set_collapsed(false),
        KeyCode::Char('-') | KeyCode::Left => app.set_collapsed(true),
        KeyCode::Char('*') => app.run(Act::ExpandAll),
        KeyCode::Char('_') => app.run(Act::CollapseAll),
        KeyCode::Char('f') => app.run(Act::ToggleFocusCurrent),
        KeyCode::Enter => {
            if app.cur_pid > 0 {
                app.run(Act::Details);
            } else {
                app.toggle_current();
            }
        }
        KeyCode::F(9) | KeyCode::Char('k') => app.run(Act::SignalMenu),
        KeyCode::F(7) | KeyCode::Char(']') => app.renice(-1),
        KeyCode::F(8) | KeyCode::Char('[') => app.renice(1),
        KeyCode::Char('U') => app.tagged.clear(),
        KeyCode::Down | KeyCode::Char('j') => app.move_sel(1),
        KeyCode::Up => app.move_sel(-1),
        KeyCode::PageDown => app.move_sel(app.page.max(1) as i64),
        KeyCode::PageUp => app.move_sel(-(app.page.max(1) as i64)),
        KeyCode::Home => app.move_sel(i64::MIN / 2),
        KeyCode::End => app.move_sel(i64::MAX / 2),
        _ => {}
    }
}

fn in_table(g: &Geometry, x: u16, y: u16) -> bool {
    let t = g.table;
    x >= t.x && x < t.x + t.width && y >= t.y && y < t.y + t.height
}

pub fn mouse(app: &mut App, g: &Geometry, m: MouseEvent) {
    let (x, y) = (m.column, m.row);
    let ctrl = m.modifiers.contains(KeyModifiers::CONTROL);
    let shift = m.modifiers.contains(KeyModifiers::SHIFT);
    if app.mouse != Some((x, y)) {
        app.mouse = Some((x, y));
        app.mouse_moved = Instant::now();
    }
    // The order holds still while the pointer is over the list.
    let over = in_table(g, x, y) && app.popup.is_none();
    if over != app.frozen {
        app.frozen = over;
        app.engine.set_frozen(over);
    }
    let hit = g.at(x, y).cloned();
    match m.kind {
        MouseEventKind::Moved => {}
        MouseEventKind::Drag(MouseButton::Left) => match &mut app.drag {
            Some(Drag::Column(_, mx)) => *mx = x,
            Some(Drag::VBar(grab)) => {
                if let Some((t, _, len)) = g.vbar {
                    let start = y as i32 - t.y as i32 - *grab as i32;
                    app.top = crate::ui::thumb_pos(t.height, len, app.page, app.rows.len(), start);
                    app.clamp_scroll();
                }
            }
            Some(Drag::HBar(grab)) => {
                if let Some((t, _, len)) = g.hbar {
                    let start = x as i32 - t.x as i32 - *grab as i32;
                    let view = t.width as usize;
                    app.hscroll = crate::ui::thumb_pos(t.width, len, view, view + app.hmax, start);
                }
            }
            Some(Drag::Resize(key, start, w0)) => {
                let w = (*w0 as i64 + x as i64 - *start as i64).clamp(2, 80) as usize;
                let key = *key;
                app.set_column_width(key, w);
            }
            None => {}
        },
        MouseEventKind::Up(MouseButton::Left) => {
            if let Some(Drag::Column(key, mx)) = app.drag.take() {
                let start = app.last_click.map(|(_, sx, _)| sx).unwrap_or(mx);
                if (mx as i32 - start as i32).abs() <= 1 {
                    app.sort_by(key);
                } else {
                    let cx = mx + app.hscroll as u16;
                    let before = g.cols.iter().find(|(_, cx0, w)| cx < cx0 + w / 2 + 1).map(|(k, _, _)| *k).unwrap_or("");
                    app.move_column(key, before);
                }
            } else {
                app.drag = None;
            }
        }
        MouseEventKind::Down(btn) => {
            let now = Instant::now();
            let double = btn == MouseButton::Left
                && app.last_click.is_some_and(|(t, cx, cy)| now.duration_since(t) < Duration::from_millis(400) && cx == x && cy == y);
            if btn == MouseButton::Left {
                app.last_click = Some((now, x, y));
            }
            // A click outside a pop-up closes it.
            let in_popup = matches!(hit, Some(Hit::Popup) | Some(Hit::Menu(..)) | Some(Hit::DetailsTab(_)) | Some(Hit::DetailsRoot) | Some(Hit::EnvFilter));
            if app.popup.is_some() && !in_popup {
                app.popup = None;
                return;
            }
            if btn == MouseButton::Left && !matches!(hit, Some(Hit::Filter)) {
                app.editing_filter = false;
            }
            match btn {
                MouseButton::Left => left(app, g, hit, x, y, double, ctrl, shift),
                MouseButton::Right => right(app, g, hit, x, y),
                _ => {}
            }
        }
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            let d: i64 = if m.kind == MouseEventKind::ScrollDown { 1 } else { -1 };
            match &mut app.popup {
                Some(Popup::Details(dd)) => {
                    let s = &mut dd.scroll[dd.tab];
                    *s = (*s as i64 + d * 3).max(0) as usize;
                }
                Some(Popup::Help(s)) => *s = (*s as i64 + d).max(0) as usize,
                Some(Popup::Menus(stack)) => stack.last_mut().unwrap().step(d as i32),
                None => {
                    // Sideways with any modifier: Konsole keeps Shift+wheel
                    // for itself, so Alt or Ctrl+wheel too.
                    let sideways = shift || ctrl || m.modifiers.contains(KeyModifiers::ALT);
                    if sideways || matches!(hit, Some(Hit::HBar)) {
                        app.hscroll_by(d * 10);
                    } else {
                        app.scroll(d * 6);
                    }
                }
            }
        }
        MouseEventKind::ScrollLeft => app.hscroll_by(-10),
        MouseEventKind::ScrollRight => app.hscroll_by(10),
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn left(app: &mut App, g: &Geometry, hit: Option<Hit>, x: u16, y: u16, double: bool, ctrl: bool, shift: bool) {
    match hit {
        Some(Hit::Menu(level, i)) => activate(app, level, i),
        Some(Hit::Act(a)) => app.run(a),
        Some(Hit::Filter) => app.editing_filter = true,
        Some(Hit::Footer(a)) => {
            let act = footer_act(a, app.view());
            app.run(act);
        }
        Some(Hit::Header(k)) => app.drag = Some(Drag::Column(k, x)),
        // A scrollbar: grab the thumb, or page towards the click.
        Some(Hit::VBar) => {
            if let Some((t, start, len)) = g.vbar {
                let at = y - t.y;
                if at >= start && at < start + len {
                    app.drag = Some(Drag::VBar(at - start));
                } else if at < start {
                    app.scroll(-(app.page as i64));
                } else {
                    app.scroll(app.page as i64);
                }
            }
        }
        Some(Hit::HBar) => {
            if let Some((t, start, len)) = g.hbar {
                let at = x - t.x;
                if at >= start && at < start + len {
                    app.drag = Some(Drag::HBar(at - start));
                } else if at < start {
                    app.hscroll_by(-(t.width as i64));
                } else {
                    app.hscroll_by(t.width as i64);
                }
            }
        }
        Some(Hit::Gap(k)) => {
            if double {
                app.fit_all_columns();
            } else {
                let w = g.cols.iter().find(|(kk, _, _)| *kk == k).map(|(_, _, w)| *w as usize).unwrap_or(8);
                app.drag = Some(Drag::Resize(k, x, w));
            }
        }
        Some(Hit::Expander(i)) => {
            app.select(i);
            if let Some(r) = app.rows.get(i) {
                let key = r["key"].as_str().unwrap_or("").to_string();
                if shift {
                    let expanded = r["expanded"].as_bool().unwrap_or(true);
                    app.engine.set_subtree_expanded(&key, !expanded);
                } else {
                    app.engine.toggle(&key);
                }
            }
        }
        Some(Hit::Row(i)) | Some(Hit::Command(i)) | Some(Hit::Cell(i, _)) | Some(Hit::Sigma(i)) => {
            app.select(i);
            if ctrl {
                let p = app.cur_pid;
                app.toggle_tag(p);
            } else if double {
                let r = &app.rows[i];
                if r["group"].as_bool() == Some(true) && r["hasChildren"].as_bool() == Some(true) {
                    app.toggle_current();
                } else {
                    let p = app.cur_pid;
                    app.toggle_focus(p);
                }
            }
        }
        Some(Hit::DetailsTab(i)) => {
            if let Some(Popup::Details(d)) = &mut app.popup {
                d.tab = i;
            }
        }
        Some(Hit::DetailsRoot) => app.inspect_as_root(),
        Some(Hit::EnvFilter) => {
            if let Some(Popup::Details(d)) = &mut app.popup {
                d.editing_filter = true;
            }
        }
        _ => {}
    }
}

fn right(app: &mut App, g: &Geometry, hit: Option<Hit>, x: u16, y: u16) {
    // A GPU card's menu, wherever under the pointer it is.
    let gpu = g.hits.iter().rev().find(|(r, h)| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height && matches!(h, Hit::GpuCard(_)));
    if let Some((_, Hit::GpuCard(p))) = gpu {
        if let Some(e) = app.gpus().into_iter().find(|e| &e.pdev == p) {
            let items = app.gpu_menu(&e);
            app.open_menu(items, x, y + 1);
        }
        return;
    }
    match hit {
        Some(Hit::Row(i)) | Some(Hit::Command(i)) | Some(Hit::Cell(i, _)) | Some(Hit::Sigma(i)) | Some(Hit::Expander(i)) => {
            app.select(i);
            let items = app.signal_menu();
            app.open_menu(items, x, y);
        }
        Some(Hit::Header(_)) | Some(Hit::Gap(_)) | Some(Hit::Act(Act::ColumnMenu)) => {
            let items = app.column_menu();
            app.open_menu(items, x, y + 1);
        }
        Some(Hit::Act(Act::ToggleResources)) => {
            let items = app.resources_menu();
            app.open_menu(items, x, y + 1);
        }
        _ => {}
    }
}
