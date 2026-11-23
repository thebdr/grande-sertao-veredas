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

#[test]
fn a_file_that_is_not_what_it_should_be_is_refused_by_its_part_and_place() {
    let file = worked_example();
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
            Ok(_) => panic!("{name}: read"),
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
}
