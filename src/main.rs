mod actions;
mod autostart;
mod containers;
mod gpu;
mod root;
mod inspect;
mod monitor;
mod net;
mod processes;
mod system;

#[cxx::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("treetop/treetop_app.h");
        #[cxx_name = "treetopRun"]
        fn run(start_hidden: bool, toggle: bool) -> i32;
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Privileged entry points (run through pkexec); no GUI.
    match args.first().map(String::as_str) {
        Some("--root-helper") => return root::helper_main(),
        Some("--inspect") => return root::inspect_main(args.get(1).and_then(|p| p.parse().ok()).unwrap_or(0)),
        _ => {}
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("treetop [--hidden] [--toggle]\n\n  --hidden  start in the tray\n  --toggle  show or hide the running window (what the panel widget calls)");
        return;
    }
    let hidden = args.iter().any(|a| a == "--hidden");
    let toggle = args.iter().any(|a| a == "--toggle");
    std::process::exit(ffi::run(hidden, toggle));
}
