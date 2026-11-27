//! The run record and the acceptance report (contract: C-204): a run kept under
//! `runs/<id>/<nnnn>/` with the exact text that ran, the rows it read and its report, read back,
//! listed, noted and signed, as `docs/report-schema.md` says, whose worked example this test loads.

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use common::*;
use veredas_comissionador::plant::{LivePlant, Plant};
use veredas_comissionador::record::{self, RecordError, Report};
use veredas_comissionador::run::{RunOptions, RunVerdict};

/// Every key path of a TOML value: tables walked, an array of tables read through its first.
fn keys(v: &toml::Value, at: &str, out: &mut BTreeSet<String>) {
    match v {
        toml::Value::Table(t) => {
            for (k, v) in t {
                let path = if at.is_empty() { k.clone() } else { format!("{at}.{k}") };
                out.insert(path.clone());
                keys(v, &path, out);
            }
        }
        toml::Value::Array(a) if a.iter().all(|x| x.is_table()) => {
            for x in a {
                keys(x, at, out);
            }
        }
        _ => {}
    }
}

fn key_set(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    keys(&toml::Value::Table(toml::from_str::<toml::Table>(text).unwrap()), "", &mut out);
    out
}

#[test]
fn the_reference_s_worked_example_reads_as_a_report() {
    let block = blocks("report-schema.md", "toml");
    assert_eq!(block.len(), 1);
    let r = record::parse(&block[0]).unwrap();
    assert_eq!((r.format.as_str(), r.version), ("veredas-report", 1));
    assert_eq!(r.note, "Destination 4 held full throughout; rerouting as specified.");
    assert_eq!((r.test.id.as_str(), r.test.file.as_str()), ("CHK-012", "test.test"));
    assert_eq!((r.test.functions[0].used_as.as_str(), r.test.functions[0].file.as_str()), ("functions/routing.fn", "functions/1.fn"));
    assert_eq!((r.run.number, r.run.mode.as_str(), r.run.session.as_str(), r.run.verdict.as_str()), (1, "live", "the press hall", "passed"));
    assert_eq!(r.run.decided_at.as_deref(), Some("summary"));
    assert_eq!(r.steps.len(), 7);
    assert_eq!((r.criteria.len(), r.reported.len(), r.writes.len(), r.evidence.len(), r.signatures.len()), (1, 3, 2, 2, 1));
    assert!(r.is_signed());
    // A key the format does not know, another stamp, a key missing: refused.
    let e = record::parse(&block[0].replace("ok = true\n", "ok = true\nmood = \"fine\"\n")).unwrap_err();
    assert!(e.contains("unknown field `mood`"), "{e}");
    let e = record::parse(&block[0].replace("format = \"veredas-report\"", "format = \"veredas-test\"")).unwrap_err();
    assert_eq!(e, "`format` is `veredas-test`, not `veredas-report`");
    let e = record::parse(&block[0].replace("version = 1\n", "version = 2\n")).unwrap_err();
    assert_eq!(e, "version 2 of `veredas-report` is not known here (this reader knows 1)");
    // A value outside its key's set, a moment out of its form, a count from 0, a file named
    // otherwise than the folder keeps it: refused by name (the first of each key edited).
    let refused = |from: &str, to: &str| {
        assert!(block[0].contains(from), "{from}");
        record::parse(&block[0].replacen(from, to, 1)).unwrap_err()
    };
    let moment = "a moment is `YYYY-MM-DD HH:MM:SS.mmm`";
    assert_eq!(refused("mode = \"live\"", "mode = \"banana\""), "`run.mode` is `banana`: `live`, `replay`");
    assert_eq!(refused("verdict = \"passed\"", "verdict = \"maybe\""), "`run.verdict` is `maybe`: `passed`, `failed`, `aborted`");
    assert_eq!(refused("outcome = \"pass\"", "outcome = \"great\""), "`step.outcome` is `great`: `pass`, `fail`, `timeout`, `error`");
    assert_eq!(refused("role = \"tester\"", "role = \"janitor\""), "a signature's role is `tester` or `witness`, not `janitor`");
    assert_eq!(refused("name = \"A. Tester\"", "name = \" \""), "a signature names its signer, a name with a letter or a digit, not ` `");
    assert_eq!(refused("name = \"A. Tester\"", "name = \"\u{200b}\""), "a signature names its signer, a name with a letter or a digit, not `\u{200b}`");
    assert_eq!(refused("from = \"2026-11-22 14:02:11.120\"", "from = \"yesterday\""), format!("`run.from` is `yesterday`: {moment}"));
    assert_eq!(refused("to = \"2026-11-22 14:02:18.311\"", "to = \"2026-11-22 14:02:18\""), format!("`run.to` is `2026-11-22 14:02:18`: {moment}"));
    assert_eq!(refused("at = \"2026-11-22 14:10:42.000\"", "at = \"\""), format!("`signature.at` is ``: {moment}"));
    assert_eq!(refused("attempt = 1", "attempt = 0"), "`step.attempt` is 0: it counts from 1");
    assert_eq!(refused("number = 1", "number = 0"), "`run.number` is 0: it counts from 1");
    assert_eq!(refused("file = \"test.test\"", "file = \"../../etc/passwd\""), "`test.file` is `../../etc/passwd`: `test.test`");
    assert_eq!(refused("file = \"functions/1.fn\"", "file = \"functions/2.fn\""), "`test.functions.file` is `functions/2.fn`: `functions/1.fn`");
    // An evidence file only where the folder keeps it: a plain table's by its name, a table of
    // another name by its place among the evidence from 1; never a path out of the folder, never
    // a table twice.
    let evidence = |table: &str, file: &str| {
        let first = "table = \"outcome\"\nfile = \"evidence/outcome.csv\"";
        assert!(block[0].contains(first));
        record::parse(&block[0].replacen(first, &format!("table = {table:?}\nfile = {file:?}"), 1))
    };
    for (table, file, kept) in [
        ("outcome", "../outcome.csv", "evidence/outcome.csv"),
        ("outcome", "evidence/table-1.csv", "evidence/outcome.csv"),
        ("outcome", "evidence/table-0.csv", "evidence/outcome.csv"),
        ("../../0001/evidence/outcome", "evidence/../../0001/evidence/outcome.csv", "evidence/table.1.csv"),
        ("../../../../../../etc/passwd", "evidence/../../../../../../etc/passwd.csv", "evidence/table.1.csv"),
        ("/abs", "evidence//abs.csv", "evidence/table.1.csv"),
        ("a.b", "evidence/a.b.csv", "evidence/table.1.csv"),
        ("a.b", "evidence/table.2.csv", "evidence/table.1.csv"),
        ("a.b", "evidence/table.007.csv", "evidence/table.1.csv"),
        ("a.b", "evidence/table-1.csv", "evidence/table.1.csv"),
    ] {
        assert_eq!(evidence(table, file).unwrap_err(), format!("`evidence.file` is `{file}`: `{kept}`"), "{table}");
    }
    assert!(evidence("a.b", "evidence/table.1.csv").is_ok());
    assert!(evidence("x-y_1", "evidence/x-y_1.csv").is_ok());
    assert_eq!(evidence("status", "evidence/status.csv").unwrap_err(), "`evidence.table` `status` is given twice");
    // Two entries at one file, or one table in two cases: refused.
    let both = |a: (&str, &str), b: (&str, &str)| {
        let from = "table = \"outcome\"\nfile = \"evidence/outcome.csv\"\nrows = 31\n\n[[evidence]]\ntable = \"status\"\nfile = \"evidence/status.csv\"";
        assert!(block[0].contains(from));
        record::parse(&block[0].replacen(from, &format!("table = {:?}\nfile = {:?}\nrows = 31\n\n[[evidence]]\ntable = {:?}\nfile = {:?}", a.0, a.1, b.0, b.1), 1))
    };
    // A place is `table.<n>`, which no plain name can be: a plain `table-1` beside a table placed
    // first are two files; two tables whose names differ past ASCII are two tables, as the store
    // keeps them.
    assert!(both(("a.b", "evidence/table.1.csv"), ("table-1", "evidence/table-1.csv")).is_ok());
    assert!(both(("table-2", "evidence/table-2.csv"), ("a.b", "evidence/table.2.csv")).is_ok());
    assert!(both(("Ärger", "evidence/table.1.csv"), ("ärger", "evidence/table.2.csv")).is_ok());
    assert_eq!(both(("outcome", "evidence/outcome.csv"), ("Outcome", "evidence/Outcome.csv")).unwrap_err(), "`evidence.table` `Outcome` is given twice");
    assert!(both(("outcome", "evidence/outcome.csv"), ("a.b", "evidence/table.2.csv")).is_ok());
    // A moment in any form but the reference's, to the byte: refused; a leap second too.
    for from in [
        "2026-11-22\t14:02:11.120",
        "2026-11-22\n14:02:11.120",
        "2026- 1-22 14:02:11.120",
        "2026-11-22  4:02:11.120",
        "2026-11-22 14: 2:11.120",
        "2026-11-22 \t4:02:11.120",
        "+026-11-22 14:02:11.120",
        "-026-11-22 14:02:11.120",
        "2026-11-22 23:59:60.000",
        "2026-02-30 14:02:11.120",
        "2026-11-22 14:02:11,120",
    ] {
        let e = record::parse(&block[0].replacen("from = \"2026-11-22 14:02:11.120\"", &format!("from = {from:?}"), 1)).unwrap_err();
        assert_eq!(e, format!("`run.from` is `{from}`: {moment}"));
    }
    // A step the run stopped inside: read.
    assert!(record::parse(&block[0].replacen("outcome = \"pass\"\n", "", 1)).is_ok());
    let e = record::parse(&block[0].replace("reason = \"`summary` passed the run\"\n", "")).unwrap_err();
    assert!(e.contains("missing field `reason`"), "{e}");
}

#[test]
fn a_run_is_kept_whole_read_back_listed_noted_and_signed() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let s = session(dir.path(), &project_text(&bench, true, 4));
    let test_text = blocks("test-schema.md", "test").remove(0);
    let fn_text = blocks("test-schema.md", "function").remove(0);
    let c = checked(&test_text);
    let runs = dir.path().join("runs");
    let plant = LivePlant::new(s.live()).unwrap();

    // A live run, kept.
    let (live, _) = run_with(&c, s.live(), &RunOptions::live()).unwrap();
    assert_eq!(live.verdict, RunVerdict::Passed);
    assert_eq!(live.tables.iter().cloned().collect::<Vec<_>>(), ["outcome", "status"], "the tables the test reads");
    let folder = record::keep(&runs, &c, &live, &RunOptions::live().mode, &plant).unwrap();
    assert_eq!(folder, runs.join("chk-012").join("0001"));
    let mut files: Vec<String> = walk(&folder);
    files.sort();
    assert_eq!(files, ["evidence/outcome.csv", "evidence/status.csv", "functions/1.fn", "report.toml", "test.test"]);
    assert_eq!(std::fs::read_to_string(folder.join("test.test")).unwrap(), test_text, "the exact text that ran");
    assert_eq!(std::fs::read_to_string(folder.join("functions/1.fn")).unwrap(), fn_text);

    let r = record::read(&folder).unwrap();
    assert_eq!((r.run.number, r.run.mode.as_str(), r.run.session.as_str()), (1, "live", "the press hall"));
    assert_eq!((r.run.verdict.as_str(), r.run.decided_at.as_deref(), r.run.reason.as_str()), ("passed", Some("summary"), "`summary` passed the run"));
    assert_eq!(r.run.from, veredas_comissionador::eval::time_text(live.start_ms));
    assert_eq!(r.run.to, veredas_comissionador::eval::time_text(live.end_ms));
    assert_eq!((r.test.id.as_str(), r.test.title.as_str()), ("CHK-012", "Destination 4 full, items come round"));
    assert_eq!(r.test.functions.len(), 1);
    assert_eq!((r.test.functions[0].used_as.as_str(), r.test.functions[0].file.as_str()), ("functions/routing.fn", "functions/1.fn"));
    // Each step's moments, the next starting where the last ended.
    for (x, s) in r.steps.iter().zip(&live.steps) {
        assert_eq!((x.from.as_str(), x.to.as_str()), (veredas_comissionador::eval::time_text(s.start_ms).as_str(), veredas_comissionador::eval::time_text(s.end_ms).as_str()));
    }
    assert!(r.steps.windows(2).all(|w| w[0].to == w[1].from), "{:?}", r.steps);
    assert_eq!(r.steps[0].from, r.run.from);
    let st: Vec<(String, u32, Option<String>, String)> = r.steps.iter().map(|x| (x.step.clone(), x.attempt, x.outcome.clone(), x.detail.clone())).collect();
    let want: Vec<(String, u32, Option<String>, String)> =
        live.steps.iter().map(|x| (x.step.clone(), x.attempt, x.outcome.map(|o| o.to_string()), x.detail.clone())).collect();
    assert_eq!(st, want);
    assert_eq!(r.criteria.len(), 1);
    assert_eq!(r.criteria[0].result, ">= 0.95 -> release");
    assert_eq!(
        (r.criteria[0].text.as_str(), r.criteria[0].measured.clone()),
        ("ratio: >= 0.95 -> release; 0.85 .. 0.95 -> retry route max 1; else -> fail_log", vec!["ratio = 1".to_string()])
    );
    assert_eq!(r.reported.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["ratio = 1", "margin = 0.050000000000000044", "rerouted = 20"]);
    assert_eq!(
        r.writes.iter().map(|w| (w.text.as_str(), w.sent, w.answer.as_str(), w.ok)).collect::<Vec<_>>(),
        [("sorter.command(code: 1)", true, "sent", true), ("sorter.command(code: 0)", true, "sent", true)]
    );
    assert_eq!((r.note.as_str(), r.signatures.len()), ("", 0));

    // The evidence: every row of each table the run read, stamped in its window, oldest first.
    for ev in &r.evidence {
        let table = plant.tables().unwrap().into_iter().find(|t| t.name == ev.table).unwrap();
        let rows = plant.rows(&table, live.start_ms - 1, live.end_ms).unwrap();
        let csv = std::fs::read_to_string(folder.join(&ev.file)).unwrap();
        let lines: Vec<&str> = csv.lines().collect();
        let header = format!("time,arrival,source,key,seq,{}", table.columns.iter().map(|c| c.name.clone()).collect::<Vec<_>>().join(","));
        assert_eq!(lines[0], header, "{}", ev.table);
        assert_eq!((lines.len() - 1) as u64, ev.rows);
        assert_eq!(ev.rows, rows.len() as u64);
        // Every row's cells: its stamp and its arrival on the store's axis, its source, key and
        // sequence, each column as the suite shows it, a hole empty.
        let local = |unix_ms: i64| chrono::DateTime::from_timestamp_millis(unix_ms).unwrap().with_timezone(&chrono::Local).naive_local().and_utc().timestamp_millis();
        for (line, row) in lines[1..].iter().zip(&rows) {
            let mut want = vec![
                veredas_comissionador::eval::time_text(row.time_ms),
                veredas_comissionador::eval::time_text(local(row.arrival_ms)),
                record::csv_cell(&row.source),
                record::csv_cell(row.key.as_deref().unwrap_or("")),
                row.seq.to_string(),
            ];
            want.extend(row.columns.iter().map(|(_, v)| v.as_ref().map(|v| record::csv_cell(&v.to_string())).unwrap_or_default()));
            assert_eq!(*line, want.join(","), "{}", ev.table);
        }
    }
    assert_eq!(r.evidence.iter().map(|e| e.table.as_str()).collect::<Vec<_>>(), ["outcome", "status"]);
    assert!(r.evidence[0].rows >= 20, "the twenty outcomes the function counted, and more");

    // The reference's keys are the engine's, once the report has a note and a signature.
    record::note(&folder, "kept").unwrap();
    record::sign(&folder, "tester", "A. Tester").unwrap();
    let written = std::fs::read_to_string(folder.join("report.toml")).unwrap();
    assert_eq!(key_set(&written), key_set(&blocks("report-schema.md", "toml")[0]));

    // Signed, it is frozen: a note, a second signature refused; the file unchanged.
    let signed = record::read(&folder).unwrap();
    assert_eq!((signed.note.as_str(), signed.signatures[0].role.as_str(), signed.signatures[0].name.as_str()), ("kept", "tester", "A. Tester"));
    // Signed now, on the plant's clock.
    let at = chrono::NaiveDateTime::parse_from_str(&signed.signatures[0].at, "%Y-%m-%d %H:%M:%S%.3f").unwrap().and_utc().timestamp_millis();
    assert!((veredas_portal::serve::store_now_ms() - at).abs() < 60_000, "{}", signed.signatures[0].at);
    assert!(matches!(record::note(&folder, "changed"), Err(RecordError::Signed { .. })));
    assert!(matches!(record::sign(&folder, "witness", "B. Witness"), Err(RecordError::Signed { .. })));
    assert_eq!(record::sign(&folder, "witness", "B. Witness").unwrap_err().to_string(), format!("{}: the report is signed, so it is frozen: a correction is a new run", folder.display()));
    assert_eq!(std::fs::read_to_string(folder.join("report.toml")).unwrap(), written);

    // A second run, its window replayed: the next number, listed newest first.
    let (replayed, _) = run_with(&c, s.live(), &RunOptions::replay(live.start_ms, live.end_ms)).unwrap();
    let second = record::keep(&runs, &c, &replayed, &RunOptions::replay(live.start_ms, live.end_ms).mode, &plant).unwrap();
    assert_eq!(second, runs.join("chk-012").join("0002"));
    let r2 = record::read(&second).unwrap();
    assert_eq!((r2.run.number, r2.run.mode.as_str(), r2.run.verdict.as_str()), (2, "replay", "passed"));
    assert!(r2.writes.iter().all(|w| !w.sent && w.answer == "not sent: replay"));
    // A folder without its report (a run being kept) is passed over.
    std::fs::create_dir_all(runs.join("chk-012").join("0003")).unwrap();
    let listed: Vec<u32> = record::list(&runs, &c).unwrap().iter().map(|(_, r)| r.as_ref().unwrap().run.number).collect();
    assert_eq!(listed, [2, 1]);
    // The next run takes the next number past the folder already there.
    let third = record::keep(&runs, &c, &live, &RunOptions::live().mode, &plant).unwrap();
    assert_eq!(third, runs.join("chk-012").join("0004"));
    // Past 9999: `10000`, kept and listed; a report that does not read is listed with its reason,
    // hiding no other run.
    let dir = runs.join("chk-012");
    std::fs::create_dir_all(dir.join("9999")).unwrap();
    std::fs::copy(third.join("report.toml"), dir.join("9999/report.toml")).unwrap();
    let past = record::keep(&runs, &c, &live, &RunOptions::live().mode, &plant).unwrap();
    assert_eq!(past, dir.join("10000"));
    assert_eq!(record::read(&past).unwrap().run.number, 10_000);
    std::fs::write(dir.join("0002/report.toml"), "format = \"veredas-report\"\nversion = 1\n[run\n").unwrap();
    let listed = record::list(&runs, &c).unwrap();
    let names: Vec<String> = listed.iter().map(|(f, _)| f.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["10000", "9999", "0004", "0002", "0001"]);
    assert!(listed.iter().filter(|(_, r)| r.is_err()).map(|(f, _)| f.clone()).eq([dir.join("0002")]));
    // A folder `0000` is no run: runs count from 1.
    std::fs::create_dir_all(dir.join("0000")).unwrap();
    std::fs::copy(third.join("report.toml"), dir.join("0000/report.toml")).unwrap();
    assert_eq!(record::list(&runs, &c).unwrap().len(), 5);
    // A run's folder that cannot be looked into: listed with why, never passed over; every run's
    // folder so, every run listed so, never a list of no run.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |name: &str, m: u32| std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(m)).unwrap();
        mode("0004", 0o000);
        let listed = record::list(&runs, &c).unwrap();
        mode("0004", 0o755);
        let shut: Vec<_> = listed.iter().filter(|(_, r)| matches!(r, Err(RecordError::Io { .. }))).map(|(f, _)| f.clone()).collect();
        assert_eq!((listed.len(), shut), (5, vec![dir.join("0004")]), "{listed:?}");
        let all = ["10000", "9999", "0004", "0002", "0001"];
        all.iter().for_each(|n| mode(n, 0o600));
        let listed = record::list(&runs, &c).unwrap();
        all.iter().for_each(|n| mode(n, 0o755));
        assert_eq!((listed.len(), listed.iter().all(|(_, r)| matches!(r, Err(RecordError::Io { .. })))), (5, true), "{listed:?}");
    }
    // A run whose moments a report cannot hold (a year past 9999): refused before any folder is
    // made, so a report kept is one `read` reads back.
    let mut far = live.clone();
    far.start_ms = 253_402_300_800_000;
    let before = std::fs::read_dir(&dir).unwrap().count();
    let e = record::keep(&runs, &c, &far, &RunOptions::live().mode, &plant).unwrap_err();
    assert_eq!(e.to_string(), "the run's moment `+10000-01-01 00:00:00.000` is no moment a report holds: a moment is `YYYY-MM-DD HH:MM:SS.mmm`");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), before);
    // A test's folder that cannot be read hides no run: `list` says why.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o300)).unwrap();
        let e = record::list(&runs, &c);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(&e, Err(RecordError::Io { path, .. }) if *path == dir.display().to_string()), "{e:?}");
    }
    // The last number a run can take, taken: the next run is refused by name, never wrapped.
    let full = runs.join("full");
    std::fs::create_dir_all(full.join("chk-012/4294967295")).unwrap();
    let e = record::keep(&full, &c, &live, &RunOptions::live().mode, &plant).unwrap_err();
    assert_eq!(e.to_string(), format!("{}: no run number is left past 4294967295", full.join("chk-012").display()));
    // A test's folder that is a file: `list` says why, as `keep` does.
    let filed = runs.join("filed");
    std::fs::create_dir_all(&filed).unwrap();
    std::fs::write(filed.join("chk-012"), "").unwrap();
    assert!(matches!(record::list(&filed, &c), Err(RecordError::Io { .. })));
    assert!(matches!(record::keep(&filed, &c, &live, &RunOptions::live().mode, &plant), Err(RecordError::Io { .. })));

    // A signature of no role the reference names, or no name; a report changed by hand: refused by
    // name.
    for (role, name, why) in [
        (" ", "x", "a signature's role is `tester` or `witness`, not ``"),
        ("janitor", "C", "a signature's role is `tester` or `witness`, not `janitor`"),
        ("witness", "\u{200b}", "a signature names its signer, a name with a letter or a digit, not `\u{200b}`"),
    ] {
        let e = record::sign(&third, role, name).unwrap_err();
        assert!(matches!(e, RecordError::BadSignature(_)), "{e}");
        assert_eq!(e.to_string(), why);
    }
    std::fs::write(third.join("report.toml"), std::fs::read_to_string(third.join("report.toml")).unwrap().replacen("[run]", "[run]\nmood = 1", 1)).unwrap();
    let e = record::read(&third).unwrap_err().to_string();
    assert!(e.starts_with(&format!("{}: ", third.join("report.toml").display())) && e.contains("unknown field `mood`"), "{e}");
    // A test that names roles to sign: its report names them, and awaits them.
    let roles = checked(&test_text.replacen("  use     functions/routing.fn\n", "  use     functions/routing.fn\n  sign    witness, tester\n", 1));
    let kept = record::keep(&runs.join("named"), &roles, &live, &RunOptions::live().mode, &plant).unwrap();
    let r = record::read(&kept).unwrap();
    assert_eq!((r.test.sign.clone(), r.standing()), (vec!["witness".to_string(), "tester".to_string()], "awaiting witness, tester".to_string()));
    let _: Option<Report> = None;
}

#[test]
fn two_changes_of_one_report_at_once_never_cross() {
    // The worked example unsigned and without its note, changed by two threads at once: two
    // signatures, or a note and a signature, 60 times.
    let block = blocks("report-schema.md", "toml").remove(0);
    let unsigned = block[..block.find("[[signature]]").unwrap()].replace("note = \"Destination 4 held full throughout; rerouting as specified.\"\n", "");
    let dir = tempfile::tempdir().unwrap();
    for trial in 0..60 {
        let folder = dir.path().join(trial.to_string());
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("report.toml"), &unsigned).unwrap();
        let both = std::sync::Barrier::new(2);
        let (a, b) = std::thread::scope(|s| {
            let a = s.spawn(|| {
                both.wait();
                record::sign(&folder, "tester", "A. Tester")
            });
            let b = s.spawn(|| {
                both.wait();
                if trial % 2 == 0 { record::sign(&folder, "witness", "B. Witness") } else { record::note(&folder, "a note") }
            });
            (a.join().unwrap(), b.join().unwrap())
        });
        let r = record::read(&folder).unwrap();
        assert_eq!(r.signatures.len(), 1, "trial {trial}: one signature kept");
        if trial % 2 == 0 {
            // One signature taken, the other refused as frozen, the one kept the one taken.
            let (taken, refused) = match (&a, &b) {
                (Ok(t), Err(e)) | (Err(e), Ok(t)) => (t, e),
                _ => panic!("trial {trial}: {a:?} {b:?}"),
            };
            assert!(matches!(refused, RecordError::Signed { .. }), "trial {trial}: {refused}");
            assert_eq!(r.signatures, taken.signatures, "trial {trial}");
        } else {
            // The signature always taken; the note before it, or refused after it.
            assert!(a.is_ok(), "trial {trial}: {a:?}");
            match &b {
                Ok(_) => assert_eq!(r.note, "a note", "trial {trial}"),
                Err(RecordError::Signed { .. }) => assert_eq!(r.note, "", "trial {trial}"),
                Err(e) => panic!("trial {trial}: {e}"),
            }
        }
    }
}

#[test]
fn the_evidence_s_arrival_is_on_the_store_s_axis_whatever_the_zone() {
    // The executable run under a zone five hours east of UTC: the channel's rows, stamped as they
    // arrive, read one moment in `time` and in `arrival`, five hours past UTC.
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    std::fs::create_dir_all(dir.path().join("functions")).unwrap();
    std::fs::write(dir.path().join("chk-012.test"), blocks("test-schema.md", "test").remove(0)).unwrap();
    std::fs::write(dir.path().join("functions/routing.fn"), blocks("test-schema.md", "function").remove(0)).unwrap();
    std::fs::write(dir.path().join("plant.log"), "").unwrap();
    std::fs::write(dir.path().join("project.toml"), project_text(&bench, true, 4)).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_veredas-comissionador"))
        .current_dir(dir.path())
        .env("TZ", "<+05>-5")
        .args(["run", "chk-012.test", "--project", "project.toml"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let csv = std::fs::read_to_string(dir.path().join("runs/chk-012/0001/evidence/outcome.csv")).unwrap();
    let ms = |t: &str| chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S%.3f").unwrap().and_utc().timestamp_millis();
    let rows: Vec<Vec<&str>> = csv.lines().skip(1).map(|l| l.split(',').collect()).collect();
    assert!(rows.len() >= 20, "{csv}");
    let east = chrono::Utc::now().timestamp_millis() + 5 * 3_600_000;
    assert!((ms(rows[0][0]) - east).abs() < 120_000, "the zone taken: {}", rows[0][0]);
    for r in &rows {
        assert!((ms(r[1]) - ms(r[0])).abs() <= 1_000, "{r:?}");
    }
}

#[test]
fn a_csv_cell_is_quoted_when_it_must_be() {
    assert_eq!(record::csv_cell("plain"), "plain");
    assert_eq!(record::csv_cell("a,b"), "\"a,b\"");
    assert_eq!(record::csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
    assert_eq!(record::csv_cell("two\nlines"), "\"two\nlines\"");
    assert_eq!(record::csv_cell(""), "");
}

/// Every file under a folder, as a path from it with `/`.
fn walk(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            for f in walk(&p) {
                out.push(format!("{}/{f}", p.file_name().unwrap().to_string_lossy()));
            }
        } else {
            out.push(p.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    out
}

#[test]
fn the_reference_is_the_text_this_test_holds() {
    let paragraphs = |text: &str| -> Vec<String> {
        let mut text = text.replace("\r\n", "\n");
        while let Some(start) = text.find("```toml\n") {
            let end = start + 8 + text[start + 8..].find("```").unwrap() + 3;
            text.replace_range(start..end, "");
        }
        text.split("\n\n").map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|p| !p.is_empty()).collect()
    };
    let actual = paragraphs(&doc("report-schema.md"));
    let expected = paragraphs(REFERENCE);
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(a, e, "paragraph {i} of docs/report-schema.md differs from the copy this test holds");
    }
    assert_eq!(actual.len(), expected.len());
}

/// The reference, `docs/report-schema.md`, as this test holds it: its text, the TOML block left
/// out. An edit to the reference is an edit here.
const REFERENCE: &str = r##"
# The acceptance report: `veredas-report`, version 1

Every run of a test (`docs/test-schema.md`) ends in an acceptance report, whatever its verdict
(`docs/comissionador-plan.md`, "Rules"). The engine keeps each run in a folder of its own,
`runs/<id, lowercased>/<nnnn>/`, numbered from `0001` in the order the runs were kept (`10000`
after `9999`): the exact text of the test that ran (`test.test`), every function file it used
(`functions/<n>.fn`), the rows of every table it read over its window (`evidence/<table>.csv`),
and `report.toml`, written last, so a folder that holds a report holds a whole run; once the
report has been changed, `report.lock` too, an empty file the engine locks while it reads, judges
and writes the report, so two changes never cross. `veredas_comissionador::record` keeps a run
(`keep`), reads a report back (`read`), lists a test's runs newest first, each with its report or
why it cannot be read (`list`), sets the tester's note (`note`) and signs a report (`sign`). The report is the record; a PDF or a page is a
drawing of it, Oficina's run-and-report screen among them (`docs/questions.md` Q312, taken
meanwhile: the record in TOML, anyone signing by a name typed).

Every plain-text file of Veredas 2 states its schema (`docs/questions.md` Q10): the report opens
with `format` and `version`, read by core's one header reader, and a key the format does not know
is refused rather than passed over.

## The worked example: the reference's test, passed live

The report the engine keeps for the worked example of `docs/test-schema.md`, run live against the
bench's machine through a portal session named `the press hall`: seven steps, one criterion, three
values reported, two sends, the rows of the two tables the test reads, a tester's note and one
signature. The times are the plant's clock on the store's axis, to the millisecond.
`crates/veredas-comissionador/tests/record.rs` loads this block, reads it as a report, holds every
statement below against what the engine writes for a real run of the same test, and keeps a copy
of this text, so an edit here is an edit there.


## Keys

Each key states in parentheses that it is required, or the value it takes when left out. A moment
is the plant's clock on the store's axis, `YYYY-MM-DD HH:MM:SS.mmm`.

**The top level.**
- `format` (required): always `"veredas-report"`.
- `version` (required): this reader knows `1`. A report of another format or an unknown version is
  refused, never guessed.
- `note` (`""`): the tester's note.
- `test` (required), `run` (required): tables.
- `step`, `criterion`, `reported`, `write`, `evidence`, `signature` (none): arrays of tables, in
  the order of the run.

**The test** (`[test]`).
- `id` (required), `title` (required): the test's, as its file states them.
- `file` (required): the copy of the test that ran, in the run's folder: `test.test`.
- `functions` (none): one table per function file the test used: `used_as`, the path its `use`
  wrote, and `file`, its copy in the run's folder (`functions/<n>.fn`, in the order of the uses).
- `sign` (none): the roles the test names to sign the report (its `sign` setting), each once.
  Each of them signs once, and the report stands `awaiting <roles>` until all have, then
  `signed`; with none named, the first signature makes it `signed` (Q312 (e)).

**The run** (`[run]`).
- `number` (required): the run's place among the test's runs, its folder's name without zeros.
- `mode` (required): `live` or `replay`.
- `session` (required): the portal session's name.
- `from` (required), `to` (required): the window the run covered: its start, and its clock where it
  ended.
- `verdict` (required): `passed`, `failed` or `aborted`.
- `decided_at` (none): the step that decided it.
- `reason` (required): why, in words.

**A step** (`[[step]]`): every step the run took, in order, a step taken again once per attempt.
- `step` (required), `attempt` (required, from 1).
- `outcome` (none): `pass`, `fail`, `timeout` or `error`; absent for a step the run stopped inside.
- `detail` (required): what its actions did, in words.
- `from` (required), `to` (required): the clock when it started and when it ended.

**A criterion** (`[[criterion]]`): every `check` and every `branch` as judged.
- `step` (required), `attempt` (required).
- `text` (required): the check's label or its own text; a branch's value and its arms.
- `measured` (none): the values it read, `name = value`.
- `result` (required): `pass`, `fail` or `error` for a check; the arm a branch took, or `no arm`.

**A value reported** (`[[reported]]`).
- `step` (required), `attempt` (required).
- `text` (required): `name = value`, a note's text, or what could not be evaluated and why.
- `note` (required): whether it is a note.

**A write** (`[[write]]`): every gated write and send, as made.
- `step` (required), `attempt` (required).
- `text` (required): `table.column := value` or `channel.message(field: value, ...)`.
- `sent` (required): whether it left portal for the plant: the plant's answer, its own refusal
  and a write whose answer never came were sent, a refusal portal made first (the switch, a tag
  not writable, a value it cannot coerce, the server not reached) was not; a replay sends nothing.
- `answer` (required): the plant's answer or portal's refusal, in their words; `not sent: replay`.
- `ok` (required): whether it was taken.

**The evidence** (`[[evidence]]`): one table per table the test reads.
- `table` (required): the portal table.
- `file` (required): its CSV in the run's folder, `evidence/<table>.csv` for a plain name (ASCII
  letters, digits, `_` and `-`, 64 at most, no name Windows keeps for a device such as `NUL` or
  `COM1`), else `evidence/table.<n>.csv`, `n` its place among the evidence from 1,
  which no plain name can take: a header of `time`, `arrival`, `source`, `key`, `seq` and the
  table's columns, then every row stamped in the run's window, oldest first, each cell as the
  suite shows its value and a hole empty. `time` is the row's stamp and `arrival` when it reached
  portal, both moments on the store's axis.
- `rows` (required): how many.

**A signature** (`[[signature]]`).
- `role` (required): what the signer signs as, `tester` or `witness`.
- `name` (required): the signer's name, as typed (no accounts, Q20), holding a letter or a digit.
- `at` (required): when.

## What the engine refuses

A report with another stamp, a key the format does not know, a key missing or of another type. A
value outside the ones its key names (a mode, a verdict, an outcome, a role), a moment not in its
form, an attempt or a run's number below 1, a file named otherwise than the run's folder keeps it
(`test.test`, `functions/<n>.fn` in the order of the uses, the evidence's names), a table's
evidence given twice, in any ASCII case (the store's names fold it). A run past the last number a run can take
(4294967295), or with a moment a report cannot hold (a year past 9999). A test's folder, or a
run's, that cannot be read, which `list` names rather than pass over. A note on a report
signed by anyone, and a signature once the report is `signed`: a signed report is frozen, a
correction being a new run (Q312). A signature as a role the test does not name, or as one that
has signed already, when it names roles; a second signature when it names none. This is the engine's refusal, nothing sealed in the file. A signature
whose role is neither `tester` nor `witness`, or whose name holds no letter or digit. A run of a
test with no `id`, which has no folder.
"##;

// ---- the architect's Q312 (e) ----------------------------------------------------------------

#[test]
fn the_roles_a_test_names_sign_its_report_each_once_and_it_awaits_them() {
    let block = blocks("report-schema.md", "toml").remove(0);
    let signature = "\n[[signature]]\nrole = \"tester\"\nname = \"A. Tester\"\nat = \"2026-11-22 14:10:42.000\"\n";
    assert!(block.contains(signature));
    let unsigned = block.replace(signature, "\n");
    let named = |roles: &str| unsigned.replacen("file = \"test.test\"\n", &format!("file = \"test.test\"\nsign = [{roles}]\n"), 1);
    let dir = tempfile::tempdir().unwrap();
    let put = |name: &str, text: &str| {
        let folder = dir.path().join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("report.toml"), text).unwrap();
        folder
    };
    // Both named: awaiting both, then the one left, then signed; each once; notes frozen at the
    // first signature.
    let f = put("both", &named("\"tester\", \"witness\""));
    assert_eq!(record::read(&f).unwrap().standing(), "awaiting tester, witness");
    assert_eq!(record::sign(&f, "witness", "W. Itness").unwrap().standing(), "awaiting tester");
    assert!(matches!(record::note(&f, "later"), Err(RecordError::Signed { .. })));
    let e = record::sign(&f, "witness", "W. Again").unwrap_err();
    assert_eq!(e.to_string(), format!("{}: `witness` has signed it already", f.display()));
    let r = record::sign(&f, "tester", "A. Tester").unwrap();
    assert_eq!((r.standing(), r.signatures.len()), ("signed".to_string(), 2));
    assert!(matches!(record::sign(&f, "tester", "B"), Err(RecordError::SignedAs { .. })));
    assert_eq!(record::read(&f).unwrap().standing(), "signed");
    // One named: the other role is refused.
    let f = put("one", &named("\"witness\""));
    let e = record::sign(&f, "tester", "A. Tester").unwrap_err();
    assert_eq!(e.to_string(), "the test names `witness` to sign, not `tester`");
    // None named: the first signature signs it, and freezes it.
    let f = put("none", &unsigned);
    assert_eq!(record::read(&f).unwrap().standing(), "unsigned");
    assert_eq!(record::sign(&f, "witness", "W").unwrap().standing(), "signed");
    assert!(matches!(record::sign(&f, "tester", "A"), Err(RecordError::Signed { .. })));
    // A report that breaks these, as written by hand: refused by `read`.
    let refused = |text: &str| record::parse(text).unwrap_err();
    assert_eq!(refused(&named("\"boss\"")), "`test.sign` is `boss`: `tester`, `witness`");
    assert_eq!(refused(&named("\"tester\", \"tester\"")), "`test.sign` names `tester` twice");
    let with = |roles: &str, sigs: &[(&str, &str)]| {
        let mut t = named(roles);
        for (role, name) in sigs {
            t.push_str(&format!("\n[[signature]]\nrole = \"{role}\"\nname = \"{name}\"\nat = \"2026-11-22 14:10:42.000\"\n"));
        }
        t
    };
    assert_eq!(refused(&with("\"witness\"", &[("tester", "A")])), "a signature as `tester`: the test names `witness` to sign, each once");
    assert_eq!(refused(&with("\"witness\"", &[("witness", "A"), ("witness", "B")])), "a signature as `witness`: the test names `witness` to sign, each once");
    assert!(record::parse(&with("\"tester\", \"witness\"", &[("witness", "W"), ("tester", "T")])).is_ok());
    let mut two = unsigned.clone();
    for name in ["A", "B"] {
        two.push_str(&format!("\n[[signature]]\nrole = \"tester\"\nname = \"{name}\"\nat = \"2026-11-22 14:10:42.000\"\n"));
    }
    assert_eq!(refused(&two), "two signatures on a report whose test names no role to sign: the first froze it");
}

// ---- refute round 4 ------------------------------------------------------------------------

/// A plant of the tables a test names, with no row: what `keep` asks of it, and nothing else.
struct Tables(Vec<veredas_comissionador::plant::Table>);

impl Plant for Tables {
    fn name(&self) -> String {
        "the press hall".to_string()
    }
    fn now_ms(&self) -> i64 {
        veredas_portal::serve::store_now_ms()
    }
    fn tables(&self) -> Result<Vec<veredas_comissionador::plant::Table>, veredas_comissionador::plant::PlantError> {
        Ok(self.0.clone())
    }
    fn rows(&self, _: &veredas_comissionador::plant::Table, _: i64, _: i64) -> Result<Vec<veredas_comissionador::eval::Row>, veredas_comissionador::plant::PlantError> {
        Ok(Vec::new())
    }
    fn latest(&self, _: &veredas_comissionador::plant::Table, _: &str, _: i64) -> Result<Option<(i64, veredas_comissionador::eval::Value)>, veredas_comissionador::plant::PlantError> {
        Ok(None)
    }
    fn armed(&self) -> bool {
        false
    }
    fn writable(&self, _: &str, _: &str) -> bool {
        false
    }
    fn sendable(&self, _: &str, _: &str) -> bool {
        false
    }
    fn write(&self, _: &str, _: &str, _: &veredas_comissionador::eval::Value) -> Result<String, veredas_comissionador::plant::PlantError> {
        unreachable!("keep writes nothing")
    }
    fn send(&self, _: &str, _: &str, _: Vec<(String, String)>) -> Result<(), veredas_comissionador::plant::PlantError> {
        unreachable!("keep sends nothing")
    }
}

#[test]
fn every_table_s_evidence_is_kept_in_a_file_of_its_own_and_reads_back() {
    // Tables whose names a store holds side by side (`a.b` and a plain `table-1`; `Ärger` and
    // `ärger`, which the store tells apart past ASCII) and one gone from the session: each kept in
    // a file of its own, its own header in it, the report read back.
    use veredas_comissionador::plant::{CellType, Column, Table};
    let table = |name: &str, col: &str| Table { name: name.to_string(), columns: vec![Column { name: col.to_string(), ty: CellType::Int }] };
    let plant = Tables(vec![table("a.b", "dest"), table("table-1", "running"), table("Ärger", "x"), table("ärger", "y")]);
    let c = checked(&test_of("  step a stop pass\n"));
    let now = veredas_portal::serve::store_now_ms();
    let r = veredas_comissionador::run::RunResult {
        verdict: RunVerdict::Passed,
        decided_at: Some("a".to_string()),
        reason: "`a` passed the run".to_string(),
        start_ms: now - 1000,
        end_ms: now - 500,
        steps: Vec::new(),
        criteria: Vec::new(),
        reported: Vec::new(),
        writes: Vec::new(),
        tables: ["_gone", "a.b", "table-1", "Ärger", "ärger"].iter().map(|t| t.to_string()).collect::<BTreeSet<_>>(),
    };
    let dir = tempfile::tempdir().unwrap();
    let folder = record::keep(dir.path(), &c, &r, &RunOptions::replay(now - 1000, now - 500).mode, &plant).unwrap();
    let report = record::read(&folder).unwrap();
    let files: Vec<(&str, &str)> = report.evidence.iter().map(|e| (e.table.as_str(), e.file.as_str())).collect();
    assert_eq!(files, [("a.b", "evidence/table.1.csv"), ("table-1", "evidence/table-1.csv"), ("Ärger", "evidence/table.3.csv"), ("ärger", "evidence/table.4.csv")]);
    for (e, col) in report.evidence.iter().zip(["dest", "running", "x", "y"]) {
        assert_eq!(std::fs::read_to_string(folder.join(&e.file)).unwrap(), format!("time,arrival,source,key,seq,{col}\n"), "{}", e.table);
    }
}

#[test]
fn a_plain_name_too_long_for_a_file_takes_its_place_number() {
    // Refute round 5: a plain table name of 252 characters, which the store holds and no file system
    // takes as a file's name, made the run's folder and lost the run there. A plain name keeps its
    // own file up to 64 characters, portal's own bound on a name; past it, its place number.
    use veredas_comissionador::plant::{CellType, Column, Table};
    let (short, long) = ("a".repeat(record::PLAIN_NAME), "b".repeat(252));
    let table = |name: &str| Table { name: name.to_string(), columns: vec![Column { name: "x".to_string(), ty: CellType::Int }] };
    let plant = Tables(vec![table(&short), table(&long)]);
    let c = checked(&test_of("  step a stop pass\n"));
    let now = veredas_portal::serve::store_now_ms();
    let r = veredas_comissionador::run::RunResult {
        verdict: RunVerdict::Passed,
        decided_at: Some("a".to_string()),
        reason: "`a` passed the run".to_string(),
        start_ms: now - 1000,
        end_ms: now - 500,
        steps: Vec::new(),
        criteria: Vec::new(),
        reported: Vec::new(),
        writes: Vec::new(),
        tables: [short.clone(), long.clone()].into_iter().collect::<BTreeSet<_>>(),
    };
    let dir = tempfile::tempdir().unwrap();
    let folder = record::keep(dir.path(), &c, &r, &RunOptions::replay(now - 1000, now - 500).mode, &plant).unwrap();
    let report = record::read(&folder).unwrap();
    let files: Vec<(&str, &str)> = report.evidence.iter().map(|e| (e.table.as_str(), e.file.as_str())).collect();
    assert_eq!(files, [(short.as_str(), format!("evidence/{short}.csv").as_str()), (long.as_str(), "evidence/table.2.csv")]);
    for e in &report.evidence {
        assert_eq!(std::fs::read_to_string(folder.join(&e.file)).unwrap(), "time,arrival,source,key,seq,x\n", "{}", e.table);
    }
    assert_eq!(record::list(dir.path(), &c).unwrap().len(), 1);
}

#[test]
fn a_device_s_name_and_a_name_past_the_bound_take_their_place_numbers() {
    // Refute round 6: a table named `nul`, `CON`, `aux`, `com1` or `LPT9` was a plain name, its file
    // a device on Windows (the rows gone into `NUL`, or the run lost); and the bound was pinned at 64
    // and 252 alone. Each takes its place number, as does a plain name of 65 characters; one of 64
    // and a name holding a device's as a part (`console`) keep their own.
    use veredas_comissionador::plant::{CellType, Column, Table};
    let names = ["nul", "CON", "aux", "com1", "LPT9", "console", &"c".repeat(65), &"d".repeat(64)].map(str::to_string);
    let table = |name: &str| Table { name: name.to_string(), columns: vec![Column { name: "x".to_string(), ty: CellType::Int }] };
    let plant = Tables(names.iter().map(|n| table(n)).collect());
    let c = checked(&test_of("  step a stop pass\n"));
    let now = veredas_portal::serve::store_now_ms();
    let r = veredas_comissionador::run::RunResult {
        verdict: RunVerdict::Passed,
        decided_at: Some("a".to_string()),
        reason: "`a` passed the run".to_string(),
        start_ms: now - 1000,
        end_ms: now - 500,
        steps: Vec::new(),
        criteria: Vec::new(),
        reported: Vec::new(),
        writes: Vec::new(),
        tables: names.iter().cloned().collect::<BTreeSet<_>>(),
    };
    let dir = tempfile::tempdir().unwrap();
    let folder = record::keep(dir.path(), &c, &r, &RunOptions::replay(now - 1000, now - 500).mode, &plant).unwrap();
    let report = record::read(&folder).unwrap();
    let files: Vec<(String, String)> = report.evidence.iter().map(|e| (e.table.clone(), e.file.clone())).collect();
    let place = |name: &str| format!("evidence/table.{}.csv", report.evidence.iter().position(|e| e.table == name).unwrap() + 1);
    for name in &names {
        let own = format!("evidence/{name}.csv");
        let want = if ["console", &"d".repeat(64)].contains(&name.as_str()) { own } else { place(name) };
        assert!(files.contains(&(name.clone(), want.clone())), "{name}: {files:?}");
        assert!(folder.join(&want).is_file(), "{want}");
    }
    assert!(record::device_name("nul.csv") && record::device_name("Com9") && !record::device_name("com10") && !record::device_name("nully"));
}
