//! Oficina's own-shell spike: the window (docs/oficina-plan.md, "The own-shell spike"). A library
//! with a thin `main`, so the tests that start the window link the same bridge it runs.

// cxx-qt-lib registers its own types through an initialiser the crate must be linked for.
extern crate cxx_qt_lib;

pub mod bridge;

/// Run the window until it closes, with the process's arguments; the exit code.
pub fn run() -> i32 {
    cxx_qt::init_crate!(oficina_spike_shell);
    cxx_qt::init_qml_module!("Oficina");
    bridge::glue::ffi::oficina_run(std::env::args().collect())
}
