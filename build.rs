use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    println!("cargo:rerun-if-changed=qml");
    println!("cargo:rerun-if-changed=native");
    let session_headers = wayland_session_headers();
    let builder = CxxQtBuilder::new_qml_module(QmlModule::new("com.benwbooth.treetop").qml_files([
        "qml/Main.qml",
        "qml/Meter.qml",
        "qml/Sparkline.qml",
        "qml/StatCard.qml",
        "qml/ProcessTable.qml",
        "qml/GpuPanel.qml",
        "qml/SectionHeader.qml",
        "qml/HiddenChip.qml",
        "qml/KineticScroll.qml",
        "qml/KeyMenuItem.qml",
    ]))
    .crate_include_root(Some("native".to_owned()))
    .qrc_resources(["qml/treetop.svg", "qml/Fmt.js"])
    .files(["src/monitor.rs", "src/main.rs"])
    .cpp_file("native/treetop_app.cpp")
    .qt_module("Gui")
    .qt_module("Network")
    .qt_module("DBus")
    .qt_module("Quick")
    .qt_module("QuickControls2")
    .qt_module("Widgets");
    // SAFETY: only include paths from the same Qt installation, and a define
    // for our own code; Qt's ABI flags are untouched.
    let builder = unsafe {
        builder.cc_builder(|cc| {
            if !session_headers.is_empty() {
                cc.includes(&session_headers);
                cc.define("TREETOP_WAYLAND_SESSION_RESTORE", None);
            }
        })
    };
    builder.build();
}

/// Qt's private QWaylandWindow headers, for setSessionRestoreId (Qt 6.11+),
/// which lets KWin put the window back where it was. Empty if unavailable.
fn wayland_session_headers() -> Vec<std::path::PathBuf> {
    let qmake = std::env::var("QMAKE").unwrap_or_else(|_| "qmake6".to_owned());
    let query = |key: &str| {
        std::process::Command::new(&qmake)
            .args(["-query", key])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    let (Some(headers), Some(version)) = (query("QT_INSTALL_HEADERS"), query("QT_VERSION")) else {
        return Vec::new();
    };
    let parts: Vec<u32> = version.split('.').filter_map(|p| p.parse().ok()).collect();
    if parts.len() < 2 || (parts[0], parts[1]) < (6, 11) {
        return Vec::new();
    }
    let headers = std::path::PathBuf::from(headers);
    let gui = headers.join("QtGui").join(&version);
    let core = headers.join("QtCore").join(&version);
    if !gui.join("QtGui/qpa/qplatformwindow_p.h").is_file() {
        return Vec::new();
    }
    vec![gui.join("QtGui"), gui, core.join("QtCore"), core]
}
