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
//!   on the plant's clock), the store holding what the project's start reads into it.
//!
//! Exit codes: 0 the file is sound or the run passed; 1 the file has problems or the run failed;
//! 2 the command line is wrong; 3 the run was aborted; 4 the run was refused before it started, or
//! the project or its session could not start.

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

/// How long a run waits for the session's OPC UA sources and channels to read live before it
/// starts anyway (its waits and its binding say what is missing).
const READY: Duration = Duration::from_secs(30);

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

/// `run` and `replay`: their options read, the test checked, the session started, the test run
/// and kept.
fn go(args: &[String]) -> ExitCode {
    let replay = args[0] == "replay";
    let Some(test) = args.get(1) else { return usage() };
    let (mut project, mut runs, mut from, mut to) = (None, None, None, None);
    let mut i = 2;
    while i < args.len() {
        let value = args.get(i + 1).cloned();
        match (args[i].as_str(), value) {
            ("--project", Some(v)) => project = Some(PathBuf::from(v)),
            ("--runs", Some(v)) => runs = Some(PathBuf::from(v)),
            ("--from", Some(v)) if replay => from = moment(&v).or_else(|| Some(i64::MIN)),
            ("--to", Some(v)) if replay => to = moment(&v).or_else(|| Some(i64::MIN)),
            _ => return usage(),
        }
        i += 2;
    }
    let Some(project) = project else { return usage() };
    let mode = match (replay, from, to) {
        (false, None, None) => RunOptions::live(),
        (true, Some(f), Some(t)) if f != i64::MIN && t != i64::MIN => RunOptions::replay(f, t),
        (true, Some(_), Some(_)) => {
            eprintln!("a moment is `YYYY-MM-DD HH:MM[:SS[.mmm]]` on the plant's clock");
            return ExitCode::from(2);
        }
        _ => return usage(),
    };
    let test_path = Path::new(test);
    let c: Checked = match load_test(test_path) {
        Ok(c) => c,
        Err(ps) => return problems(ps),
    };
    let runs = runs.unwrap_or_else(|| test_path.parent().map(Path::to_path_buf).unwrap_or_default().join("runs"));

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
    // The OPC UA sources and the channels read live first, or the wait ends; a log reads when its
    // lines come. A replay waits too for what the project's start reads into the store: until the
    // ingest's count of lines holds still across two refreshes.
    let started = Instant::now();
    let mut lines = None;
    while started.elapsed() < READY {
        let ready = live
            .sources
            .iter()
            .filter(|s| s.kind != "log")
            .all(|s| live.health.read().map(|h| h.get(&s.id).is_some_and(|h| h.state == HealthState::Live)).unwrap_or(false));
        let settled = if replay {
            let now = live.counters.read().map(|c| c.lines).unwrap_or(0);
            let still = lines == Some(now);
            lines = Some(now);
            still
        } else {
            true
        };
        if ready && settled {
            break;
        }
        std::thread::sleep(Duration::from_millis(if replay { 600 } else { 100 }));
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
            return ExitCode::from(4);
        }
    }
    match r.verdict {
        RunVerdict::Passed => ExitCode::SUCCESS,
        RunVerdict::Failed => ExitCode::from(1),
        RunVerdict::Aborted => ExitCode::from(3),
    }
}
