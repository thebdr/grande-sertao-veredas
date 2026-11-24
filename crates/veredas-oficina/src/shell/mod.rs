//! The one seam between the QML interface and the Rust model: nothing outside this module names
//! the bridge (docs/oficina-plan.md, "One seam for the bridge"). Built with the `shell` feature.

// cxx-qt-lib registers its own types through an initialiser the crate must be linked for.
extern crate cxx_qt_lib;

mod glue;
mod tree;

/// Run the window until it closes, with the process's arguments; the exit code.
pub fn run() -> i32 {
    cxx_qt::init_crate!(veredas_oficina);
    cxx_qt::init_qml_module!("Oficina");
    glue::ffi::oficina_run(std::env::args().collect())
}
