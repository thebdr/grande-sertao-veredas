//! `veredas-maquinar <bench.toml> [--check]`: run a bench file until stdin closes or says `stop`, or
//! the process is asked to end (SIGTERM or SIGINT on Unix, as a service manager or a terminal asks),
//! printing every part's status line once at the start and then once a second (a server's line
//! carries the address it got); with `--check`, read and check the file and its referenced files,
//! print the plan, and exit without starting anything. `--version` prints the executable's name and
//! version.
//!
//! Exit codes: 0 when the bench ran and stopped, the check passed or the version was printed; 2 when
//! the file or a referenced file is refused, or a part could not start, with the reason on stderr;
//! 1 on a usage error. A stdout closed or no longer read changes none of them, nor a stderr closed.

use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

const USAGE: &str = "usage: veredas-maquinar <bench.toml> [--check] | --version";

/// How long a write that found a non-blocking stdout full waits before it tries again.
const AGAIN: Duration = Duration::from_millis(20);

/// Prints a line whole, a stdout closed or failing ignored: what the bench does does not hang on
/// it. A stdout left non-blocking (a parent's shared terminal leaves its stdout so with its stdin)
/// that is full takes the rest of the line a moment later (round 8: the line was dropped, the
/// start's lines among them, whenever the reader fell behind).
fn say(line: &str) {
    use std::io::ErrorKind::{Interrupted, WouldBlock};
    let mut out = std::io::stdout().lock();
    let bytes = format!("{line}\n");
    let mut rest = bytes.as_bytes();
    while !rest.is_empty() {
        match out.write(rest) {
            Ok(0) => return,
            Ok(n) => rest = &rest[n..],
            Err(e) if e.kind() == Interrupted => {}
            Err(e) if e.kind() == WouldBlock => std::thread::sleep(AGAIN),
            Err(_) => return,
        }
    }
    loop {
        match out.flush() {
            Ok(()) => return,
            Err(e) if e.kind() == Interrupted => {}
            Err(e) if e.kind() == WouldBlock => std::thread::sleep(AGAIN),
            Err(_) => return,
        }
    }
}

/// The most of a stdin line kept to be judged: a longer line is no `stop`, read to its end and past
/// without being kept (round 8: a line without an end, `/dev/zero`, was kept whole till the
/// process ran out of memory).
const LONGEST_LINE: usize = 4096;

/// Writes a line to stderr, a stderr closed or failing ignored: the exit code stays the one stated.
fn complain(line: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{line}");
}

/// Lines enough for some seconds of a large bench's status: a status line that finds the writer this
/// far behind is dropped (a second later another says the same).
const BEHIND: usize = 1024;

/// A run's lines go out through a thread of their own that the run never waits for, so a consumer
/// that has stopped reading never holds the start, a stop, a signal or the end of the run (round 6:
/// a bounded queue held the end once it was full); its write's failure is ignored.
struct Out {
    lines: Sender<String>,
    queued: Arc<AtomicUsize>,
    done: Receiver<()>,
}

impl Out {
    fn start() -> Out {
        let (lines, rx) = mpsc::channel::<String>();
        let queued = Arc::new(AtomicUsize::new(0));
        let (done_tx, done) = mpsc::channel();
        let written = Arc::clone(&queued);
        std::thread::spawn(move || {
            for line in rx {
                say(&line);
                written.fetch_sub(1, Ordering::SeqCst);
            }
            let _ = done_tx.send(());
        });
        Out { lines, queued, done }
    }

    /// A line that must go out, the start's and the end's: queued however far behind the writer is.
    fn send(&self, line: String) {
        self.queued.fetch_add(1, Ordering::SeqCst);
        if self.lines.send(line).is_err() {
            self.queued.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// A status line, dropped while the writer is `BEHIND` lines behind.
    fn offer(&self, line: String) -> bool {
        if self.queued.load(Ordering::SeqCst) >= BEHIND {
            return false;
        }
        self.send(line);
        true
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
            complain(&format!("unknown option `{}`\n{USAGE}", a.to_string_lossy()));
            std::process::exit(1);
        } else if file.is_some() {
            complain(&format!("a second bench file `{}`\n{USAGE}", a.to_string_lossy()));
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
        complain(USAGE);
        std::process::exit(1);
    };
    let spec = match bench::load_file(&file) {
        Ok(spec) => spec,
        Err(e) => {
            complain(&format!("{}: {e}", file.display()));
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
            complain(&format!("{}: {e}", file.display()));
            std::process::exit(2);
        }
    };
    // Stdin closing is the stop signal: a line "stop" too, and on Unix a SIGTERM or a SIGINT, each
    // heard from before the start's lines go out.
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
        // Read as bytes: a line that is not UTF-8 is a line like any other, not an end. A stdin
        // left non-blocking (a parent's shared terminal) has nothing yet, not its end: it is read
        // again a moment later, the part of a line already read kept (round 7: it ended the run).
        // A line's first `LONGEST_LINE` bytes are kept, the rest read past.
        let mut stdin = std::io::stdin().lock();
        let mut line = Vec::new();
        let mut cut = false;
        loop {
            let buf = match stdin.fill_buf() {
                Ok(buf) => buf,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                Err(_) => break,
            };
            if buf.is_empty() {
                break;
            }
            let (taken, ends) = match buf.iter().position(|b| *b == b'\n') {
                Some(i) => (i + 1, true),
                None => (buf.len(), false),
            };
            let keep = taken.min(LONGEST_LINE.saturating_sub(line.len()));
            cut |= keep < taken;
            line.extend_from_slice(&buf[..keep]);
            stdin.consume(taken);
            if ends {
                if !cut && String::from_utf8_lossy(&line).trim() == "stop" {
                    break;
                }
                line.clear();
                cut = false;
            }
        }
        let _ = tx.send(());
    });
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
