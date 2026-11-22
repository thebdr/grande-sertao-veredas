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
        // inside the sheet: the edited cell changed; any other changed cell is a formula whose
        // value the edit moved; every other cell as it was
        let after_elements = read_elements(&std::fs::read(&dst).unwrap()).unwrap();
        for ((name, _, cells), (_, _, cells_after)) in elements.iter().zip(after_elements.iter()) {
            for (at, el) in cells {
                let now = cells_after.get(at);
                if *at == (row, col) && *name == sheet {
                    assert_ne!(now, Some(el), "{rel}: {cell} not written");
                    continue;
                }
                if now != Some(el) {
                    assert!(el.contains("<f"), "{rel}: {name}!{at:?} changed and is no formula: {el} → {now:?}");
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
