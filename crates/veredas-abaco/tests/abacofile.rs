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
        // a misspelt key replacing the field it meant, and one under an optional field: named by
        // the file's own path (refute round 11)
        ("workbook.toml", Box::new(|t| Some(t.replacen("tz = \"UTC\"", "tzz = \"UTC\"", 1))), "workbook.toml: `settings.tzz` is a key nothing reads"),
        ("workbook.toml", Box::new(|t| Some(t.replacen("dxfs = []", "dxfs = [{ font = { szz = 1 } }]", 1))), "workbook.toml: `styles.dxfs.0.font.szz` is a key nothing reads"),
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
        // a key replacing a tagged object's field, by its path (refute round 12)
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("links = []", "links = [[1, 1, { type = \"External\", targett = \"https://example.org\" }]]"))), "sheets/01-Loads.toml: `links.0.2.targett` is a key nothing reads"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("links = []", "links = [[1, 1, { type = \"Internal\", locationn = \"Loads!A1\" }]]"))), "sheets/01-Loads.toml: `links.0.2.locationn` is a key nothing reads"),
        // two links each replacing their field, and a link's tag replaced (refute round 13)
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("links = []", "links = [[1, 1, { type = \"External\", targett = \"a\" }], [2, 1, { type = \"External\", targett = \"b\" }]]"))), "sheets/01-Loads.toml: `links.0.2.targett` is a key nothing reads"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("links = []", "links = [[1, 1, { typ = \"External\", target = \"https://example.org\" }]]"))), "sheets/01-Loads.toml: `links.0.2.typ` is a key nothing reads"),
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
        // a formula of white space alone, which the writer refuses too
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { array = \"\", size = [1, 1] }"))), "sheets/01-Loads.toml B4: an empty formula"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { array = \" \", size = [1, 1] }"))), "sheets/01-Loads.toml B4: an empty formula"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { f = \" \" }"))), "sheets/01-Loads.toml B4: an empty formula"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { f = \"\" }"))), "sheets/01-Loads.toml B4: an empty formula"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { dynamic = \"\\t\" }"))), "sheets/01-Loads.toml B4: an empty formula"),
        // a formula the engine cannot read as stated, which it would enter as its error or repaired
        // (`SUM(B1:B2` as `SUM(B1:B2)`): refused by its address, as the writer refuses one (round 9)
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { f = \"1+\" }"))), "sheets/01-Loads.toml B4: a formula the engine cannot read"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { f = \"=B1\" }"))), "sheets/01-Loads.toml B4: a formula the engine cannot read"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { f = \"B1)\" }"))), "sheets/01-Loads.toml B4: a formula the engine cannot read"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { f = \"{1,2\" }"))), "sheets/01-Loads.toml B4: a formula the engine cannot read"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { f = \"SUM(B1:B2\" }"))), "sheets/01-Loads.toml B4: a formula the engine cannot read"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { f = \"SUM(B1,,\" }"))), "sheets/01-Loads.toml B4: a formula the engine cannot read"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { array = \"1+\", size = [1, 1] }"))), "sheets/01-Loads.toml B4: a formula the engine cannot read"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(t.replace("B4 = { b = true }", "B4 = { dynamic = \"SEQUENCE(3\" }"))), "sheets/01-Loads.toml B4: a formula the engine cannot read"),
        // a cell stated inside a legacy array's area, in either order of the keys
        ("sheets/01-Loads.toml", Box::new(|t| Some(format!("{t}E1 = {{ array = \"1\", size = [1, 3] }}\nE2 = {{ n = 5 }}\n"))), "sheets/01-Loads.toml E2: a cell stated inside the array formula at E1"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(format!("{t}G2 = {{ array = \"7\", size = [1, 9] }}\nG10 = {{ array = \"7\", size = [1, 1] }}\n"))), "sheets/01-Loads.toml G10: an array formula over the one at G2"),
        // two legacy arrays whose areas overlap, neither anchor in the other's area
        ("sheets/01-Loads.toml", Box::new(|t| Some(format!("{t}B10 = {{ array = \"1\", size = [1, 2] }}\nA11 = {{ array = \"5\", size = [2, 1] }}\n"))), "sheets/01-Loads.toml A11: an array formula over the one at B10"),
        // a legacy array past the sheet's edge, refused before any sum is taken (no overflow)
        ("sheets/01-Loads.toml", Box::new(|t| Some(format!("{t}E1 = {{ array = \"1\", size = [1, 2147483647] }}\n"))), "sheets/01-Loads.toml E1: an array formula of 1 by 2147483647, which runs past the sheet's edge"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(format!("{t}E1 = {{ array = \"1\", size = [2147483647, 1] }}\n"))), "sheets/01-Loads.toml E1: an array formula of 2147483647 by 1, which runs past the sheet's edge"),
        ("sheets/01-Loads.toml", Box::new(|t| Some(format!("{t}E2 = {{ array = \"1\", size = [1, 1048576] }}\n"))), "sheets/01-Loads.toml E2: an array formula of 1 by 1048576, which runs past the sheet's edge"),
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
    // and replacing the field it meant, which the rule then misses (refute round 12); a field no
    // kind of value fills, a field below the rule, the tag itself (refute round 13)
    for (from, to, want) in [
        ("operator = ", "operatorr = ", ".cf_rule.operatorr` is a key nothing reads"),
        ("cfvo = ", "cfvoo = ", ".cfvoo` is a key nothing reads"),
        ("type = \"CellIs\"", "typ = \"CellIs\"", ".cf_rule.typ` is a key nothing reads"),
    ] {
        let parts: Vec<(String, Vec<u8>)> = abacofile::parts(&file)
            .unwrap()
            .into_iter()
            .map(|(n, b)| {
                let t = String::from_utf8(b).unwrap();
                if n.starts_with("sheets/") && t.contains(from) { (n, t.replacen(from, to, 1).into_bytes()) } else { (n, t.into_bytes()) }
            })
            .collect();
        std::fs::write(&path, abacofile::zip_of(&parts).unwrap()).unwrap();
        let got = abacofile::read(&path).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(got.contains(want), "{to}: {got}");
    }
    // and replacing the field it meant, which the rule then misses (refute round 12)
    let parts: Vec<(String, Vec<u8>)> = abacofile::parts(&file)
        .unwrap()
        .into_iter()
        .map(|(n, b)| {
            if n == "sheets/01-Sheet1.toml" {
                let t = String::from_utf8(b).unwrap();
                (n, t.replacen("stop_if_true = false\ntype = \"CellIs\"", "stop_if_ture = false\ntype = \"CellIs\"", 1).into_bytes())
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
        (Box::new(|w| w.defined_names[0].name = "$b$1".into()), "workbook.toml", "name = \"Spam\"", "name = \"$b$1\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "b$1".into()), "workbook.toml", "name = \"Spam\"", "name = \"b$1\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "$b1".into()), "workbook.toml", "name = \"Spam\"", "name = \"$b1\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "r[1]c[1]".into()), "workbook.toml", "name = \"Spam\"", "name = \"r[1]c[1]\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "r[-1]c".into()), "workbook.toml", "name = \"Spam\"", "name = \"r[-1]c\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "Sheet1!B1".into()), "workbook.toml", "name = \"Spam\"", "name = \"Sheet1!B1\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "A1:B2".into()), "workbook.toml", "name = \"Spam\"", "name = \"A1:B2\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "A:A".into()), "workbook.toml", "name = \"Spam\"", "name = \"A:A\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "1:1".into()), "workbook.toml", "name = \"Spam\"", "name = \"1:1\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = " B1".into()), "workbook.toml", "name = \"Spam\"", "name = \" B1\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[1].name = "SPAN".into()), "workbook.toml", "name = \"Spam\"", "name = \"SPAN\"", "is stated in its scope already, as `defined_names."),
        // a name whose Unicode capitals read as a cell (the engine's lexer capitalises so: `ß1` is SS1)
        (Box::new(|w| w.defined_names[0].name = "ß1".into()), "workbook.toml", "name = \"Spam\"", "name = \"ß1\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "ı1".into()), "workbook.toml", "name = \"Spam\"", "name = \"ı1\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "ſ1".into()), "workbook.toml", "name = \"Spam\"", "name = \"ſ1\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "ﬀ1".into()), "workbook.toml", "name = \"Spam\"", "name = \"ﬀ1\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "ﬁ1".into()), "workbook.toml", "name = \"Spam\"", "name = \"ﬁ1\"", "is no name (it reads as a cell)"),
        (Box::new(|w| w.defined_names[0].name = "ﬃ1".into()), "workbook.toml", "name = \"Spam\"", "name = \"ﬃ1\"", "is no name (it reads as a cell)"),
        // a name opening with an operator or holding one, which a formula reads as the cell
        (Box::new(|w| w.defined_names[0].name = "@B1".into()), "workbook.toml", "name = \"Spam\"", "name = \"@B1\"", "is no name (a letter, `_`, `\\` or a character beyond ASCII first"),
        (Box::new(|w| w.defined_names[0].name = "(B1)".into()), "workbook.toml", "name = \"Spam\"", "name = \"(B1)\"", "is no name (a letter, `_`, `\\` or a character beyond ASCII first"),
        (Box::new(|w| w.defined_names[0].name = "+B1".into()), "workbook.toml", "name = \"Spam\"", "name = \"+B1\"", "is no name (a letter, `_`, `\\` or a character beyond ASCII first"),
        (Box::new(|w| w.defined_names[0].name = "B1+0".into()), "workbook.toml", "name = \"Spam\"", "name = \"B1+0\"", "is no name (a letter, `_`, `\\` or a character beyond ASCII first"),
        (Box::new(|w| w.defined_names[0].name = "true".into()), "workbook.toml", "name = \"Spam\"", "name = \"true\"", "is no name (a letter, `_`, `\\` or a character beyond ASCII first"),
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
    // and the names Excel spells, in any script, a backslash first, a built-in one, are written and
    // read back
    for name in ["Área", "Ação.Total", "\\Loads", "_xlnm.Print_Area", "B1_load"] {
        let mut file = make();
        file.book.model_mut().workbook.defined_names[0].name = name.into();
        abacofile::write(&mut file, &path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let back = abacofile::read(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(back.book.model().workbook.defined_names.iter().any(|d| d.name == name), "{name}");
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

/// `book` exported to `path` with its part `part` passed through `edit`, which must change it: an
/// `.xlsx` no engine writes, as a malformed file would hold it.
fn patched_xlsx(book: &mut Workbook, path: &Path, part: &str, edit: impl Fn(&str) -> String) {
    book.export(path).unwrap();
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let name = entry.name().unwrap().to_string();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
        if name == part {
            let text = String::from_utf8(bytes).unwrap();
            let edited = edit(&text);
            assert_ne!(edited, text, "{part} unchanged");
            bytes = edited.into_bytes();
        }
        parts.push((name, bytes));
    }
    std::fs::write(path, abacofile::zip_of(&parts).unwrap()).unwrap();
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
    // a legacy array of no size, which the engine's own API enters
    for (w, h) in [(0, 1), (1, 0), (-1, 2)] {
        let mut book = Workbook::new("Arrays").unwrap();
        book.model_mut().set_user_array_formula(0, 1, 1, w, h, "=1").unwrap();
        let got = refused(&mut AbacoFile::new(book));
        assert!(got.contains(&format!("sheets/01-Sheet1.toml A1: an array formula of {w} by {h}")), "{got}");
    }
    // an empty formula, from a malformed `.xlsx` (`<f></f>`)
    let xlsx = dir.path().join("empty.xlsx");
    let mut book = Workbook::new("Empty").unwrap();
    book.set_formula(0, Addr::parse("A1").unwrap(), "=1+1").unwrap();
    patched_xlsx(&mut book, &xlsx, "xl/worksheets/sheet1.xml", |t| t.replace("<f>1+1</f>", "<f></f>"));
    let got = refused(&mut AbacoFile::new(open(&xlsx)));
    assert!(got.contains("sheets/01-Sheet1.toml A1: an empty formula"), "{got}");
    // a formula the engine cannot read (`<f>SUM(A1:A3</f>`), which it would read back repaired
    let xlsx = dir.path().join("unclosed.xlsx");
    let mut book = Workbook::new("Unclosed").unwrap();
    for r in 1..=3 {
        book.set_value(0, Addr::new(r, 1), &Value::Number(f64::from(r))).unwrap();
    }
    book.set_formula(0, Addr::parse("B1").unwrap(), "=SUM(A1:A3)").unwrap();
    patched_xlsx(&mut book, &xlsx, "xl/worksheets/sheet1.xml", |t| t.replace("<f>SUM(A1:A3)</f>", "<f>SUM(A1:A3</f>"));
    let got = refused(&mut AbacoFile::new(open(&xlsx)));
    assert!(got.contains("sheets/01-Sheet1.toml B1: a formula the engine cannot read"), "{got}");
    // two legacy arrays whose areas overlap, neither anchor in the other's (`ref` widened)
    let xlsx = dir.path().join("overlap.xlsx");
    let mut book = Workbook::new("Overlap").unwrap();
    book.model_mut().set_user_array_formula(0, 1, 2, 1, 2, "={1;2}").unwrap();
    book.model_mut().set_user_array_formula(0, 2, 1, 1, 1, "=5").unwrap();
    patched_xlsx(&mut book, &xlsx, "xl/worksheets/sheet1.xml", |t| t.replace("ref=\"A2:A2\"", "ref=\"A2:B2\""));
    let got = refused(&mut AbacoFile::new(open(&xlsx)));
    assert!(got.contains("sheets/01-Sheet1.toml A2: an array formula over the one at B1"), "{got}");
    // a workbook of no sheet, which the engine's own API never leaves (`Cannot delete only sheet`)
    let xlsx = dir.path().join("no-sheet.xlsx");
    let mut book = Workbook::new("Bare").unwrap();
    patched_xlsx(&mut book, &xlsx, "xl/workbook.xml", |t| {
        let (a, b) = (t.find("<sheets>").unwrap(), t.find("</sheets>").unwrap());
        format!("{}<sheets></sheets>{}", &t[..a], &t[b + "</sheets>".len()..])
    });
    let got = refused(&mut AbacoFile::new(open(&xlsx)));
    assert!(got.contains("manifest.toml: a workbook holds a sheet at least"), "{got}");
    // settings the engine refuses, set past its setters
    let mut book = Workbook::new("Settings").unwrap();
    book.model_mut().workbook.settings.tz = "Mars/Olympus".into();
    let got = refused(&mut AbacoFile::new(book));
    assert!(got.contains("workbook.toml: `settings`: "), "{got}");
    // a tool's state nested past what TOML's reader reads
    let mut file = worked_example();
    let mut deep = toml::Value::Integer(1);
    for _ in 0..200 {
        let mut t = toml::Table::new();
        t.insert("a".into(), deep);
        deep = toml::Value::Table(t);
    }
    file.workspace.state.insert("deep".into(), deep.as_table().unwrap().clone());
    let got = refused(&mut file);
    assert!(got.contains("workspace/state/deep.toml: TOML's reader cannot read it back"), "{got}");
    // a part it kept, which a sheet added since would overwrite
    let mut parts = abacofile::parts(&worked_example()).unwrap();
    parts.push(("sheets/02-Other.toml".into(), b"a newer Abaco's part\n".to_vec()));
    let kept = dir.path().join("kept.abaco");
    std::fs::write(&kept, abacofile::zip_of(&parts).unwrap()).unwrap();
    let mut file = abacofile::read(&kept).unwrap();
    file.book.add_sheet("Other").unwrap();
    let got = refused(&mut file);
    assert!(got.contains("sheets/02-Other.toml: a part this file kept without reading it has the name of one it now writes"), "{got}");
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
fn a_quote_prefixed_cell_an_array_spills_into_keeps_its_style() {
    // entering the array gives its spilled cells a derived style (the quote prefix taken off): each
    // keeps the style the file states, with and without a plain copy of it in the workbook
    for (bold, italic) in [(true, true), (false, false)] {
        let mut book = Workbook::new("Spill").unwrap();
        book.model_mut().set_user_array_formula(0, 1, 1, 1, 3, "={1;2;3}").unwrap();
        let a2 = Addr::parse("A2").unwrap();
        let mut style = book.style(0, a2).unwrap();
        style.quote_prefix = true;
        style.font.b = bold;
        style.font.i = italic;
        book.set_style(0, a2, &style).unwrap();
        let mut file = AbacoFile::new(book);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spill.abaco");
        abacofile::write(&mut file, &path).unwrap();
        let mut back = abacofile::read(&path).unwrap();
        assert!(differences(&file.book, &back.book).is_empty(), "{:?}", differences(&file.book, &back.book));
        assert_eq!(back.book.style(0, a2).unwrap(), file.book.style(0, a2).unwrap());
        let again = dir.path().join("again.abaco");
        abacofile::write(&mut back, &again).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(&again).unwrap(), "bold {bold}");
    }
}

#[test]
fn a_spilled_cell_of_style_0_reads_back_with_style_0() {
    // the writer leaves out a spilled cell of style 0; the engine creates it again on reading with its
    // row's or its column's style, and a legacy array with a quote prefix's plain copy of style 0: it
    // holds style 0, for a legacy array and a dynamic one, and typing in it reads its style
    let dir = tempfile::tempdir().unwrap();
    for case in ["column", "row", "quote prefix"] {
        let mut book = Workbook::new("Spill").unwrap();
        book.model_mut().set_user_array_formula(0, 1, 1, 1, 3, "={1;2;3}").unwrap();
        book.set_formula(0, Addr::parse("D1").unwrap(), "=SEQUENCE(3)").unwrap();
        book.recalc();
        let mut bold = book.style(0, Addr::parse("F1").unwrap()).unwrap();
        bold.font.b = true;
        let model = book.model_mut();
        match case {
            "column" => {
                model.set_column_style(0, 1, &bold).unwrap();
                model.set_column_style(0, 4, &bold).unwrap();
            }
            "row" => {
                model.set_row_style(0, 2, &bold).unwrap();
                model.set_row_style(0, 3, &bold).unwrap();
            }
            _ => model.workbook.styles.cell_xfs[0].quote_prefix = true,
        }
        let spilled: Vec<Addr> = ["A2", "A3", "D2", "D3"].iter().map(|a| Addr::parse(a).unwrap()).collect();
        for a in &spilled {
            assert_eq!((book.content(0, *a).unwrap(), book.style_index(0, *a).unwrap()), (Content::Spill, 0), "{case} {a}");
        }
        let mut file = AbacoFile::new(book);
        let path = dir.path().join("spill.abaco");
        abacofile::write(&mut file, &path).unwrap();
        let mut back = abacofile::read(&path).unwrap();
        assert!(differences(&file.book, &back.book).is_empty(), "{case}: {:?}", differences(&file.book, &back.book));
        for a in &spilled {
            assert_eq!(back.book.style_index(0, *a).unwrap(), 0, "{case} {a}");
        }
        let again = dir.path().join("again.abaco");
        abacofile::write(&mut back, &again).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(&again).unwrap(), "{case}");
        // typing into a spilled cell: the engine reads its style, which the workbook holds
        for a in ["D2", "A2"] {
            back.book.set_value(0, Addr::parse(a).unwrap(), &Value::Number(9.0)).unwrap();
        }
    }
}

#[test]
fn a_legacy_array_reading_currency_and_a_percentage_reads_back_with_the_formats_the_file_holds() {
    // entering the array works out a number format from the units of what it reads (currency times
    // a percentage: `$#,##0.00`), which the engine adds to its formats: reading puts the file's
    // styles back whole, so the workbook reads back as written and writes the same bytes
    let mut book = Workbook::new("Units").unwrap();
    let (a1, b1) = (Addr::parse("A1").unwrap(), Addr::parse("B1").unwrap());
    book.set_value(0, a1, &Value::Number(10.0)).unwrap();
    book.set_value(0, b1, &Value::Number(0.5)).unwrap();
    book.model_mut().set_user_array_formula(0, 1, 3, 1, 1, "=A1*B1").unwrap();
    for (at, format) in [(a1, "$#,##0"), (b1, "0%")] {
        let mut style = book.style(0, at).unwrap();
        style.num_fmt = format.into();
        book.set_style(0, at, &style).unwrap();
    }
    let mut file = AbacoFile::new(book);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("units.abaco");
    abacofile::write(&mut file, &path).unwrap();
    let mut back = abacofile::read(&path).unwrap();
    assert!(differences(&file.book, &back.book).is_empty(), "{:?}", differences(&file.book, &back.book));
    let again = dir.path().join("again.abaco");
    abacofile::write(&mut back, &again).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(&again).unwrap());
}

#[test]
fn a_style_whose_font_fill_or_border_the_workbook_does_not_hold_is_refused_at_both_ends() {
    // the engine indexes a style's font, fill and border unchecked when a formula reads its cell or
    // is entered in it: the writer refuses such a style, and so does the reader, never a panic
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("styles.abaco");
    let make = || {
        let mut book = Workbook::new("Styles").unwrap();
        let (a1, b1) = (Addr::parse("A1").unwrap(), Addr::parse("B1").unwrap());
        book.set_value(0, a1, &Value::Number(2.0)).unwrap();
        book.set_formula(0, b1, "=1+1").unwrap();
        book.model_mut().set_user_array_formula(0, 2, 2, 1, 1, "=A1*3").unwrap();
        let mut bold = book.style(0, a1).unwrap();
        bold.font.b = true;
        book.set_style(0, a1, &bold).unwrap();
        book.set_style(0, b1, &bold).unwrap();
        AbacoFile::new(book)
    };
    for (what, set) in [("fill_id", 99), ("font_id", 99), ("border_id", -1)] {
        let mut file = make();
        let xfs = &mut file.book.model_mut().workbook.styles.cell_xfs;
        let last = xfs.len() - 1;
        xfs[last].quote_prefix = true;
        match what {
            "fill_id" => xfs[last].fill_id = set,
            "font_id" => xfs[last].font_id = set,
            _ => xfs[last].border_id = set,
        }
        let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| abacofile::write(&mut file, &path))).expect("no panic");
        let got = got.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(got.contains(&format!("workbook.toml: `styles.cell_xfs.{last}.{what}` {set} is none of the workbook's")), "{got}");
        assert!(!path.exists());
    }
    // the reader: a file whose bold style names a font the workbook does not hold, A1 read by an array
    let mut file = make();
    abacofile::write(&mut file, &path).unwrap();
    let parts: Vec<(String, Vec<u8>)> = abacofile::parts(&file)
        .unwrap()
        .into_iter()
        .map(|(n, b)| {
            if n == "workbook.toml" {
                let t = String::from_utf8(b).unwrap();
                assert!(t.contains("font_id = 1"), "{t}");
                (n, t.replacen("font_id = 1", "font_id = 99", 1).into_bytes())
            } else {
                (n, b)
            }
        })
        .collect();
    std::fs::write(&path, abacofile::zip_of(&parts).unwrap()).unwrap();
    let got = std::panic::catch_unwind(|| abacofile::read(&path).err().map(|e| e.to_string())).expect("no panic").unwrap_or_default();
    assert!(got.contains("workbook.toml: `styles.cell_xfs.") && got.contains(".font_id` 99 is none of the workbook's"), "{got}");
    // a number format no id names (a negative one, which the engine indexed past its built-in
    // formats when an array read the cell): refused at both ends, never a panic (refute round 14)
    let mut file = make();
    let xfs = &mut file.book.model_mut().workbook.styles.cell_xfs;
    let last = xfs.len() - 1;
    xfs[last].num_fmt_id = -1;
    let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| abacofile::write(&mut file, &path))).expect("no panic");
    let got = got.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(got.contains(&format!("workbook.toml: `styles.cell_xfs.{last}.num_fmt_id` -1 is no number format's")), "{got}");
    // the reader: a cell's style, and a named style, which applying it copies into a cell's
    for table in ["cell_xfs", "cell_style_xfs"] {
        let mut file = make();
        abacofile::write(&mut file, &path).unwrap();
        let parts: Vec<(String, Vec<u8>)> = abacofile::parts(&file)
            .unwrap()
            .into_iter()
            .map(|(n, b)| {
                let t = String::from_utf8(b).unwrap();
                if n != "workbook.toml" {
                    return (n, t.into_bytes());
                }
                let start = [format!("[[styles.{table}]]"), format!("{table} = [")].iter().find_map(|m| t.find(m.as_str())).unwrap_or_else(|| panic!("{t}"));
                let at = start + t[start..].find("num_fmt_id = 0").unwrap_or_else(|| panic!("{t}"));
                (n, format!("{}num_fmt_id = -1{}", &t[..at], &t[at + "num_fmt_id = 0".len()..]).into_bytes())
            })
            .collect();
        std::fs::write(&path, abacofile::zip_of(&parts).unwrap()).unwrap();
        let got = std::panic::catch_unwind(|| abacofile::read(&path).err().map(|e| e.to_string())).expect("no panic").unwrap_or_default();
        assert!(got.contains(&format!("workbook.toml: `styles.{table}.")) && got.contains(".num_fmt_id` -1 is no number format's"), "{table}: {got}");
    }
    // and a named style's font the workbook does not hold, refused before applying the style
    // copies it into a cell's
    let mut file = make();
    let named = &mut file.book.model_mut().workbook.styles.cell_style_xfs;
    named[0].font_id = 99;
    let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| abacofile::write(&mut file, &path))).expect("no panic");
    let got = got.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(got.contains("workbook.toml: `styles.cell_style_xfs.0.font_id` 99 is none of the workbook's"), "{got}");
}

#[test]
fn a_field_deleted_is_refused_as_missing_and_a_key_replacing_one_by_its_path() {
    // refute round 14: a replacing key named only when the value written back lacks it, a field
    // deleted refused in the deserialiser's own words, never as a key it reads
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fields.abaco");
    let refusal = |parts: &[(String, Vec<u8>)], part: &str, edit: &dyn Fn(&str) -> String| -> String {
        let edited: Vec<(String, Vec<u8>)> = parts
            .iter()
            .map(|(n, b)| if n == part || (part == "sheets/" && n.starts_with("sheets/")) { (n.clone(), edit(std::str::from_utf8(b).unwrap()).into_bytes()) } else { (n.clone(), b.clone()) })
            .collect();
        std::fs::write(&path, abacofile::zip_of(&edited).unwrap()).unwrap();
        abacofile::read(&path).err().map(|e| e.to_string()).unwrap_or_else(|| "read, not refused".into())
    };
    let mut example = worked_example();
    example.book.ensure_calculated();
    let parts = abacofile::parts(&example).unwrap();
    // deleted: the deserialiser's words
    for (part, from, to, field) in [
        ("workbook.toml", "tz = \"UTC\"\n", "", "tz"),
        ("workbook.toml", "sz = 12\n", "", "sz"),
        ("sheets/01-Loads.toml", "cols = []", "cols = [{ max = 1, width = 10.0, custom_width = true, hidden = false, style = 0 }]", "min"),
        ("sheets/01-Loads.toml", "rows = []", "rows = [{ height = 15.0, custom_format = false, custom_height = false, s = 0, hidden = false }]", "r"),
        ("sheets/01-Loads.toml", "merged_cells = []", "merged_cells = [{ column = 1, width = 2, height = 1 }]", "row"),
    ] {
        let got = refusal(&parts, part, &|t| t.replacen(from, to, 1));
        assert!(got.contains(&format!("missing field `{field}`")) && !got.contains("is a key nothing reads"), "{field}: {got}");
    }
    let got = refusal(&parts, "workbook.toml", &|t| t.lines().filter(|l| !l.starts_with("created = ")).map(|l| format!("{l}\n")).collect());
    assert!(got.contains("missing field `created`") && !got.contains("is a key nothing reads"), "created: {got}");
    // a link's tag or field replaced by a far name, two links each with one: by its path
    for (links, want) in [
        ("[[1, 1, { kind = \"External\", target = \"https://example.org\" }]]", "`links.0.2.kind` is a key nothing reads"),
        ("[[1, 1, { type = \"External\", url = \"https://example.org\" }]]", "`links.0.2.url` is a key nothing reads"),
        ("[[1, 1, { type = \"External\", url = \"a\" }], [2, 1, { type = \"Internal\", locationn = \"Loads!A1\" }]]", "`links.0.2.url` is a key nothing reads"),
    ] {
        let got = refusal(&parts, "sheets/01-Loads.toml", &|t| t.replace("links = []", &format!("links = {links}")));
        assert!(got.contains(want), "{links}: {got}");
    }
    // refute round 15: a link's tag deleted, the deserialiser's words (no read key named for the
    // first variant that moves on); beside another link holding `url`, that one named; a replacing
    // key past 64 links that lack the field, each link read alone
    let internal = |row: usize| format!("[{row}, 1, {{ type = \"Internal\", location = \"Loads!A1\", tooltip = nan }}]");
    for (links, want) in [
        ("[[1, 1, { location = \"Loads!A1\", tooltip = nan }]]".to_string(), None),
        ("[[1, 1, { location = \"Loads!A1\", tooltip = nan }], [2, 1, { type = \"External\", url = \"a\" }]]".to_string(), Some("`links.1.2.url` is a key nothing reads")),
        (format!("[{}, [65, 1, {{ type = \"External\", url = \"a\" }}]]", (1..=64).map(internal).collect::<Vec<_>>().join(", ")), Some("`links.64.2.url` is a key nothing reads")),
        (format!("[{}, [201, 1, {{ type = \"External\", url = \"a\" }}]]", (1..=200).map(internal).collect::<Vec<_>>().join(", ")), Some("`links.200.2.url` is a key nothing reads")),
    ] {
        let got = refusal(&parts, "sheets/01-Loads.toml", &|t| t.replace("links = []", &format!("links = {links}")));
        match want {
            Some(want) => assert!(got.contains(want), "{}: {got}", &links[..links.len().min(120)]),
            None => assert!(got.contains("missing field `type`") && !got.contains("is a key nothing reads"), "{links}: {got}"),
        }
    }
    // a format's rule: two fields replaced, the tag replaced by `kind`, a threshold's field far
    // renamed, a rule's field deleted
    let rules = AbacoFile::new(open(&corpus_dir().join("conditional_formatting/cf_tests.xlsx")));
    let parts = abacofile::parts(&rules).unwrap();
    let cellis = |t: &str| t.contains("stop_if_true = false\ntype = \"CellIs\"");
    let got = refusal(&parts, "sheets/", &|t| if cellis(t) { t.replacen("formula = ", "formulaa = ", 1).replacen("dxf_id = ", "dxf_idd = ", 1) } else { t.to_string() });
    assert!(got.contains(".cf_rule.dxf_idd` is a key nothing reads") || got.contains(".cf_rule.formulaa` is a key nothing reads"), "{got}");
    let got = refusal(&parts, "sheets/", &|t| t.replacen("type = \"CellIs\"", "kind = \"CellIs\"", 1));
    assert!(got.contains(".cf_rule.kind` is a key nothing reads"), "{got}");
    let got = refusal(&parts, "sheets/", &|t| t.replacen("cfvo = ", "value = ", 1));
    assert!(got.contains(".value` is a key nothing reads"), "{got}");
    let got = refusal(&parts, "sheets/", &|t| if cellis(t) { t.replacen("formula = ", "# formula = ", 1) } else { t.to_string() });
    assert!(got.contains("missing field `formula`") && !got.contains("is a key nothing reads"), "{got}");
    // refute round 15: a colour replaced inside a rule (an untagged `Color`, whose refusal is no
    // missing field), by its path; a rule's tag deleted, of every kind the file holds, the
    // deserialiser's words
    for (from, to, want) in [
        ("positive_color = ", "positive_colour = ", ".cf_rule.positive_colour` is a key nothing reads"),
        ("negative_color = ", "negativecolor = ", ".cf_rule.negativecolor` is a key nothing reads"),
        ("\ncolor = \"#", "\ncolour = \"#", ".colour` is a key nothing reads"),
    ] {
        assert!(parts.iter().any(|(n, b)| n.starts_with("sheets/") && std::str::from_utf8(b).unwrap().contains(from)), "{from}");
        let got = refusal(&parts, "sheets/", &|t| t.replacen(from, to, 1));
        assert!(got.contains(want), "{to}: {got}");
        if from.contains("\ncolor") {
            assert!(got.contains(".cf_rule.thresholds."), "{got}");
        }
    }
    for kind in ["CellIs", "DataBar", "ColorScale", "IconSet", "Top10", "Text"] {
        let tag = format!("type = \"{kind}\"\n");
        if !parts.iter().any(|(n, b)| n.starts_with("sheets/") && std::str::from_utf8(b).unwrap().contains(&tag)) {
            assert!(!["CellIs", "DataBar", "ColorScale"].contains(&kind), "{kind}");
            continue;
        }
        let got = refusal(&parts, "sheets/", &|t| t.replacen(&tag, "", 1));
        assert!(got.contains("missing field `type`") && !got.contains("is a key nothing reads"), "{kind}: {got}");
    }
    // refute round 16: every rule's tag replaced, one at a time, by `kind` and by `Type`, by its path
    // (an icon set's and a colour scale's spent the readings following every value of every field)
    let mut tags = 0;
    for (part, text) in parts.iter().filter(|(n, _)| n.starts_with("sheets/")).map(|(n, b)| (n.clone(), String::from_utf8(b.clone()).unwrap())) {
        for (at, _) in text.match_indices("\ntype = \"") {
            for key in ["kind", "Type"] {
                let got = refusal(&parts, &part, &|t| format!("{}{key}{}", &t[..at + 1], &t[at + 1 + "type".len()..]));
                assert!(got.contains(&format!(".cf_rule.{key}` is a key nothing reads")), "{part} at {at}, {key}: {got}");
            }
            tags += 1;
        }
    }
    assert!(tags >= 50, "{tags} rules");
    // a key beside a threshold's variant, and a variant misspelt, by their paths
    let iconsets = parts.iter().find(|(n, b)| n.starts_with("sheets/") && std::str::from_utf8(b).unwrap().contains("\nPercent = ")).map(|(n, _)| n.clone()).unwrap();
    let got = refusal(&parts, &iconsets, &|t| t.replacen("\nPercent = ", "\nnote = 1\nPercent = ", 1));
    assert!(got.contains(".cfvo.note` is a key nothing reads") && got.contains(".cf_rule.thresholds."), "{got}");
    let got = refusal(&parts, &iconsets, &|t| t.replacen("\nPercent = ", "\nPercnt = ", 1));
    assert!(got.contains(".cfvo.Percnt` is a key nothing reads"), "{got}");
    // and beside a linked workbook's cached cell's variant
    let linked = AbacoFile::new(open(&corpus_dir().join("link_test.xlsx")));
    let parts = abacofile::parts(&linked).unwrap();
    let book = String::from_utf8(parts.iter().find(|(n, _)| n == "workbook.toml").unwrap().1.clone()).unwrap();
    assert!(book.contains("cells = []"), "{book}");
    let cells = |value: &'static str| move |t: &str| t.replacen("cells = []", &format!("cells = [[1, 1, {{ {value} }}]]"), 1);
    assert!(!refusal(&parts, "workbook.toml", &cells("Number = 1.0")).contains("is a key nothing reads"));
    let got = refusal(&parts, "workbook.toml", &cells("Number = 1.0, note = 2"));
    assert!(got.contains("workbook.toml: `external_books.0.sheets.0.cells.0.2.note` is a key nothing reads"), "{got}");
    let got = refusal(&parts, "workbook.toml", &cells("Numbr = 1.0"));
    assert!(got.contains("`external_books.0.sheets.0.cells.0.2.Numbr` is a key nothing reads"), "{got}");
    // two keys of which none reads alone: the deserialiser's words
    let got = refusal(&parts, "workbook.toml", &cells("note = 2, Numbr = 1.0"));
    assert!(!got.contains("is a key nothing reads") && got.contains("workbook.toml"), "{got}");
}

#[test]
fn a_manifest_listing_a_part_twice_is_refused_by_the_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("twice.abaco");
    let parts = abacofile::parts(&worked_example()).unwrap();
    let manifest = String::from_utf8(parts.iter().find(|(n, _)| n == "manifest.toml").unwrap().1.clone()).unwrap();
    let mut table: toml::Table = toml::from_str(&manifest).unwrap();
    let sheets = table.get_mut("sheets").and_then(toml::Value::as_array_mut).unwrap();
    let again = sheets[0].clone();
    sheets.push(again);
    let edited: Vec<(String, Vec<u8>)> =
        parts.iter().map(|(n, b)| if n == "manifest.toml" { (n.clone(), toml::to_string(&table).unwrap().into_bytes()) } else { (n.clone(), b.clone()) }).collect();
    std::fs::write(&path, abacofile::zip_of(&edited).unwrap()).unwrap();
    let got = abacofile::read(&path).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(got.contains("manifest.toml: `sheets.1` lists the part `sheets/01-Loads.toml`, which `sheets.0` lists"), "{got}");
    let mut table: toml::Table = toml::from_str(&manifest).unwrap();
    let macros = table.get("macros").and_then(toml::Value::as_array).cloned().unwrap_or_default();
    assert!(!macros.is_empty(), "the worked example names a macro");
    let mut twice = macros.clone();
    twice.push(macros[0].clone());
    table.insert("macros".into(), toml::Value::Array(twice));
    let edited: Vec<(String, Vec<u8>)> =
        parts.iter().map(|(n, b)| if n == "manifest.toml" { (n.clone(), toml::to_string(&table).unwrap().into_bytes()) } else { (n.clone(), b.clone()) }).collect();
    std::fs::write(&path, abacofile::zip_of(&edited).unwrap()).unwrap();
    let got = abacofile::read(&path).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(got.contains(&format!("manifest.toml: `macros.{}` lists the macro", macros.len())), "{got}");
}

#[test]
fn a_number_in_a_formula_reads_back_exactly() {
    // the engine spells a formula's number in 15 digits when they hold it, and otherwise in the
    // shortest spelling that does (fork change 29): each formula reads back to the same content and
    // value, and writes the same bytes again
    let mut book = Workbook::new("Literals").unwrap();
    let literals = ["=0.30000000000000004", "=1.0000000000000002", "=1.2345678901234567", "=9007199254740993", "=1.7976931348623157e308", "=0.1+0.2", "=5e-324"];
    for (i, f) in literals.iter().enumerate() {
        book.set_formula(0, Addr::new(i as i32 + 1, 1), f).unwrap();
    }
    book.recalc();
    let mut file = AbacoFile::new(book);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("literals.abaco");
    abacofile::write(&mut file, &path).unwrap();
    let mut back = abacofile::read(&path).unwrap();
    assert!(differences(&file.book, &back.book).is_empty(), "{:?}", differences(&file.book, &back.book));
    assert_eq!(back.book.value_now(0, Addr::new(1, 1)).unwrap(), Value::Number(0.30000000000000004));
    assert_eq!(back.book.value_now(0, Addr::new(5, 1)).unwrap(), Value::Number(f64::MAX));
    let again = dir.path().join("again.abaco");
    abacofile::write(&mut back, &again).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(&again).unwrap());
}

#[test]
fn a_formula_s_brackets_and_its_array_constant_s_numbers_read_back() {
    // the engine spells a formula so it reads back as the same formula (fork change 30): the
    // brackets that change its meaning kept, an array constant's numbers to the last bit; each
    // typed formula reads back to the same content and value, and writes the same bytes again
    let typed = [
        "=(Z1&Z2)*1", "=(2&3)+1", "=1-(2&3)", "=(Z1+Z2)%", "=2*(Z1+Z2)%", "=(Z1/Z2)%", "=IF(TRUE,(Z1/Z2)%)", "=(Z1^2)%",
        "=\"x\"&(Z1>0)", "=(Z1>0)&\"x\"", "=(Z1<Z2)=(Z2<Z3)", "=(1=1)=(1=1)", "=0.1+(0.2+0.3)", "=-(Z1*Z2)",
        "=SUM({0.30000000000000004})", "=SUM({-0.30000000000000004})", "=SUM({1.2345678901234567})", "=SUM({9007199254740993})",
        "=INDEX({1.0000000000000002;2},1)-1",
    ];
    let mut book = Workbook::new("Brackets").unwrap();
    for (r, v) in [(1, 3.0), (2, 4.0), (3, 5.0)] {
        book.set_value(0, Addr::new(r, 26), &Value::Number(v)).unwrap();
    }
    for (i, f) in typed.iter().enumerate() {
        book.set_formula(0, Addr::new(i as i32 + 1, 1), f).unwrap();
    }
    book.model_mut().set_user_array_formula(0, 1, 3, 1, 2, "={0.30000000000000004;1}").unwrap();
    book.set_formula(0, Addr::parse("E1").unwrap(), "={0.30000000000000004}").unwrap();
    book.recalc();
    assert_eq!(book.value_now(0, Addr::new(1, 1)).unwrap(), Value::Number(34.0));
    let mut file = AbacoFile::new(book);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("brackets.abaco");
    abacofile::write(&mut file, &path).unwrap();
    let mut back = abacofile::read(&path).unwrap();
    assert!(differences(&file.book, &back.book).is_empty(), "{:?}", differences(&file.book, &back.book));
    let again = dir.path().join("again.abaco");
    abacofile::write(&mut back, &again).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(&again).unwrap());
}

#[test]
fn a_formula_a_locale_spells_unreadably_is_refused_by_the_writer() {
    // in a locale whose decimal is a comma the engine spells an array's rows with `/`, which its
    // own lexer does not read: the writer refuses that formula by its address, nothing written
    let mut book = Workbook::new("Rows").unwrap();
    book.set_formula(0, Addr::parse("B2").unwrap(), "=ROWS({1;2})").unwrap();
    book.model_mut().set_locale("de").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rows.abaco");
    let got = abacofile::write(&mut AbacoFile::new(book), &path).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(got.contains("sheets/01-Sheet1.toml B2: a formula whose spelling the reader would read as another formula"), "{got}");
    assert!(!path.exists());
}

#[test]
fn a_formula_is_written_as_the_reader_reads_it_whatever_the_model_s_language_or_locale() {
    // the reader reads in English and the workbook settings' locale: the writer spells so (fork
    // change 31b), a Spanish or French model's `SUMA`, `SOMME` written `SUM`, a locale the settings
    // name but the model does not hold written with its own decimal and separators
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("spelled.abaco");
    let at = |b: &Workbook, a: &str| b.value_now(0, Addr::parse(a).unwrap()).unwrap();
    for language in ["es", "fr"] {
        let mut book = Workbook::new("Lengua").unwrap();
        book.set_value(0, Addr::parse("A1").unwrap(), &Value::Number(1.0)).unwrap();
        book.set_value(0, Addr::parse("A2").unwrap(), &Value::Number(2.0)).unwrap();
        book.set_formula(0, Addr::parse("B1").unwrap(), "=SUM(A1:A2)").unwrap();
        book.set_formula(0, Addr::parse("B2").unwrap(), "=IF(TRUE,1,2)").unwrap();
        book.model_mut().set_language(language).unwrap();
        let mut file = AbacoFile::new(book);
        abacofile::write(&mut file, &path).unwrap();
        let mut back = abacofile::read(&path).unwrap();
        assert_eq!((at(&back.book, "B1"), at(&back.book, "B2")), (Value::Number(3.0), Value::Number(1.0)), "{language}");
        assert_eq!(back.book.content(0, Addr::parse("B1").unwrap()).unwrap(), Content::Formula("=SUM(A1:A2)".into()), "{language}");
        let again = dir.path().join("again.abaco");
        abacofile::write(&mut back, &again).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(&again).unwrap(), "{language}");
    }
    let mut book = Workbook::new("Locale").unwrap();
    book.set_formula(0, Addr::parse("B1").unwrap(), "=SUM(1,2)").unwrap();
    book.set_formula(0, Addr::parse("B2").unwrap(), "=1.5*2").unwrap();
    book.model_mut().workbook.settings.locale = "de".into();
    let mut file = AbacoFile::new(book);
    abacofile::write(&mut file, &path).unwrap();
    let back = abacofile::read(&path).unwrap();
    assert_eq!((at(&back.book, "B1"), at(&back.book, "B2")), (Value::Number(3.0), Value::Number(3.0)));
}

#[test]
fn a_named_function_an_unknown_one_and_a_range_s_brackets_read_back_as_typed() {
    // a lambda's name, a user function's, a function the engine lacks, typed in Ábaco in any case,
    // and the range operator's bracketed operands (fork change 33): each reads back to the same
    // content and value, and writes the same bytes again
    let mut book = Workbook::new("Named").unwrap();
    for r in 1..=3 {
        book.set_value(0, Addr::new(r, 1), &Value::Number(f64::from(r))).unwrap();
    }
    book.set_name("Fn", "=LAMBDA(x,x*2)", None).unwrap();
    let typed = [
        "=Fn(3)", "=FN(3)", "=fn(3)", "=LET(F,LAMBDA(x,x*3),F(2))", "=LET(x,1,X+1)", "=MYUDF(1)", "=GETPIVOTDATA(\"x\",A1)",
        "=EUROCONVERT(1,\"EUR\",\"DEM\")", "=IMAGE(\"x\")", "=WEBSERVICE(\"x\")", "=BAHTTEXT(1)", "=STOCKHISTORY(\"x\",1)",
        "=SUM(A1:(A2:A3))", "=SUM(A1:(A3))", "=SUM((A1):A3)",
    ];
    for (i, f) in typed.iter().enumerate() {
        book.set_formula(0, Addr::new(i as i32 + 1, 3), f).unwrap();
    }
    book.recalc();
    assert_eq!(book.value_now(0, Addr::new(1, 3)).unwrap(), Value::Number(6.0));
    assert_eq!(book.value_now(0, Addr::new(5, 3)).unwrap(), Value::Number(2.0));
    let mut file = AbacoFile::new(book);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("named.abaco");
    abacofile::write(&mut file, &path).unwrap();
    let mut back = abacofile::read(&path).unwrap();
    assert!(differences(&file.book, &back.book).is_empty(), "{:?}", differences(&file.book, &back.book));
    let again = dir.path().join("again.abaco");
    abacofile::write(&mut back, &again).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(&again).unwrap());
}

#[test]
fn two_dynamic_arrays_meeting_spill_on_reading_in_the_order_the_engine_opens_a_workbook_in() {
    // a limit, pinned: which of two dynamic arrays whose spills meet spills is the engine's order
    // of evaluation, the order they were entered in while the workbook is open; the file does not
    // carry it, and a workbook read spills them row by row, as the engine opens any
    let mut book = Workbook::new("Meeting").unwrap();
    book.set_formula(0, Addr::parse("A2").unwrap(), "=SEQUENCE(1,3)").unwrap();
    book.recalc();
    book.set_formula(0, Addr::parse("C1").unwrap(), "=SEQUENCE(3)").unwrap();
    book.set_formula(0, Addr::parse("E5").unwrap(), "=C2").unwrap();
    book.recalc();
    let at = |b: &Workbook, a: &str| b.value_now(0, Addr::parse(a).unwrap()).unwrap();
    assert_eq!((at(&book, "C2"), at(&book, "E5")), (Value::Number(3.0), Value::Number(3.0)));
    assert_eq!(at(&book, "C1"), Value::Error("#SPILL!".into()));
    let mut file = AbacoFile::new(book);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meeting.abaco");
    abacofile::write(&mut file, &path).unwrap();
    let back = abacofile::read(&path).unwrap();
    assert_eq!(at(&back.book, "A2"), Value::Error("#SPILL!".into()));
    assert_eq!((at(&back.book, "C1"), at(&back.book, "C2"), at(&back.book, "E5")), (Value::Number(1.0), Value::Number(2.0), Value::Number(2.0)));
}

#[test]
fn a_destination_named_near_the_file_system_s_limit_is_written_with_its_bak() {
    // the temporary file's name is its own, not grown from the destination's: a name of 245 bytes
    // (the `.bak` four more) is written, and written over
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("{}.abaco", "a".repeat(239)));
    let mut file = worked_example();
    abacofile::write(&mut file, &path).unwrap();
    let first = std::fs::read(&path).unwrap();
    file.book.set_value(0, Addr::parse("B2").unwrap(), &Value::Number(13.0)).unwrap();
    abacofile::write(&mut file, &path).unwrap();
    let mut bak = path.clone().into_os_string();
    bak.push(".bak");
    assert_eq!(std::fs::read(&bak).unwrap(), first);
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
    // a linked workbook's sheet stated twice, in its spelling or another case the engine takes as
    // one: the engine's map would keep one sheet's cells (refute round 13)
    for upper in [false, true] {
        let got = read_with(&parts, "workbook.toml", &|t| {
            let mut table: toml::Table = toml::from_str(t).unwrap();
            let books = table.get_mut("external_books").and_then(toml::Value::as_array_mut).unwrap();
            let sheets = books[0].get_mut("sheets").and_then(toml::Value::as_array_mut).unwrap();
            let mut twin = sheets[0].clone();
            if upper {
                let name = twin.get("name").and_then(toml::Value::as_str).unwrap().to_uppercase();
                twin.as_table_mut().unwrap().insert("name".into(), toml::Value::String(name));
            }
            sheets.push(twin);
            toml::to_string(&table).unwrap()
        });
        let last = linked.book.model().workbook.external_books[0].sheets.len();
        assert!(got.contains(&format!("workbook.toml: `external_books.0.sheets.{last}` names the sheet `external_books.0.sheets.0` names")), "{upper}: {got}");
    }
    // and the writer, a model holding such a book
    let mut twice = AbacoFile::new(open(&corpus_dir().join("link_test.xlsx")));
    let first = twice.book.model().workbook.external_books[0].sheets[0].clone();
    twice.book.model_mut().workbook.external_books[0].sheets.push(first);
    let got = abacofile::write(&mut twice, &path).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(got.contains("names the sheet `external_books.0.sheets.0` names"), "{got}");
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

// ---- refute round 10 --------------------------------------------------------------------------

#[test]
fn a_table_whose_range_or_sheet_the_engine_cannot_read_is_refused_at_both_ends() {
    // the repository's own tables workbook; Table3's range a range no longer, or its sheet none of
    // the workbook's: the engine's parser reads a structured reference by them (and panicked on
    // the range), so the reader refuses the file by the table's path, a formula naming the table or
    // not, and the writer a model holding such a table
    let src = corpus_dir().join("calc_test_no_export/tables.xlsx");
    let mut file = AbacoFile::new(open(&src));
    file.book.ensure_calculated();
    let good = abacofile::parts(&file).unwrap();
    let text = |name: &str| String::from_utf8(good.iter().find(|(n, _)| n == name).unwrap().1.clone()).unwrap();
    let book_toml = text("workbook.toml");
    let section = book_toml.find("[tables.Table3]").unwrap_or_else(|| panic!("no `[tables.Table3]`:\n{book_toml}"));
    let end = book_toml[section + 1..].find("\n[").map(|e| section + 1 + e).unwrap_or(book_toml.len());
    let in_table = |key: &str, value: &str| -> String {
        let mut out = book_toml.clone();
        let line = out[section..end].find(&format!("\n{key} = ")).map(|l| section + l + 1).unwrap_or_else(|| panic!("no `{key}` in Table3:\n{}", &book_toml[section..end]));
        let line_end = out[line..].find('\n').map(|e| line + e).unwrap_or(out.len());
        out.replace_range(line..line_end, &format!("{key} = {value}"));
        out
    };
    let sheet_name = book_toml[section..end].lines().find_map(|l| l.strip_prefix("sheet_name = \"")).map(|l| l.trim_end_matches('"').to_string()).unwrap();
    let sheet_part = good.iter().map(|(n, _)| n.clone()).find(|n| n.starts_with("sheets/") && n.ends_with(&format!("-{sheet_name}.toml"))).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tables.abaco");
    for (key, value, want) in [
        ("reference", "\"nonsense\"", "workbook.toml: `tables.Table3.reference` `nonsense` is no range of cells"),
        ("reference", "\"A1\"", "workbook.toml: `tables.Table3.reference` `A1` is no range of cells"),
        ("reference", "\"\"", "workbook.toml: `tables.Table3.reference` `` is no range of cells"),
        ("sheet_name", "\"Nowhere\"", "workbook.toml: `tables.Table3.sheet_name` `Nowhere` is none of the workbook's sheets"),
        // refute round 11: counts the parser's sums overflowed on, or that pass the table's rows,
        // and a range whose corners are out of order
        ("header_row_count", "2147483647", "workbook.toml: `tables.Table3.header_row_count` 2147483647 is none of Excel's (0 or 1)"),
        ("header_row_count", "50", "workbook.toml: `tables.Table3.header_row_count` 50 is none of Excel's (0 or 1)"),
        ("totals_row_count", "2147483648", "workbook.toml: `tables.Table3.totals_row_count` 2147483648 is none of Excel's (0 or 1)"),
        ("totals_row_count", "50", "workbook.toml: `tables.Table3.totals_row_count` 50 is none of Excel's (0 or 1)"),
        ("reference", "\"E6:A1\"", "workbook.toml: `tables.Table3.reference` `E6:A1` has its corners out of order"),
        ("reference", "\"A1:E1\"", "workbook.toml: `tables.Table3` holds no data row"),
        // refute round 12: its sheet in another case than the sheet's, and columns other than its
        // range's width (one past the sheet's edge)
        ("sheet_name", "\"HIDDEN\"", "workbook.toml: `tables.Table3.sheet_name` `HIDDEN` is none of the workbook's sheets, as the sheet spells its name"),
        ("reference", "\"A1:B6\"", "workbook.toml: `tables.Table3.columns` holds"),
        ("reference", "\"XFB1:XFD6\"", "workbook.toml: `tables.Table3.columns` holds"),
    ] {
        for naming in [None, Some("SUM(Table3[The one])"), Some("SUM(Table3[#Data])"), Some("SUM(Table3[#All])")] {
            let parts: Vec<(String, Vec<u8>)> = good
                .iter()
                .map(|(n, b)| match n.as_str() {
                    "workbook.toml" => (n.clone(), in_table(key, value).into_bytes()),
                    _ if *n == sheet_part && naming.is_some() => {
                        let sheet = String::from_utf8(b.clone()).unwrap();
                        (n.clone(), format!("{sheet}ZZ999 = {{ f = \"{}\" }}\n", naming.unwrap()).into_bytes())
                    }
                    _ => (n.clone(), b.clone()),
                })
                .collect();
            std::fs::write(&path, abacofile::zip_of(&parts).unwrap()).unwrap();
            let got = abacofile::read(&path).err().map(|e| e.to_string()).unwrap_or_else(|| "read, not refused".into());
            assert!(got.contains(want), "{key} = {value}, naming {naming:?}: {got}");
        }
    }
    // the writer: a model whose table holds such a range or sheet, set past the engine's API
    for (reference, sheet, want) in [("nonsense", None, "`tables.Table3.reference` `nonsense` is no range of cells"), ("A1:E6", Some("Nowhere"), "`tables.Table3.sheet_name` `Nowhere` is none of the workbook's sheets")] {
        let mut book = open(&src);
        let table = book.model_mut().workbook.tables.get_mut("Table3").unwrap();
        table.reference = reference.to_string();
        if let Some(sheet) = sheet {
            table.sheet_name = sheet.to_string();
        }
        let got = abacofile::write(&mut AbacoFile::new(book), &path).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(got.contains(want), "{got}");
    }
    let mut book = open(&src);
    book.model_mut().workbook.tables.get_mut("Table3").unwrap().header_row_count = 50;
    let got = abacofile::write(&mut AbacoFile::new(book), &path).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(got.contains("`tables.Table3.header_row_count` 50 is none of Excel's (0 or 1)"), "{got}");
    let mut book = open(&src);
    let upper = book.model().workbook.tables["Table3"].sheet_name.to_uppercase();
    book.model_mut().workbook.tables.get_mut("Table3").unwrap().sheet_name = upper;
    let got = abacofile::write(&mut AbacoFile::new(book), &path).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(got.contains("`tables.Table3.sheet_name`") && got.contains("as the sheet spells its name"), "{got}");
    let mut book = open(&src);
    book.model_mut().workbook.tables.get_mut("Table3").unwrap().reference = "A1:B6".into();
    let got = abacofile::write(&mut AbacoFile::new(book), &path).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(got.contains("`tables.Table3.columns` holds"), "{got}");
}

#[test]
fn a_table_s_sheet_renamed_or_deleted_by_the_engine_writes_and_reads_back() {
    // the engine's own rename and delete carry a sheet's tables (fork change 38), so the workbook
    // they leave is one both ends take (refute round 11)
    let src = corpus_dir().join("calc_test_no_export/tables.xlsx");
    let dir = tempfile::tempdir().unwrap();
    let table_sheet = open(&src).model().workbook.tables["Table3"].sheet_name.clone();
    for edit in ["Escondido", "lower", "delete"] {
        let mut book = open(&src);
        let index = book.model().workbook.worksheets.iter().position(|w| w.name == table_sheet).unwrap() as u32;
        match edit {
            "delete" => book.model_mut().delete_sheet(index).unwrap(),
            "lower" => book.model_mut().rename_sheet_by_index(index, &table_sheet.to_lowercase()).unwrap(),
            name => book.model_mut().rename_sheet_by_index(index, name).unwrap(),
        }
        let mut file = AbacoFile::new(book);
        let path = dir.path().join("renamed.abaco");
        abacofile::write(&mut file, &path).unwrap_or_else(|e| panic!("{edit}: {e}"));
        let first = std::fs::read(&path).unwrap();
        let mut back = read(&path);
        assert_eq!(differences(&file.book, &back.book), Vec::<String>::new(), "{edit}");
        let again = dir.path().join("again.abaco");
        abacofile::write(&mut back, &again).unwrap();
        assert_eq!(std::fs::read(&again).unwrap(), first, "{edit}");
    }
}

#[test]
fn an_error_in_a_formula_reads_back_as_itself() {
    // `#N/IMPL!`, which the engine spelled `#N/IMPL` (fork change 36): typed and stated in a file,
    // read back with no difference and written again to the same bytes
    let mut book = Workbook::new("Errors").unwrap();
    for (cell, f) in [("A1", "=#N/IMPL!"), ("A2", "=IFERROR(#N/IMPL!,1)"), ("A3", "=ISERROR({#N/IMPL!,1})")] {
        book.set_formula(0, Addr::parse(cell).unwrap(), f).unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("errors.abaco");
    let mut file = AbacoFile::new(book);
    abacofile::write(&mut file, &path).unwrap();
    let first = std::fs::read(&path).unwrap();
    let mut back = read(&path);
    assert_eq!(differences(&file.book, &back.book), Vec::<String>::new());
    assert_eq!(back.book.value_now(0, Addr::parse("A2").unwrap()).unwrap(), Value::Number(1.0));
    let again = dir.path().join("again.abaco");
    abacofile::write(&mut back, &again).unwrap();
    assert_eq!(std::fs::read(&again).unwrap(), first);
    let sheet = String::from_utf8(abacofile::parts(&back).unwrap().into_iter().find(|(n, _)| n.starts_with("sheets/")).unwrap().1).unwrap();
    assert!(sheet.contains("f = \"IFERROR(#N/IMPL!,1)\""), "{sheet}");
}


#[test]
fn the_reader_knows_every_tagged_type_of_the_engine() {
    // refute rounds 15 and 16: the reader reads each internally tagged object of a part alone, at its
    // place (`links.N.2`, a `Link`; `conditional_formatting.N.cf_rule`, a `CfRule`), since inside
    // one serde reads from a buffer `serde_ignored` never sees; the engine's source holds no other
    // (`CfRuleInput` is its API's input, no part's), and its one untagged type, `Color`, holds no key
    fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                sources(&p, out);
            } else if p.extension().is_some_and(|e| e == "rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    for krate in ["base", "xlsx"] {
        sources(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../forks/ironcalc").join(krate).join("src"), &mut files);
    }
    let (mut tagged, mut untagged) = (Vec::new(), Vec::new());
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            let attr = l.trim();
            if !(attr.starts_with("#[serde(") && (attr.contains("tag") || attr.contains("untagged"))) {
                continue;
            }
            let item = lines[i + 1..].iter().map(|l| l.trim()).find(|l| !l.starts_with("#[") && !l.starts_with("//")).unwrap_or_default();
            let name = item.split_whitespace().skip_while(|w| *w != "enum" && *w != "struct").nth(1).unwrap_or(item).trim_end_matches('{').to_string();
            if attr.contains("untagged") {
                untagged.push(name);
            } else {
                tagged.push(name);
            }
        }
    }
    tagged.sort();
    assert_eq!(tagged, ["CfRule", "CfRuleInput", "Link"], "the engine's internally tagged types");
    assert_eq!(untagged, ["Color"], "the engine's untagged types");
    // and (refute round 16) its externally tagged enums with a variant of data, each read alone at
    // its place by the reader when a part holds it: `Cfvo` (a format's thresholds, a data bar's
    // ends) and `ExternalValue` (a linked workbook's cached cells); the others are the engine's own
    // views and tokens (`MergeStructure`, `CellArrayStructure`, `TableReference`, `TokenType`), no
    // part's
    let mut external = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            let Some(rest) = l.trim().strip_prefix("pub enum ") else { continue };
            let name = rest.split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap_or_default().to_string();
            // the attributes above it, past its doc comments
            let attrs: Vec<&str> = lines[..i].iter().rev().map(|l| l.trim()).take_while(|l| l.starts_with("#[") || l.starts_with("//")).filter(|l| l.starts_with("#[")).collect();
            let derives = attrs.iter().any(|a| a.contains("Deserialize"));
            let represented = attrs.iter().any(|a| a.contains("serde(") && (a.contains("tag") || a.contains("untagged")));
            if !derives || represented {
                continue;
            }
            // a variant of data: a line of the body, at the enum's indentation and four more, a name
            // followed by `(` or `{`
            let indent = l.len() - l.trim_start().len() + 4;
            let data = lines[i + 1..]
                .iter()
                .take_while(|v| !v.starts_with(&format!("{}}}", " ".repeat(indent - 4))))
                .filter(|v| v.len() - v.trim_start().len() == indent)
                .any(|v| {
                    let t = v.trim();
                    let head = t.split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap_or_default();
                    !head.is_empty() && (t[head.len()..].trim_start().starts_with('(') || t[head.len()..].trim_start().starts_with('{'))
                });
            if data {
                external.push(name);
            }
        }
    }
    external.sort();
    assert_eq!(external, ["CellArrayStructure", "Cfvo", "ExternalValue", "MergeStructure", "TableReference", "TokenType"], "the engine's externally tagged enums of data");
    // `Color`'s variants are a text, a tuple and none: no key of its own
    let types = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../forks/ironcalc/base/src/types.rs")).unwrap();
    let body = &types[types.find("pub enum Color {").unwrap()..];
    let body = &body[..body.find("\n}").unwrap()];
    assert!(!body.lines().skip(1).any(|l| l.trim_end().ends_with('{')), "a variant of `Color` holds keys:\n{body}");
}
