use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    println!("cargo:rerun-if-changed=qml");
    println!("cargo:rerun-if-changed=native");
    CxxQtBuilder::new_qml_module(QmlModule::new("com.benwbooth.treetop").qml_files([
        "qml/Main.qml",
        "qml/Meter.qml",
        "qml/Sparkline.qml",
        "qml/StatCard.qml",
        "qml/ProcessTable.qml",
        "qml/GpuPanel.qml",
    ]))
    .crate_include_root(Some("native".to_owned()))
    .qrc_resources(["qml/treetop.svg", "qml/Fmt.js"])
    .files(["src/monitor.rs", "src/main.rs"])
    .cpp_file("native/treetop_app.cpp")
    .qt_module("Gui")
    .qt_module("Network")
    .qt_module("Quick")
    .qt_module("QuickControls2")
    .qt_module("Widgets")
    .build();
}
