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
//!   on the plant's clock), once portal says the store holds every log's history (C-096).
//!
//! Each option is given once, followed by its value, never an empty one. Exit codes: 0 the file is
//! sound or the run passed; 1 the file has problems or the run failed; 2 the command line is wrong;
//! 3 the run was aborted; 4 the run was refused before it started (its folder cannot be made, the
//! project or its session cannot start, a log's history is not in the store, the engine refuses
//! it, a replay's window opening before the store holds a table it reads among its refusals),
//! nothing run, nothing kept and the folders made for it removed; 5 the run ran, its verdict
//! printed, but could not be kept.
//!
//! `VEREDAS_COMISSIONADOR_READ_IN` sets, in seconds, how long a replay waits for every log's history
//! to be in the store (600 when unset), so a test can reach the refusal.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use veredas_comissionador::plant::{LivePlant, Plant};
use veredas_comissionador::run::{Event, RunMode, RunOptions, RunVerdict, gated_writes, run};
use veredas_comissionador::{Checked, Problem, load_functions, load_test, record};
use veredas_portal::project::LogKind;
use veredas_portal::session::{HealthState, Session};
use veredas_portal::store::TableKind;

/// A line on stdout: one nobody reads (a pipe closed) never ends the run, whose verdict is its
/// exit code and which is kept all the same.
macro_rules! say {
    ($($t:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), $($t)*);
    }};
}

/// A line on stderr, the same.
macro_rules! tell {
    ($($t:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), $($t)*);
    }};
}

const USAGE: &str = "usage: veredas-comissionador check <file.test | file.fn>
       veredas-comissionador run <file.test> --project <project.toml> [--runs <folder>] [--arm <fingerprint>]
       veredas-comissionador replay <file.test> --project <project.toml> --from <moment> --to <moment> [--runs <folder>]";

/// How long a live run waits for the session's OPC UA sources and channels to read live before it
/// starts anyway (its waits and its binding say what is missing).
const READY: Duration = Duration::from_secs(30);

/// How long a replay waits for every log's start to have read its history into the store before
/// it is refused, unless `VEREDAS_COMISSIONADOR_READ_IN` says otherwise.
const READ_IN: Duration = Duration::from_secs(600);

fn read_in() -> Duration {
    std::env::var("VEREDAS_COMISSIONADOR_READ_IN").ok().and_then(|v| v.parse().ok()).map(Duration::from_secs).unwrap_or(READ_IN)
}

/// A name as the listing shows it: as written when it is a plain word, in quotes otherwise, so a
/// listing reads one way only.
fn word(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) { s.to_string() } else { format!("{s:?}") }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn usage() -> ExitCode {
    tell!("{USAGE}");
    ExitCode::from(2)
}

fn problems(ps: Vec<Problem>) -> ExitCode {
    for p in ps {
        tell!("{p}");
    }
    ExitCode::from(1)
}

fn main() -> ExitCode {
    // An argument that is not UTF-8 is a command line this executable cannot read.
    let args: Vec<String> = match std::env::args_os().skip(1).map(|a| a.into_string()).collect::<Result<Vec<_>, _>>() {
        Ok(args) => args,
        Err(bad) => {
            tell!("an argument is not UTF-8: {}", bad.to_string_lossy());
            return usage();
        }
    };
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
            say!("{line}");
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
    const OPTIONS: [&str; 5] = ["--project", "--runs", "--from", "--to", "--arm"];
    let mut values: [Option<String>; 5] = Default::default();
    let mut i = 2;
    while i < args.len() {
        let Some(k) = OPTIONS.iter().position(|o| *o == args[i]) else { return usage() };
        // `--from` and `--to` are a replay's, `--arm` a live run's.
        if (k == 2 || k == 3) != replay && k >= 2 || values[k].is_some() {
            return usage();
        }
        let Some(value) = args.get(i + 1).filter(|v| !v.is_empty() && !v.starts_with("--")) else { return usage() };
        values[k] = Some(value.clone());
        i += 2;
    }
    let [project, runs, from, to, arm] = values;
    let Some(project) = project.map(PathBuf::from) else { return usage() };
    let mode = match (replay, from, to) {
        (false, ..) => match arm {
            Some(fp) => RunOptions::live().confirming(fp),
            None => RunOptions::live(),
        },
        (true, Some(f), Some(t)) => match (moment(&f), moment(&t)) {
            (Some(f), Some(t)) => RunOptions::replay(f, t),
            _ => {
                tell!("a moment is `YYYY-MM-DD HH:MM[:SS[.mmm]]` on the plant's clock");
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
            tell!("the run cannot be kept: {e}");
            return ExitCode::from(4);
        }
    };
    let (made, folder_made) = make_folders(&folder);
    let code = match folder_made {
        Err(e) => {
            tell!("the run cannot be kept: {}: {e}", folder.display());
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
        // A folder already there is not made again: a root a system will not let be made among them.
        if p.is_dir() {
            continue;
        }
        match std::fs::create_dir(p) {
            Ok(()) => made.push(p.to_path_buf()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && p.is_dir() => {}
            Err(e) => return (made, Err(e)),
        }
    }
    (made, Ok(()))
}

/// The session stopped, once the plant that holds its arming is gone.
fn stop(session: std::rc::Rc<Session>, plant: LivePlant) {
    drop(plant);
    if let Ok(s) = std::rc::Rc::try_unwrap(session) {
        s.stop();
    }
}

/// The plant's session started from its project file, the test run against it and kept: the exit
/// code.
fn with_session(project: &Path, c: &Checked, runs: &Path, mode: &RunOptions) -> u8 {

    // The plant's portal session, started from its project file in this process.
    let loaded = match veredas_portal::project::load_file(project) {
        Ok(p) => p,
        Err(e) => {
            tell!("{}: {e}", project.display());
            return 4;
        }
    };
    // The session's own replies, which the armed switch lets reach the plant as well: shown with
    // the test's gated writes and confirmed with them (Q321), each rule in one reading only (a
    // value in quotes, a field it copies after `from`, a name in quotes unless it is a plain
    // word), so two rules that send differently never show, nor fingerprint, alike.
    let replies: Vec<String> = loaded
        .channels
        .iter()
        .flat_map(|ch| {
            ch.replies.iter().map(move |r| {
                let fields: Vec<String> = r
                    .fields
                    .iter()
                    .map(|f| match (&f.from, &f.value) {
                        (Some(from), _) => format!("{}: from {}", word(&f.name), word(from)),
                        (_, Some(v)) => format!("{}: {:?}", word(&f.name), v.text()),
                        _ => word(&f.name),
                    })
                    .collect();
                format!("{}: on {} reply {}({})", word(&ch.source_id), word(&r.on), word(&r.send), fields.join(", "))
            })
        })
        .collect();
    // The session's start on the store's axis.
    let begun_at = chrono::Local::now().naive_local();
    let config = loaded.session_config(begun_at);
    // A session's store is fresh, so it holds each table's rows from the point its sources' start
    // reads from, as portal reads them: a log's, the project's start in the past given, from it
    // for a family's backfill and whole for a file read from its first line, and from the
    // session's start for a log with no stamps or a live start; a rule's table from the latest of
    // the logs it reads; every other table of the session's own (an OPC UA source's, a channel's)
    // from the session's start. A view is not judged: its sources are its SELECT's. A replay's
    // window opening before it is refused.
    let begun_ms = begun_at.and_utc().timestamp_millis();
    let logs: std::collections::BTreeMap<&str, i64> = loaded
        .logs
        .iter()
        .map(|l| {
            let at = match (config.start, l.timestamp.is_some(), &l.kind) {
                (Some(_), true, LogKind::File { .. }) => i64::MIN,
                (Some(t), true, LogKind::Family { .. }) => t.and_utc().timestamp_millis(),
                _ => begun_ms,
            };
            (l.id.trim(), at)
        })
        .collect();
    let ruled: std::collections::BTreeMap<String, i64> = loaded
        .rules
        .iter()
        .filter_map(|rule| {
            let at = match rule.scope.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(id) => logs.get(id).copied(),
                None => logs.values().copied().max(),
            }?;
            Some((rule.name.trim().to_ascii_lowercase(), at))
        })
        .collect();
    let session = match Session::start(config) {
        Ok(s) => std::rc::Rc::new(s),
        Err(e) => {
            tell!("{}: the session did not start: {e}", project.display());
            return 4;
        }
    };
    let live = session.live();
    let health = |id: &str, ok: &dyn Fn(&HealthState) -> bool| live.health.read().map(|h| h.get(id).is_some_and(|h| ok(&h.state))).unwrap_or(false);
    let started = Instant::now();
    if let RunMode::Replay { to_ms, .. } = &mode.mode {
        // A replay reads the store once every log's start has read its history into it, as portal
        // says in the log's health (`history_read`, C-096, Q319 (a)): every line of every file as
        // it stood when the start opened it, stored. A log with no stamps indexes its lines on
        // their arrival, none of them in a window that ended before the session started, so it
        // is not waited for then. A history that cannot be whole is never said read: the replay
        // is refused at its bound.
        let begun_ms = begun_at.and_utc().timestamp_millis();
        let mut left: Vec<&str> = live
            .sources
            .iter()
            .filter(|s| s.kind == "log" && (s.has_timestamp || *to_ms >= begun_ms))
            .map(|s| s.id.as_str())
            .collect();
        let begun = Instant::now();
        let limit = read_in();
        loop {
            // The logs read in, once and for all; the lines the others have read so far.
            let mut lines = 0;
            if let Ok(h) = live.health.read() {
                left.retain(|id| match h.get(*id) {
                    Some(s) if s.history_read => false,
                    Some(s) => {
                        lines += s.count;
                        true
                    }
                    None => true,
                });
            }
            if left.is_empty() {
                break;
            }
            if begun.elapsed() >= limit {
                let names: Vec<String> = left.iter().map(|id| format!("`{id}`")).collect();
                tell!("the history of {} was not in the store after {} s ({lines} lines read): the replay is refused", names.join(", "), limit.as_secs());
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
    // The session's own way to arm its switch, handed to the run: it arms it only for a test whose
    // gated writes `--arm` confirmed, and disarms it at the run's end (Q314 (b)).
    let arming: Box<dyn Fn(bool)> = {
        let s = session.clone();
        Box::new(move |on| s.set_writes_armed(on))
    };
    let rows_from: std::collections::BTreeMap<String, i64> = live
        .store
        .list_tables()
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t.kind == TableKind::Base)
        .map(|t| {
            let name = t.name.to_ascii_lowercase();
            let at = ruled.get(&name).copied().unwrap_or(begun_ms);
            (name, at)
        })
        .collect();
    let plant = match LivePlant::with_arming(live, arming, replies).map(|p| p.with_rows_from(rows_from)) {
        Ok(p) => p,
        Err(e) => {
            tell!("{e}");
            return 4;
        }
    };
    let result = run(c, &plant, mode, &mut |e| match e {
        Event::StepEnded(s) => {
            let outcome = s.outcome.map(|o| o.to_string()).unwrap_or_else(|| "cut short".to_string());
            say!("{} #{} {outcome}: {}", s.step, s.attempt, s.detail);
        }
        Event::Ended { verdict, decided_at, reason } => {
            let at = decided_at.as_deref().map(|s| format!(" at {s}")).unwrap_or_default();
            say!("{verdict}{at}: {reason}");
        }
        _ => {}
    });
    let r = match result {
        Ok(r) => r,
        Err(refused) => {
            tell!("{refused}");
            // The writes a confirmation is of, each on its line.
            if refused.0.iter().any(|p| p.starts_with("the test writes, and portal's write switch is off:")) {
                for w in gated_writes(c) {
                    tell!("  {w}");
                }
                for r in plant.also_armed() {
                    tell!("  {r}");
                }
            }
            stop(session, plant);
            return 4;
        }
    };
    let kept = record::keep(runs, c, &r, &mode.mode, &plant);
    stop(session, plant);
    match kept {
        Ok(folder) => say!("kept: {}", folder.display()),
        Err(e) => {
            tell!("the run was not kept: {e}");
            return 5;
        }
    }
    match r.verdict {
        RunVerdict::Passed => 0,
        RunVerdict::Failed => 1,
        RunVerdict::Aborted => 3,
    }
}
