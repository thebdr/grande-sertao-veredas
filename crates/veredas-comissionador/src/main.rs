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
//! Each option is given once, followed by its value. Exit codes: 0 the file is sound or the run
//! passed; 1 the file has problems or the run failed; 2 the command line is wrong; 3 the run was
//! aborted; 4 the run was refused before it started (its folder cannot be made, the project or its
//! session cannot start, the store is still being read in, the engine refuses it), nothing run and
//! nothing kept; 5 the run ran, its verdict printed, but could not be kept.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use veredas_comissionador::plant::LivePlant;
use veredas_comissionador::run::{Event, RunOptions, RunVerdict, run};
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
/// refused.
const READ_IN: Duration = Duration::from_secs(600);

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

/// A moment on the plant's clock, as the store's axis reads it.
fn moment(text: &str) -> Option<i64> {
    ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"]
        .iter()
        .find_map(|f| chrono::NaiveDateTime::parse_from_str(text.trim(), f).ok())
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
        let Some(value) = args.get(i + 1).filter(|v| !v.starts_with("--")) else { return usage() };
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
    // plant untouched.
    let folder = record::test_folder(&runs, &c)
        .map_err(|e| e.to_string())
        .and_then(|d| std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display())));
    if let Err(e) = folder {
        eprintln!("the run cannot be kept: {e}");
        return ExitCode::from(4);
    }

    // The plant's portal session, started from its project file in this process.
    let loaded = match veredas_portal::project::load_file(&project) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}: {e}", project.display());
            return ExitCode::from(4);
        }
    };
    let session = match Session::start(loaded.session_config(chrono::Local::now().naive_local())) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: the session did not start: {e}", project.display());
            return ExitCode::from(4);
        }
    };
    let live = session.live();
    let health = |id: &str, ok: &dyn Fn(&HealthState) -> bool| live.health.read().map(|h| h.get(id).is_some_and(|h| ok(&h.state))).unwrap_or(false);
    let started = Instant::now();
    if replay {
        // A replay reads the store: every log has read its first line, or READY has passed (an
        // empty log reads none); then the ingest's count of lines holds still across two of
        // portal's refreshes, each known by a new count of the pump's passes.
        let logs: Vec<&str> = live.sources.iter().filter(|s| s.kind == "log").map(|s| s.id.as_str()).collect();
        while started.elapsed() < READY && !logs.iter().all(|id| health(id, &|s| *s != HealthState::Waiting)) {
            std::thread::sleep(Duration::from_millis(100));
        }
        let (mut last, mut still) = (None::<(u64, u64)>, 0);
        let begun = Instant::now();
        while still < 2 {
            let (lines, passes) = live.counters.read().map(|c| (c.lines, c.passes)).unwrap_or((0, 0));
            match last {
                Some((l, p)) if p != passes => {
                    still = if l == lines { still + 1 } else { 0 };
                    last = Some((lines, passes));
                }
                None => last = Some((lines, passes)),
                _ => {}
            }
            if still < 2 && begun.elapsed() >= READ_IN {
                eprintln!("the store was still being read in after {} s ({lines} lines): the replay is refused", READ_IN.as_secs());
                return ExitCode::from(4);
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
            return ExitCode::from(4);
        }
    };
    let result = run(&c, &plant, &mode, &mut |e| match e {
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
            return ExitCode::from(4);
        }
    };
    let kept = record::keep(&runs, &c, &r, &mode.mode, &plant);
    session.stop();
    match kept {
        Ok(folder) => println!("kept: {}", folder.display()),
        Err(e) => {
            eprintln!("the run was not kept: {e}");
            return ExitCode::from(5);
        }
    }
    match r.verdict {
        RunVerdict::Passed => ExitCode::SUCCESS,
        RunVerdict::Failed => ExitCode::from(1),
        RunVerdict::Aborted => ExitCode::from(3),
    }
}
