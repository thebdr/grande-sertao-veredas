//! An ORCL project for the language-server tests of the model and of the shell (included by
//! path from both): a program with an error planted, and the things that leave a marker if anything
//! of the folder runs them.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// An ORCL program with an error planted on line 23 (zero-based 22), and the planted things that
/// leave a marker in `markers/` if anything runs them: a build script, a compiler wrapper named by
/// the folder's cargo configuration, and a toolchain file naming a toolchain made of scripts.
pub fn orcl_project(root: &Path) -> PathBuf {
    let markers = root.join("markers");
    std::fs::create_dir_all(root.join("src")).expect("src");
    std::fs::create_dir_all(&markers).expect("markers");
    let program = root.join("src/main.rs");
    std::fs::write(&program, PROGRAM).expect("the program");
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"conveyor\"\nversion = \"0.1.0\"\nedition = \"2024\"\nbuild = \"build.rs\"\n").expect("a manifest");
    std::fs::write(root.join("build.rs"), format!("fn main() {{ std::fs::write({:?}, \"\").unwrap(); }}\n", markers.join("build-script"))).expect("a build script");
    let script = |name: &str| {
        let path = root.join("tools").join(name);
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("tools");
        std::fs::write(&path, format!("#!/bin/sh\ntouch {:?}\nexit 1\n", markers.join(name.replace('/', "-")))).expect("a script");
        #[cfg(unix)]
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).expect("executable");
        path
    };
    let wrapper = script("wrapper");
    std::fs::create_dir_all(root.join(".cargo")).expect(".cargo");
    std::fs::write(root.join(".cargo/config.toml"), format!("[build]\nrustc-wrapper = {:?}\n", wrapper)).expect("a cargo configuration");
    for tool in ["toolchain/bin/rustc", "toolchain/bin/cargo", "toolchain/bin/rust-analyzer"] {
        script(tool);
    }
    std::fs::write(root.join("rust-toolchain.toml"), format!("[toolchain]\npath = {:?}\n", root.join("tools/toolchain"))).expect("a toolchain file");
    program
}

pub const PROGRAM: &str = "\
//! A conveyor that runs once its start button has been held for half a second.
use orcl_std::{Program, Scan, Ton, ms};

#[derive(Clone, Copy, Default)]
pub struct Inputs {
    pub start: bool,
}

#[derive(Clone, Copy, Default)]
pub struct Outputs {
    pub motor: bool,
}

#[derive(Default)]
pub struct Conveyor {
    timer: Ton,
}

impl Program for Conveyor {
    type Inputs = Inputs;
    type Outputs = Outputs;
    fn scan(&mut self, scan: &Scan, inputs: &Inputs, outputs: &mut Outputs) {
        outputs.motor = self.timer.scna(scan, inputs.start, ms(500));
    }
}

fn main() {}
";

pub fn sysroot() -> PathBuf {
    let out = std::process::Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into())).args(["--print", "sysroot"]).output().expect("rustc runs");
    PathBuf::from(String::from_utf8(out.stdout).expect("UTF-8").trim())
}

pub fn orcl_std() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../crates/orcl-std/src/lib.rs").canonicalize().expect("orcl-std in this repository")
}

