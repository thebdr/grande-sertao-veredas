//! `veredas-maquinar <bench.toml> [--check]`: run a bench file until stdin closes or the process
//! is ended, printing every part's status line once a second; with `--check`, read and check
//! the file and its referenced files, print the plan, and exit without starting anything.
//!
//! Exit codes: 0 when the bench ran or the check passed; 2 when the file or a referenced file is
//! refused, or a part could not start, with the reason on stderr; 1 on a usage error.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

use veredas_maquinar::bench::{self, Bench};

fn main() {
    // The host's timer held fine for the whole run: a 20 ms tick on every platform (Q13).
    let _timer = veredas_maquinar::timer::FineTimer::hold();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut check = false;
    let mut file: Option<PathBuf> = None;
    for a in &args {
        match a.as_str() {
            "--check" => check = true,
            other if other.starts_with("--") => {
                eprintln!("unknown option `{other}`\nusage: veredas-maquinar <bench.toml> [--check]");
                std::process::exit(1);
            }
            other => file = Some(PathBuf::from(other)),
        }
    }
    let Some(file) = file else {
        eprintln!("usage: veredas-maquinar <bench.toml> [--check]");
        std::process::exit(1);
    };
    let spec = match bench::load_file(&file) {
        Ok(spec) => spec,
        Err(e) => {
            eprintln!("{}: {e}", file.display());
            std::process::exit(2);
        }
    };
    let plan = spec.plan();
    if check {
        println!("{}: {} part(s)", file.display(), plan.len());
        for line in plan {
            println!("  {line}");
        }
        return;
    }
    let bench = match Bench::start(spec) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{}: {e}", file.display());
            std::process::exit(2);
        }
    };
    println!("{}: {} part(s) running; close stdin or end the process to stop", file.display(), plan.len());
    for line in &plan {
        println!("  {line}");
    }
    // Stdin closing is the stop signal: a line "stop" too.
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(l) if l.trim() == "stop" => break,
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        let _ = tx.send(());
    });
    let mut stdout = std::io::stdout();
    loop {
        if rx.recv_timeout(Duration::from_secs(1)).is_ok() {
            break;
        }
        for line in bench.lines() {
            let _ = writeln!(stdout, "{line}");
        }
        let _ = stdout.flush();
    }
    bench.stop();
    println!("stopped");
}
