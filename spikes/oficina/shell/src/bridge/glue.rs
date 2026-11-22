//! The entry into the C++ side of the seam.

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("oficina-spike-shell/cpp/shell.h");

        /// Run the application until its window closes.
        fn oficina_run(args: Vec<String>) -> i32;
    }
}
