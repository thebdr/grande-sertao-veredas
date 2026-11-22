//! The patcher on real workbooks (contract: Ábaco's patcher clause): every workbook among the
//! fork's test files, most of them saved by Excel, patched twice.
//!
//! - With nothing to do, every part of the package comes out as it went in.
//! - With one number cell changed (the first number of the first sheet that holds one), every
//!   part but the worksheets comes out as it went in, the calculation chain at most dropped; in
//!   every worksheet, every `<c>` element but the edited one and formulas whose value the edit
//!   moved comes out byte for byte; the patcher's own verification re-reads the written file
//!   through the engine and finds every cell as the model left it.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use veredas_abaco::links::{LinkChoice, LinkPolicy};
use veredas_abaco::{Addr, Content, Value, Workbook};
use veredas_abaco::patch::cached::{read_elements, read_sheets};
use veredas_abaco::patch::{Edit, PatchOptions, Status, patch};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../forks/ironcalc/xlsx/tests").canonicalize().unwrap()
}

/// Workbooks upstream's own tests write into the folder while the fork's suite runs, and remove.
const SCRATCH: [&str; 2] = ["conditional_formatting/cf_tests_round_trip.xlsx", "templates/crossword_round_trip.xlsx"];

/// Every workbook of the folder, by its path below it, sorted.
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

/// Every part of a package, by name, its uncompressed bytes.
fn parts(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        out.insert(name, b);
    }
    out
}

fn options() -> PatchOptions {
    PatchOptions { verify: true, ..Default::default() }
}

/// The workbooks the patcher refuses, each with the start of its reason: not a workbook it can
/// read, the way the engine refuses them too.
const REFUSED: [(&str, &str); 0] = [];

#[test]
fn with_nothing_to_do_every_part_comes_out_as_it_went_in() {
    let dir = tempfile::tempdir().unwrap();
    let mut patched = 0;
    let mut refused = Vec::new();
    for rel in corpus() {
        let src = corpus_dir().join(&rel);
        let dst = dir.path().join("out.xlsx");
        let _ = std::fs::remove_file(&dst);
        match patch(&src, &dst, &[], &options()) {
            Ok(report) => {
                let (before, after) = (parts(&src), parts(&dst));
                let differing: Vec<&String> = before
                    .keys()
                    .chain(after.keys())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .filter(|k| before.get(*k) != after.get(*k))
                    .collect();
                assert!(differing.is_empty(), "{rel}: {differing:?} differ, first: {}", first_difference(&before, &after, &differing));
                assert!(report.outcomes.is_empty(), "{rel}");
                patched += 1;
            }
            Err(e) => refused.push((rel, e.to_string())),
        }
    }
    let expected: Vec<&str> = REFUSED.iter().map(|(p, _)| *p).collect();
    let got: Vec<&str> = refused.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(got, expected, "refused: {refused:?}");
    for ((_, why), (_, start)) in refused.iter().zip(REFUSED.iter()) {
        assert!(why.starts_with(start), "{why}");
    }
    assert_eq!(patched + refused.len(), corpus().len());
    assert!(patched >= 250, "{patched}");
}

#[test]
fn one_number_changed_changes_its_sheet_alone() {
    let dir = tempfile::tempdir().unwrap();
    let mut patched = 0;
    let mut without_a_number = Vec::new();
    for rel in corpus() {
        if REFUSED.iter().any(|(p, _)| *p == rel) {
            continue;
        }
        let src = corpus_dir().join(&rel);
        // the first number of the first sheet holding one: a number the file states with no
        // formula and the engine holds as a plain value too (not a cell an array spilled into)
        let sheets = read_sheets(&src, |_| true).unwrap();
        let wb = Workbook::open(&src, LinkPolicy::AlwaysUseCached, |_| LinkChoice::UseCached).unwrap();
        let target = sheets.iter().find_map(|(name, cells)| {
            let index = wb.sheet_index(name).ok()?;
            cells.iter().find_map(|((row, col), c)| match (&c.formula, c.shared, &c.value) {
                (None, false, Value::Number(n))
                    if wb.content(index, Addr::new(*row, *col)).ok()? == Content::Value(Value::Number(*n)) =>
                {
                    Some((name.clone(), *row, *col, *n))
                }
                _ => None,
            })
        });
        let Some((sheet, row, col, n)) = target else {
            without_a_number.push(rel);
            continue;
        };
        let cell = format!("{}{row}", column_letters(col));
        // a number that is not the one there (n + 1 is n itself for a number past 2^53)
        let new = if n == 1234.5 { 4321.5 } else { 1234.5 };
        let edit = Edit::Value { sheet: sheet.clone(), cell: cell.clone(), value: serde_json::json!(new), expect: None };
        let dst = dir.path().join("out.xlsx");
        let _ = std::fs::remove_file(&dst);
        let report = patch(&src, &dst, &[edit], &options()).unwrap_or_else(|e| panic!("{rel}: {e}"));
        assert_eq!(report.outcomes.len(), 1, "{rel}");
        assert_eq!(report.outcomes[0].status, Status::Applied, "{rel}");
        // the parts: only worksheets differ (the edited one, and one whose formulas read the
        // edited cell), the calculation chain at most dropped; the edited sheet's part did change
        let before = parts(&src);
        let after = parts(&dst);
        let elements = read_elements(&std::fs::read(&src).unwrap()).unwrap();
        let sheet_parts: BTreeSet<&String> = elements.iter().map(|(_, p, _)| p).collect();
        let sheet_part = elements.iter().find(|(n, _, _)| *n == sheet).map(|(_, p, _)| p.clone()).unwrap();
        assert_ne!(before.get(&sheet_part), after.get(&sheet_part), "{rel}: the edited sheet unchanged");
        for (name, bytes) in &before {
            match after.get(name) {
                Some(b) if b == bytes => {}
                Some(_) => assert!(sheet_parts.contains(name), "{rel}: {name} rewritten"),
                None => assert_eq!(name, "xl/calcChain.xml", "{rel}: {name} dropped"),
            }
        }
        assert!(after.keys().all(|k| before.contains_key(k)), "{rel}: a part added");
        // inside the sheet: the edited cell changed; any other changed cell is one the engine
        // computes (a formula, or a cell an array spilled into) whose value the edit moved, its
        // formula text as it was; every other cell as it was
        let after_elements = read_elements(&std::fs::read(&dst).unwrap()).unwrap();
        let wb_after = Workbook::open(&dst, LinkPolicy::AlwaysUseCached, |_| LinkChoice::UseCached).unwrap();
        for ((name, _, cells), (_, _, cells_after)) in elements.iter().zip(after_elements.iter()) {
            for (at, el) in cells {
                let now = cells_after.get(at);
                if *at == (row, col) && *name == sheet {
                    assert_ne!(now, Some(el), "{rel}: {cell} not written");
                    continue;
                }
                if now != Some(el) {
                    let i = wb.sheet_index(name).unwrap();
                    let a = Addr::new(at.0, at.1);
                    let (was, is) = (wb.value_now(i, a).unwrap(), wb_after.value_now(i, a).unwrap());
                    assert_ne!(was, is, "{rel}: {name}!{at:?} rewritten and its value did not move: {el} → {now:?}");
                    match formula_of(el) {
                        Some(f) => assert_eq!(now.and_then(|n| formula_of(n)), Some(f), "{rel}: {name}!{at:?} formula changed"),
                        None => assert!(
                            computed(&wb, i, a),
                            "{rel}: {name}!{at:?} a value the engine does not compute rewritten: {el} → {now:?}"
                        ),
                    }
                }
            }
            assert!(cells_after.keys().all(|k| cells.contains_key(k)), "{rel}: a cell added in {name}");
        }
        // the patcher's own verification read every cell back
        assert!(report.verified_cells > 0, "{rel}");
        patched += 1;
    }
    eprintln!("{patched} workbooks patched; {} hold no number to change: {without_a_number:?}", without_a_number.len());
    assert!(patched >= 200, "{patched}");
}

/// The structural edits every workbook takes, one at a time, on its first worksheet: rows and
/// columns inserted and deleted, a defined name set, global and local.
fn structural(sheet: &str) -> Vec<(&'static str, Edit)> {
    let s = sheet.to_string();
    let target = format!("='{}'!$A$1", sheet.replace('\'', "''"));
    vec![
        ("insert row 2", Edit::InsertRows { sheet: s.clone(), at: 2, count: 1 }),
        ("delete row 2", Edit::DeleteRows { sheet: s.clone(), at: 2, count: 1 }),
        ("insert column B", Edit::InsertColumns { sheet: s.clone(), at: "B".into(), count: 1 }),
        ("delete column B", Edit::DeleteColumns { sheet: s.clone(), at: "B".into(), count: 1 }),
        ("set a name", Edit::SetName { name: "AbacoProbe".into(), formula: target.clone(), scope: None }),
        ("set a local name", Edit::SetName { name: "AbacoProbe".into(), formula: target, scope: Some(s) }),
    ]
}

/// The structural edits the patcher refuses on a corpus workbook, by workbook, edit and the start
/// of the reason: what it does not carry yet, said before anything is written.
const STRUCTURAL_REFUSED: [(&str, &str, &str); 13] = [
    // upstream's malformed merges: C2 holds a value under the merge B2:C3, which the engine does
    // not show; with column B deleted it is the merge's first cell, as Excel keeps it, and the
    // patcher's verification refuses a file the engine reads otherwise than its model
    ("bad_merge_cells.xlsx", "delete column B", "verification failed: 1 cell(s) differ; first: Feuille 1!B2: model Empty"),
    ("calc_test_no_export/tables.xlsx", "insert row 2", "not supported by the patcher yet: the move cuts into table Table1"),
    ("calc_test_no_export/tables.xlsx", "delete row 2", "not supported by the patcher yet: the move cuts into table Table1"),
    ("calc_test_no_export/tables.xlsx", "insert column B", "not supported by the patcher yet: the move cuts into table Table1"),
    ("calc_test_no_export/tables.xlsx", "delete column B", "not supported by the patcher yet: the move cuts into table Table1"),
    // the engine's own refusals, Excel's: a cell in the last column, an array cut in two
    ("calc_tests/LOOKUP_AND_REFERENCE/ROW_COLUM.xlsx", "insert column B", "edit 0 (insert_columns): Cannot shift cells because that would delete cells at the end of a row"),
    ("calc_tests/spill_operator.xlsx", "delete row 2", "edit 0 (delete_rows): Cannot delete rows because that would break an array formula"),
    ("dynamic_arrays.xlsx", "insert row 2", "edit 0 (insert_rows): Cannot insert rows because that would break an array formula"),
    ("dynamic_arrays.xlsx", "delete row 2", "edit 0 (delete_rows): Cannot delete rows because that would break an array formula"),
    ("example.xlsx", "insert row 2", "not supported by the patcher yet: rows/columns moved in a workbook with pivot tables"),
    ("example.xlsx", "delete row 2", "not supported by the patcher yet: rows/columns moved in a workbook with pivot tables"),
    ("example.xlsx", "insert column B", "not supported by the patcher yet: rows/columns moved in a workbook with pivot tables"),
    ("example.xlsx", "delete column B", "not supported by the patcher yet: rows/columns moved in a workbook with pivot tables"),
];

#[test]
fn structural_edits_are_verified_or_refused_and_a_refusal_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut verified = 0;
    let mut refused: Vec<(String, &str, String)> = Vec::new();
    for rel in corpus() {
        let src = corpus_dir().join(&rel);
        let wb = Workbook::open(&src, LinkPolicy::AlwaysUseCached, |_| LinkChoice::UseCached).unwrap();
        let first = wb.sheet_names()[0].clone();
        for (label, edit) in structural(&first) {
            let dst = dir.path().join("out.xlsx");
            let _ = std::fs::remove_file(&dst);
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| patch(&src, &dst, &[edit], &options())));
            match run {
                Ok(Ok(report)) => {
                    assert_eq!(report.outcomes.len(), 1, "{rel}: {label}");
                    assert!(report.verified_cells > 0 || wb.cells(0).unwrap().is_empty(), "{rel}: {label}: nothing verified");
                    assert!(dst.exists(), "{rel}: {label}: nothing written");
                    verified += 1;
                }
                Ok(Err(e)) => {
                    // a refusal leaves the destination as it was: here, absent
                    assert!(!dst.exists(), "{rel}: {label}: refused ({e}) and written");
                    refused.push((rel.clone(), label, e.to_string()));
                }
                Err(_) => refused.push((rel.clone(), label, "PANIC".into())),
            }
        }
        // nothing left beside the destination
        let left: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).filter(|n| n != "out.xlsx").collect();
        assert!(left.is_empty(), "{rel}: {left:?} left beside the destination");
    }
    for r in &refused {
        eprintln!("REFUSED {} | {} | {}", r.0, r.1, r.2.chars().take(300).collect::<String>());
    }
    let got: Vec<(&str, &str)> = refused.iter().map(|(p, l, _)| (p.as_str(), *l)).collect();
    let expected: Vec<(&str, &str)> = STRUCTURAL_REFUSED.iter().map(|(p, l, _)| (*p, *l)).collect();
    assert_eq!(got, expected, "{} refused", refused.len());
    for ((_, _, why), (_, _, start)) in refused.iter().zip(STRUCTURAL_REFUSED.iter()) {
        assert!(why.starts_with(start), "{why}");
    }
    eprintln!("{verified} structural edits verified");
}

/// The text of a `<c>` element's `<f>`, empty for a shared child's `<f …/>`; `None` without one.
fn formula_of(el: &str) -> Option<&str> {
    let at = el.find("<f")?;
    let open_end = at + el[at..].find('>')?;
    if el[..open_end].ends_with('/') {
        return Some("");
    }
    let close = el[open_end..].find("</f>")? + open_end;
    Some(&el[open_end + 1..close])
}

/// Whether the engine computes the cell's value though the file holds no formula there: a cell
/// an array spilled into.
fn computed(wb: &Workbook, sheet: u32, a: Addr) -> bool {
    wb.model()
        .workbook
        .worksheet(sheet)
        .ok()
        .and_then(|ws| ws.cell(a.row, a.column))
        .is_some_and(|c| matches!(c.as_ref(), ironcalc_base::types::Cell::SpillCell { .. }))
}

fn column_letters(mut col: i32) -> String {
    let mut s = Vec::new();
    while col > 0 {
        let r = ((col - 1) % 26) as u8;
        s.push((b'A' + r) as char);
        col = (col - 1) / 26;
    }
    s.iter().rev().collect()
}

/// Where the first differing part first differs: the bytes around it on both sides.
fn first_difference(before: &BTreeMap<String, Vec<u8>>, after: &BTreeMap<String, Vec<u8>>, names: &[&String]) -> String {
    let Some(name) = names.first() else { return String::new() };
    let (a, b) = (before.get(*name).cloned().unwrap_or_default(), after.get(*name).cloned().unwrap_or_default());
    let at = a.iter().zip(b.iter()).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
    let around = |v: &[u8]| String::from_utf8_lossy(&v[at.saturating_sub(60)..(at + 120).min(v.len())]).to_string();
    format!("{name} at byte {at}: was {:?}, is {:?}", around(&a), around(&b))
}
