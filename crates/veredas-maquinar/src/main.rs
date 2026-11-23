//! `veredas-maquinar <bench.toml> [--check]`: run a bench file until stdin closes or says `stop`, or
//! the process is asked to end (SIGTERM or SIGINT on Unix, as a service manager or a terminal asks),
//! printing every part's status line once at the start and then once a second (a server's line
//! carries the address it got); with `--check`, read and check the file and its referenced files,
//! print the plan, and exit without starting anything. `--version` prints the executable's name and
//! version.
//!
//! Exit codes: 0 when the bench ran and stopped, the check passed or the version was printed; 2 when
//! the file or a referenced file is refused, or a part could not start, with the reason on stderr;
//! 1 on a usage error.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

use veredas_maquinar::bench::{self, Bench};

fn main() {
    // The host's timer held fine for the whole run: a 20 ms tick on every platform (Q13).
    let _timer = veredas_maquinar::timer::FineTimer::hold();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("veredas-maquinar {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let mut check = false;
    let mut file: Option<PathBuf> = None;
    for a in &args {
        match a.as_str() {
            "--check" => check = true,
            // Any argument beginning with a dash is an option (`-` alone is a file's name).
            other if other.starts_with('-') && other != "-" => {
                eprintln!("unknown option `{other}`\nusage: veredas-maquinar <bench.toml> [--check] | --version");
                std::process::exit(1);
            }
            other if file.is_some() => {
                eprintln!("a second bench file `{other}`\nusage: veredas-maquinar <bench.toml> [--check] | --version");
                std::process::exit(1);
            }
            other => file = Some(PathBuf::from(other)),
        }
    }
    // SIGTERM and SIGINT are taken before the bench starts, so one that comes early ends the run as
    // stdin closing does, every part stopped and joined, never the process killed under them.
    #[cfg(unix)]
    let signals = (!check).then(stop_signals).flatten();
    let Some(file) = file else {
        eprintln!("usage: veredas-maquinar <bench.toml> [--check] | --version");
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
    // Every part's status at once, with the addresses the servers got: a consumer that started the
    // bench on port 0 reads its port here rather than a second later.
    for line in bench.lines() {
        println!("{line}");
    }
    // Stdin closing is the stop signal: a line "stop" too, and on Unix a SIGTERM or a SIGINT.
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    #[cfg(unix)]
    if let Some(signals) = signals {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let _ = signals.recv();
            let _ = tx.send(());
        });
    }
    std::thread::spawn(move || {
        // Read as bytes: a line that is not UTF-8 is a line like any other, not an end.
        let mut stdin = std::io::stdin().lock();
        let mut line = Vec::new();
        loop {
            line.clear();
            match stdin.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) if String::from_utf8_lossy(&line).trim() == "stop" => break,
                Ok(_) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
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

/// A thread that waits for SIGTERM or SIGINT, its handlers installed when this returns; the
/// receiver gets one message when either comes. `None` when they could not be installed.
#[cfg(unix)]
fn stop_signals() -> Option<std::sync::mpsc::Receiver<()>> {
    let (installed_tx, installed_rx) = std::sync::mpsc::channel::<bool>();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
            let _ = installed_tx.send(false);
            return;
        };
        rt.block_on(async {
            use tokio::signal::unix::{SignalKind, signal};
            let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else {
                let _ = installed_tx.send(false);
                return;
            };
            let _ = installed_tx.send(true);
            tokio::select! {
                _ = term.recv() => {}
                _ = int.recv() => {}
            }
            let _ = tx.send(());
        });
    });
    installed_rx.recv().ok().filter(|ok| *ok).map(|_| rx)
}
