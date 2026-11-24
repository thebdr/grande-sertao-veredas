//! `veredas-comissionador`: the testing platform from the command line (`docs/comissionador-plan.md`,
//! phase 5), for a commissioning laptop and a pipeline with no Oficina.
//!
//! - `check <file>`: read a test file (or a function file, `.fn`) and the function files a test
//!   uses, check them whole, print what the designer's problems bar counts or every problem.
//! - `run <test> --project <project.toml> [--runs <folder>]`: start the plant's portal session from
//!   its project file in this process, run the test live, print every step as it ends and the
//!   verdict, keep the run under `<folder>/<id>/<nnnn>/` (`runs` beside the test by default).
//! - `replay <test> --project <project.toml> --from <moment> --to <moment> [--runs <folder>]`: the
//!   same over the window of the session's store between the two moments (`YYYY-MM-DD HH:MM[:SS[.mmm]]`
//!   on the plant's clock), once the store holds what the project's start reads into it.
//!
//! Each option is given once, followed by its value, never an empty one. Exit codes: 0 the file is
//! sound or the run passed; 1 the file has problems or the run failed; 2 the command line is wrong;
//! 3 the run was aborted; 4 the run was refused before it started (its folder cannot be made, the
//! project or its session cannot start, the store is still being read in, the engine refuses it),
//! nothing run, nothing kept and the folders made for it removed; 5 the run ran, its verdict
//! printed, but could not be kept.
//!
//! `VEREDAS_COMISSIONADOR_READ_IN` sets, in seconds, how long a replay waits for the store to be
//! read in (600 when unset), so a test can reach the refusal.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use veredas_comissionador::plant::LivePlant;
use veredas_comissionador::run::{Event, RunMode, RunOptions, RunVerdict, run};
use veredas_comissionador::{Checked, Problem, load_functions, load_test, record};
use veredas_portal::session::{HealthState, Session};

const USAGE: &str = "usage: veredas-comissionador check <file.test | file.fn>
       veredas-comissionador run <file.test> --project <project.toml> [--runs <folder>]
       veredas-comissionador replay <file.test> --project <project.toml> --from <moment> --to <moment> [--runs <folder>]";

/// How long a live run waits for the session's OPC UA sources and channels to read live before it
/// starts anyway (its waits and its binding say what is missing); how long a replay waits for
/// every log to read its first line (an empty log has none).
const READY: Duration = Duration::from_secs(30);

/// How long a replay waits for the project's start to finish reading into the store before it is
/// refused, unless `VEREDAS_COMISSIONADOR_READ_IN` says otherwise.
const READ_IN: Duration = Duration::from_secs(600);

fn read_in() -> Duration {
    std::env::var("VEREDAS_COMISSIONADOR_READ_IN").ok().and_then(|v| v.parse().ok()).map(Duration::from_secs).unwrap_or(READ_IN)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn problems(ps: Vec<Problem>) -> ExitCode {
    for p in ps {
        eprintln!("{p}");
    }
    ExitCode::from(1)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") if args.len() == 2 => check(&args[1]),
        Some("run") | Some("replay") => go(&args),
        _ => usage(),
    }
}

fn check(file: &str) -> ExitCode {
    let path = Path::new(file);
    let outcome = if path.extension().is_some_and(|e| e == "fn") {
        load_functions(path).map(|ff| {
            let names: Vec<String> = ff.functions.iter().map(|f| f.signature()).collect();
            format!("{file}: {}: {}", plural(names.len(), "function", "functions"), names.join("; "))
        })
    } else {
        load_test(path).map(|c| {
            format!(
                "{file}: {} · {} · {} · {}",
                plural(c.counts.steps, "step", "steps"),
                plural(c.counts.branches, "branch", "branches"),
                plural(c.counts.functions, "function", "functions"),
                plural(c.counts.gated_writes, "gated write", "gated writes"),
            )
        })
    };
    match outcome {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(ps) => problems(ps),
    }
}

/// A moment on the plant's clock, as the store's axis reads it: `YYYY-MM-DD HH:MM[:SS[.mmm]]`, a
/// year of four digits (a run's report holds no other), its digits ASCII.
fn moment(text: &str) -> Option<i64> {
    let t = text.trim();
    let b = t.as_bytes();
    let form = |pattern: &[u8], at: &[u8]| pattern.len() == at.len() && pattern.iter().zip(at).all(|(p, c)| if *p == b'd' { c.is_ascii_digit() } else { p == c });
    let rest = b.get(16..)?;
    let rest_ok = rest.is_empty() || (rest.len() >= 3 && form(b":dd", &rest[..3]) && (rest.len() == 3 || (rest[3] == b'.' && rest.len() > 4 && rest[4..].iter().all(u8::is_ascii_digit))));
    if !form(b"dddd-dd-dd dd:dd", &b[..16]) || !rest_ok {
        return None;
    }
    ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"]
        .iter()
        .find_map(|f| chrono::NaiveDateTime::parse_from_str(t, f).ok())
        .map(|t| t.and_utc().timestamp_millis())
}

/// `run` and `replay`: their options read, the test checked, its folder made, the session started,
/// the test run and kept.
fn go(args: &[String]) -> ExitCode {
    let replay = args[0] == "replay";
    let Some(test) = args.get(1).filter(|t| !t.starts_with("--")) else { return usage() };
    // Each option once, its value the word after it, never another option.
    const OPTIONS: [&str; 4] = ["--project", "--runs", "--from", "--to"];
    let mut values: [Option<String>; 4] = Default::default();
    let mut i = 2;
    while i < args.len() {
        let Some(k) = OPTIONS.iter().position(|o| *o == args[i]) else { return usage() };
        if (k >= 2 && !replay) || values[k].is_some() {
            return usage();
        }
        let Some(value) = args.get(i + 1).filter(|v| !v.is_empty() && !v.starts_with("--")) else { return usage() };
        values[k] = Some(value.clone());
        i += 2;
    }
    let [project, runs, from, to] = values;
    let Some(project) = project.map(PathBuf::from) else { return usage() };
    let mode = match (replay, from, to) {
        (false, ..) => RunOptions::live(),
        (true, Some(f), Some(t)) => match (moment(&f), moment(&t)) {
            (Some(f), Some(t)) => RunOptions::replay(f, t),
            _ => {
                eprintln!("a moment is `YYYY-MM-DD HH:MM[:SS[.mmm]]` on the plant's clock");
                return ExitCode::from(2);
            }
        },
        _ => return usage(),
    };
    let test_path = Path::new(test);
    let c: Checked = match load_test(test_path) {
        Ok(c) => c,
        Err(ps) => return problems(ps),
    };
    let runs = runs.map(PathBuf::from).unwrap_or_else(|| test_path.parent().map(Path::to_path_buf).unwrap_or_default().join("runs"));
    // The test's folder made before anything runs: a run that could not be kept is refused, the
    // plant untouched. The folders made here are removed again when the run is refused.
    let folder = match record::test_folder(&runs, &c) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("the run cannot be kept: {e}");
            return ExitCode::from(4);
        }
    };
    let (made, folder_made) = make_folders(&folder);
    let code = match folder_made {
        Err(e) => {
            eprintln!("the run cannot be kept: {}: {e}", folder.display());
            4
        }
        Ok(()) => with_session(&project, &c, &runs, &mode),
    };
    if code == 4 {
        // Deepest first; a folder something else has filled meanwhile stays.
        for p in made.iter().rev() {
            if std::fs::remove_dir(p).is_err() {
                break;
            }
        }
    }
    ExitCode::from(code)
}

/// The folders of `path` made one by one, outermost first, and those this call made: a folder
/// already there (a `..` reaching one among them) is never counted as made, so never removed.
fn make_folders(path: &Path) -> (Vec<PathBuf>, std::io::Result<()>) {
    let mut made = Vec::new();
    let chain: Vec<&Path> = path.ancestors().filter(|p| !p.as_os_str().is_empty()).collect();
    for p in chain.into_iter().rev() {
        match std::fs::create_dir(p) {
            Ok(()) => made.push(p.to_path_buf()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && p.is_dir() => {}
            Err(e) => return (made, Err(e)),
        }
    }
    (made, Ok(()))
}

/// The plant's session started from its project file, the test run against it and kept: the exit
/// code.
fn with_session(project: &Path, c: &Checked, runs: &Path, mode: &RunOptions) -> u8 {

    // The plant's portal session, started from its project file in this process.
    let loaded = match veredas_portal::project::load_file(project) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}: {e}", project.display());
            return 4;
        }
    };
    // The session's start on the store's axis: a log stamped at or after it has read its history.
    let begun_at = chrono::Local::now().naive_local();
    let session = match Session::start(loaded.session_config(begun_at)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: the session did not start: {e}", project.display());
            return 4;
        }
    };
    let live = session.live();
    let health = |id: &str, ok: &dyn Fn(&HealthState) -> bool| live.health.read().map(|h| h.get(id).is_some_and(|h| ok(&h.state))).unwrap_or(false);
    let started = Instant::now();
    if let RunMode::Replay { to_ms, .. } = &mode.mode {
        // A replay reads the store: every log has read its first line, or READY has passed (an
        // empty log reads none). A log that has read a line stamped at or after the session's
        // start has read its history up to the present (its files read whole, the newest last); a
        // log with no stamps indexes its lines on their arrival, none of them in a window that
        // ended before the session started; the lines of the others hold still across two of
        // portal's refreshes, each known by a new count of the pump's passes.
        let logs: Vec<&str> = live.sources.iter().filter(|s| s.kind == "log").map(|s| s.id.as_str()).collect();
        let unstamped: std::collections::BTreeSet<&str> = live.sources.iter().filter(|s| s.kind == "log" && !s.has_timestamp).map(|s| s.id.as_str()).collect();
        while started.elapsed() < READY && !logs.iter().all(|id| health(id, &|s| *s != HealthState::Waiting)) {
            std::thread::sleep(Duration::from_millis(100));
        }
        let begun_ms = begun_at.and_utc().timestamp_millis();
        // The logs read up to the present, once and for all (portal shows the last stamp read,
        // which a late line can set back).
        let mut past: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        let (mut last, mut still) = (None::<(u64, u64)>, 0);
        let begun = Instant::now();
        let limit = read_in();
        loop {
            // The lines of the logs not yet past the window, and whether any is left.
            let (mut lines, mut left) = (0, false);
            if let Ok(h) = live.health.read() {
                for id in &logs {
                    let Some(s) = h.get(*id) else { continue };
                    let present = s.last_ts.is_some_and(|t| t.and_utc().timestamp_millis() >= begun_ms);
                    if past.contains(id) || present || (unstamped.contains(id) && *to_ms < begun_ms) {
                        past.insert(id);
                    } else {
                        lines += s.count;
                        left = true;
                    }
                }
            }
            if !left {
                break;
            }
            // DECISION PENDING (Q319): lines holding still are taken for the read-in's end; a start
            // that reads files with no line for a second or more in a row (a family's empty members)
            // can pass for it.
            let passes = live.counters.read().map(|c| c.passes).unwrap_or(0);
            match last {
                Some((l, p)) if p != passes => {
                    still = if l == lines { still + 1 } else { 0 };
                    last = Some((lines, passes));
                }
                None => last = Some((lines, passes)),
                _ => {}
            }
            if still >= 2 {
                break;
            }
            if begun.elapsed() >= limit {
                eprintln!("the store was still being read in after {} s ({lines} lines): the replay is refused", limit.as_secs());
                return 4;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    } else {
        // A live run: the OPC UA sources and the channels read live first, or READY passes; a log
        // reads when its lines come.
        let sources: Vec<&str> = live.sources.iter().filter(|s| s.kind != "log").map(|s| s.id.as_str()).collect();
        while started.elapsed() < READY && !sources.iter().all(|id| health(id, &|s| *s == HealthState::Live)) {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let plant = match LivePlant::new(live) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 4;
        }
    };
    let result = run(c, &plant, mode, &mut |e| match e {
        Event::StepEnded(s) => {
            let outcome = s.outcome.map(|o| o.to_string()).unwrap_or_else(|| "cut short".to_string());
            println!("{} #{} {outcome}: {}", s.step, s.attempt, s.detail);
        }
        Event::Ended { verdict, decided_at, reason } => {
            let at = decided_at.as_deref().map(|s| format!(" at {s}")).unwrap_or_default();
            println!("{verdict}{at}: {reason}");
        }
        _ => {}
    });
    let r = match result {
        Ok(r) => r,
        Err(refused) => {
            eprintln!("{refused}");
            session.stop();
            return 4;
        }
    };
    let kept = record::keep(runs, c, &r, &mode.mode, &plant);
    session.stop();
    match kept {
        Ok(folder) => println!("kept: {}", folder.display()),
        Err(e) => {
            eprintln!("the run was not kept: {e}");
            return 5;
        }
    }
    match r.verdict {
        RunVerdict::Passed => 0,
        RunVerdict::Failed => 1,
        RunVerdict::Aborted => 3,
    }
}
