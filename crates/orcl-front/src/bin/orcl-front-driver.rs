//! `orcl-front-driver`: rustc, run as the compiler of an ORCL program's crates, writing the program file of the one
//! it is told is the program (`orcl_front::driver`). It links the pinned nightly's own `rustc_driver`.
#![feature(rustc_private)]
extern crate rustc_driver;

fn main() -> std::process::ExitCode {
    orcl_front::driver::main()
}
