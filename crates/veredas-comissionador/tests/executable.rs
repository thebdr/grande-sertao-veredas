//! The executable (contract: C-205): `veredas-comissionador check | run | replay` from the command
//! line, against the bench through a portal session the executable starts from a project file, the
//! verdict in the exit code, every run kept.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::*;
use veredas_comissionador::record;

fn exe(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_veredas-comissionador")).current_dir(dir).args(args).output().unwrap()
}

fn out(o: &Output) -> (i32, Vec<String>, Vec<String>) {
    let lines = |b: &[u8]| String::from_utf8_lossy(b).lines().map(str::to_string).collect::<Vec<_>>();
    (o.status.code().unwrap_or(-1), lines(&o.stdout), lines(&o.stderr))
}

/// The worked example and its function file written into `dir`.
fn example(dir: &Path) {
    std::fs::create_dir_all(dir.join("functions")).unwrap();
    std::fs::write(dir.join("chk-012.test"), blocks("test-schema.md", "test").remove(0)).unwrap();
    std::fs::write(dir.join("functions/routing.fn"), blocks("test-schema.md", "function").remove(0)).unwrap();
}

#[test]
fn check_reads_a_file_and_names_its_problems() {
    let dir = tempfile::tempdir().unwrap();
    example(dir.path());
    let (code, stdout, _) = out(&exe(dir.path(), &["check", "chk-012.test"]));
    assert_eq!((code, stdout), (0, vec!["chk-012.test: 8 steps · 1 branch · 1 function · 3 gated writes".to_string()]));
    let (code, stdout, _) = out(&exe(dir.path(), &["check", "functions/routing.fn"]));
    assert_eq!((code, stdout), (0, vec!["functions/routing.fn: 1 function: ItemRoute(dest: int, items: int) -> (delivered: int, rerouted: int)".to_string()]));
    std::fs::write(dir.path().join("bad.test"), test_of("  step a check ghost == 1\n  step a stop pass\n")).unwrap();
    let (code, _, stderr) = out(&exe(dir.path(), &["check", "bad.test"]));
    // Every problem, in the order the check names them (C-201).
    assert_eq!((code, stderr), (1, vec!["bad.test:7:8: step `a` is defined twice".to_string(), "bad.test:6:16: `ghost` is read but nothing assigns it".to_string()]));
    let (code, _, stderr) = out(&exe(dir.path(), &["check", "none.test"]));
    assert_eq!(code, 1);
    assert!(stderr[0].starts_with("none.test:1:1: cannot be read: "), "{stderr:?}");
    // A command line that is wrong: no command, an unknown one, a missing file or option, an option
    // without its value (the next word another option), an option given twice, a replay's option
    // to a run.
    let wrong: [&[&str]; 12] = [
        &[],
        &["check"],
        &["check", "a", "b"],
        &["verify", "a"],
        &["run", "chk-012.test"],
        &["replay", "chk-012.test", "--project", "p.toml"],
        &["run", "--project", "p.toml"],
        &["run", "chk-012.test", "--project", "--runs"],
        &["run", "chk-012.test", "--runs", "--from", "--project", "p.toml"],
        &["run", "chk-012.test", "--project", "p.toml", "--project", "p.toml"],
        &["replay", "chk-012.test", "--project", "p.toml", "--from", "2026-11-23 10:00", "--from", "2026-11-23 10:01", "--to", "2026-11-23 10:02"],
        &["run", "chk-012.test", "--project", "p.toml", "--from", "2026-11-23 10:00"],
    ];
    for args in wrong {
        let (code, _, stderr) = out(&exe(dir.path(), args));
        assert_eq!(code, 2, "{args:?}");
        assert!(stderr[0].starts_with("usage: veredas-comissionador check"), "{args:?}: {stderr:?}");
    }
}

#[test]
fn run_starts_the_session_runs_live_keeps_the_run_and_answers_by_its_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    example(dir.path());
    std::fs::write(dir.path().join("plant.log"), "").unwrap();
    // Both projects written while the bench's machine still says where it listens.
    std::fs::write(dir.path().join("project.toml"), project_text(&bench, true, 4)).unwrap();
    std::fs::write(dir.path().join("off.toml"), project_text(&bench, false, 4)).unwrap();
    let (code, stdout, stderr) = out(&exe(dir.path(), &["run", "chk-012.test", "--project", "project.toml"]));
    assert_eq!(code, 0, "{stdout:?} {stderr:?}");
    assert_eq!(stdout[0], "start #1 pass: sorter.command(code: 1): sent");
    assert_eq!(stdout[2], "route #1 pass: delivered 0 · rerouted 20");
    assert_eq!(stdout[7], "passed at summary: `summary` passed the run");
    let folder = dir.path().join("runs/chk-012/0001");
    assert_eq!(stdout[8], format!("kept: {}", Path::new("runs/chk-012/0001").display()));
    assert_eq!(stdout.len(), 9);
    let r = record::read(&folder).unwrap();
    assert_eq!((r.run.mode.as_str(), r.run.verdict.as_str(), r.run.session.as_str()), ("live", "passed", "the press hall"));
    // Elsewhere, under `--runs`.
    let (code, stdout, _) = out(&exe(dir.path(), &["run", "chk-012.test", "--project", "project.toml", "--runs", "kept"]));
    assert_eq!((code, stdout.last().cloned()), (0, Some(format!("kept: {}", Path::new("kept/chk-012/0001").display()))));
    // A failed run, an aborted one, a refused one: each its code, the first two kept.
    std::fs::write(dir.path().join("fail.test"), test_of("  step a check plc_main.level > 5\n")).unwrap();
    let (code, stdout, _) = out(&exe(dir.path(), &["run", "fail.test", "--project", "project.toml"]));
    assert_eq!((code, stdout[1].as_str()), (1, "failed at a: `a` failed: plc_main.level > 5: fail (plc_main.level = 0.9)"));
    std::fs::write(dir.path().join("abort.test"), test_of("  step a wait plc_main.speed >= 9 timeout 300ms\n")).unwrap();
    let (code, stdout, _) = out(&exe(dir.path(), &["run", "abort.test", "--project", "project.toml"]));
    assert_eq!((code, stdout[1].as_str()), (3, "aborted at a: `a` timed out: plc_main.speed >= 9 did not hold within 300ms"));
    assert!(dir.path().join("runs/t/0002/report.toml").exists(), "both runs of `T` kept");
    let (code, stdout, stderr) = out(&exe(dir.path(), &["run", "chk-012.test", "--project", "off.toml"]));
    assert_eq!((code, stdout.len()), (4, 0));
    assert_eq!(stderr, ["the run is refused: the test writes, and portal's write switch is off"]);
    assert!(!dir.path().join("runs/chk-012/0002").exists(), "a refused run is no run");
    // A project that does not load: refused, with the project's own words.
    std::fs::write(dir.path().join("broken.toml"), "format = \"veredas-project\"\nversion = 9\nname = \"x\"\n").unwrap();
    let (code, _, stderr) = out(&exe(dir.path(), &["run", "chk-012.test", "--project", "broken.toml"]));
    assert_eq!(code, 4);
    assert!(stderr[0].starts_with("broken.toml: "), "{stderr:?}");
}

#[test]
fn replay_reads_the_window_the_project_s_start_fills() {
    let dir = tempfile::tempdir().unwrap();
    // A log of ten lines a second apart, an hour old, read whole by a start two hours back; the
    // lines and the window's ends placed from one moment.
    std::fs::write(dir.path().join("plant.log"), "").unwrap();
    let base = veredas_portal::serve::store_now_ms() - 3_600_000;
    for i in 0..10 {
        log_line_at(dir.path(), base + i * 1000, &format!("{}.0", 20 + i));
    }
    let t0 = veredas_comissionador::eval::time_text(base - 500);
    let t9 = veredas_comissionador::eval::time_text(base + 9_500);
    std::fs::write(
        dir.path().join("project.toml"),
        "format = \"veredas-project\"\nversion = 1\nname = \"the log\"\n\n[session]\nstart = { last_hours = 2 }\n\n[[log]]\nid = \"plant\"\nfile = \"plant.log\"\ntimestamp = { pattern = \"^(?P<ts>\\\\S+ \\\\S+)\", formats = [\"%Y-%m-%d %H:%M:%S%.3f\"] }\n\n[[rule]]\nname = \"readings\"\npattern = \"temp=(?P<temp>[0-9.]+)\"\nscope = \"plant\"\ntypes = { temp = \"float\" }\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("t.test"), test_of("  step w wait readings.temp >= 25.0 timeout 1min\n  step r report readings.temp\n")).unwrap();
    let (code, stdout, stderr) = out(&exe(dir.path(), &["replay", "t.test", "--project", "project.toml", "--from", &t0, "--to", &t9]));
    assert_eq!(code, 0, "{stdout:?} {stderr:?}");
    assert_eq!(stdout[0], "w #1 pass: readings.temp >= 25.0 held after 5500ms", "the sixth line, 5.5 s after the window's start");
    assert_eq!(stdout[2], "passed at r: the run passed its last step");
    let r = record::read(&dir.path().join("runs/t/0001")).unwrap();
    assert_eq!((r.run.mode.as_str(), r.reported[0].text.as_str()), ("replay", "readings.temp = 25"));
    assert_eq!(r.evidence[0].rows, 6, "the lines of the run's window: from its start to the stamp it ended on");
    // A folder the run cannot be kept in: refused before the session starts, nothing run.
    std::fs::write(dir.path().join("afile"), "").unwrap();
    let (code, stdout, stderr) = out(&exe(dir.path(), &["replay", "t.test", "--project", "project.toml", "--from", &t0, "--to", &t9, "--runs", "afile"]));
    assert_eq!((code, stdout.len()), (4, 0), "{stderr:?}");
    assert!(stderr[0].starts_with(&format!("the run cannot be kept: {}: ", Path::new("afile/t").display())), "{stderr:?}");
    // A run that ran but could not be kept: its verdict printed, 5.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let shut = dir.path().join("shut");
        std::fs::create_dir_all(shut.join("t")).unwrap();
        std::fs::set_permissions(shut.join("t"), std::fs::Permissions::from_mode(0o555)).unwrap();
        let (code, stdout, stderr) = out(&exe(dir.path(), &["replay", "t.test", "--project", "project.toml", "--from", &t0, "--to", &t9, "--runs", "shut"]));
        std::fs::set_permissions(shut.join("t"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!((code, stdout.last().map(String::as_str)), (5, Some("passed at r: the run passed its last step")), "{stderr:?}");
        assert!(stderr[0].starts_with("the run was not kept: "), "{stderr:?}");
        assert_eq!(std::fs::read_dir(shut.join("t")).unwrap().count(), 0);
    }
    // A moment that is none: the command line's fault.
    let (code, _, stderr) = out(&exe(dir.path(), &["replay", "t.test", "--project", "project.toml", "--from", "yesterday", "--to", &t9]));
    assert_eq!((code, stderr[0].as_str()), (2, "a moment is `YYYY-MM-DD HH:MM[:SS[.mmm]]` on the plant's clock"));
}

#[test]
fn a_replay_waits_until_the_store_holds_what_the_start_reads() {
    // A log of 200 000 lines 20 ms apart, the last an hour old and the only one at 999: read in
    // over several of portal's refreshes, the window its last five seconds.
    let dir = tempfile::tempdir().unwrap();
    let last = veredas_portal::serve::store_now_ms() - 3_600_000;
    let n = 200_000;
    let mut log = String::with_capacity(n as usize * 34);
    for i in 0..n {
        let at = chrono::DateTime::from_timestamp_millis(last - (n - 1 - i) * 20).unwrap().naive_utc();
        log.push_str(&format!("{} temp={}\n", at.format("%Y-%m-%d %H:%M:%S%.3f"), if i == n - 1 { "999.0" } else { "20.0" }));
    }
    std::fs::write(dir.path().join("plant.log"), log).unwrap();
    std::fs::write(
        dir.path().join("project.toml"),
        "format = \"veredas-project\"\nversion = 1\nname = \"the log\"\n\n[session]\nstart = { last_hours = 2 }\n\n[[log]]\nid = \"plant\"\nfile = \"plant.log\"\ntimestamp = { pattern = \"^(?P<ts>\\\\S+ \\\\S+)\", formats = [\"%Y-%m-%d %H:%M:%S%.3f\"] }\n\n[[rule]]\nname = \"readings\"\npattern = \"temp=(?P<temp>[0-9.]+)\"\nscope = \"plant\"\ntypes = { temp = \"float\" }\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("t.test"), test_of("  step w wait readings.temp >= 999.0 timeout 1min\n  step r report readings.temp\n")).unwrap();
    let from = veredas_comissionador::eval::time_text(last - 5_000);
    let to = veredas_comissionador::eval::time_text(last + 1_000);
    // Twice, the same: the wait met at the last line, its window's 251 lines kept.
    for k in 1..=2 {
        let (code, stdout, stderr) = out(&exe(dir.path(), &["replay", "t.test", "--project", "project.toml", "--from", &from, "--to", &to]));
        assert_eq!(code, 0, "replay {k}: {stdout:?} {stderr:?}");
        let r = record::read(&dir.path().join(format!("runs/t/000{k}"))).unwrap();
        assert_eq!((r.reported[0].text.as_str(), r.evidence[0].rows), ("readings.temp = 999", 251), "replay {k}");
    }
}

