//! `veredas-supervisao <supervisao.toml> [--check]`: run a supervisão file's sessions and serve
//! them until stdin closes, or read and check it and print the plan without starting anything.

// The compiler refuses any foreign function, so none reaches a file, a program or a socket (C-054).
#![forbid(unsafe_code)]

use std::io::BufRead;
use std::path::PathBuf;

use veredas_supervisao::{Host, config};

const USAGE: &str = "usage: veredas-supervisao <supervisao.toml> [--check]";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (path, check) = match args.as_slice() {
        [p] if !p.starts_with("--") => (PathBuf::from(p), false),
        [p, c] if c == "--check" && !p.starts_with("--") => (PathBuf::from(p), true),
        [c, p] if c == "--check" && !p.starts_with("--") => (PathBuf::from(p), true),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(64);
        }
    };
    let cfg = match config::load_file(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            std::process::exit(2);
        }
    };
    if check {
        print!("{}", config::plan(&cfg));
        return;
    }
    let names: Vec<String> = cfg.sessions.iter().map(|s| s.name.clone()).collect();
    let host = match Host::start(cfg) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    println!("serving on http://{}/", host.addr());
    for n in &names {
        println!("session `{n}` at http://{}/s/{n}/", host.addr());
    }
    println!("running; close stdin to stop");
    // Read as bytes: a line in any encoding is no close, only the end of stdin is (or a read that
    // fails other than by an interruption, which no later read would mend).
    let stdin = std::io::stdin();
    let mut line = Vec::new();
    loop {
        match stdin.lock().read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => line.clear(),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    for (name, ends) in host.stop() {
        for (id, r) in ends {
            if let Err(e) = r {
                eprintln!("session `{name}`, {id}: {e}");
            }
        }
    }
}
