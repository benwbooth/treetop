mod actions;
mod containers;
mod gpu;
mod monitor;
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
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("treetop [--hidden] [--toggle]\n\n  --hidden  start in the tray\n  --toggle  show or hide the running window (what the panel widget calls)");
        return;
    }
    let hidden = args.iter().any(|a| a == "--hidden");
    let toggle = args.iter().any(|a| a == "--toggle");
    std::process::exit(ffi::run(hidden, toggle));
}
