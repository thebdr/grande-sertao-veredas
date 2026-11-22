//! The engine at parity (contract: Ábaco's parity clause; `docs/abaco-plan.md`, "The oracle and
//! parity"; `docs/questions.md` Q344).
//!
//! The corpus is every workbook Microsoft Excel saved among the fork's test files; the baseline is
//! what Veredas 1's oracle and fork measured of it on this machine
//! (`tools/abaco-oracle/baseline/veredas-1.toml`, pinned by its hash and its totals). Veredas 2's
//! engine, opened through Ábaco's link policy, must match Excel's cached value on at least as many
//! formula cells as Veredas 1's did, file by file, under the cached and the linked resolution, and
//! measure every file Veredas 1 measured. The oracle itself is checked first: values Excel never
//! cached, planted in a copy of a corpus workbook, are counted as the mismatches they are.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use abaco_oracle::{Baseline, BaselineError, Class, LinksMode, Options, Probe, corpus, excel_application, run_file, scan_raw};
use sha2::{Digest, Sha256};
use veredas_abaco::links::LinkOutcome;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn baseline() -> Baseline {
    Baseline::read(&root().join("tools/abaco-oracle/baseline/veredas-1.toml")).unwrap()
}

fn corpus_dir(b: &Baseline) -> PathBuf {
    root().join(&b.corpus)
}

/// The baseline's day, the clock every measure here is pinned to: the workbooks' formulas on
/// TODAY() (two calendars build a whole year from it) read the day Veredas 1 measured, whatever
/// day the test runs.
fn options(links: LinksMode) -> Options {
    let clock = abaco_oracle::noon_utc(&baseline().measured_on);
    assert!(clock.is_some());
    Options { links, max_examples: 3, roundtrip_dir: None, verbose: false, clock }
}

/// The workbooks of the folder the rule leaves out, each for what its package says of itself.
const NOT_EXCEL: [(&str, &str); 9] = [
    ("bad_merge_cells.xlsx", "no application, no Excel file version"),
    ("calc_tests/LOOKUP_AND_REFERENCE/XMATCH_arrays.xlsx", "IronCalc Sheets"),
    ("calc_tests/STATISTICAL/GEOMEAN.xlsx", "no application, no Excel file version"),
    ("calc_tests/array_in_scalar_repro.xlsx", "no application, no Excel file version"),
    ("libreoffice_888_example.xlsx", "LibreOffice"),
    ("missing_r_on_row.xlsx", "no application, no Excel file version"),
    ("openpyxl_example.xlsx", "openpyxl, calling itself Microsoft Excel Compatible"),
    ("optional_xf_id.xlsx", "no application, no Excel file version"),
    ("shared_formula_volatile.xlsx", "no application, no Excel file version"),
];

#[test]
fn the_corpus_is_every_workbook_excel_saved_and_the_baseline_names_each_once() {
    let b = baseline();
    assert_eq!(b.corpus, "forks/ironcalc/xlsx/tests");
    let dir = corpus_dir(&b);
    let rel = |p: &Path| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/");
    let found: Vec<String> = corpus(&dir).unwrap().iter().map(|p| rel(p)).collect();
    let listed: Vec<String> = b.file.iter().map(|f| f.path.clone()).collect();
    assert_eq!(found.len(), 257);
    assert_eq!(listed.iter().collect::<BTreeSet<_>>().len(), listed.len(), "a file listed twice");
    assert_eq!(found.iter().collect::<BTreeSet<_>>(), listed.iter().collect::<BTreeSet<_>>());
    // the rule that chose them: Excel on Windows, on the Mac, online and on Android, by the
    // package's own word
    let mut apps = BTreeSet::new();
    for f in &listed {
        apps.insert(excel_application(&dir.join(f)).unwrap());
    }
    assert_eq!(
        apps.into_iter().collect::<Vec<_>>(),
        vec!["Excel Android", "Microsoft Excel", "Microsoft Excel Online", "Microsoft Macintosh Excel"]
    );
    // every other workbook of the folder is left out, these and no more
    let mut all = Vec::new();
    let mut stack = vec![dir.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "xlsx") {
                all.push(rel(&p));
            }
        }
    }
    let left_out: BTreeSet<String> = all.iter().filter(|p| !found.contains(p)).cloned().collect();
    let expected: BTreeSet<String> = NOT_EXCEL.iter().map(|(p, _)| p.to_string()).collect();
    assert_eq!(left_out, expected);
    for (p, why) in NOT_EXCEL {
        assert_eq!(excel_application(&dir.join(p)), None, "{p}: {why}");
    }
}

#[test]
fn a_cached_value_excel_never_wrote_is_counted_as_a_mismatch() {
    // ABS.xlsx: every formula matches Excel's cache in both oracles' runs
    let b = baseline();
    let source = corpus_dir(&b).join("calc_tests/MATH_AND_TRIGONOMETRY/ABS.xlsx");
    let base = b.file.iter().find(|f| f.path == "calc_tests/MATH_AND_TRIGONOMETRY/ABS.xlsx").unwrap();
    let base = base.cached.as_ref().unwrap();
    assert_eq!(base.matched, base.formula_cells);

    let raw = scan_raw(&source).unwrap();
    let target = raw
        .formula_cells
        .iter()
        .find(|c| matches!(c.expected, abaco_oracle::Expected::Number(n) if n != 0.0))
        .unwrap()
        .clone();
    let abaco_oracle::Expected::Number(old) = target.expected else { unreachable!() };

    // copies whose one cached value is one Excel never wrote: moved by one, infinite, beyond any
    // double, not a number, a word
    let dir = tempfile::tempdir().unwrap();
    let mut probe = Probe::default();
    let moved = format!("{}", old + 1.0);
    for (i, (value, class)) in
        [(moved.as_str(), "numeric"), ("inf", "number_vs_text"), ("1E+999", "number_vs_text"), ("NaN", "number_vs_text"), ("abc", "number_vs_text")]
            .into_iter()
            .enumerate()
    {
        let planted = dir.path().join(format!("planted{i}.xlsx"));
        rewrite_cell_value(&source, &planted, &target.sheet, &target.reference, value);
        let report = run_file(&planted, &options(LinksMode::Cached), &mut probe);
        let m = report.measure().unwrap();
        assert_eq!(m.formula_cells, base.formula_cells, "{value}");
        assert_eq!(m.matched, base.matched - 1, "{value}");
        assert_eq!(m.classes.get(class), Some(&1), "{value}: {:?}", m.classes);
        let (_, examples) = report.examples.iter().next().unwrap();
        assert_eq!((examples[0].sheet.as_str(), examples[0].reference.as_str()), (target.sheet.as_str(), target.reference.as_str()));
    }
    let report = run_file(&dir.path().join("planted0.xlsx"), &options(LinksMode::Cached), &mut probe);
    assert_eq!(report.examples[&Class::Numeric][0].expected, moved);
    // the untouched original still matches whole
    let report = run_file(&source, &options(LinksMode::Cached), &mut probe);
    assert_eq!(report.measure().unwrap().matched, base.matched);
}

/// Copies the workbook at `from` to `to`, the cached `<v>` of `sheet!reference` replaced by `value`.
fn rewrite_cell_value(from: &Path, to: &Path, sheet: &str, reference: &str, value: &str) {
    let part = sheet_part(from, sheet);
    let cell = regex::Regex::new(&format!(r#"(?s)(<c r="{reference}"[^>]*>.*?<v>)([^<]*)(</v>)"#)).unwrap();
    rewrite_part(from, to, &part, |text| cell.replace(text, |c: &regex::Captures| format!("{}{value}{}", &c[1], &c[3])).to_string());
}

fn read_part(from: &Path, name: &str) -> String {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(from).unwrap()).unwrap();
    let mut s = String::new();
    archive.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

/// The part that holds `sheet`, through the workbook and its relationships.
fn sheet_part(from: &Path, sheet: &str) -> String {
    let workbook = read_part(from, "xl/workbook.xml");
    let rels = read_part(from, "xl/_rels/workbook.xml.rels");
    let rid = regex::Regex::new(&format!(r#"<sheet\s[^>]*?name="{}"[^>]*?r:id="([^"]+)""#, regex::escape(sheet)))
        .unwrap()
        .captures(&workbook)
        .unwrap()[1]
        .to_string();
    let target = regex::Regex::new(&format!(r#"Id="{}"[^>]*?Target="([^"]+)""#, regex::escape(&rid)))
        .unwrap()
        .captures(&rels)
        .map(|c| c[1].to_string())
        .unwrap_or_else(|| {
            regex::Regex::new(&format!(r#"Target="([^"]+)"[^>]*?Id="{}""#, regex::escape(&rid))).unwrap().captures(&rels).unwrap()[1].to_string()
        });
    format!("xl/{}", target.trim_start_matches("/xl/"))
}

/// Copies the package at `from` to `to`, the text of `part` passed through `edit`, which must change
/// it; every other part copied as it is.
fn rewrite_part(from: &Path, to: &Path, part: &str, edit: impl Fn(&str) -> String) {
    let out = std::fs::File::create(to).unwrap();
    let mut writer = zip::ZipWriter::new(out);
    let mut archive = zip::ZipArchive::new(std::fs::File::open(from).unwrap()).unwrap();
    let mut changed = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let name = entry.name().unwrap().to_string();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        if name == part {
            let text = String::from_utf8(bytes).unwrap();
            let replaced = edit(&text);
            assert_ne!(replaced, text, "{part} unchanged");
            changed += 1;
            bytes = replaced.into_bytes();
        }
        writer.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(&bytes).unwrap();
    }
    writer.finish().unwrap();
    assert_eq!(changed, 1);
}

#[test]
fn a_package_that_only_claims_excel_is_not_in_the_corpus() {
    let b = baseline();
    let source = corpus_dir(&b).join("calc_tests/logical.xlsx");
    assert_eq!(excel_application(&source).as_deref(), Some("Microsoft Excel"));
    let dir = tempfile::tempdir().unwrap();
    let forged: [(&str, &str, &str, &str); 6] = [
        ("compatible", "docProps/app.xml", "<Application>Microsoft Excel</Application>", "<Application>Microsoft Excel Compatible / Openpyxl 3.1.5</Application>"),
        ("other name", "docProps/app.xml", "<Application>Microsoft Excel</Application>", "<Application>Microsoft Excel 2016</Application>"),
        ("version 12", "docProps/app.xml", "<AppVersion>16.0300</AppVersion>", "<AppVersion>12.0000</AppVersion>"),
        ("no version", "docProps/app.xml", "<AppVersion>16.0300</AppVersion>", ""),
        ("edited by 4", "xl/workbook.xml", "lastEdited=\"7\"", "lastEdited=\"4\""),
        ("another app's file version", "xl/workbook.xml", "appName=\"xl\"", "appName=\"xlsxwriter\""),
    ];
    for (what, part, from, to) in forged {
        let path = dir.path().join(format!("{}.xlsx", what.replace(' ', "_")));
        rewrite_part(&source, &path, part, |text| text.replacen(from, to, 1));
        assert_eq!(excel_application(&path), None, "{what}");
    }
    // the same package, copied with no change, is still Excel's
    let copy = dir.path().join("copy.xlsx");
    rewrite_part(&source, &copy, "docProps/app.xml", |text| format!("{text}\n"));
    assert_eq!(excel_application(&copy).as_deref(), Some("Microsoft Excel"));
    // the corpus reads a folder whole, its subfolders included, and finds the copy
    let found = corpus(dir.path()).unwrap();
    assert_eq!(found, vec![copy]);
}

#[test]
fn the_baseline_is_read_through_core_s_header_reader_and_refuses_a_stray_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.toml");
    std::fs::write(&path, "format = \"veredas-abaco-prefs\"\nversion = 1\n").unwrap();
    let err = Baseline::read(&path).unwrap_err();
    assert!(matches!(err, BaselineError::Header { .. }), "{err}");
    assert!(err.to_string().contains("`format` is `veredas-abaco-prefs`, not `veredas-abaco-oracle-baseline`"), "{err}");
    std::fs::write(&path, "format = \"veredas-abaco-oracle-baseline\"\nversion = 2\n").unwrap();
    assert!(Baseline::read(&path).unwrap_err().to_string().contains("version 2"), "version");
    std::fs::write(
        &path,
        "format = \"veredas-abaco-oracle-baseline\"\nversion = 1\nmeasured_by = \"x\"\nmeasured_on = \"y\"\ncorpus = \"z\"\nfile = []\nmeasure_by = \"typo\"\n",
    )
    .unwrap();
    let err = Baseline::read(&path).unwrap_err();
    assert!(matches!(err, BaselineError::Body { .. }), "{err}");
    assert!(err.to_string().contains("measure_by"), "{err}");
}

/// The baseline as taken on 2026-11-22 (`.zen/evidence/C-222.md`): any change to the file, a
/// re-measure or a hand's edit, fails here until it is reviewed and this pin moved with it.
const BASELINE_SHA256: &str = "b39b985f8f510a512fb9f39923cde850ce49f6217dbe4829a4713993845591be";

#[test]
fn the_baseline_is_veredas_1_s_measure_as_taken_and_adds_up() {
    let bytes = std::fs::read(root().join("tools/abaco-oracle/baseline/veredas-1.toml")).unwrap();
    let hash: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hash, BASELINE_SHA256);
    let b = baseline();
    assert!(b.measured_by.starts_with("Veredas 1's abaco-oracle"), "{}", b.measured_by);
    assert_eq!(b.measured_on, "2026-11-22");
    let unmeasured: Vec<&str> = b.file.iter().filter(|f| f.unmeasured.is_some()).map(|f| f.path.as_str()).collect();
    assert_eq!(unmeasured, vec!["calc_tests/LOOKUP_AND_REFERENCE/INDIRECT.xlsx"]);
    for (mode, pick) in [("cached", 0), ("linked", 1)] {
        let mut files = 0;
        let (mut formulas, mut matched) = (0usize, 0usize);
        for f in &b.file {
            let m = if pick == 0 { f.cached.as_ref() } else { f.linked.as_ref() };
            let Some(m) = m else {
                assert!(f.unmeasured.is_some(), "{}: no numbers under {mode} and no reason", f.path);
                continue;
            };
            assert!(f.unmeasured.is_none(), "{}", f.path);
            assert_eq!(m.classes.values().sum::<usize>(), m.formula_cells, "{} {mode}", f.path);
            assert_eq!(m.classes.get("match").copied().unwrap_or(0), m.matched, "{} {mode}", f.path);
            let known: BTreeSet<&str> = Class::ALL.iter().map(|c| c.id()).collect();
            assert!(m.classes.keys().all(|k| known.contains(k.as_str())), "{} {mode}: {:?}", f.path, m.classes);
            files += 1;
            formulas += m.formula_cells;
            matched += m.matched;
        }
        assert_eq!((files, formulas, matched), (256, 76_245, 75_743), "{mode}");
    }
}

#[test]
fn the_engine_s_clock_is_pinned_to_the_baseline_s_day_while_a_workbook_is_measured() {
    assert_eq!(abaco_oracle::noon_utc("1970-01-01"), Some(12 * 3_600_000));
    assert_eq!(abaco_oracle::noon_utc("2026-11-22"), Some(20_779 * 86_400_000 + 12 * 3_600_000));
    assert_eq!(abaco_oracle::noon_utc("2000-02-29"), Some(11_016 * 86_400_000 + 12 * 3_600_000));
    assert_eq!(abaco_oracle::noon_utc("2024-02-29"), Some(19_782 * 86_400_000 + 12 * 3_600_000));
    for bad in ["2026-13-01", "2026-11", "2026-11-22-1", "x", "", "2026-02-29", "2026-02-31", "2026-04-31", "1900-02-29", "2026-00-10"] {
        assert_eq!(abaco_oracle::noon_utc(bad), None, "{bad}");
    }
    // a workbook with TODAY() and YEAR(TODAY()), written and measured with the clock pinned to a
    // day that is not today: the engine reads that day (Excel's serial 46348 is 2026-11-22)
    let dir = tempfile::tempdir().unwrap();
    let mut model = ironcalc_base::Model::new_empty("today.xlsx", "en", "UTC", "en").unwrap();
    model.set_user_input(0, 1, 1, "=TODAY()".to_string()).unwrap();
    model.set_user_input(0, 2, 1, "=YEAR(TODAY())".to_string()).unwrap();
    ironcalc_base::pin_clock(abaco_oracle::noon_utc("2026-11-22"));
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 1, 1).unwrap(), "11/22/2026");
    assert_eq!(model.get_cell_value_by_index(0, 1, 1).unwrap(), ironcalc_base::cell::CellValue::Number(46348.0));
    assert_eq!(model.get_formatted_cell_value(0, 2, 1).unwrap(), "2026");
    let path = dir.path().join("today.xlsx");
    ironcalc::export::save_to_xlsx(&model, path.to_str().unwrap()).unwrap();
    ironcalc_base::pin_clock(None);
    // measured on the pinned day: both match what was cached; on another day: neither TODAY
    let mut probe = Probe::default();
    let on_the_day = run_file(&path, &options(LinksMode::Cached), &mut probe);
    assert_eq!(on_the_day.measure().unwrap().matched, 2);
    let mut later = options(LinksMode::Cached);
    later.clock = abaco_oracle::noon_utc("2027-03-01");
    let report = run_file(&path, &later, &mut probe);
    assert_eq!(report.measure().unwrap().matched, 0);
    // and the pin is released when the measure ends, to what it was before it
    assert_eq!(ironcalc_base::pinned_clock(), None);
    ironcalc_base::pin_clock(Some(7));
    run_file(&path, &later, &mut probe);
    assert_eq!(ironcalc_base::pinned_clock(), Some(7));
    ironcalc_base::pin_clock(None);
}

/// The engine against the baseline under one resolution of the links, file by file.
fn parity(links_mode: LinksMode) {
    let b = baseline();
    let dir = corpus_dir(&b);
    let mut probe = Probe::default();
    let opts = options(links_mode);
    let mut short = Vec::new();
    let (mut base_matched, mut matched, mut formulas, mut better) = (0usize, 0usize, 0usize, Vec::new());
    let mut links: BTreeMap<String, LinkOutcome> = BTreeMap::new();
    for f in &b.file {
        let report = run_file(&dir.join(&f.path), &opts, &mut probe);
        if let Some(outcome) = &report.link_outcome
            && *outcome != LinkOutcome::NoLinks
        {
            links.insert(f.path.clone(), outcome.clone());
        }
        let now = report.measure();
        let then = match links_mode {
            LinksMode::Cached => f.cached.as_ref(),
            LinksMode::Linked => f.linked.as_ref(),
            LinksMode::Ask => unreachable!(),
        };
        match (then, now) {
            (Some(then), Some(now)) => {
                base_matched += then.matched;
                matched += now.matched;
                formulas += now.formula_cells;
                if now.formula_cells != then.formula_cells {
                    short.push(format!("{}: {} formula cells read, Veredas 1 read {}", f.path, now.formula_cells, then.formula_cells));
                }
                if now.matched < then.matched {
                    short.push(format!(
                        "{}: {} matched, Veredas 1 matched {} ({:?} against {:?})",
                        f.path, now.matched, then.matched, now.classes, then.classes
                    ));
                } else if now.matched > then.matched {
                    better.push(format!("{}: {} matched, Veredas 1 {}", f.path, now.matched, then.matched));
                }
            }
            (Some(_), None) => short.push(format!(
                "{}: not measured ({}), Veredas 1 measured it",
                f.path,
                report.load_error.or(report.raw_error).unwrap_or_default()
            )),
            (None, Some(now)) => {
                assert!(f.unmeasured.is_some(), "{}: no numbers and no reason in the baseline", f.path);
                // the one file Veredas 1 could not measure: every formula cell read and matched
                assert_eq!(f.path, "calc_tests/LOOKUP_AND_REFERENCE/INDIRECT.xlsx");
                assert_eq!((now.formula_cells, now.matched), (114, 114), "{}", f.path);
                formulas += now.formula_cells;
                matched += now.matched;
                better.push(format!("{}: measured ({} of {} matched), Veredas 1 could not", f.path, now.matched, now.formula_cells));
            }
            (None, None) => short.push(format!("{}: measured by neither oracle", f.path)),
        }
    }
    eprintln!(
        "parity ({}): {matched} of {formulas} formula cells match Excel's cache; Veredas 1 matched {base_matched} on the files it measured",
        links_mode.label()
    );
    for line in &better {
        eprintln!("  better: {line}");
    }
    assert!(short.is_empty(), "below Veredas 1's numbers:\n{}", short.join("\n"));
    assert!(matched >= base_matched);
    // Veredas 2's own totals as measured on 2026-11-22: a change is a decision, made here
    assert_eq!((matched, formulas, base_matched), (75_857, 76_359, 75_743));
    // the corpus's workbooks with links: three, none of whose linked workbooks is here, so the
    // linked resolution reports each unavailable and answers from its cache (C-221's tests open
    // present ones; a plant's real linked workbooks are owed, Q344)
    let linked: Vec<&String> = links.keys().collect();
    assert_eq!(
        linked,
        vec!["calc_tests/INFORMATION/SHEET_SHEETS.xlsx", "calc_tests/MATH_AND_TRIGONOMETRY/COMBIN_COMBINA.xlsx", "link_test.xlsx"]
    );
    for (path, outcome) in &links {
        match (links_mode_is_linked(links_mode), outcome) {
            (false, LinkOutcome::Cached { books }) => assert!(*books >= 1, "{path}"),
            (true, LinkOutcome::Linked(statuses)) => {
                assert!(!statuses.is_empty(), "{path}");
                for s in statuses {
                    assert!(matches!(s.state, ironcalc::import::LinkState::Unavailable(_)), "{path}: {s:?}");
                }
            }
            (_, other) => panic!("{path}: {other:?}"),
        }
    }
}

fn links_mode_is_linked(mode: LinksMode) -> bool {
    mode == LinksMode::Linked
}

#[test]
fn parity_under_the_cached_resolution() {
    parity(LinksMode::Cached);
}

#[test]
fn parity_under_the_linked_resolution() {
    parity(LinksMode::Linked);
}
