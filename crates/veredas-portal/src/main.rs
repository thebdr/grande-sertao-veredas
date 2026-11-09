//! `veredas-portal <project.toml> [--check]`: run a project until stdin closes or the process is
//! ended, printing every source's health once a second; with `--check`, read and check the file
//! and its referenced files, print the plan, and exit without starting anything. With
//! `--from-v1 <folder>`, read a Veredas 1 project folder into a project file on stdout, the
//! notes on stderr, and exit.
//!
//! Exit codes: 0 when the project ran or the check passed; 2 when the file, a referenced file or
//! a Veredas 1 folder is refused, or the session or the server could not start, with the reason
//! on stderr; 1 on a usage error.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

use veredas_portal::project::{self, LogKind, Project, Start, Store};
use veredas_portal::serve;
use veredas_portal::session::{HealthState, Session};

const USAGE: &str = "usage: veredas-portal <project.toml> [--check] | veredas-portal --from-v1 <folder>";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut check = false;
    let mut from_v1: Option<PathBuf> = None;
    let mut file: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--check" => check = true,
            "--from-v1" => match it.next() {
                Some(dir) => from_v1 = Some(PathBuf::from(dir)),
                None => usage("`--from-v1` names a folder"),
            },
            other if other.starts_with("--") => usage(&format!("unknown option `{other}`")),
            other => file = Some(PathBuf::from(other)),
        }
    }
    if let Some(dir) = from_v1 {
        if file.is_some() || check {
            usage("`--from-v1` takes the folder alone");
        }
        match project::from_v1(&dir) {
            Ok(migrated) => {
                print!("{}", migrated.toml);
                for note in &migrated.notes {
                    eprintln!("note: {note}");
                }
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("{}: {e}", dir.display());
                std::process::exit(2);
            }
        }
    }
    let Some(file) = file else { usage("a project file is needed") };
    let project = match project::load_file(&file) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}: {e}", file.display());
            std::process::exit(2);
        }
    };
    print!("{}", plan(&project));
    if check {
        std::process::exit(0);
    }
    run(project);
}

fn usage(why: &str) -> ! {
    eprintln!("{why}\n{USAGE}");
    std::process::exit(1)
}

/// The plan, one line per part: what the project would start.
fn plan(p: &Project) -> String {
    let mut out = format!("project `{}`\n", p.name);
    let start = match &p.start {
        Start::Live => "live, every log from its end".to_string(),
        Start::Midnight => "from midnight".to_string(),
        Start::LastHours(h) => format!("from {h} hours ago"),
        Start::At(t) => format!("from {t}"),
    };
    let store = match &p.store {
        Store::Ram => "in memory".to_string(),
        Store::Disk { path, memory_limit_mb } => format!("on disk at {} (memory limit {memory_limit_mb} MB)", path.display()),
        Store::PerSession { dir, memory_limit_mb } => format!("on disk, a file of this run's own in {} (memory limit {memory_limit_mb} MB)", dir.display()),
    };
    out.push_str(&format!("  session: {start}; store {store}; writes {}; idle after {} ms\n", if p.writes_armed { "armed" } else { "off" }, p.idle_after_ms));
    match p.server {
        Some(addr) => out.push_str(&format!("  server: {addr}\n")),
        None => out.push_str("  server: none\n"),
    }
    for l in &p.logs {
        let what = match &l.kind {
            LogKind::File { path } => format!("file {path}"),
            LogKind::Family { dir, pattern, .. } => format!("family {pattern} in {dir}"),
        };
        out.push_str(&format!("  log `{}`: {what}{}\n", l.id, if l.timestamp.is_some() { ", stamped" } else { "" }));
    }
    for r in &p.rules {
        out.push_str(&format!("  rule `{}`{}\n", r.name, r.scope.as_deref().map(|s| format!(" on `{s}`")).unwrap_or_default()));
    }
    for o in &p.opc {
        out.push_str(&format!("  opc `{}`: {} every {} ms, {} rule(s)\n", o.source_id, o.endpoint_url, o.poll_ms, o.rules.len()));
    }
    for c in &p.channels {
        out.push_str(&format!("  channel `{}`: {} message(s) in its catalog, {} reply rule(s)\n", c.source_id, c.catalog.messages.len(), c.replies.len()));
    }
    for (name, _) in &p.tables {
        out.push_str(&format!("  table `{name}`\n"));
    }
    out
}

/// Run the session (and the server when the project names one) until stdin closes.
fn run(project: Project) {
    let now = chrono::Local::now().naive_local();
    let session = match Session::start(project.session_config(now)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("the session did not start: {e}");
            std::process::exit(2);
        }
    };
    let live = session.live();
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("the server's runtime did not start: {e}");
            std::process::exit(2);
        }
    };
    if let Some(addr) = project.server {
        let listener = match runtime.block_on(tokio::net::TcpListener::bind(addr)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("the server cannot bind {addr}: {e}");
                std::process::exit(2);
            }
        };
        // The address bound, not the one asked: a port of 0 is the system's to pick.
        println!("serving on http://{}/", listener.local_addr().unwrap_or(addr));
        runtime.spawn(serve::serve(listener, session.live()));
    }
    println!("running; close stdin to stop");
    // Stdin closing is the stop signal; a status line a second meanwhile.
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut line = String::new();
        while let Ok(n) = stdin.lock().read_line(&mut line) {
            if n == 0 {
                break;
            }
            line.clear();
        }
        let _ = stop_tx.send(());
    });
    let mut stdout = std::io::stdout();
    loop {
        if stop_rx.recv_timeout(Duration::from_secs(1)).is_ok() {
            break;
        }
        let mut status = String::new();
        if let Ok(health) = live.health.read() {
            for (id, h) in health.iter() {
                let state = match &h.state {
                    HealthState::Waiting => "waiting".to_string(),
                    HealthState::Live => "live".to_string(),
                    HealthState::Idle { silence_ms } => format!("idle {} s", silence_ms / 1000),
                    HealthState::Retrying(why) => format!("retrying: {why}"),
                    HealthState::Ended => "ended".to_string(),
                    HealthState::Failed(why) => format!("failed: {why}"),
                };
                status.push_str(&format!("{id}: {state} ({}) ", h.count));
            }
        }
        if let Ok(v) = live.vitals.read() {
            status.push_str(&format!("| {} values", v.samples));
        }
        let _ = writeln!(stdout, "{status}");
        let _ = stdout.flush();
    }
    for (id, r) in session.stop() {
        if let Err(e) = r {
            eprintln!("{id}: {e}");
        }
    }
}
