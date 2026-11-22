//! Builds the QML module `Oficina`, the bridge, and the C++ side of the seam, and links Qt,
//! KDDockWidgets and Spix. `QMAKE` names Qt's qmake (cxx-qt finds Qt through it);
//! `OFICINA_DEPS` names where KDDockWidgets, Spix and AnyRPC are installed
//! (`~/.local/opt/oficina-deps` by default; docs/oficina-plan.md, "Where its code lives").

use std::path::PathBuf;

use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    println!("cargo::rerun-if-env-changed=OFICINA_DEPS");
    let deps = std::env::var_os("OFICINA_DEPS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").expect("HOME is set")).join(".local/opt/oficina-deps"));
    let lib = deps.join("lib");
    let include = deps.join("include");

    let qml = QmlModule::new("Oficina").qml_files(["qml/Main.qml", "qml/ProjectPane.qml", "qml/EditorPane.qml", "qml/TablePane.qml", "qml/WebPane.qml"]);
    let builder = CxxQtBuilder::new_qml_module(qml)
        .qt_module("Gui")
        .qt_module("Quick")
        .qt_module("QuickControls2")
        .qt_module("WebEngineQuick")
        .qt_module("WebChannel")
        .files(["src/bridge/glue.rs", "src/bridge/tree.rs", "src/bridge/document.rs", "src/bridge/table.rs"])
        .cpp_file("cpp/shell.cpp");
    // SAFETY: the closure only adds include folders to the C++ build.
    let builder = unsafe {
        builder.cc_builder(|cc| {
            cc.include(&include);
            cc.include(include.join("kddockwidgets-qt6"));
            // What KDDockWidgets' CMake package passes to its users (its INTERFACE definitions).
            cc.define("KDDW_FRONTEND_QTQUICK", None);
            cc.define("KDDW_FRONTEND_QT", None);
            cc.flag_if_supported("-std=c++17");
        })
    };
    builder.build();

    println!("cargo::rustc-link-search=native={}", lib.display());
    for l in ["kddockwidgets-qt6", "SpixQtQuick", "SpixCore", "anyrpc"] {
        println!("cargo::rustc-link-lib={l}");
    }
    println!("cargo::rustc-link-arg=-Wl,-rpath,{}", lib.display());
    // The spike runs from its build folder: Qt's own libraries are found where qmake says they are.
    if let Some(qt_libs) = qmake_query("QT_INSTALL_LIBS") {
        println!("cargo::rustc-link-arg=-Wl,-rpath,{qt_libs}");
    }
}

fn qmake_query(var: &str) -> Option<String> {
    let qmake = std::env::var_os("QMAKE")?;
    let out = std::process::Command::new(qmake).args(["-query", var]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
