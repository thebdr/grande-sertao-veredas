//! Builds Oficina's window when the `shell` feature asks for it: the QML module `Oficina`, the
//! bridge and the C++ side of the seam, linking Qt and KDDockWidgets, and Spix with AnyRPC when the
//! tests' `driver` is on. `shell` is on by default (docs/questions.md Q370); without it
//! (`--no-default-features`) the script builds nothing and the model needs no Qt.
//!
//! `QMAKE` names Qt's qmake, in cargo's environment, since cxx-qt's own build scripts read it too
//! (unset, cxx-qt looks on `PATH`). `OFICINA_DEPS` names where KDDockWidgets, Spix and AnyRPC are
//! installed (`~/.local/opt/oficina-deps` by default, the home folder being `USERPROFILE` on
//! Windows).

fn main() {
    #[cfg(feature = "shell")]
    shell::build();
}

#[cfg(feature = "shell")]
mod shell {
    use std::path::PathBuf;

    use cxx_qt_build::{CxxQtBuilder, QmlModule};

    fn home() -> PathBuf {
        PathBuf::from(std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).expect("HOME or USERPROFILE is set"))
    }

    pub fn build() {
        println!("cargo::rerun-if-env-changed=QMAKE");
        println!("cargo::rerun-if-env-changed=OFICINA_DEPS");
        // The compiler's word on the feature, not the environment's: a `CARGO_FEATURE_DRIVER` a
        // parent process left set would reach a build script that read it (refute round 2).
        let driver = cfg!(feature = "driver");
        let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
        let deps = std::env::var_os("OFICINA_DEPS").map(PathBuf::from).unwrap_or_else(|| home().join(".local/opt/oficina-deps"));
        let lib = deps.join("lib");
        let include = deps.join("include");

        let qml = QmlModule::new("Oficina").qml_files(["qml/Main.qml", "qml/ProjectPane.qml"]);
        let builder = CxxQtBuilder::new_qml_module(qml)
            .qt_module("Gui")
            .qt_module("Quick")
            .qt_module("QuickControls2")
            .files(["src/shell/glue.rs", "src/shell/tree.rs"])
            .cpp_file("cpp/shell.cpp");
        // SAFETY: the closure only adds include folders, definitions and flags to the C++ build.
        let builder = unsafe {
            builder.cc_builder(move |cc| {
                cc.include(&include);
                cc.include(include.join("kddockwidgets-qt6"));
                // What KDDockWidgets' CMake package passes to its users (its INTERFACE definitions).
                cc.define("KDDW_FRONTEND_QTQUICK", None);
                cc.define("KDDW_FRONTEND_QT", None);
                if driver {
                    cc.define("OFICINA_DRIVER", None);
                }
                cc.flag_if_supported("-std=c++17");
                // MSVC: the standard Qt 6 asks for, and `__cplusplus` telling it.
                cc.flag_if_supported("/std:c++17");
                cc.flag_if_supported("/Zc:__cplusplus");
                cc.flag_if_supported("/permissive-");
            })
        };
        builder.build();

        println!("cargo::rustc-link-search=native={}", lib.display());
        // KDDockWidgets names its Windows library after its major version too (kddockwidgets-qt62),
        // its Linux one by the SONAME alone.
        println!("cargo::rustc-link-lib={}", if windows { "kddockwidgets-qt62" } else { "kddockwidgets-qt6" });
        if driver {
            for l in ["SpixQtQuick", "SpixCore", "anyrpc"] {
                println!("cargo::rustc-link-lib={l}");
            }
        }
        if !windows {
            // Oficina runs from its build folder: the libraries are found where they were linked
            // from. Windows finds them on `PATH` (Qt's `bin`, the dependencies' `bin`).
            println!("cargo::rustc-link-arg=-Wl,-rpath,{}", lib.display());
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
}
