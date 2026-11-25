//! `veredas-maquinar <bench.toml> [--check]`: run a bench file until stdin closes or says `stop`, or
//! the process is asked to end (SIGTERM or SIGINT on Unix, as a service manager or a terminal asks),
//! printing every part's status line once at the start and then once a second (a server's line
//! carries the address it got); with `--check`, read and check the file and its referenced files,
//! print the plan, and exit without starting anything. `--version` prints the executable's name and
//! version.
//!
//! Exit codes: 0 when the bench ran and stopped, the check passed or the version was printed; 2 when
//! the file or a referenced file is refused, or a part could not start, with the reason on stderr;
//! 1 on a usage error. A stdout closed or no longer read changes none of them.

use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::time::Duration;

const USAGE: &str = "usage: veredas-maquinar <bench.toml> [--check] | --version";

/// Prints a line, a stdout closed or failing ignored: what the bench does does not hang on it.
fn say(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// A run's lines go out through a thread of their own, so a consumer that has stopped reading never
/// holds a stop, a signal or the end of the run; its write's failure is ignored.
struct Out {
    lines: SyncSender<String>,
    done: Receiver<()>,
}

impl Out {
    fn start() -> Out {
        // Lines enough for some seconds of a large bench's status: a status line that finds the
        // writer this far behind is dropped (a second later another says the same).
        let (lines, rx) = mpsc::sync_channel::<String>(1024);
        let (done_tx, done) = mpsc::channel();
        std::thread::spawn(move || {
            for line in rx {
                say(&line);
            }
            let _ = done_tx.send(());
        });
        Out { lines, done }
    }

    /// A line that must go out: the start's and the end's.
    fn send(&self, line: String) {
        let _ = self.lines.send(line);
    }

    /// A status line, dropped while the writer is this far behind.
    fn offer(&self, line: String) -> bool {
        !matches!(self.lines.try_send(line), Err(TrySendError::Full(_)))
    }

    /// Every line handed over written, or a few seconds past: a reader that never reads again does
    /// not keep the process.
    fn finish(self) {
        drop(self.lines);
        let _ = self.done.recv_timeout(Duration::from_secs(2));
    }
}

use veredas_maquinar::bench::{self, Bench};

fn main() {
    // The host's timer held fine for the whole run: a 20 ms tick on every platform (Q13).
    let _timer = veredas_maquinar::timer::FineTimer::hold();
    // Read as the system gives them: a path need not be UTF-8.
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        say(&format!("veredas-maquinar {}", env!("CARGO_PKG_VERSION")));
        return;
    }
    let mut check = false;
    let mut file: Option<PathBuf> = None;
    for a in &args {
        let bytes = a.as_encoded_bytes();
        if a == "--check" {
            check = true;
        } else if bytes.first() == Some(&b'-') && bytes != b"-" {
            // Any argument beginning with a dash is an option (`-` alone is a file's name).
            eprintln!("unknown option `{}`\n{USAGE}", a.to_string_lossy());
            std::process::exit(1);
        } else if file.is_some() {
            eprintln!("a second bench file `{}`\n{USAGE}", a.to_string_lossy());
            std::process::exit(1);
        } else {
            file = Some(PathBuf::from(a));
        }
    }
    // SIGTERM and SIGINT are taken before the bench starts, so one that comes early ends the run as
    // stdin closing does, every part stopped and joined, never the process killed under them.
    #[cfg(unix)]
    let signals = (!check).then(stop_signals).flatten();
    let Some(file) = file else {
        eprintln!("{USAGE}");
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
        say(&format!("{}: {} part(s)", file.display(), plan.len()));
        for line in plan {
            say(&format!("  {line}"));
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
    let out = Out::start();
    out.send(format!("{}: {} part(s) running; close stdin or end the process to stop", file.display(), plan.len()));
    for line in &plan {
        out.send(format!("  {line}"));
    }
    // Every part's status at once, with the addresses the servers got: a consumer that started the
    // bench on port 0 reads its port here rather than a second later.
    for line in bench.lines() {
        out.send(line);
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
    loop {
        if rx.recv_timeout(Duration::from_secs(1)).is_ok() {
            break;
        }
        for line in bench.lines() {
            if !out.offer(line) {
                break;
            }
        }
    }
    bench.stop();
    out.send("stopped".into());
    out.finish();
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
