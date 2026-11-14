mod autostart;
mod monitor;

use treetop_core::root;

#[cxx::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("treetop/treetop_app.h");
        #[cxx_name = "treetopRun"]
        fn run(start_hidden: bool, toggle: bool) -> i32;
    }
}

/// Set once this process is the running window (not a second launch that
/// only told the first one to show itself).
static SESSION: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Called by the C++ side once it's the single instance: starts the
/// window's private settings copy and returns its path (free with
/// treetop_string_free).
#[no_mangle]
pub extern "C" fn treetop_settings_begin() -> *mut std::os::raw::c_char {
    SESSION.store(true, std::sync::atomic::Ordering::SeqCst);
    let p = treetop_core::settings::begin_session("window");
    std::ffi::CString::new(p.display().to_string()).unwrap_or_default().into_raw()
}

#[no_mangle]
pub extern "C" fn treetop_string_free(s: *mut std::os::raw::c_char) {
    if !s.is_null() {
        unsafe { drop(std::ffi::CString::from_raw(s)) };
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
    let code = ffi::run(hidden, toggle);
    // What changed in the window's settings goes into the shared ones now.
    if SESSION.load(std::sync::atomic::Ordering::SeqCst) {
        treetop_core::settings::end_session("window");
    }
    std::process::exit(code);
}
