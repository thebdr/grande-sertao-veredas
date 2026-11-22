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
    let st: Vec<(String, u32, Option<String>, String)> = r.steps.iter().map(|x| (x.step.clone(), x.attempt, x.outcome.clone(), x.detail.clone())).collect();
    let want: Vec<(String, u32, Option<String>, String)> =
        live.steps.iter().map(|x| (x.step.clone(), x.attempt, x.outcome.map(|o| o.to_string()), x.detail.clone())).collect();
    assert_eq!(st, want);
    assert_eq!(r.criteria.len(), 1);
    assert_eq!(r.criteria[0].result, ">= 0.95 -> release");
    assert_eq!(r.reported.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["ratio = 1", "margin = 0.050000000000000044", "rerouted = 20"]);
    assert_eq!(r.writes.iter().map(|w| (w.text.as_str(), w.sent, w.ok)).collect::<Vec<_>>(), [("sorter.command(code: 1)", true, true), ("sorter.command(code: 0)", true, true)]);
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
        if let Some(first) = rows.first() {
            assert!(lines[1].starts_with(&veredas_comissionador::eval::time_text(first.time_ms)), "{}", lines[1]);
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
    let listed: Vec<u32> = record::list(&runs, &c).unwrap().iter().map(|(_, r)| r.run.number).collect();
    assert_eq!(listed, [2, 1]);
    // The next run takes the next number past the folder already there.
    let third = record::keep(&runs, &c, &live, &RunOptions::live().mode, &plant).unwrap();
    assert_eq!(third, runs.join("chk-012").join("0004"));

    // A blank signature, a report changed by hand: refused by name.
    assert!(matches!(record::sign(&second, " ", "x"), Err(RecordError::BlankSignature)));
    std::fs::write(second.join("report.toml"), std::fs::read_to_string(second.join("report.toml")).unwrap().replacen("[run]", "[run]\nmood = 1", 1)).unwrap();
    let e = record::read(&second).unwrap_err().to_string();
    assert!(e.starts_with(&format!("{}: ", second.join("report.toml").display())) && e.contains("unknown field `mood`"), "{e}");
    let _: Option<Report> = None;
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
`runs/<id, lowercased>/<nnnn>/`, numbered from `0001` in the order the runs were kept: the exact
text of the test that ran (`test.test`), every function file it used (`functions/<n>.fn`), the
rows of every table it read over its window (`evidence/<table>.csv`), and `report.toml`, written
last, so a folder that holds a report holds a whole run. `veredas_comissionador::record` keeps a
run (`keep`), reads a report back (`read`), lists a test's runs newest first (`list`), sets the
tester's note (`note`) and signs a report (`sign`). The report is the record; a PDF or a page is a
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
- `sent` (required): whether it went to the plant; a replay sends nothing.
- `answer` (required): the plant's answer or portal's refusal, in their words; `not sent: replay`.
- `ok` (required): whether it was taken.

**The evidence** (`[[evidence]]`): one table per table the test reads.
- `table` (required): the portal table.
- `file` (required): its CSV in the run's folder, `evidence/<table>.csv` (`evidence/table-<n>.csv`
  for a name that is not plain): a header of `time`, `arrival`, `source`, `key`, `seq` and the
  table's columns, then every row stamped in the run's window, oldest first, each cell as the
  suite shows its value and a hole empty.
- `rows` (required): how many.

**A signature** (`[[signature]]`).
- `role` (required): what the signer signs as, `tester` or `witness`.
- `name` (required): the signer's name, as typed (no accounts, Q20).
- `at` (required): when.

## What the engine refuses

A report with another stamp, a key the format does not know, a key missing or of another type. A
note or a signature on a report already signed: a signed report is frozen, a correction being a new
run (Q312, (d) taken meanwhile). This is the engine's refusal, nothing sealed in the file. A
signature with a blank role or name. A run of a test with no `id`, which has no folder.
"##;
