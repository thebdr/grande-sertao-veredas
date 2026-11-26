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

fn exe_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_veredas-comissionador")).current_dir(dir).args(args).envs(env.iter().copied()).output().unwrap()
}

/// A project of one log, `plant`, located by `at` (`file = ...`, or a family's `dir` and
/// `pattern`), its `temp=` read into `readings`, its start two hours back.
fn log_project(at: &str) -> String {
    format!(
        "format = \"veredas-project\"\nversion = 1\nname = \"the log\"\n\n[session]\nstart = {{ last_hours = 2 }}\n\n[[log]]\nid = \"plant\"\n{at}\ntimestamp = {{ pattern = \"^(?P<ts>\\\\S+ \\\\S+)\", formats = [\"%Y-%m-%d %H:%M:%S%.3f\"] }}\n\n[[rule]]\nname = \"readings\"\npattern = \"temp=(?P<temp>[0-9.]+)\"\nscope = \"plant\"\ntypes = {{ temp = \"float\" }}\n"
    )
}

/// A log line stamped `at_ms` on the store's axis, carrying `temp`.
fn stamped(at_ms: i64, temp: &str) -> String {
    format!("{} temp={temp}\n", chrono::DateTime::from_timestamp_millis(at_ms).unwrap().naive_utc().format("%Y-%m-%d %H:%M:%S%.3f"))
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
    // without its value (the next word another option) or with an empty one, an option given
    // twice, a replay's option to a run.
    let wrong: [&[&str]; 14] = [
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
        &["run", "chk-012.test", "--project", ""],
        &["replay", "chk-012.test", "--project", "p.toml", "--from", "2026-11-23 10:00", "--to", "2026-11-23 10:01", "--runs", ""],
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
    std::fs::write(dir.path().join("off5.toml"), project_text(&bench, false, 4).replacen("value = 4 }", "value = 5 }", 1)).unwrap();
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
    // With the switch off: refused, the gated writes listed with their fingerprint; confirmed by
    // it, the run arms the switch for itself, passes, and leaves it off (Q314 (b)).
    // The project's own reply goes through the switch as well: listed under the writes and
    // confirmed with them (Q321).
    let c = veredas_comissionador::load_test(&dir.path().join("chk-012.test")).unwrap();
    let writes = veredas_comissionador::run::gated_writes(&c);
    let (code, stdout, stderr) = out(&exe(dir.path(), &["run", "chk-012.test", "--project", "off.toml"]));
    assert_eq!((code, stdout.len()), (4, 0));
    let fp = stderr[0].split('`').nth(1).unwrap().to_string();
    let mut want = vec![format!("the run is refused: the test writes, and portal's write switch is off: its 3 gated writes and the session's 1 reply, confirmed as `{fp}`, arm it for the run")];
    want.extend(writes.iter().map(|w| format!("  {w}")));
    want.push("  sorter: on announce reply answer(item: from item, dest: 4, flag: 1)".to_string());
    assert_eq!(stderr, want);
    assert_ne!(fp, veredas_comissionador::run::writes_fingerprint(&c, &[]), "the reply counted in the fingerprint");
    // A reply edited in the project asks again.
    let (code, _, stderr) = out(&exe(dir.path(), &["run", "chk-012.test", "--project", "off5.toml", "--arm", &fp]));
    assert_eq!((code, stderr[0].contains("confirms no list of this test's")), (4, true), "{stderr:?}");
    assert_eq!(writes, ["send sorter.command(code: 1)", "send sorter.command(code: 0)", "send sorter.command(code: 0)"]);
    assert!(!dir.path().join("runs/chk-012/0002").exists(), "a refused run is no run");
    let (code, _, stderr) = out(&exe(dir.path(), &["run", "chk-012.test", "--project", "off.toml", "--arm", "0123456789abcdef"]));
    assert_eq!((code, stderr[0].starts_with("the run is refused: the test writes, and portal's write switch is off: `0123456789abcdef` confirms no list of this test's")), (4, true), "{stderr:?}");
    let (code, stdout, stderr) = out(&exe(dir.path(), &["run", "chk-012.test", "--project", "off.toml", "--arm", &fp]));
    assert_eq!((code, stdout[7].as_str()), (0, "passed at summary: `summary` passed the run"), "{stdout:?} {stderr:?}");
    assert_eq!(stdout[8], format!("kept: {}", Path::new("runs/chk-012/0002").display()));
    // `--arm` is a live run's: a replay given it is a wrong command line.
    let (code, ..) = out(&exe(dir.path(), &["replay", "chk-012.test", "--project", "off.toml", "--from", "2026-11-23 10:00", "--to", "2026-11-23 10:01", "--arm", &fp]));
    assert_eq!(code, 2);
    // The folders made for a refused run are removed, those that were there kept.
    let (code, ..) = out(&exe(dir.path(), &["run", "chk-012.test", "--project", "off.toml", "--runs", "fresh/deep"]));
    assert_eq!((code, dir.path().join("fresh").exists()), (4, false));
    assert!(dir.path().join("runs/chk-012/0001").exists());
    // A project that does not load: refused, with the project's own words.
    std::fs::write(dir.path().join("broken.toml"), "format = \"veredas-project\"\nversion = 9\nname = \"x\"\n").unwrap();
    let (code, _, stderr) = out(&exe(dir.path(), &["run", "chk-012.test", "--project", "broken.toml", "--runs", "broke"]));
    assert_eq!((code, dir.path().join("broke").exists()), (4, false));
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
    // A year of anything but four digits is no moment: a run's report holds none other.
    for m in ["+10000-01-01 00:00", "-0001-01-01 00:00", "10000-01-01 00:00", "2026-11-23T10:00", "2026-11-23 10:00:5", "２０２６-11-23 10:00"] {
        let (code, _, stderr) = out(&exe(dir.path(), &["replay", "t.test", "--project", "project.toml", "--from", m, "--to", &t9]));
        assert_eq!((code, stderr[0].as_str()), (2, "a moment is `YYYY-MM-DD HH:MM[:SS[.mmm]]` on the plant's clock"), "{m}");
    }
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
    // Given one second for it (its ten minutes set lower, as a test may), the read-in still moving:
    // refused, nothing run, nothing kept, its folders removed.
    let o = exe_env(dir.path(), &["replay", "t.test", "--project", "project.toml", "--from", &from, "--to", &to, "--runs", "refused"], &[("VEREDAS_COMISSIONADOR_READ_IN", "1")]);
    let (code, stdout, stderr) = out(&o);
    assert_eq!((code, stdout.len(), dir.path().join("refused").exists()), (4, 0, false), "{stderr:?}");
    assert!(stderr[0].starts_with("the history of `plant` was not in the store after 1 s (") && stderr[0].ends_with(" lines read): the replay is refused"), "{stderr:?}");
}

#[test]
fn a_replay_starts_once_its_window_is_read_however_the_plant_keeps_writing() {
    // The replay test's ten lines an hour old, and a line stamped now appended every 100 ms while
    // the replay runs: the log has read past the window once its first live line is read, so the
    // replay starts then, where it waited for the plant to fall silent (minutes on a loaded
    // machine, and refused at ten on a quiet one). Beside it a second log, quiet, whose lines hold
    // still on their own: the live one's lines no longer count. Sixty seconds is far past the
    // seconds it takes, far short of that.
    let dir = tempfile::tempdir().unwrap();
    let base = veredas_portal::serve::store_now_ms() - 3_600_000;
    let ten: String = (0..10).map(|i| stamped(base + i * 1000, &format!("{}.0", 20 + i))).collect();
    std::fs::write(dir.path().join("plant.log"), ten).unwrap();
    std::fs::write(dir.path().join("other.log"), (0..3).map(|i| stamped(base - 1_800_000 + i * 1000, "10.0")).collect::<String>()).unwrap();
    let other = "\n[[log]]\nid = \"other\"\nfile = \"other.log\"\ntimestamp = { pattern = \"^(?P<ts>\\\\S+ \\\\S+)\", formats = [\"%Y-%m-%d %H:%M:%S%.3f\"] }\n";
    std::fs::write(dir.path().join("project.toml"), log_project("file = \"plant.log\"") + other).unwrap();
    std::fs::write(dir.path().join("t.test"), test_of("  step w wait readings.temp >= 25.0 timeout 1min\n  step r report readings.temp\n")).unwrap();
    let (t0, t9) = (veredas_comissionador::eval::time_text(base - 500), veredas_comissionador::eval::time_text(base + 9_500));
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer = {
        let (stop, log) = (stop.clone(), dir.path().join("plant.log"));
        std::thread::spawn(move || {
            use std::io::Write;
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
                f.write_all(stamped(veredas_portal::serve::store_now_ms(), "99.0").as_bytes()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        })
    };
    let begun = std::time::Instant::now();
    let o = exe_env(dir.path(), &["replay", "t.test", "--project", "project.toml", "--from", &t0, "--to", &t9], &[("VEREDAS_COMISSIONADOR_READ_IN", "120")]);
    let took = begun.elapsed();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    writer.join().unwrap();
    let (code, stdout, stderr) = out(&o);
    assert_eq!(code, 0, "{stdout:?} {stderr:?}");
    assert_eq!(stdout[0], "w #1 pass: readings.temp >= 25.0 held after 5500ms");
    assert!(took < std::time::Duration::from_secs(60), "{took:?}");
    let r = record::read(&dir.path().join("runs/t/0001")).unwrap();
    assert_eq!((r.reported[0].text.as_str(), r.evidence[0].rows), ("readings.temp = 25", 6));
}


// ---- refute round 3 ------------------------------------------------------------------------

/// The replay test's window, `t.test` waiting for 25.0, and its two moments.
fn window_test(dir: &Path, base: i64) -> (String, String) {
    std::fs::write(dir.join("t.test"), test_of("  step w wait readings.temp >= 25.0 timeout 1min\n  step r report readings.temp\n")).unwrap();
    (veredas_comissionador::eval::time_text(base - 500), veredas_comissionador::eval::time_text(base + 9_500))
}

#[test]
fn a_late_line_in_a_rotated_member_does_not_end_the_read_in() {
    // A family: its oldest member three lines a minute before the window and then a late one 20 s
    // past it, an empty member (half a second of portal's pauses after it, which a refresh falls
    // in), the newest the window's ten lines. A line past the window but before the session's start
    // is history, not the present: the replay waits for the newest member, every time.
    let dir = tempfile::tempdir().unwrap();
    let base = veredas_portal::serve::store_now_ms() - 3_600_000;
    let fam = dir.path().join("fam");
    std::fs::create_dir(&fam).unwrap();
    let mut rotated: String = (0..3).map(|i| stamped(base - 60_000 + i * 1000, "10.0")).collect();
    rotated.push_str(&stamped(base + 20_000, "40.0"));
    std::fs::write(fam.join("plant1.log"), rotated).unwrap();
    std::fs::write(fam.join("plant2.log"), "").unwrap();
    std::fs::write(fam.join("plant3.log"), (0..10).map(|i| stamped(base + i * 1000, &format!("{}.0", 20 + i))).collect::<String>()).unwrap();
    std::fs::write(dir.path().join("project.toml"), log_project("dir = \"fam\"\npattern = \"plant(?P<idx>\\\\d+)\\\\.log\"")).unwrap();
    let (t0, t9) = window_test(dir.path(), base);
    // Twelve replays at once, each its own session and its own runs folder.
    let runs: Vec<(i32, Vec<String>, u64)> = std::thread::scope(|sc| {
        let handles: Vec<_> = (0..12)
            .map(|k| {
                let (dir, t0, t9) = (dir.path(), &t0, &t9);
                sc.spawn(move || {
                    let folder = format!("r{k}");
                    let (code, stdout, _) = out(&exe(dir, &["replay", "t.test", "--project", "project.toml", "--from", t0, "--to", t9, "--runs", &folder]));
                    let rows = record::read(&dir.join(&folder).join("t/0001")).map(|r| r.evidence[0].rows).unwrap_or(0);
                    (code, stdout, rows)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for (code, stdout, rows) in &runs {
        assert_eq!((code, rows, stdout[0].as_str()), (&0, &6, "w #1 pass: readings.temp >= 25.0 held after 5500ms"), "{runs:?}");
    }
}

#[test]
fn a_log_with_no_stamps_written_live_does_not_hold_a_replay() {
    // Beside the window's log, a console log with no stamps, a line every 100 ms: its lines are
    // indexed on their arrival, none in a window that ended before the session started, so the
    // replay does not wait for it to fall silent (it was refused after its ten minutes).
    let dir = tempfile::tempdir().unwrap();
    let base = veredas_portal::serve::store_now_ms() - 3_600_000;
    std::fs::write(dir.path().join("plant.log"), (0..10).map(|i| stamped(base + i * 1000, &format!("{}.0", 20 + i))).collect::<String>()).unwrap();
    std::fs::write(dir.path().join("console.log"), "started\n").unwrap();
    std::fs::write(dir.path().join("project.toml"), log_project("file = \"plant.log\"") + "\n[[log]]\nid = \"console\"\nfile = \"console.log\"\n").unwrap();
    let (t0, t9) = window_test(dir.path(), base);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer = {
        let (stop, log) = (stop.clone(), dir.path().join("console.log"));
        std::thread::spawn(move || {
            use std::io::Write;
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                std::fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(b"a line with no stamp\n").unwrap();
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        })
    };
    let begun = std::time::Instant::now();
    let o = exe_env(dir.path(), &["replay", "t.test", "--project", "project.toml", "--from", &t0, "--to", &t9], &[("VEREDAS_COMISSIONADOR_READ_IN", "120")]);
    let took = begun.elapsed();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    writer.join().unwrap();
    let (code, stdout, stderr) = out(&o);
    assert_eq!(code, 0, "{stdout:?} {stderr:?}");
    assert_eq!(stdout[0], "w #1 pass: readings.temp >= 25.0 held after 5500ms");
    assert!(took < std::time::Duration::from_secs(60), "{took:?}");
}

// ---- portal says a log's history is read (Q319 (a), C-096) --------------------------------

#[test]
fn a_replay_waits_for_portal_to_say_each_history_is_read() {
    let base = veredas_portal::serve::store_now_ms() - 3_600_000;
    let window: String = (0..10).map(|i| stamped(base + i * 1000, &format!("{}.0", 20 + i))).collect();
    let family = "dir = \"fam\"\npattern = \"plant(?P<idx>\\\\d+)\\\\.log\"";
    let replay = |dir: &Path| {
        let (t0, t9) = window_test(dir, base);
        out(&exe_env(dir, &["replay", "t.test", "--project", "project.toml", "--from", &t0, "--to", &t9], &[("VEREDAS_COMISSIONADOR_READ_IN", "120")]))
    };
    // A family whose six empty members lie between its history and the window's lines: seconds of
    // portal's pauses with no line read, which lines holding still across two refreshes took for
    // the read-in's end (the replay then aborted at the window's end, its rows not stored yet).
    let dir = tempfile::tempdir().unwrap();
    let fam = dir.path().join("fam");
    std::fs::create_dir(&fam).unwrap();
    std::fs::write(fam.join("plant1.log"), (0..3).map(|i| stamped(base - 60_000 + i * 1000, "10.0")).collect::<String>()).unwrap();
    for k in 2..=7 {
        std::fs::write(fam.join(format!("plant{k}.log")), "").unwrap();
    }
    std::fs::write(fam.join("plant8.log"), &window).unwrap();
    std::fs::write(dir.path().join("project.toml"), log_project(family)).unwrap();
    let (code, stdout, stderr) = replay(dir.path());
    assert_eq!((code, stdout.first().map(String::as_str)), (0, Some("w #1 pass: readings.temp >= 25.0 held after 5500ms")), "{stdout:?} {stderr:?}");
    assert_eq!(record::read(&dir.path().join("runs/t/0001")).unwrap().evidence[0].rows, 6);
    // A family whose oldest member holds a line stamped an hour ahead (a clock ahead), the window's
    // lines in the newest: a line past the session's start was taken for the present.
    let dir = tempfile::tempdir().unwrap();
    let fam = dir.path().join("fam");
    std::fs::create_dir(&fam).unwrap();
    let mut ahead: String = (0..3).map(|i| stamped(base - 60_000 + i * 1000, "10.0")).collect();
    ahead.push_str(&stamped(veredas_portal::serve::store_now_ms() + 3_600_000, "10.0"));
    std::fs::write(fam.join("plant1.log"), ahead).unwrap();
    std::fs::write(fam.join("plant2.log"), &window).unwrap();
    std::fs::write(dir.path().join("project.toml"), log_project(family)).unwrap();
    let (code, stdout, stderr) = replay(dir.path());
    assert_eq!((code, stdout.first().map(String::as_str)), (0, Some("w #1 pass: readings.temp >= 25.0 held after 5500ms")), "{stdout:?} {stderr:?}");
    assert_eq!(record::read(&dir.path().join("runs/t/0001")).unwrap().evidence[0].rows, 6);
    // A log nobody may read, from a start in the past: its history never in the store, the replay
    // refused at its bound with the log named, nothing run and nothing kept (an empty store was
    // replayed, aborted at the window's end).
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plant.log");
    std::fs::write(&file, &window).unwrap();
    std::fs::set_permissions(&file, std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();
    std::fs::write(dir.path().join("project.toml"), log_project("file = \"plant.log\"")).unwrap();
    let (t0, t9) = window_test(dir.path(), base);
    let (code, stdout, stderr) = out(&exe_env(dir.path(), &["replay", "t.test", "--project", "project.toml", "--from", &t0, "--to", &t9], &[("VEREDAS_COMISSIONADOR_READ_IN", "3")]));
    assert_eq!((code, stdout.len(), dir.path().join("runs").exists()), (4, 0, false), "{stderr:?}");
    assert_eq!(stderr, ["the history of `plant` was not in the store after 3 s (0 lines read): the replay is refused"]);
}

#[test]
fn the_folders_made_for_a_refused_run_are_removed_and_no_other() {
    let dir = tempfile::tempdir().unwrap();
    // A run folder that cannot be made, part of the way made: the part made removed.
    let long = "a".repeat(300);
    std::fs::write(dir.path().join("long.test"), format!("{HEAD}test \"t\" {{\n  id {long}\n  step a stop pass\n}}\n")).unwrap();
    let (code, stdout, stderr) = out(&exe(dir.path(), &["run", "long.test", "--project", "nothere.toml", "--runs", "fresh/deep"]));
    assert_eq!((code, stdout.len(), dir.path().join("fresh").exists()), (4, 0, false), "{stderr:?}");
    assert!(stderr[0].starts_with(&format!("the run cannot be kept: {}", Path::new("fresh/deep").join(&long).display())), "{stderr:?}");
    // A `..` reaching a folder already there: that folder kept, only the ones the run made gone.
    std::fs::write(dir.path().join("t.test"), test_of("  step a stop pass\n")).unwrap();
    std::fs::create_dir(dir.path().join("existing")).unwrap();
    let (code, ..) = out(&exe(dir.path(), &["run", "t.test", "--project", "nothere.toml", "--runs", "new/../existing"]));
    assert_eq!(code, 4);
    assert!(dir.path().join("existing").is_dir(), "the folder already there kept");
    assert_eq!((dir.path().join("new").exists(), dir.path().join("existing/t").exists()), (false, false));
}

// ---- refute round 4 ------------------------------------------------------------------------

#[test]
fn a_stdout_nobody_reads_ends_no_run_and_an_argument_not_utf_8_is_a_wrong_command_line() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    example(dir.path());
    std::fs::write(dir.path().join("plant.log"), "").unwrap();
    std::fs::write(dir.path().join("project.toml"), project_text(&bench, true, 4)).unwrap();
    // stdout a pipe whose reader is gone: the verdict is still the exit code, the run still kept.
    let mut child = Command::new(env!("CARGO_BIN_EXE_veredas-comissionador"))
        .current_dir(dir.path())
        .args(["run", "chk-012.test", "--project", "project.toml"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(dir.path().join("runs/chk-012/0001/report.toml").exists());
    // An argument that is not UTF-8.
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::OsStr::from_bytes(b"caf\xe9.test");
        let o = Command::new(env!("CARGO_BIN_EXE_veredas-comissionador")).current_dir(dir.path()).arg("check").arg(name).output().unwrap();
        let (code, _, stderr) = out(&o);
        assert_eq!((code, stderr[0].as_str()), (2, "an argument is not UTF-8: caf\u{fffd}.test"));
    }
}
