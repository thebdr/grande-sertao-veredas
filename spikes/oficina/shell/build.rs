//! Builds the QML module `Oficina`, the bridge, and the C++ side of the seam, and links Qt,
//! KDDockWidgets and Spix. `QMAKE` names Qt's qmake (cxx-qt finds Qt through it);
//! `OFICINA_DEPS` names where KDDockWidgets, Spix and AnyRPC are installed
//! (`~/.local/opt/oficina-deps` by default, the home folder being `USERPROFILE` on Windows;
//! docs/oficina-plan.md, "Where its code lives").

use std::path::PathBuf;

use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    println!("cargo::rerun-if-env-changed=OFICINA_DEPS");
    let deps = std::env::var_os("OFICINA_DEPS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).expect("HOME or USERPROFILE is set")).join(".local/opt/oficina-deps"));
    let lib = deps.join("lib");
    let include = deps.join("include");

    let qml = QmlModule::new("Oficina").qml_files(["qml/Main.qml", "qml/ProjectPane.qml", "qml/EditorPane.qml", "qml/TablePane.qml", "qml/WebPane.qml", "qml/CodePane.qml"]);
    let builder = CxxQtBuilder::new_qml_module(qml)
        .qt_module("Gui")
        .qt_module("Quick")
        .qt_module("QuickControls2")
        .qt_module("WebEngineQuick")
        .qt_module("WebChannel")
        .files(["src/bridge/glue.rs", "src/bridge/tree.rs", "src/bridge/document.rs", "src/bridge/table.rs", "src/bridge/code.rs"])
        .cpp_file("cpp/shell.cpp")
        .cpp_file("cpp/keys.h");
    // SAFETY: the closure only adds include folders to the C++ build.
    let builder = unsafe {
        builder.cc_builder(|cc| {
            cc.include(&include);
            cc.include(include.join("kddockwidgets-qt6"));
            // What KDDockWidgets' CMake package passes to its users (its INTERFACE definitions).
            cc.define("KDDW_FRONTEND_QTQUICK", None);
            cc.define("KDDW_FRONTEND_QT", None);
            cc.flag_if_supported("-std=c++17");
            // MSVC: the standard Qt 6 asks for, and `__cplusplus` telling it.
            cc.flag_if_supported("/std:c++17");
            cc.flag_if_supported("/Zc:__cplusplus");
            cc.flag_if_supported("/permissive-");
        })
    };
    builder.build();

    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    println!("cargo::rustc-link-search=native={}", lib.display());
    // KDDockWidgets names its Windows library after its major version too (kddockwidgets-qt62.lib
    // and .dll), its Linux one by the SONAME alone.
    let kddw = if windows { "kddockwidgets-qt62" } else { "kddockwidgets-qt6" };
    for l in [kddw, "SpixQtQuick", "SpixCore", "anyrpc"] {
        println!("cargo::rustc-link-lib={l}");
    }
    if windows {
        // The drawing probe asks DXGI for Direct3D 11's adapter (cpp/shell.cpp). Windows finds
        // the libraries at run time on PATH (Qt's `bin`, the dependencies' `bin`), as the
        // Windows run sets it (run-windows.ps1).
        println!("cargo::rustc-link-lib=dxgi");
    } else {
        println!("cargo::rustc-link-arg=-Wl,-rpath,{}", lib.display());
        // The spike runs from its build folder: Qt's own libraries are found where qmake says.
        if let Some(qt_libs) = qmake_query("QT_INSTALL_LIBS") {
            println!("cargo::rustc-link-arg=-Wl,-rpath,{qt_libs}");
        }
    }
}

fn qmake_query(var: &str) -> Option<String> {
    let qmake = std::env::var_os("QMAKE")?;
    let out = std::process::Command::new(qmake).args(["-query", var]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
