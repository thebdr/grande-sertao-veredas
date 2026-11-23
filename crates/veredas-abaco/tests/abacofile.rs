//! The `.abaco` file (contract: Ábaco's `.abaco` clause; `docs/abaco-file.md`).
//!
//! - Every workbook of the corpus (the fork's test files, most of them saved by Excel) opened,
//!   written as `.abaco` and read back: every cell's content, style and value, and every part the
//!   engine holds besides the cells, equal; written again to the same bytes; exported to `.xlsx`
//!   through the fork's writer and opened again to the same values.
//! - The reference's worked example is what the writer writes, and reads back.
//! - A file that is not what it should be is refused, naming its part and place.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use veredas_abaco::abacofile::{self, AbacoFile};
use veredas_abaco::links::{LinkChoice, LinkPolicy};
use veredas_abaco::workspace::Reference;
use veredas_abaco::{Addr, Content, Value, Workbook};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../forks/ironcalc/xlsx/tests").canonicalize().unwrap()
}

/// Workbooks upstream's own tests write into the folder while the fork's suite runs, and remove.
const SCRATCH: [&str; 2] = ["conditional_formatting/cf_tests_round_trip.xlsx", "templates/crossword_round_trip.xlsx"];

fn corpus() -> Vec<String> {
    let dir = corpus_dir();
    let mut out = Vec::new();
    let mut stack = vec![dir.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "xlsx") {
                let rel = p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/");
                if !SCRATCH.contains(&rel.as_str()) {
                    out.push(rel);
                }
            }
        }
    }
    out.sort();
    out
}

/// Random draws and the clock the same on every evaluation.
fn fixed_evaluation() {
    ironcalc_base::seed_random(Some(1));
    ironcalc_base::pin_clock(Some(1_795_348_800_000)); // 2026-11-22, noon UTC
}

fn open(path: &Path) -> Workbook {
    fixed_evaluation();
    Workbook::open(path, LinkPolicy::AlwaysUseCached, |_| LinkChoice::UseCached).unwrap()
}

fn read(path: &Path) -> AbacoFile {
    fixed_evaluation();
    abacofile::read(path).unwrap()
}

/// Where two workbooks differ, cell by cell and part by part; empty when they are the same.
fn differences(a: &Workbook, b: &Workbook) -> Vec<String> {
    let mut out = Vec::new();
    let (ea, eb) = (&a.model().workbook, &b.model().workbook);
    macro_rules! same {
        ($what:expr, $x:expr, $y:expr) => {
            if $x != $y {
                out.push(format!("{}: {:?} / {:?}", $what, $x, $y));
            }
        };
    }
    same!("name", ea.name, eb.name);
    same!("settings", ea.settings, eb.settings);
    same!("metadata", ea.metadata, eb.metadata);
    same!("defined names", ea.defined_names, eb.defined_names);
    same!("styles", ea.styles, eb.styles);
    same!("theme", ea.theme, eb.theme);
    same!("tables", ea.tables, eb.tables);
    same!("external books", ea.external_books, eb.external_books);
    same!("sheets", ea.worksheets.len(), eb.worksheets.len());
    for (i, (wa, wb)) in ea.worksheets.iter().zip(&eb.worksheets).enumerate() {
        let s = &wa.name;
        same!(format!("{s} name"), wa.name, wb.name);
        same!(format!("{s} id"), wa.sheet_id, wb.sheet_id);
        same!(format!("{s} state"), wa.state, wb.state);
        same!(format!("{s} color"), wa.color, wb.color);
        same!(format!("{s} dimension"), wa.dimension, wb.dimension);
        same!(format!("{s} cols"), wa.cols, wb.cols);
        same!(format!("{s} rows"), wa.rows, wb.rows);
        same!(format!("{s} merged"), wa.merged_cells, wb.merged_cells);
        same!(format!("{s} comments"), wa.comments, wb.comments);
        same!(format!("{s} frozen"), (wa.frozen_rows, wa.frozen_columns), (wb.frozen_rows, wb.frozen_columns));
        same!(format!("{s} grid lines"), wa.show_grid_lines, wb.show_grid_lines);
        same!(format!("{s} formats"), wa.conditional_formatting, wb.conditional_formatting);
        same!(format!("{s} links"), wa.links, wb.links);
        let sheet = i as u32;
        let cells = |w: &Workbook| -> BTreeMap<Addr, i32> {
            w.cells(sheet)
                .unwrap()
                .into_iter()
                .filter(|(a, s)| *s != 0 || w.content(sheet, *a).unwrap() != Content::Spill)
                .collect()
        };
        let (ca, cb) = (cells(a), cells(b));
        for addr in ca.keys().chain(cb.keys().filter(|k| !ca.contains_key(k))) {
            same!(format!("{s}!{addr} style"), ca.get(addr), cb.get(addr));
            same!(format!("{s}!{addr} content"), a.content(sheet, *addr).unwrap(), b.content(sheet, *addr).unwrap());
            same!(format!("{s}!{addr} value"), a.value_now(sheet, *addr).unwrap(), b.value_now(sheet, *addr).unwrap());
        }
    }
    out
}

/// The workbooks the round trip refuses, each with the start of its reason.
const REFUSED: [(&str, &str); 0] = [];

/// The workbooks whose `.xlsx` export does not read back to the same values, each with why: the
/// fork's writer writes no tables (upstream keeps the workbook in its `calc_test_no_export`), so a
/// structured reference reads other cells. Pinned both ways: such a workbook's export differs, and
/// no other's does.
const EXPORT_LIMITS: [(&str, &str); 1] = [("calc_test_no_export/tables.xlsx", "the fork's writer writes no tables")];

#[test]
fn every_workbook_of_the_corpus_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let mut failed = Vec::new();
    let mut refused = Vec::new();
    let mut done = 0;
    for rel in corpus() {
        let src = corpus_dir().join(&rel);
        let original = open(&src);
        let mut file = AbacoFile::new(open(&src));
        let first = dir.path().join("first.abaco");
        if let Err(e) = abacofile::write(&mut file, &first) {
            refused.push((rel, e.to_string()));
            continue;
        }
        // one line per cell in every sheet's `[cells]`
        for (name, bytes) in abacofile::parts(&file).unwrap().into_iter().filter(|(n, _)| n.starts_with("sheets/")) {
            let text = String::from_utf8(bytes).unwrap();
            let cells = text.split_once("\n[cells]\n").map(|(_, c)| c).unwrap_or("");
            let keys = toml::from_str::<toml::Table>(cells).unwrap().len();
            if unicode_lines(cells) != keys {
                failed.push(format!("{rel}: {name} holds {keys} cells on {} lines", unicode_lines(cells)));
            }
        }
        let mut back = match std::panic::catch_unwind(|| read(&first)) {
            Ok(back) => back,
            Err(_) => {
                failed.push(format!("{rel}: read back failed"));
                continue;
            }
        };
        let diffs = differences(&original, &back.book);
        if !diffs.is_empty() {
            failed.push(format!("{rel}: {} difference(s), first: {}", diffs.len(), diffs[0]));
            continue;
        }
        // written again: the same bytes
        let again = dir.path().join("again.abaco");
        abacofile::write(&mut back, &again).unwrap();
        if std::fs::read(&first).unwrap() != std::fs::read(&again).unwrap() {
            failed.push(format!("{rel}: written again to other bytes"));
            continue;
        }
        // exported and opened again: the same values
        let xlsx = dir.path().join("export.xlsx");
        back.book.export(&xlsx).unwrap();
        let exported = open(&xlsx);
        let values: Vec<String> = differences(&original, &exported).into_iter().filter(|d| d.contains(" value:")).collect();
        let limited = EXPORT_LIMITS.iter().any(|(l, _)| *l == rel);
        if values.is_empty() == limited {
            failed.push(match values.first() {
                Some(first) => format!("{rel}: exported, {} value(s) differ, first: {first}", values.len()),
                None => format!("{rel}: exported to the same values, though pinned as a limit of the export"),
            });
            continue;
        }
        done += 1;
    }
    let refused_now: Vec<(&str, &str)> = refused.iter().map(|(r, e)| (r.as_str(), e.as_str())).collect();
    for ((rel, reason), (pin_rel, pin_reason)) in refused_now.iter().zip(REFUSED.iter()) {
        assert_eq!(rel, pin_rel);
        assert!(reason.starts_with(pin_reason), "{rel}: {reason}");
    }
    assert_eq!(refused_now.len(), REFUSED.len(), "refused: {refused_now:#?}");
    assert!(failed.is_empty(), "{} workbook(s) did not round trip:\n{}", failed.len(), failed.join("\n"));
    assert_eq!(done + refused.len(), corpus().len());
    assert!(done >= 260, "only {done} round trips");
}

/// How many lines a text holds to a reader that knows Unicode's line breaks (Python's
/// `splitlines`: line feed, carriage return and the pair, U+000B, U+000C, U+001C to U+001E,
/// U+0085, U+2028, U+2029), a last line with no break counted.
fn unicode_lines(text: &str) -> usize {
    let breaks = ['\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}'];
    let text = text.replace("\r\n", "\n");
    let n = text.chars().filter(|c| breaks.contains(c)).count();
    if text.ends_with(breaks) || text.is_empty() { n } else { n + 1 }
}

/// The reference's worked example (`docs/abaco-file.md`): a beam check and the session around it.
fn worked_example() -> AbacoFile {
    let mut book = Workbook::new("Beam check").unwrap();
    book.rename_sheet("Sheet1", "Loads").unwrap();
    let at = |a: &str| Addr::parse(a).unwrap();
    for (cell, text) in [("A1", "Span (m)"), ("A2", "Load (kN/m)"), ("A3", "Moment (kNm)"), ("A4", "Checked"), ("A5", "Ratio")] {
        book.set_value(0, at(cell), &Value::Text(text.into())).unwrap();
    }
    book.set_value(0, at("B1"), &Value::Number(6.5)).unwrap();
    book.set_value(0, at("B2"), &Value::Number(12.0)).unwrap();
    book.set_formula(0, at("B3"), "=B2*B1^2/8").unwrap();
    book.set_value(0, at("B4"), &Value::Bool(true)).unwrap();
    book.set_formula(0, at("B5"), "=B3/0").unwrap();
    book.set_formula(0, at("D1"), "=SEQUENCE(3)").unwrap();
    let mut bold = book.style(0, at("A3")).unwrap();
    bold.font.b = true;
    book.set_style(0, at("A3"), &bold).unwrap();
    book.set_style(0, at("C5"), &bold).unwrap();
    let mut file = AbacoFile::new(book);
    file.macros.insert("beam_report".into(), "import abaco\n\nprint(abaco.value(\"Loads!B3\"))\n".into());
    file.workspace.references.push(Reference {
        role: "loads_csv".into(),
        path: "loads.csv".into(),
        size: Some(4),
        modified: Some("2026-11-23T10:04:05Z".into()),
        sha256: Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into()),
    });
    let record = |text: &str| -> toml::Table { toml::from_str(text).unwrap() };
    file.workspace.decisions.push(record("tool = \"check\"\ncell = \"Loads!B3\"\nverdict = \"ok\""));
    file.workspace.queue.push(record("tool = \"check\"\ncell = \"Loads!B5\""));
    file.workspace.state.insert("check".into(), record("last = \"B3\"\nruns = 2"));
    file.workspace.log = vec!["opened Beam check".into(), "B5: #DIV/0!\tleft for the engineer".into()];
    file
}

/// The reference's TOML blocks, each by the part its first line names (`# manifest.toml`), that
/// line taken off.
fn reference_blocks() -> BTreeMap<String, String> {
    let doc = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/abaco-file.md")).unwrap();
    let mut out = BTreeMap::new();
    for block in doc.split("```toml\n").skip(1) {
        let body = block.split("```").next().unwrap();
        let (first, rest) = body.split_once('\n').unwrap();
        out.insert(first.trim_start_matches("# ").to_string(), rest.to_string());
    }
    out
}

#[test]
fn the_reference_s_worked_example_is_what_is_written_and_reads_back() {
    let mut file = worked_example();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("beam.abaco");
    abacofile::write(&mut file, &path).unwrap();
    let written = abacofile::parts(&file).unwrap();
    let names: Vec<&str> = written.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "manifest.toml",
            "workbook.toml",
            "sheets/01-Loads.toml",
            "macros/beam_report.py",
            "workspace/references.toml",
            "workspace/decisions.toml",
            "workspace/queue.toml",
            "workspace/state/check.toml",
            "workspace/log.txt"
        ]
    );
    let text = |name: &str| String::from_utf8(written.iter().find(|(n, _)| n == name).unwrap().1.clone()).unwrap();
    let blocks = reference_blocks();
    for name in ["manifest.toml", "sheets/01-Loads.toml", "workspace/references.toml", "workspace/decisions.toml", "workspace/state/check.toml"] {
        let block = blocks.get(name).map(|b| b.replace("<version>", env!("CARGO_PKG_VERSION")));
        assert_eq!(block.as_deref(), Some(text(name).as_str()), "{name} as written:\n{}", text(name));
    }
    assert_eq!(text("workspace/log.txt"), "opened Beam check\nB5: #DIV/0!\\tleft for the engineer\n");
    // read back: the same cells, the session whole
    let back = abacofile::read(&path).unwrap();
    assert!(differences(&file.book, &back.book).is_empty(), "{:?}", differences(&file.book, &back.book));
    assert_eq!(back.macros, file.macros);
    assert_eq!(back.workspace, file.workspace);
    assert!(back.kept_parts().is_empty());
    assert_eq!(back.book.origin, veredas_abaco::Origin::Abaco { path: path.clone() });
}

/// `text` with its first line opening with `start` replaced by `line`.
fn replace_line(text: &str, start: &str, line: &str) -> String {
    let at = text.lines().position(|l| l.starts_with(start)).unwrap_or_else(|| panic!("no line {start}"));
    text.lines().enumerate().map(|(i, l)| if i == at { line.to_string() } else { l.to_string() } + "\n").collect()
}

#[test]
fn a_file_that_is_not_what_it_should_be_is_refused_by_its_part_and_place() {
    let mut file = worked_example();
    // evaluated, as the writer writes it: each formula with its last value
    file.book.ensure_calculated();
    let good = abacofile::parts(&file).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.abaco");
    // the example's parts, one replaced (or dropped, `None`), read: the refusal's text
    let refusal = |name: &str, edit: &dyn Fn(&str) -> Option<String>| -> String {
        let parts: Vec<(String, Vec<u8>)> = good
            .iter()
            .filter_map(|(n, b)| {
                if n != name {
                    return Some((n.clone(), b.clone()));
                }
                edit(std::str::from_utf8(b).unwrap()).map(|t| (n.clone(), t.into_bytes()))
            })
            .collect();
        std::fs::write(&path, abacofile::zip_of(&parts).unwrap()).unwrap();
        match abacofile::read(&path) {
            Err(e) => e.to_string(),
            Ok(_) => format!("{name}: read, not refused"),
        }
    };
    let cases: Vec<(&str, Box<dyn Fn(&str) -> Option<String>>, &str)> = vec![
        ("manifest.toml", Box::new(|_| None), "no part `manifest.toml`"),
        ("manifest.toml", Box::new(|t| Some(t.replace("\"veredas-abaco\"", "\"veredas-abacus\""))), "manifest.toml: `format` is `veredas-abacus`, not `veredas-abaco`"),
        ("manifest.toml", Box::new(|t| Some(t.replace("version = 1", "version = 2"))), "manifest.toml: version 2 of `veredas-abaco` is not known here (this reader knows 1)"),
        ("manifest.toml", Box::new(|t| Some(t.replace("version = 1\n", ""))), "manifest.toml: no `version`: manifest.toml states `format` and `version`"),
        ("manifest.toml", Box::new(|t| Some(t.replace("written_by", "writen_by"))), "line 3, column 1"), ("manifest.toml", Box::new(|t| Some(t.replace("written_by", "writen_by"))), "unknown field `writen_by`"),
        ("manifest.toml", Box::new(|t| Some(t.replace("name = \"Loads\"", "name = \"Other\""))), "sheets/01-Loads.toml: the sheet `Loads`, where the manifest says `Other`"),
        ("workbook.toml", Box::new(|t| Some(t.replace("veredas-abaco-workbook", "veredas-abaco-sheet"))), "workbook.toml: `format` is `veredas-abaco-sheet`, not `veredas-abaco-workbook`"),
        // the format decided whole before the version, whatever the version holds (C-045)
        ("manifest.toml", Box::new(|t| Some(t.replace("\"veredas-abaco\"", "\"veredas-other\"").replace("version = 1", "version = 18446744073709551616"))), "manifest.toml: `format` is `veredas-other`, not `veredas-abaco`"),
        // a key the engine's structures do not read, at any depth, by its path
        ("workbook.toml", Box::new(|t| Some(t.replacen("tz = \"UTC\"", "tz = \"UTC\"\ntzz = \"UTC\"", 1))), "workbook.toml: `settings.tzz` is a key nothing reads"),
        ("workbook.toml", Box::new(|t| Some(t.replacen("sz = 12", "sz = 12\nszz = 12", 1))), "workbook.toml: `styles.fonts.0.szz` is a key nothing reads"),
        ("sheets/01-Loads.toml", Box::new(|_| None), "no part `sheets/01-Loads.toml`"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B1 = { n = 6.5 }", "B1 = { n = 6.5, s = \"x\" }"))), "sheets/01-Loads.toml B1: more than one of `n`, `s`, `b`, `e`, `f`, `array`, `dynamic`"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { b = true, v = 1 }"))), "sheets/01-Loads.toml B4: `v` without `f`"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { e = \"#NOPE!\" }"))), "sheets/01-Loads.toml B4: \"#NOPE!\" is no error"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { array = \"A1\" }"))), "sheets/01-Loads.toml B4: `size` goes with `array`, and only with it"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = {", "B4x = {"))), "sheets/01-Loads.toml B4x:"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { bool = true }"))), "sheets/01-Loads.toml B4: unknown field `bool`"),
        // TOML's own types, by the cell's address: a NaN is no number, and stands for nothing else
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B1 = { n = 6.5 }", "B1 = { n = nan }"))), "sheets/01-Loads.toml B1: NaN is no finite number"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B1 = { n = 6.5 }", "B1 = { n = inf }"))), "sheets/01-Loads.toml B1: inf is no finite number"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B1 = { n = 6.5 }", "B1 = { s = nan }"))), "sheets/01-Loads.toml B1: invalid type"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B1 = { n = 6.5 }", "B1 = { b = nan }"))), "sheets/01-Loads.toml B1: invalid type"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B1 = { n = 6.5 }", "B1 = { f = nan }"))), "sheets/01-Loads.toml B1: invalid type"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B1 = { n = 6.5 }", "B1 = { n = \"x\" }"))), "sheets/01-Loads.toml B1: invalid type"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B1 = { n = 6.5 }", "B1 = { n = 6.5, style = \"x\" }"))), "sheets/01-Loads.toml B1: invalid type"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B1 = { n = 6.5 }", "B1 = { n = 6.5, style = 999 }"))), "sheets/01-Loads.toml B1: style 999 is none of the workbook's 2"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B1 = { n = 6.5 }", "B1 = { n = 6.5, style = -3 }"))), "sheets/01-Loads.toml B1: style -3 is no style"),
        // one cell stated twice under two spellings: an address is written as Ábaco writes it
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B1 = { n = 6.5 }", "B1 = { n = 6.5 }\nb1 = { n = 7 }"))), "sheets/01-Loads.toml b1: an address is written `B1`"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B1 = { n = 6.5 }", "B1 = { n = 6.5 }\n\"$B$1\" = { n = 7 }"))), "sheets/01-Loads.toml $B$1: an address is written `B1`"),
        // a key nothing reads inside a link (an internally tagged enum, read from serde's buffer)
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("links = []", "links = [[1, 1, { type = \"External\", target = \"https://example.org\", tooltip = nan, targett = \"x\" }]]"))), "sheets/01-Loads.toml: `links.0.2.targett` is a key nothing reads"),
        ("manifest.toml", Box::new(|t| Some(t[..t.find("[[sheets]]").unwrap()].replace("macros = [\"beam_report\"]", "macros = [\"beam_report\"]\nsheets = []"))), "manifest.toml: a workbook holds a sheet at least"),
        // a formula's last value: one number, text or boolean, finite, never beside its error
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("v = 63.375", "v = nan"))), "sheets/01-Loads.toml B3: `v` is NaN, no finite number"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("v = 63.375", "v = inf"))), "sheets/01-Loads.toml B3: `v` is inf, no finite number"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("v = 63.375", "v = { zz = 1 }"))), "sheets/01-Loads.toml B3: `v` is a table, where a formula's last value is a number, a text or a boolean"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("v = 63.375", "v = [1, 2]"))), "sheets/01-Loads.toml B3: `v` is a array, where"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("v = 63.375", "v = 1979-05-27"))), "sheets/01-Loads.toml B3: `v` is the date-time 1979-05-27, which no cell holds"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("v = 63.375", "v = 2, e = \"#N/A\""))), "sheets/01-Loads.toml B3: both `v` and `e`: a formula's last value is one"),
        // an integer no double holds exactly, in a value and a formula's last value
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B1 = { n = 6.5 }", "B1 = { n = 9007199254740993 }"))), "sheets/01-Loads.toml B1: `n = 9007199254740993` is an integer no number of the file holds exactly"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("v = 63.375", "v = 123456789012345678"))), "sheets/01-Loads.toml B3: `v = 123456789012345678` is an integer no number of the file holds exactly"),
        // a row's and a column's style one of the workbook's
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("cols = []", "cols = [{ min = 1, max = 1, width = 10.0, custom_width = true, hidden = false, style = 999 }]"))), "sheets/01-Loads.toml: `cols.0.style` 999 is none of the workbook's 2 styles"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("cols = []", "cols = [{ min = 1, max = 1, width = 10.0, custom_width = true, hidden = false, style = -1 }]"))), "sheets/01-Loads.toml: `cols.0.style` -1 is no style"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("rows = []", "rows = [{ r = 1, height = 15.0, custom_format = true, custom_height = false, s = 999, hidden = false }]"))), "sheets/01-Loads.toml: `rows.0.s` 999 is none of the workbook's 2 styles"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("rows = []", "rows = [{ r = 1, height = 15.0, custom_format = true, custom_height = false, s = -2, hidden = false }]"))), "sheets/01-Loads.toml: `rows.0.s` -2 is no style"),
        // a date-time where the engine holds a text, by its path
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("dimension = \"A1\"", "dimension = 1979-05-27"))), "sheets/01-Loads.toml: `dimension` is the date-time 1979-05-27, which nothing of the engine holds"),
        ("workbook.toml", Box::new(|t| Some(replace_line(t, "created = ", "created = 1979-05-27T07:32:00Z"))), "workbook.toml: `metadata.created` is the date-time 1979-05-27T07:32:00Z, which nothing of the engine holds"),
        // what the engine refuses of the settings, by the part
        ("workbook.toml", Box::new(|t| Some(t.replacen("tz = \"UTC\"", "tz = \"Mars/Olympus\"", 1))), "workbook.toml: `settings`: "),
        ("workbook.toml", Box::new(|t| Some(replace_line(t, "locale = ", "locale = \"xx-nowhere\""))), "workbook.toml: `settings`: Invalid locale"),
        ("workspace/references.toml", Box::new(|t| Some(t.replace("ba7816bf8f01cfea", "BA7816BF8F01CFEA"))), "workspace/references.toml reference 1: `sha256` is 64 lowercase hex digits"),
        ("workspace/references.toml", Box::new(|t| Some(t.replace("2026-11-23T10:04:05Z", "2026-11-23 10:04"))), "workspace/references.toml reference 1: `modified` is RFC 3339"),
        ("workspace/decisions.toml", Box::new(|t| Some(t.replace("veredas-abaco-records", "veredas-abaco-state"))), "workspace/decisions.toml: `format` is `veredas-abaco-state`, not `veredas-abaco-records`"),
        ("workspace/log.txt", Box::new(|_| Some("fine\nbad \\x escape\n".into())), "workspace/log.txt line 2: `\\x` is no escape"),
        ("macros/beam_report.py", Box::new(|_| None), "no part `macros/beam_report.py`"),
    ];
    for (name, edit, want) in &cases {
        let got = refusal(name, edit.as_ref());
        assert!(got.contains(want), "{name}: {got}\n  wanted: {want}");
        assert!(got.starts_with(&path.display().to_string()), "{name}: the file is named: {got}");
    }
    // a part that is no UTF-8, and a file that is no zip
    let mut parts = good.clone();
    parts.iter_mut().find(|(n, _)| n == "workspace/log.txt").unwrap().1 = vec![0xff, 0xfe];
    std::fs::write(&path, abacofile::zip_of(&parts).unwrap()).unwrap();
    assert!(abacofile::read(&path).err().unwrap().to_string().contains("workspace/log.txt: not UTF-8"));
    std::fs::write(&path, "not a zip").unwrap();
    assert!(abacofile::read(&path).err().unwrap().to_string().contains("not a .abaco file (a zip)"));
}

#[test]
fn a_part_it_does_not_read_is_kept_and_written_back_and_the_same_file_writes_the_same_bytes() {
    let mut file = worked_example();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("beam.abaco");
    let mut parts = abacofile::parts(&file).unwrap();
    parts.push(("extras/notes.md".into(), b"# a newer Abaco's part\n".to_vec()));
    std::fs::write(&path, abacofile::zip_of(&parts).unwrap()).unwrap();
    let mut back = abacofile::read(&path).unwrap();
    assert_eq!(back.kept_parts().keys().collect::<Vec<_>>(), ["extras/notes.md"]);
    let again = dir.path().join("again.abaco");
    abacofile::write(&mut back, &again).unwrap();
    let names: Vec<String> = abacofile::parts(&back).unwrap().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names.last().map(String::as_str), Some("extras/notes.md"));
    // the part as it came, byte for byte, in the file written back
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&again).unwrap()).unwrap();
    let mut kept = Vec::new();
    std::io::Read::read_to_end(&mut zip.by_name("extras/notes.md").unwrap(), &mut kept).unwrap();
    assert_eq!(kept, b"# a newer Abaco's part\n");
    assert_eq!(abacofile::read(&again).unwrap().kept_parts()["extras/notes.md"], b"# a newer Abaco's part\n");
    // written twice: the same bytes
    abacofile::write(&mut file, &path).unwrap();
    let first = std::fs::read(&path).unwrap();
    abacofile::write(&mut file, &path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), first);
    // over an existing file, through a file beside it: the new one in place, the one it replaced
    // kept as `.bak`, nothing left beside them
    file.book.set_value(0, Addr::parse("B2").unwrap(), &Value::Number(13.0)).unwrap();
    abacofile::write(&mut file, &path).unwrap();
    let second = std::fs::read(&path).unwrap();
    assert_ne!(second, first);
    assert_eq!(std::fs::read(dir.path().join("beam.abaco.bak")).unwrap(), first);
    assert_eq!(abacofile::read(&path).unwrap().book.value_now(0, Addr::parse("B2").unwrap()).unwrap(), Value::Number(13.0));
    let left: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    let mut left = left;
    left.sort();
    assert_eq!(left, ["again.abaco", "beam.abaco", "beam.abaco.bak"]);
}

#[test]
fn every_error_the_engine_holds_reads_back_and_a_format_s_rule_refuses_a_key_nothing_reads() {
    use ironcalc_base::expressions::token::Error as E;
    let mut book = Workbook::new("Errors").unwrap();
    let errors = [E::REF, E::NAME, E::VALUE, E::DIV, E::NA, E::NUM, E::ERROR, E::NIMPL, E::SPILL, E::CALC, E::CIRC, E::NULL];
    for (i, e) in errors.iter().enumerate() {
        book.model_mut().workbook.worksheet_mut(0).unwrap().set_cell_with_error(i as i32 + 1, 1, e.clone(), 0).unwrap();
    }
    // a formula whose value is an error the engine spells `#N/IMPL` (a function it lacks)
    book.set_value(0, Addr::parse("C1").unwrap(), &Value::Number(1.0)).unwrap();
    book.set_formula(0, Addr::parse("B1").unwrap(), "=FORECAST.ETS(4,C1:C3,D1:D3)").unwrap();
    book.mark_stale();
    let mut file = AbacoFile::new(book);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("errors.abaco");
    abacofile::write(&mut file, &path).unwrap();
    assert!(String::from_utf8(abacofile::parts(&file).unwrap()[2].1.clone()).unwrap().contains(r##"e = "#N/IMPL""##));
    let back = abacofile::read(&path).unwrap();
    assert!(differences(&file.book, &back.book).is_empty(), "{:?}", differences(&file.book, &back.book));
    for (i, e) in errors.iter().enumerate() {
        assert_eq!(back.book.value_now(0, Addr::new(i as i32 + 1, 1)).unwrap(), Value::Error(e.to_string()));
    }
    // a conditional format's rule (an internally tagged enum): a key nothing reads, by its path
    let src = corpus_dir().join("conditional_formatting/cf_tests.xlsx");
    let mut file = AbacoFile::new(open(&src));
    let good = abacofile::parts(&file).unwrap();
    abacofile::write(&mut file, &path).unwrap();
    let parts: Vec<(String, Vec<u8>)> = good
        .into_iter()
        .map(|(n, b)| {
            if n == "sheets/01-Sheet1.toml" {
                let t = String::from_utf8(b).unwrap();
                assert!(t.contains("stop_if_true = false\ntype = \"CellIs\""));
                (n, t.replacen("stop_if_true = false\ntype = \"CellIs\"", "stop_if_true = false\nstop_if_ture = true\ntype = \"CellIs\"", 1).into_bytes())
            } else {
                (n, b)
            }
        })
        .collect();
    std::fs::write(&path, abacofile::zip_of(&parts).unwrap()).unwrap();
    let got = abacofile::read(&path).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(got.contains("sheets/01-Sheet1.toml: `conditional_formatting.0.cf_rule.stop_if_ture` is a key nothing reads"), "{got}");
    // and in a struct below one (a colour scale's and an icon set's thresholds)
    let good = abacofile::parts(&file).unwrap();
    let mut probed = 0;
    for (name, bytes) in &good {
        let text = String::from_utf8(bytes.clone()).unwrap();
        let header = "[[conditional_formatting.cf_rule.thresholds]]\n";
        if !text.contains(header) {
            continue;
        }
        probed += 1;
        let parts: Vec<(String, Vec<u8>)> = good
            .iter()
            .map(|(n, b)| if n == name { (n.clone(), text.replacen(header, &format!("{header}colr = \"#FF0000\"\n"), 1).into_bytes()) } else { (n.clone(), b.clone()) })
            .collect();
        std::fs::write(&path, abacofile::zip_of(&parts).unwrap()).unwrap();
        let got = abacofile::read(&path).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(got.contains(&format!("{name}: `conditional_formatting.")) && got.contains(".cf_rule.thresholds.0.colr` is a key nothing reads"), "{name}: {got}");
    }
    assert!(probed >= 2, "{probed} sheets with thresholds");
}

#[test]
fn a_text_with_a_line_break_keeps_its_cell_on_one_line_and_reads_back() {
    let mut book = Workbook::new("Lines").unwrap();
    let at = |a: &str| Addr::parse(a).unwrap();
    let texts = [
        "line one\nline two",
        "a\r\nb",
        "tab\there",
        "quote \" and back\\slash",
        "bell \u{7} del \u{7f} nul-free",
        "\u{1}\u{1f}",
        "next \u{85} line",
        "line \u{2028} and \u{2029} paragraph",
        "csi \u{9b} c1",
    ];
    for (i, t) in texts.iter().enumerate() {
        book.set_value(0, Addr::new(i as i32 + 1, 1), &Value::Text(t.to_string())).unwrap();
    }
    book.set_formula(0, at("B1"), "=CHAR(10)&\"x\"").unwrap();
    book.set_formula(0, at("B2"), "=\"a\"&CHAR(10)").unwrap();
    let mut file = AbacoFile::new(book);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lines.abaco");
    abacofile::write(&mut file, &path).unwrap();
    let sheet = String::from_utf8(abacofile::parts(&file).unwrap()[2].1.clone()).unwrap();
    let cells = sheet.split_once("\n[cells]\n").unwrap().1;
    assert_eq!(cells.lines().count(), texts.len() + 2, "{cells}");
    assert_eq!(unicode_lines(cells), texts.len() + 2, "{cells}");
    assert!(cells.contains(r#"A1 = { s = "line one\nline two" }"#), "{cells}");
    assert!(cells.contains(r#"B1 = { f = "CHAR(10)&\"x\"", v = "\nx" }"#), "{cells}");
    assert!(cells.contains(r#"A5 = { s = "bell \u0007 del \u007F nul-free" }"#), "{cells}");
    let back = abacofile::read(&path).unwrap();
    assert!(differences(&file.book, &back.book).is_empty(), "{:?}", differences(&file.book, &back.book));
    for (i, t) in texts.iter().enumerate() {
        assert_eq!(back.book.value_now(0, Addr::new(i as i32 + 1, 1)).unwrap(), Value::Text(t.to_string()));
    }
    assert_eq!(back.book.value_now(0, at("B1")).unwrap(), Value::Text("\nx".into()));
}

#[test]
fn names_the_engine_refuses_are_refused_by_the_writer_and_the_reader() {
    // two sheets and two names, written; then each state the engine refuses, made in the model
    // (the writer refuses it) and in the file's text (the reader refuses it)
    let make = || {
        let mut book = Workbook::new("Names").unwrap();
        book.rename_sheet("Sheet1", "Loads").unwrap();
        book.add_sheet("Other").unwrap();
        book.set_name("Span", "=Loads!$B$1", None).unwrap();
        book.set_name("Spam", "=Loads!$B$2", None).unwrap();
        AbacoFile::new(book)
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("names.abaco");
    let good = abacofile::parts(&make()).unwrap();
    let cases: Vec<(Box<dyn Fn(&mut ironcalc_base::types::Workbook)>, &str, &str, &str, &str)> = vec![
        (Box::new(|w| w.worksheets[1].name = "LOADS".into()), "manifest.toml", "name = \"Other\"", "name = \"LOADS\"", "manifest.toml: sheet 2 is named `LOADS` as sheet 1 is"),
        (Box::new(|w| w.worksheets[1].name = "a/b:[c]".into()), "manifest.toml", "name = \"Other\"", "name = \"a/b:[c]\"", "manifest.toml: sheet 2 `a/b:[c]` is no sheet name"),
        (Box::new(|w| w.worksheets[0].name = String::new()), "manifest.toml", "name = \"Loads\"", "name = \"\"", "manifest.toml: sheet 1 `` is no sheet name"),
        (Box::new(|w| w.worksheets[1].sheet_id = w.worksheets[0].sheet_id), "sheets/02-Other.toml", "sheet_id = 2", "sheet_id = 1", "sheets/02-Other.toml: `sheet_id` 1 is sheet 1's too"),
        (Box::new(|w| w.defined_names[0].name = "B1".into()), "workbook.toml", "name = \"Spam\"", "name = \"B1\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "$B$1".into()), "workbook.toml", "name = \"Spam\"", "name = \"$B$1\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "B$1".into()), "workbook.toml", "name = \"Spam\"", "name = \"B$1\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "R[1]C[1]".into()), "workbook.toml", "name = \"Spam\"", "name = \"R[1]C[1]\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "RC[2]".into()), "workbook.toml", "name = \"Spam\"", "name = \"RC[2]\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[1].name = "SPAN".into()), "workbook.toml", "name = \"Spam\"", "name = \"SPAN\"", "is stated in its scope already, as `defined_names."),
    ];
    for (in_model, part, from, to, want) in &cases {
        // the writer
        let mut file = make();
        in_model(&mut file.book.model_mut().workbook);
        let got = abacofile::write(&mut file, &path).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(got.contains(want), "writing: {got}\n  wanted: {want}");
        assert!(!path.exists());
        // the reader (a sheet's name stands in the manifest and in its part)
        let parts: Vec<(String, Vec<u8>)> = good
            .iter()
            .map(|(n, b)| {
                let t = String::from_utf8(b.clone()).unwrap();
                let edited = n == part || (*part == "manifest.toml" && n.starts_with("sheets/") && t.contains(from));
                (n.clone(), if edited { t.replacen(from, to, 1).into_bytes() } else { b.clone() })
            })
            .collect();
        std::fs::write(&path, abacofile::zip_of(&parts).unwrap()).unwrap();
        let got = abacofile::read(&path).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(got.contains(want), "reading: {got}\n  wanted: {want}");
        std::fs::remove_file(&path).unwrap();
    }
}

#[test]
fn a_number_no_file_holds_refuses_the_write_by_its_path() {
    // a NaN or an infinity a malformed workbook gives the engine (a column's width, a colour's
    // tint, a row's height): JSON would write it as the engine's none, which the reader refuses
    use ironcalc_base::cf_types::{CfRule, Cfvo, ColorScaleThreshold, ConditionalFormatting, Icon};
    use ironcalc_base::types::{Col, Color, Row};
    let format = |cf_rule: CfRule| ConditionalFormatting { range: "A1:A3".into(), cf_rule, priority: 1 };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nan.abaco");
    let cases: Vec<(Box<dyn Fn(&mut ironcalc_base::types::Worksheet)>, &str)> = vec![
        (
            Box::new(|w| {
                w.cols.push(Col { min: 1, max: 1, width: f64::NAN, custom_width: true, hidden: false, style: None, outline_level: 0, collapsed: false })
            }),
            "sheets/01-Sheet1.toml: `cols.0.width` is a number that is no finite number",
        ),
        (Box::new(|w| w.color = Color::Theme(1, f64::NAN)), "sheets/01-Sheet1.toml: `color.1` is a number that is no finite number"),
        (
            Box::new(|w| w.rows.push(Row { r: 1, height: f64::INFINITY, custom_format: false, custom_height: true, s: 0, hidden: false })),
            "sheets/01-Sheet1.toml: `rows.0.height` is a number that is no finite number",
        ),
        // inside a conditional format's rule (an internally tagged enum), beside nones of its own
        (
            Box::new(|w| {
                w.conditional_formatting.push(format(CfRule::DataBar {
                    min: Some(Cfvo::Number(f64::NAN)),
                    max: None,
                    positive_color: Color::None,
                    negative_color: Color::None,
                    is_gradient: true,
                    show_value: true,
                }))
            }),
            "sheets/01-Sheet1.toml: `conditional_formatting.0.cf_rule.min.Number` is a number",
        ),
        (
            Box::new(|w| {
                w.conditional_formatting.push(format(CfRule::DataBar {
                    min: None,
                    max: None,
                    positive_color: Color::Theme(1, f64::NAN),
                    negative_color: Color::None,
                    is_gradient: true,
                    show_value: true,
                }))
            }),
            "sheets/01-Sheet1.toml: `conditional_formatting.0.cf_rule.positive_color.1` is a number",
        ),
        (
            Box::new(|w| {
                w.conditional_formatting.push(format(CfRule::ColorScale {
                    thresholds: vec![
                        ColorScaleThreshold { cfvo: Cfvo::Min, color: Color::None },
                        ColorScaleThreshold { cfvo: Cfvo::Number(f64::INFINITY), color: Color::Rgb("FF0000".into()) },
                    ],
                }))
            }),
            "sheets/01-Sheet1.toml: `conditional_formatting.0.cf_rule.thresholds.1.cfvo.Number` is a number",
        ),
        (
            Box::new(|w| {
                w.conditional_formatting.push(format(CfRule::IconRating {
                    icon: Icon::Star,
                    color: Color::None,
                    thresholds: vec![(Cfvo::Percent(f64::NAN), false)],
                    show_value: true,
                }))
            }),
            "sheets/01-Sheet1.toml: `conditional_formatting.0.cf_rule.thresholds.0.0.Percent` is a number",
        ),
    ];
    for (bad, want) in &cases {
        let mut file = AbacoFile::new(Workbook::new("Nan").unwrap());
        bad(&mut file.book.model_mut().workbook.worksheets[0]);
        let got = abacofile::write(&mut file, &path).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(got.contains(want), "got: {got}\n  wanted: {want}");
        assert!(!path.exists());
    }
}

#[test]
fn what_the_reader_refuses_the_writer_refuses_by_its_place() {
    use ironcalc_base::expressions::types::Area;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("refused.abaco");
    let refused = |file: &mut AbacoFile| abacofile::write(file, &path).err().map(|e| e.to_string()).unwrap_or_default();
    // a one-cell legacy array over merged cells, which the engine's own merge lets stand
    let mut book = Workbook::new("Arrays").unwrap();
    book.model_mut().set_user_array_formula(0, 1, 1, 1, 1, "=SUM(C1:C2*D1:D2)").unwrap();
    book.model_mut().merge_cells(&Area { sheet: 0, row: 1, column: 1, width: 2, height: 1 }).unwrap();
    let got = refused(&mut AbacoFile::new(book));
    assert!(got.contains("sheets/01-Sheet1.toml A1: an array formula over merged cells"), "{got}");
    // a cell's style the workbook does not hold
    for (style, want) in [(99, "style 99 is none of the workbook's"), (-1, "style -1 is no style")] {
        let mut book = Workbook::new("Styles").unwrap();
        book.model_mut().workbook.worksheet_mut(0).unwrap().set_cell_with_number(1, 1, 5.0, style).unwrap();
        let got = refused(&mut AbacoFile::new(book));
        assert!(got.contains(&format!("sheets/01-Sheet1.toml A1: {want}")), "{got}");
    }
    // a reference not in its form
    for (bad, want) in [
        (Reference { role: "r".into(), path: "r.csv".into(), size: None, modified: None, sha256: Some("XYZ".into()) }, "`sha256` is 64 lowercase hex digits"),
        (Reference { role: "r".into(), path: "r.csv".into(), size: None, modified: Some("2026-11-23T10:04:05+00:00".into()), sha256: None }, "`modified` is RFC 3339"),
    ] {
        let mut file = worked_example();
        file.workspace.references = vec![bad];
        let got = refused(&mut file);
        assert!(got.contains(&format!("workspace/references.toml reference 1: {want}")), "{got}");
    }
    assert!(!path.exists());
}

#[test]
fn a_link_or_a_linked_cell_stated_twice_is_refused_by_its_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("twice.abaco");
    let read_with = |parts: &[(String, Vec<u8>)], name: &str, edit: &dyn Fn(&str) -> String| {
        let edited: Vec<(String, Vec<u8>)> =
            parts.iter().map(|(n, b)| if n == name { (n.clone(), edit(std::str::from_utf8(b).unwrap()).into_bytes()) } else { (n.clone(), b.clone()) }).collect();
        std::fs::write(&path, abacofile::zip_of(&edited).unwrap()).unwrap();
        abacofile::read(&path).err().map(|e| e.to_string()).unwrap_or_default()
    };
    let parts = abacofile::parts(&worked_example()).unwrap();
    let link = |t: &str| format!(r#"[1, 1, {{ type = "External", target = "https://example.org/{t}", tooltip = nan }}]"#);
    let got = read_with(&parts, "sheets/01-Loads.toml", &|t| t.replace("links = []", &format!("links = [{}, {}]", link("a"), link("b"))));
    assert!(got.contains("sheets/01-Loads.toml: `links.1` states the link of a cell `links.0` states already"), "{got}");
    // a linked workbook's cached cell, in the workbook part
    let linked = AbacoFile::new(open(&corpus_dir().join("link_test.xlsx")));
    let parts = abacofile::parts(&linked).unwrap();
    let got = read_with(&parts, "workbook.toml", &|t| {
        assert!(t.contains("cells = []"), "{t}");
        t.replacen("cells = []", "cells = [[1, 1, { Number = 1.0 }], [1, 1, { Number = 2.0 }]]", 1)
    });
    assert!(got.contains("workbook.toml: `external_books.0.sheets.0.cells.1` states a cell its sheet states already"), "{got}");
}

#[test]
#[cfg(unix)]
fn a_file_name_is_the_system_s_whatever_its_encoding_and_no_link_beside_it_is_followed() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    // two names that one lossy spelling would make one: each keeps its own `.bak`
    let mut file = worked_example();
    let names: Vec<std::path::PathBuf> = [b"caf\xe9.abaco".as_slice(), b"caf\xe8.abaco".as_slice()]
        .iter()
        .map(|n| dir.path().join(std::ffi::OsStr::from_bytes(n)))
        .collect();
    for (i, name) in names.iter().enumerate() {
        file.book.set_value(0, Addr::parse("B2").unwrap(), &Value::Number(i as f64)).unwrap();
        abacofile::write(&mut file, name).unwrap();
        file.book.set_value(0, Addr::parse("B2").unwrap(), &Value::Number(10.0 + i as f64)).unwrap();
        abacofile::write(&mut file, name).unwrap();
    }
    for (i, name) in names.iter().enumerate() {
        let mut bak = name.as_os_str().to_os_string();
        bak.push(".bak");
        let first = abacofile::read(std::path::Path::new(&bak)).unwrap();
        assert_eq!(first.book.value_now(0, Addr::parse("B2").unwrap()).unwrap(), Value::Number(i as f64));
    }
    // a link where a temporary file of the old fixed name stood: never followed
    let victim = dir.path().join("victim.txt");
    std::fs::write(&victim, "untouched").unwrap();
    let dest = dir.path().join("b.abaco");
    std::os::unix::fs::symlink(&victim, dir.path().join("b.abaco.tmp")).unwrap();
    abacofile::write(&mut file, &dest).unwrap();
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "untouched");
    assert!(std::fs::symlink_metadata(&dest).unwrap().file_type().is_file());
    let left: BTreeMap<String, ()> =
        std::fs::read_dir(dir.path()).unwrap().map(|e| (e.unwrap().file_name().to_string_lossy().to_string(), ())).collect();
    assert!(!left.keys().any(|n| n.ends_with(".tmp") && n != "b.abaco.tmp"), "{left:?}");
}

#[test]
fn a_write_that_fails_leaves_the_destination_and_nothing_beside_it() {
    let mut file = worked_example();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.abaco");
    abacofile::write(&mut file, &path).unwrap();
    let before = std::fs::read(&path).unwrap();
    // a `.bak` that cannot be replaced (a folder)
    std::fs::create_dir(dir.path().join("b.abaco.bak")).unwrap();
    std::fs::write(dir.path().join("b.abaco.bak/inside"), "x").unwrap();
    file.book.set_value(0, Addr::parse("B2").unwrap(), &Value::Number(13.0)).unwrap();
    assert!(abacofile::write(&mut file, &path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut left: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    left.sort();
    assert_eq!(left, ["b.abaco", "b.abaco.bak"]);
}
