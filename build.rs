use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    println!("cargo:rerun-if-changed=qml");
    println!("cargo:rerun-if-changed=native");
    println!("cargo:rerun-if-env-changed=TREETOP_FONT_DIR");
    let (private_headers, session_restore) = qt_private_headers();
    let qt_dirs = qt_runtime_dirs();
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
        "qml/TipGate.qml",
        "qml/WindowControls.qml",
        "qml/AppMenu.qml",
    ]))
    .crate_include_root(Some("native".to_owned()))
    .qrc_resources(["qml/treetop.svg", "qml/Fmt.js"])
    .files(["src/monitor.rs", "src/main.rs"])
    .cpp_file("native/treetop_app.cpp")
    // The menus in the desktop's global menu bar (QPA private headers).
    .cpp_file("native/global_menu.h")
    .cpp_file("native/global_menu.cpp")
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
            // Shown in About treetop.
            cc.define("TREETOP_VERSION", Some(format!("{:?}", env!("CARGO_PKG_VERSION")).as_str()));
            // Qt's private headers: the global menu bar (QPA) on any Qt,
            // and on 6.11+ the window's session restore id.
            cc.includes(&private_headers);
            if session_restore {
                cc.define("TREETOP_WAYLAND_SESSION_RESTORE", None);
            }
            // The plugin and QML directories of the Qt we build against,
            // put first at startup so a desktop session's own (other
            // version) Qt modules can't shadow them.
            // Fonts shipped with the app (Hack, from the flake), registered
            // at startup.
            if let Ok(fonts) = std::env::var("TREETOP_FONT_DIR") {
                cc.define("TREETOP_FONT_DIR", Some(format!("{fonts:?}").as_str()));
            }
            if let Some((plugins, qml)) = &qt_dirs {
                cc.define("TREETOP_QT_PLUGINS", Some(format!("{plugins:?}").as_str()));
                cc.define("TREETOP_QT_QML", Some(format!("{qml:?}").as_str()));
            }
        })
    };
    builder.build();
}

/// (QT_INSTALL_PLUGINS, QT_INSTALL_QML) of the Qt being built against,
/// when they're Nix store paths (immutable, so safe to bake in).
fn qt_runtime_dirs() -> Option<(String, String)> {
    let qmake = std::env::var("QMAKE").unwrap_or_else(|_| "qmake6".to_owned());
    let query = |key: &str| {
        std::process::Command::new(&qmake)
            .args(["-query", key])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    let (plugins, qml) = (query("QT_INSTALL_PLUGINS")?, query("QT_INSTALL_QML")?);
    (plugins.starts_with("/nix/store/") && qml.starts_with("/nix/store/")).then_some((plugins, qml))
}

/// Qt's private QtGui/QtCore header directories (the global menu bar needs
/// the QPA ones on any Qt), and whether this Qt is 6.11+, for
/// QWaylandWindow::setSessionRestoreId, which lets the compositor put the
/// window back where it was. Empty and false if they aren't installed.
fn qt_private_headers() -> (Vec<std::path::PathBuf>, bool) {
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
        return (Vec::new(), false);
    };
    let headers = std::path::PathBuf::from(headers);
    let gui = headers.join("QtGui").join(&version);
    let core = headers.join("QtCore").join(&version);
    if !gui.join("QtGui/qpa/qplatformwindow_p.h").is_file() {
        return (Vec::new(), false);
    }
    let mut dirs = vec![gui.join("QtGui"), gui, core.join("QtCore"), core];
    // The private headers include qplatformdefs.h, from the mkspec.
    if let (Some(data), Some(spec)) = (query("QT_INSTALL_ARCHDATA"), query("QMAKE_XSPEC")) {
        let mkspec = std::path::Path::new(&data).join("mkspecs").join(spec);
        if mkspec.join("qplatformdefs.h").is_file() {
            dirs.push(mkspec);
        }
    }
    let parts: Vec<u32> = version.split('.').filter_map(|p| p.parse().ok()).collect();
    let session_restore = parts.len() >= 2 && (parts[0], parts[1]) >= (6, 11);
    (dirs, session_restore)
}
