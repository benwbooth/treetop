//! treetop-tui: treetop in a terminal. The same engine, settings, columns,
//! keys and behaviour as the Qt app, drawn with ratatui.

mod app;
mod input;
mod theme;
mod ui;
mod widgets;

use app::{App, Msg};
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::Write;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use treetop_core::root;
use treetop_core::Engine;

const HELP: &str = "treetop-tui — an htop-style system monitor in your terminal

USAGE: treetop-tui [--help] [--version] [--self-test]

Process tree with folding and ×N totals, CPU/memory/GPU/disk/network graphs,
every temperature and fan, Docker/Podman containers, per-process GPU use,
and root access (polkit, or sudo over SSH). Press h inside for the keys.

Settings are shared with the treetop window (~/.config/treetop/treetop.conf).";

fn enter() -> std::io::Result<()> {
    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen, EnableMouseCapture, crossterm::cursor::Hide)
}

fn leave() {
    let _ = execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen, crossterm::cursor::Show);
    let _ = disable_raw_mode();
}

/// One sample, printed: a check that sampling works here (used in CI).
fn self_test() -> i32 {
    let e = Engine::new();
    let (tx, rx) = mpsc::channel();
    e.start(move |v| tx.send(v).is_ok());
    let mut last = serde_json::Value::Null;
    for _ in 0..2 {
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(v) => last = v,
            Err(_) => {
                eprintln!("no sample within 10 s");
                return 1;
            }
        }
    }
    for l in treetop_core::layout::summary(&last, &[]) {
        println!("{l}");
    }
    println!("{} processes in {} rows", last["procs"]["tasks"], last["rows"]["count"]);
    if last["rows"]["count"].as_u64().unwrap_or(0) == 0 {
        return 1;
    }
    0
}

/// Root over sudo: ask for the password on the terminal (outside the
/// screen), then start the helper with the cached credentials.
fn sudo_then_root(app: &mut App) {
    leave();
    println!("treetop-tui: root access asks for your password (sudo).");
    let ok = std::process::Command::new(root::which_sudo()).arg("-v").status().map(|s| s.success()).unwrap_or(false);
    let _ = enter();
    if ok {
        app.engine.set_root_mode_with(true, root::Launcher::Sudo);
    } else {
        app.notify("Root access: sudo didn't authorize");
        app.opts.root_mode = false;
    }
}

static QUIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_quit_signal(_: libc::c_int) {
    QUIT.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn run() -> std::io::Result<()> {
    for sig in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT] {
        unsafe { libc::signal(sig, on_quit_signal as *const () as usize) };
    }
    let engine = Engine::new();
    let (tx, rx) = mpsc::channel::<Msg>();
    let mut app = App::new(engine.clone(), tx.clone());
    {
        let tx = tx.clone();
        engine.start(move |v| tx.send(Msg::Update(v)).is_ok());
    }
    if app.opts.root_mode {
        let on = true;
        app.set_root(on);
    }
    enter()?;
    let mut term = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    term.clear()?;
    let mut geo = None;
    let mut redraw = true;
    let mut last_draw = Instant::now();
    loop {
        // TERM/HUP/INT: leave like q, so the settings are saved.
        if QUIT.load(std::sync::atomic::Ordering::SeqCst) {
            app.quit = true;
        }
        while let Ok(m) = rx.try_recv() {
            app.on_msg(m);
            redraw = true;
        }
        if app.want_sudo {
            app.want_sudo = false;
            sudo_then_root(&mut app);
            term.clear()?;
            redraw = true;
        }
        // Tooltips appear after a pause: redraw while the pointer rests.
        if app.mouse_moved.elapsed() < Duration::from_millis(1500) && last_draw.elapsed() > Duration::from_millis(100) {
            redraw = true;
        }
        if redraw {
            let mut g = None;
            term.draw(|f| g = Some(ui::draw(f, &mut app)))?;
            geo = g;
            redraw = false;
            last_draw = Instant::now();
        }
        if app.quit {
            break;
        }
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(k) if k.kind != event::KeyEventKind::Release => input::key(&mut app, k),
                Event::Mouse(m) => {
                    if let Some(g) = &geo {
                        input::mouse(&mut app, g, m);
                    }
                }
                Event::Resize(..) => {}
                _ => continue,
            }
            redraw = true;
        }
    }
    // Settings carry over on exit: only what changed in this session is
    // merged into the shared file (the window picks it up on its next start).
    app.save();
    leave();
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Privileged entry points (run through pkexec or sudo); no screen.
    match args.first().map(String::as_str) {
        Some("--root-helper") => return root::helper_main(),
        Some("--inspect") => return root::inspect_main(args.get(1).and_then(|p| p.parse().ok()).unwrap_or(0)),
        Some("-h") | Some("--help") => {
            println!("{HELP}");
            return;
        }
        Some("-V") | Some("--version") => {
            println!("treetop-tui {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some("--self-test") => std::process::exit(self_test()),
        _ => {}
    }
    // Put the terminal back however we leave.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        leave();
        hook(info);
    }));
    if let Err(e) = run() {
        leave();
        let _ = writeln!(std::io::stderr(), "treetop-tui: {e}");
        std::process::exit(1);
    }
}
