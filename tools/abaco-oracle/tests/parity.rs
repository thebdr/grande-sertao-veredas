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

use abaco_oracle::{Baseline, BaselineError, Class, LinksMode, Options, Probe, corpus, excel_application, run_file, scan_raw, xml_text};
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
/// day the test runs. And the random functions seeded: a RANDARRAY of integers 0 to 100 matches
/// Excel's cache by chance now and then, the same chance on every run once seeded.
fn options(links: LinksMode) -> Options {
    let clock = abaco_oracle::noon_utc(&baseline().measured_on);
    assert!(clock.is_some());
    Options { links, max_examples: 3, roundtrip_dir: None, verbose: false, clock, random_seed: Some(1) }
}

/// Workbooks upstream's own tests write into the corpus folder while the fork's suite runs (beside
/// these tests) and remove after: written by the fork's own writer, which the rule never takes.
const UPSTREAM_SCRATCH: [&str; 2] = ["conditional_formatting/cf_tests_round_trip.xlsx", "templates/crossword_round_trip.xlsx"];

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
            } else if p.extension().is_some_and(|x| x == "xlsx") && !UPSTREAM_SCRATCH.contains(&rel(&p).as_str()) {
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

#[test]
fn a_boolean_cache_excel_never_wrote_is_counted_as_a_mismatch() {
    // AND_OR_XOR.xlsx: C2 is `=OR(0)`, cached FALSE as Excel writes it, `<v>0</v>`
    let b = baseline();
    let path = "calc_tests/LOGICAL/AND_OR_XOR.xlsx";
    let source = corpus_dir(&b).join(path);
    let base = b.file.iter().find(|f| f.path == path).unwrap().cached.as_ref().unwrap();
    assert_eq!(base.matched, base.formula_cells);
    let dir = tempfile::tempdir().unwrap();
    let mut probe = Probe::default();
    for (i, value) in ["abc", "2", "", "TRUE"].into_iter().enumerate() {
        let planted = dir.path().join(format!("planted{i}.xlsx"));
        rewrite_cell_value(&source, &planted, "AND XOR OR", "C2", value);
        let report = run_file(&planted, &options(LinksMode::Cached), &mut probe);
        let m = report.measure().unwrap();
        assert_eq!((m.formula_cells, m.matched), (base.formula_cells, base.matched - 1), "{value:?}");
        let mismatch = report.examples.values().flatten().find(|e| e.sheet == "AND XOR OR" && e.reference == "C2");
        assert!(mismatch.is_some(), "{value:?}: the mismatch is not at AND XOR OR!C2");
    }
    // the one value Excel writes for TRUE: a mismatch too (the engine says FALSE); the original whole
    rewrite_cell_value(&source, &dir.path().join("true.xlsx"), "AND XOR OR", "C2", "1");
    assert_eq!(run_file(&dir.path().join("true.xlsx"), &options(LinksMode::Cached), &mut probe).measure().unwrap().matched, base.matched - 1);
    assert_eq!(run_file(&source, &options(LinksMode::Cached), &mut probe).measure().unwrap().matched, base.matched);
}

/// Every mismatch a measure of `path` finds, as `sheet!reference`, with the measure's totals.
fn mismatches(path: &Path, probe: &mut Probe) -> (BTreeSet<String>, usize, usize) {
    let mut opts = options(LinksMode::Cached);
    opts.max_examples = 1_000;
    let report = run_file(path, &opts, probe);
    let m = report.measure().unwrap();
    let at = report.examples.values().flatten().map(|e| format!("{}!{}", e.sheet, e.reference)).collect();
    (at, m.formula_cells, m.matched)
}

/// The sheet of `path` whose formula cell at `reference` caches what `is` says.
fn sheet_of(path: &Path, reference: &str, is: impl Fn(&abaco_oracle::Expected) -> bool) -> String {
    scan_raw(path).unwrap().formula_cells.into_iter().find(|c| c.reference == reference && is(&c.expected)).unwrap().sheet
}

#[test]
fn a_text_cache_is_read_as_xml_and_excel_write_it() {
    // XML's line ends and references, then Excel's `_xHHHH_` (`_x005F_` its literal underscore)
    assert_eq!(xml_text("a\r\nb\rc&#13;d &amp;&#x41; _x0001_ _x005F_x0041_ _xD83D__xDE00_"), "a\nb\nc\rd &A \u{1} _x0041_ \u{1F600}");
    // four hex digits make an escape, nothing else: a sign or a space is the text it is
    assert_eq!(xml_text("_x+041_ _x 41_ id_x+0ab_z"), "_x+041_ _x 41_ id_x+0ab_z");
    // the corpus's texts Excel escaped (CHAR's control characters, a line ending's CR LF, a literal
    // underscore escape) read as the engine computes them: every one a match
    let b = baseline();
    let dir = corpus_dir(&b);
    let mut probe = Probe::default();
    for (path, matched, at) in [
        ("calc_tests/TEXT/CHAR.xlsx", 257, &["B1", "B8", "B10", "B11", "B31"][..]),
        ("calc_tests/LOGICAL/NOT.xlsx", 87, &["A17"][..]),
        ("calc_tests/TEXT/PROPER.xlsx", 11, &["B2"][..]),
        ("calc_tests/TEXT/ENCODEURL.xlsx", 65, &["A14", "A15", "A16"][..]),
    ] {
        let (wrong, _, m) = mismatches(&dir.join(path), &mut probe);
        assert_eq!(m, matched, "{path}: {wrong:?}");
        for r in at {
            assert!(!wrong.iter().any(|w| w.ends_with(&format!("!{r}"))), "{path}: {r} counted a mismatch");
        }
    }
    // a copy whose `_x0001_` is written with its underscore escaped: the literal text, a mismatch
    let source = dir.join("calc_tests/TEXT/CHAR.xlsx");
    let sheet = sheet_of(&source, "B1", |e| *e == abaco_oracle::Expected::Text("\u{1}".into()));
    let planted = tempfile::tempdir().unwrap();
    let copy = planted.path().join("char.xlsx");
    rewrite_cell_value(&source, &copy, &sheet, "B1", "_x005F_x0001_");
    let (wrong, _, m) = mismatches(&copy, &mut probe);
    assert_eq!(m, 256);
    assert!(wrong.contains(&format!("{sheet}!B1")), "{wrong:?}");
}

#[test]
fn a_text_that_reads_like_an_error_is_a_text_on_either_side() {
    // the engine's texts `#div/0!` and the like, which Excel cached as texts: matches
    let b = baseline();
    let dir = corpus_dir(&b);
    let mut probe = Probe::default();
    for (path, at) in [
        ("calc_tests/TEXT/STRING_HANDLING.xlsx", &["C8", "D8", "G8", "L8", "O8"][..]),
        ("calc_tests/TEXT/T_VALUE_VALUETOTEXT.xlsx", &["D7"][..]),
        ("calc_tests/MATH_AND_TRIGONOMETRY/MROUND_TRUNC_INT.xlsx", &["MROUND!A28", "TRUNC!A28", "INT!A25"][..]),
    ] {
        let (wrong, _, _) = mismatches(&dir.join(path), &mut probe);
        for r in at {
            let hit = if r.contains('!') { wrong.contains(*r) } else { wrong.iter().any(|w| w.ends_with(&format!("!{r}"))) };
            assert!(!hit, "{path}: {r} counted a mismatch ({wrong:?})");
        }
    }
    // the copy whose D8 caches the error `#DIV/0!` where the engine computes the text: one mismatch
    let source = dir.join("calc_tests/TEXT/STRING_HANDLING.xlsx");
    let sheet = sheet_of(&source, "D8", |e| matches!(e, abaco_oracle::Expected::Text(t) if t.starts_with('#')));
    let (_, cells, matched) = mismatches(&source, &mut probe);
    let planted = tempfile::tempdir().unwrap();
    let copy = planted.path().join("strings.xlsx");
    rewrite_cell(&source, &copy, &sheet, "D8", |attrs, inner| {
        let f = regex::Regex::new(r"(?s)<f[^>]*?(/>|>.*?</f>)").unwrap().find(inner).unwrap().as_str().to_string();
        format!(r#"{} t="e">{f}<v>#DIV/0!</v>"#, strip_type(attrs))
    });
    let (wrong, cells_now, matched_now) = mismatches(&copy, &mut probe);
    assert_eq!((cells_now, matched_now), (cells, matched - 1));
    assert!(wrong.contains(&format!("{sheet}!D8")), "{wrong:?}");
}

#[test]
fn an_error_or_a_text_excel_never_wrote_is_a_mismatch_and_never_stops_a_measure() {
    let b = baseline();
    let dir = corpus_dir(&b);
    let mut probe = Probe::default();
    let planted = tempfile::tempdir().unwrap();
    // ABS.xlsx, matched whole: a cell made to compute an error only the engine has, cached with that
    // error's name, is no error Excel writes
    let source = dir.join("calc_tests/MATH_AND_TRIGONOMETRY/ABS.xlsx");
    let (_, cells, matched) = mismatches(&source, &mut probe);
    assert_eq!(cells, matched);
    let target = scan_raw(&source)
        .unwrap()
        .formula_cells
        .into_iter()
        .find(|c| matches!(c.expected, abaco_oracle::Expected::Number(_)) && !c.shared_copy)
        .unwrap();
    // each cached as the engine spells it on its value side (`#N/IMPL!` there)
    for (i, (formula, error)) in
        [(target.reference.as_str(), "#CIRC!"), (r#"SUM(VALUE(A1:A3&amp;""))"#, "#N/IMPL!"), (r#"FORMULATEXT("x")"#, "#ERROR!")]
            .into_iter()
            .enumerate()
    {
        let copy = planted.path().join(format!("abs{i}.xlsx"));
        rewrite_cell(&source, &copy, &target.sheet, &target.reference, |attrs, _| {
            format!(r#"{} t="e"><f>{formula}</f><v>{error}</v>"#, strip_type(attrs))
        });
        let mut opts = options(LinksMode::Cached);
        opts.max_examples = 1_000;
        let report = run_file(&copy, &opts, &mut probe);
        let m = report.measure().unwrap();
        assert_eq!((m.formula_cells, m.matched), (cells, matched - 1), "{error}");
        let e = report.examples.values().flatten().find(|e| e.reference == target.reference).unwrap();
        // the engine computes that very error: the old reading counted it a match
        assert_eq!((e.expected.as_str(), e.actual.as_str()), (format!("(never written so: {error:?})").as_str(), error), "{error}");
    }
    // ISREF.xlsx's D1, where the engine computes "": a shared-string cache that is no index, one past
    // the table, and no cached value at all, each one mismatch, none stopping the measure
    let source = dir.join("calc_tests/INFORMATION/ISREF.xlsx");
    let sheet = sheet_of(&source, "D1", |e| *e == abaco_oracle::Expected::Text(String::new()));
    let (_, cells, matched) = mismatches(&source, &mut probe);
    for (i, (t, v)) in [(Some("s"), Some("abc")), (Some("s"), Some("99999")), (None, None)].into_iter().enumerate() {
        let copy = planted.path().join(format!("isref{i}.xlsx"));
        rewrite_cell(&source, &copy, &sheet, "D1", |attrs, inner| {
            let f = regex::Regex::new(r"(?s)<f[^>]*?(/>|>.*?</f>)").unwrap().find(inner).unwrap().as_str().to_string();
            let t = t.map(|t| format!(r#" t="{t}""#)).unwrap_or_default();
            let v = v.map(|v| format!("<v>{v}</v>")).unwrap_or_default();
            format!("{}{t}>{f}{v}", strip_type(attrs))
        });
        let (wrong, cells_now, matched_now) = mismatches(&copy, &mut probe);
        assert_eq!((cells_now, matched_now), (cells, matched - 1), "{t:?} {v:?}");
        assert!(wrong.contains(&format!("{sheet}!D1")), "{t:?} {v:?}: {wrong:?}");
    }
}

#[test]
fn excel_s_spill_and_calc_errors_are_read_through_their_value_metadata() {
    // Excel writes `#SPILL!` and `#CALC!` as `#VALUE!` with value metadata pointing at a rich
    // value whose `errorType` names the error; the engine computes each as Excel did
    let b = baseline();
    let dir = corpus_dir(&b);
    let mut probe = Probe::default();
    for (path, at) in [
        ("calc_tests/INFORMATION/ISREF.xlsx", &["D7", "D9"][..]),
        ("calc_tests/LOOKUP_AND_REFERENCE/DROP_TAKE.xlsx", &["C2", "E2", "F2", "I2", "L2"][..]),
        ("calc_tests/LOOKUP_AND_REFERENCE/FILTER_ERRORS.xlsx", &["E1"][..]),
        ("calc_tests/MATH_AND_TRIGONOMETRY/MAX_MIN.xlsx", &["H42"][..]),
        ("calc_tests/MATH_AND_TRIGONOMETRY/SEQUENCE.xlsx", &["C16", "E18"][..]),
    ] {
        let (wrong, cells, matched) = mismatches(&dir.join(path), &mut probe);
        assert_eq!(matched, cells, "{path}: not whole: {wrong:?}");
        for r in at {
            assert!(scan_raw(&dir.join(path)).unwrap().formula_cells.iter().any(|c| c.reference == *r
                && matches!(&c.expected, abaco_oracle::Expected::Error(e) if e == "#SPILL!" || e == "#CALC!")), "{path}: {r}");
        }
    }
    // ISREF's D7 (`#CALC!`): its value metadata taken off, the `#VALUE!` the file then says, one
    // mismatch; its rich value's `errorType` made 8, `#SPILL!`, one mismatch
    let source = dir.join("calc_tests/INFORMATION/ISREF.xlsx");
    let sheet = sheet_of(&source, "D7", |e| *e == abaco_oracle::Expected::Error("#CALC!".into()));
    let (_, cells, matched) = mismatches(&source, &mut probe);
    let planted = tempfile::tempdir().unwrap();
    let copy = planted.path().join("no-vm.xlsx");
    rewrite_cell(&source, &copy, &sheet, "D7", |attrs, inner| {
        format!("{}>{inner}", regex::Regex::new(r#"\svm="[^"]*""#).unwrap().replace(attrs, ""))
    });
    let (wrong, cells_now, matched_now) = mismatches(&copy, &mut probe);
    assert_eq!((cells_now, matched_now), (cells, matched - 1));
    assert!(wrong.contains(&format!("{sheet}!D7")), "{wrong:?}");
    let copy = planted.path().join("spill.xlsx");
    rewrite_part(&source, &copy, "xl/richData/rdrichvalue.xml", |text| text.replacen("<rv s=\"0\"><v>13</v>", "<rv s=\"0\"><v>8</v>", 1));
    let raw = scan_raw(&copy).unwrap();
    assert_eq!(raw.formula_cells.iter().find(|c| c.reference == "D7").unwrap().expected, abaco_oracle::Expected::Error("#SPILL!".into()));
    let (wrong, _, matched_now) = mismatches(&copy, &mut probe);
    assert_eq!(matched_now, matched - 1);
    assert!(wrong.contains(&format!("{sheet}!D7")), "{wrong:?}");
}

#[test]
fn a_cache_whose_type_and_text_excel_never_pairs_matches_nothing_the_engine_computes() {
    let b = baseline();
    let dir = corpus_dir(&b);
    let mut probe = Probe::default();
    let planted = tempfile::tempdir().unwrap();
    // STRING_HANDLING's C8, the engine's text `#div/0!` as Excel cached it (`t="str"`), retyped
    let source = dir.join("calc_tests/TEXT/STRING_HANDLING.xlsx");
    let sheet = sheet_of(&source, "C8", |e| *e == abaco_oracle::Expected::Text("#div/0!".into()));
    let (_, cells, matched) = mismatches(&source, &mut probe);
    for (i, t) in [Some("e"), Some("s"), Some("b"), None].into_iter().enumerate() {
        let copy = planted.path().join(format!("c8-{i}.xlsx"));
        rewrite_cell(&source, &copy, &sheet, "C8", |attrs, inner| {
            format!("{}{}>{inner}", strip_type(attrs), t.map(|t| format!(r#" t="{t}""#)).unwrap_or_default())
        });
        let (wrong, cells_now, matched_now) = mismatches(&copy, &mut probe);
        assert_eq!((cells_now, matched_now), (cells, matched - 1), "{t:?}");
        assert!(wrong.contains(&format!("{sheet}!C8")), "{t:?}: {wrong:?}");
    }
    // ABS.xlsx's cell made to compute a text or a number, cached with a type Excel never gives it
    let source = dir.join("calc_tests/MATH_AND_TRIGONOMETRY/ABS.xlsx");
    let (_, cells, matched) = mismatches(&source, &mut probe);
    let target = scan_raw(&source)
        .unwrap()
        .formula_cells
        .into_iter()
        .find(|c| matches!(c.expected, abaco_oracle::Expected::Number(_)) && !c.shared_copy)
        .unwrap();
    let cases: [(&str, &str); 38] = [
        (r##"<f>"#CIRC!"</f>"##, r##" t="e"><f>"#CIRC!"</f><v>#CIRC!</v>"##),
        (r##"<f>"TRUE"</f>"##, r##" t="b"><f>"TRUE"</f><v>TRUE</v>"##),
        (r##"<f>"abc"</f>"##, r##" t="b"><f>"abc"</f><v>abc</v>"##),
        (r##"<f>"abc"</f>"##, r##" t="s"><f>"abc"</f><v>abc</v>"##),
        (r##"<f>"abc"</f>"##, r##"><f>"abc"</f><v>abc</v>"##),
        (r##"<f>"abc"</f>"##, r##" t="n"><f>"abc"</f><v>abc</v>"##),
        (r##"<f>"inf"</f>"##, r##"><f>"inf"</f><v>inf</v>"##),
        (r##"<f>""</f>"##, r##"><f>""</f><v/>"##),
        (r##"<f>""</f>"##, r##"><f>""</f><v></v>"##),
        ("<f>5</f>", r##" t="d"><f>5</f><v>5</v>"##),
        ("<f>5</f>", r##" t="zz"><f>5</f><v>5</v>"##),
        (r##"<f>"abc"</f>"##, r##" t="inlineStr"><f>"abc"</f><v>abc</v>"##),
        // an inline string on a formula, plain, in runs with a phonetic run, and empty
        (r##"<f>"abc"</f>"##, r##" t="inlineStr"><f>"abc"</f><is><t>abc</t></is>"##),
        (r##"<f>"abc"</f>"##, r##" t="inlineStr"><f>"abc"</f><is><r><t>ab</t></r><r><t>c</t></r><rPh sb="0" eb="1"><t>X</t></rPh></is>"##),
        (r##"<f>""</f>"##, r##" t="inlineStr"><f>""</f><is><t></t></is>"##),
        // an empty `<f ca="1"/>` outside every array: the engine computes nothing there, its value
        // the cache itself
        ("", r##"><f ca="1"/><v>999</v>"##),
        ("", r##" t="str"><f ca="1"/><v>zzz</v>"##),
        ("", r##" t="b"><f ca="1"/><v>1</v>"##),
        ("", r##" t="e"><f ca="1"/><v>#N/A</v>"##),
        // an empty type, and two values
        ("<f>5</f>", r##" t=""><f>5</f><v>5</v>"##),
        ("<f>5</f>", r##"><f>5</f><v>5</v><v>6</v>"##),
        // two value tags in any form, the self-closing among them
        ("", r##" t="str"><f>""</f><v/><v/>"##),
        ("", r##" t="str"><f>""</f><v/><v />"##),
        ("", r##" t="str"><f>"&lt;v&gt;abc"</f><v /><v>abc</v>"##),
        // an inline string in any form beside a value
        ("", r##" t="str"><f>"abc"</f><is/><v>abc</v>"##),
        ("", r##" t="str"><f>"abc"</f><v>abc</v><is xml:space="preserve"><t>abc</t></is>"##),
        ("", r##" t="str"><f>"abc"</f><v>abc</v><is ><t>abc</t></is>"##),
        // a space XSD does not take off: part of the value
        ("", r##"><f>5</f><v>5&#160;</v>"##),
        ("", r##" t="b"><f>TRUE</f><v>1&#xA0;</v>"##),
        ("", r##" t="b"><f>TRUE</f><v>&#x3000;1</v>"##),
        ("", r##" t="e"><f>NA()</f><v>#N/A&#xA0;</v>"##),
        // the value as an XML parser reads it, whatever markup spells it: the engine's text is the
        // markup, Excel's cache the text
        ("", r##" t="str"><f>""</f><v>abc</v >"##),
        ("", " t=\"str\"><f>\"\"</f><v>abc</v\n>"),
        ("", r##" t="str"><f>"&lt;![CDATA[abc]]&gt;"</f><v><![CDATA[abc]]></v>"##),
        ("", r##" t="str"><f>"ab&lt;!--x--&gt;c"</f><v>ab<!--x-->c</v>"##),
        ("", r##" t="str"><f>"&amp;#00000000065;"</f><v>&#00000000065;</v>"##),
        (r##"<f>"abc"</f>"##, r##" t="e"><f>"abc"</f><is><t>abc</t></is>"##),
        (r##"<f>"abc"</f>"##, r##"><f>"abc"</f><is><t>abc</t></is>"##),
    ];
    for (i, (_, cell)) in cases.iter().enumerate() {
        let copy = planted.path().join(format!("abs-{i}.xlsx"));
        rewrite_cell(&source, &copy, &target.sheet, &target.reference, |attrs, _| format!("{}{cell}", strip_type(attrs)));
        let (wrong, cells_now, matched_now) = mismatches(&copy, &mut probe);
        assert_eq!((cells_now, matched_now), (cells, matched - 1), "{cell}");
        assert!(wrong.contains(&format!("{}!{}", target.sheet, target.reference)), "{cell}: {wrong:?}");
    }
    // and the same text cached as Excel writes it: a match (`_x+041_` is no escape)
    for (i, cell) in [
        r#" t="str"><f>"abc"</f><v>abc</v>"#,
        r#" t="str"><f>"_x+041_"</f><v>_x+041_</v>"#,
        // the same text in other markup an XML parser reads alike
        r#" t="str"><f>"abc"</f><v>abc</v >"#,
        r#" t="str"><f>"abc"</f><v><![CDATA[abc]]></v>"#,
        r#" t="str"><f>"abc"</f><v>ab<!--x-->c</v>"#,
        r#" t="str"><f>"A"</f><v>&#00000000065;</v>"#,
    ]
    .into_iter()
    .enumerate()
    {
        let copy = planted.path().join(format!("abs-str-{i}.xlsx"));
        rewrite_cell(&source, &copy, &target.sheet, &target.reference, |attrs, _| format!("{}{cell}", strip_type(attrs)));
        let (wrong, _, matched_now) = mismatches(&copy, &mut probe);
        assert_eq!(matched_now, matched, "{cell}: {wrong:?}");
    }
}

#[test]
fn the_sheet_and_its_parts_are_read_as_an_xml_parser_reads_them() {
    // comments, CDATA, tags in any spelling the schema allows, anywhere in a sheet or the metadata:
    // the cells, their formulas and their errors as an XML parser reads them
    let b = baseline();
    let dir = corpus_dir(&b);
    let mut probe = Probe::default();
    let planted = tempfile::tempdir().unwrap();
    let measure = |path: &Path, probe: &mut Probe| {
        let (_, cells, matched) = mismatches(path, probe);
        (cells, matched)
    };
    let sheet1 = "xl/worksheets/sheet1.xml";
    let abs = dir.join("calc_tests/MATH_AND_TRIGONOMETRY/ABS.xlsx");
    let b2 = r#"<c r="B2" s="2"><f>ABS(A2)</f><v>1</v></c>"#;
    assert!(read_part(&abs, sheet1).contains(b2));
    let whole = measure(&abs, &mut probe);
    // each copy reads as the original: no cell made, none lost, none read otherwise
    let same: [(&str, String); 12] = [
        ("a comment holding a formula cell", read_part(&abs, sheet1).replacen("<sheetData>", r#"<sheetData><!--<c r="Z99" t="str"><f>""</f><v/></c>-->"#, 1)),
        ("a comment holding B2's own cell", read_part(&abs, sheet1).replacen(b2, &format!("<!--{b2}-->{b2}"), 1)),
        ("`</c>` in CDATA", read_part(&abs, sheet1).replacen(b2, r#"<c r="B2" s="2" t="str"><f>"a&lt;/c&gt;b"</f><v><![CDATA[a</c>b]]></v></c>"#, 1)),
        ("`</c>` in a comment between the formula and the value", read_part(&abs, sheet1).replacen(b2, r#"<c r="B2" s="2"><f>ABS(A2)</f><!--</c>--><v>1</v></c>"#, 1)),
        ("`</c>` in a comment in the value", read_part(&abs, sheet1).replacen(b2, r#"<c r="B2" s="2"><f>ABS(A2)</f><v>1<!--</c>--></v></c>"#, 1)),
        ("a cell's text in CDATA", read_part(&abs, sheet1).replacen(b2, r#"<c r="B2" s="2" t="str"><f>"x"</f><v><![CDATA[<c r="Z98"><f>1</f><v>1</v></c>]]></v></c>"#, 1).replace(r#"t="str"><f>"x"</f>"#, r#"t="str"><f>"&lt;c r=""Z98""&gt;&lt;f&gt;1&lt;/f&gt;&lt;v&gt;1&lt;/v&gt;&lt;/c&gt;"</f>"#)),
        ("end tags with a space", read_part(&abs, sheet1).replacen(b2, r#"<c r="B2" s="2"><f>ABS(A2)</f ><v>1</v ></c >"#, 1)),
        ("a start tag with two spaces", read_part(&abs, sheet1).replacen(b2, r#"<c  r="B2"  s="2"><f>ABS(A2)</f><v>1</v></c>"#, 1)),
        ("a start tag across lines", read_part(&abs, sheet1).replacen(b2, "<c\nr=\"B2\"\ns=\"2\"><f>ABS(A2)</f><v>1</v></c>", 1)),
        ("`r` not first, single quotes", read_part(&abs, sheet1).replacen(b2, "<c s='2' r='B2'><f>ABS(A2)</f><v>1</v></c>", 1)),
        ("a prefix bound to the main namespace", read_part(&abs, sheet1).replacen(b2, r#"<c r="B2" s="2"><m:f xmlns:m="http://schemas.openxmlformats.org/spreadsheetml/2006/main">ABS(A2)</m:f><m:v xmlns:m="http://schemas.openxmlformats.org/spreadsheetml/2006/main">1</m:v></c>"#, 1)),
        ("a type spelled with a reference", read_part(&abs, sheet1).replacen(b2, r#"<c r="B2" s="2" t="&#115;tr"><f>"x"</f><v>x</v></c>"#, 1)),
    ];
    for (i, (what, text)) in same.iter().enumerate() {
        let copy = planted.path().join(format!("abs-{i}.xlsx"));
        rewrite_part(&abs, &copy, sheet1, |_| text.clone());
        assert_eq!(measure(&copy, &mut probe), whole, "{what}");
    }
    // a spilled value holding a comment with a formula in it: no formula cell made
    let sequence = dir.join("calc_tests/MATH_AND_TRIGONOMETRY/SEQUENCE.xlsx");
    let whole = measure(&sequence, &mut probe);
    let copy = planted.path().join("sequence.xlsx");
    rewrite_part(&sequence, &copy, sheet1, |t| t.replacen(r#"<c r="A2"><v>2</v></c>"#, r#"<c r="A2"><!--<f/>--><v>2</v></c>"#, 1));
    assert_eq!(measure(&copy, &mut probe), whole);
    // ISREF's D7 (`#CALC!`) and D9 (`#SPILL!`) with their `vm` swapped: two mismatches, a comment in
    // the value metadata changing nothing; a `vm` with spaces or spelled with a reference read as
    // the number it is
    let isref = dir.join("calc_tests/INFORMATION/ISREF.xlsx");
    let (cells, matched) = measure(&isref, &mut probe);
    let swapped = planted.path().join("swapped.xlsx");
    rewrite_part(&isref, &swapped, sheet1, |t| t.replacen(r#"<c r="D7" t="e" cm="1" vm="1">"#, r#"<c r="D7" t="e" cm="1" vm="2">"#, 1).replacen(r#"<c r="D9" t="e" cm="1" vm="2">"#, r#"<c r="D9" t="e" cm="1" vm="1">"#, 1));
    assert_eq!(measure(&swapped, &mut probe), (cells, matched - 2));
    let commented = planted.path().join("commented.xlsx");
    rewrite_part(&swapped, &commented, "xl/metadata.xml", |t| t.replacen("<valueMetadata count=\"2\">", r#"<valueMetadata count="2"><!--<bk><rc t="2" v="1"/></bk>-->"#, 1));
    assert_eq!(measure(&commented, &mut probe), (cells, matched - 2));
    for vm in [r#"vm=" 1 ""#, r#"vm="&#49;""#] {
        let copy = planted.path().join("vm.xlsx");
        rewrite_part(&isref, &copy, sheet1, |t| t.replacen(r#"<c r="D7" t="e" cm="1" vm="1">"#, &format!(r#"<c r="D7" t="e" cm="1" {vm}>"#), 1));
        assert_eq!(measure(&copy, &mut probe), (cells, matched), "{vm}");
    }
}

/// A cell's attributes without its `t`.
fn strip_type(attrs: &str) -> String {
    regex::Regex::new(r#"\st="[^"]*""#).unwrap().replace(attrs, "").to_string()
}

/// Copies the workbook at `from` to `to`, the cell `sheet!reference` rewritten: `cell` gets the
/// open tag up to its `>` (`<c r="D8" s="3"`, attributes kept) and the cell's inner XML, and
/// returns the new cell up to its `</c>`.
fn rewrite_cell(from: &Path, to: &Path, sheet: &str, reference: &str, cell: impl Fn(&str, &str) -> String) {
    let part = sheet_part(from, sheet);
    let re = regex::Regex::new(&format!(r#"(?s)(<c r="{reference}"(?:\s[^>]*?)?)(?:/>|>(.*?)</c>)"#)).unwrap();
    rewrite_part(from, to, &part, |text| {
        re.replace(text, |c: &regex::Captures| format!("{}</c>", cell(&c[1], c.get(2).map(|m| m.as_str()).unwrap_or("")))).to_string()
    });
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
    // the rule's limits: version 13 refused, 14 and a workbook edited by version 5 taken
    let edges = tempfile::tempdir().unwrap();
    for (what, part, from, to, taken) in [
        ("version 13", "docProps/app.xml", "<AppVersion>16.0300</AppVersion>", "<AppVersion>13.0000</AppVersion>", false),
        ("version 14", "docProps/app.xml", "<AppVersion>16.0300</AppVersion>", "<AppVersion>14.0000</AppVersion>", true),
        ("edited by 5", "xl/workbook.xml", "lastEdited=\"7\"", "lastEdited=\"5\"", true),
    ] {
        let path = edges.path().join(format!("{}.xlsx", what.replace(' ', "_")));
        rewrite_part(&source, &path, part, |text| text.replacen(from, to, 1));
        assert_eq!(excel_application(&path).is_some(), taken, "{what}");
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
    // and the pin is released when the measure ends, to what it was before it, the random functions
    // returned to chance
    assert_eq!(ironcalc_base::pinned_clock(), None);
    assert!(!ironcalc_base::random_is_seeded());
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
    // every example kept that the random array's nine draws need
    let mut opts = options(links_mode);
    opts.max_examples = 9;
    let mut short = Vec::new();
    let (mut base_matched, mut matched, mut formulas, mut better) = (0usize, 0usize, 0usize, Vec::new());
    let mut links: BTreeMap<String, LinkOutcome> = BTreeMap::new();
    for f in &b.file {
        let report = run_file(&dir.join(&f.path), &opts, &mut probe);
        // the random array, measured partway through the run whose totals are pinned: seeded
        if f.path == "DynamicArrays.xlsx" {
            assert_eq!(draws_of(&report), seeded_draws(), "{links_mode:?}: the random array inside the run");
        }
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
    // Veredas 2's own totals, a change a decision made here: 75 857 measured on 2026-11-22; 75 901
    // on 2026-11-23, the expected side reading Excel's escaped texts and the engine's side its
    // texts that read like errors as they are (C-222's third confirmation round, 44 cells); 75 912
    // the same day, Excel's `#SPILL!` and `#CALC!` read through their value metadata (the fourth,
    // 11 cells)
    assert_eq!((matched, formulas, base_matched), (75_912, 76_359, 75_743));
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

/// The cells of `DynamicArrays.xlsx`'s `RANDARRAY(3,3,0,100,TRUE)`.
const RANDOM_CELLS: [&str; 9] = ["A19", "B19", "C19", "A20", "B20", "C20", "A21", "B21", "C21"];

/// Its nine draws seeded with 1, under either resolution, on every run (none equals Excel's
/// cached one, so each is a mismatch its report lists with the engine's value).
fn seeded_draws() -> BTreeMap<String, String> {
    RANDOM_CELLS.iter().zip(["80", "40", "61", "45", "53", "44", "16", "65", "82"]).map(|(c, v)| (c.to_string(), v.to_string())).collect()
}

/// The nine draws a report of `DynamicArrays.xlsx` lists (its examples, at least nine kept).
fn draws_of(report: &abaco_oracle::FileReport) -> BTreeMap<String, String> {
    report
        .examples
        .values()
        .flatten()
        .filter(|e| RANDOM_CELLS.contains(&e.reference.as_str()))
        .map(|e| (e.reference.clone(), e.actual.clone()))
        .collect()
}

fn draws(options: &Options, probe: &mut Probe) -> BTreeMap<String, String> {
    draws_of(&run_file(&corpus_dir(&baseline()).join("DynamicArrays.xlsx"), options, probe))
}

#[test]
fn every_measure_draws_the_same_numbers_and_an_unseeded_one_others() {
    for links in [LinksMode::Cached, LinksMode::Linked] {
        let mut o = options(links);
        o.max_examples = 50;
        // inside a run: another workbook measured first, with the same probe
        let mut probe = Probe::default();
        run_file(&corpus_dir(&baseline()).join("calc_tests/MATH_AND_TRIGONOMETRY/ABS.xlsx"), &o, &mut probe);
        assert_eq!(draws(&o, &mut probe), seeded_draws(), "{links:?}: the first measure of it");
        assert_eq!(draws(&o, &mut probe), seeded_draws(), "{links:?}: the second");
        // a measure left to chance draws otherwise (all nine alike by chance: one in 101^9)
        let mut unseeded = options(links);
        unseeded.max_examples = 50;
        unseeded.random_seed = None;
        assert_ne!(draws(&unseeded, &mut probe), seeded_draws(), "{links:?}: an unseeded measure drew the seeded numbers");
    }
}

/// The oracle's binary, its exit code and its report.
fn oracle(args: &[&str]) -> (i32, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_abaco-oracle")).args(args).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).to_string())
}

#[test]
fn the_binary_seeds_its_measures_and_measures_a_baseline_on_its_day() {
    let source = corpus_dir(&baseline()).join("DynamicArrays.xlsx");
    let path = source.to_str().unwrap();
    // a report: the nine draws, every run (its lines `| sheet!cell | formula | Excel's | the engine's |`)
    for _ in 0..2 {
        let (code, report) = oracle(&["--no-roundtrip", "--examples", "20", "--today", "2026-11-22", path]);
        assert_eq!(code, 0, "{report}");
        let got: BTreeMap<String, String> = report
            .lines()
            .filter_map(|l| {
                let cols: Vec<&str> = l.split('|').map(str::trim).collect();
                let cell = cols.get(1)?.strip_prefix("DynamicArrays!")?;
                let engine = cols.get(4)?;
                RANDOM_CELLS.contains(&cell).then(|| (cell.to_string(), engine.to_string()))
            })
            .collect();
        assert_eq!(got, seeded_draws(), "{report}");
    }
    // a baseline: measured on the day it names, both resolutions seeded
    let dir = tempfile::tempdir().unwrap();
    let corpus = dir.path().join("corpus");
    std::fs::create_dir(&corpus).unwrap();
    std::fs::copy(&source, corpus.join("DynamicArrays.xlsx")).unwrap();
    let out = dir.path().join("baseline.toml");
    let (code, _) = oracle(&[
        "--no-roundtrip", "--baseline-out", out.to_str().unwrap(), "--measured-by", "parity test", "--measured-on", "2026-11-22",
        "--corpus", corpus.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    let written = Baseline::read(&out).unwrap();
    assert_eq!(written.measured_on, "2026-11-22");
    let file = written.file.iter().find(|f| f.path == "DynamicArrays.xlsx").unwrap();
    assert_eq!((file.cached.as_ref().unwrap().matched, file.linked.as_ref().unwrap().matched), (5, 5));
    // a baseline's measures seeded: a copy whose cache holds the nine seeded draws matches them
    // all (14 of 23; an unseeded measure would match all nine about once in 101^9 runs), under
    // either resolution, and with `--links linked`, whose cached measure is the baseline's own
    let planted_dir = dir.path().join("planted");
    std::fs::create_dir(&planted_dir).unwrap();
    let mut from = source.clone();
    for (i, (cell, v)) in seeded_draws().into_iter().enumerate() {
        let to = dir.path().join(format!("draw{i}.xlsx"));
        rewrite_cell_value(&from, &to, "DynamicArrays", &cell, &v);
        from = to;
    }
    std::fs::copy(&from, planted_dir.join("DynamicArrays.xlsx")).unwrap();
    for links in ["cached", "linked"] {
        let (code, _) = oracle(&[
            "--no-roundtrip", "--links", links, "--baseline-out", out.to_str().unwrap(), "--measured-by", "parity test", "--measured-on",
            "2026-11-22", "--corpus", planted_dir.to_str().unwrap(),
        ]);
        assert_eq!(code, 0);
        let written = Baseline::read(&out).unwrap();
        let file = &written.file[0];
        assert_eq!((file.cached.as_ref().unwrap().matched, file.linked.as_ref().unwrap().matched), (14, 14), "--links {links}");
    }
    // and measured on its day: the yearly calendar builds its year from TODAY()
    let calendar_dir = dir.path().join("calendar");
    std::fs::create_dir(&calendar_dir).unwrap();
    std::fs::copy(corpus_dir(&baseline()).join("templates/yearly_calendar.xlsx"), calendar_dir.join("yearly_calendar.xlsx")).unwrap();
    for (day, matched) in [("2026-11-22", 529), ("2027-03-01", 136)] {
        let (code, _) = oracle(&[
            "--no-roundtrip", "--baseline-out", out.to_str().unwrap(), "--measured-by", "parity test", "--measured-on", day, "--corpus",
            calendar_dir.to_str().unwrap(),
        ]);
        assert_eq!(code, 0);
        let written = Baseline::read(&out).unwrap();
        let file = &written.file[0];
        assert_eq!((file.cached.as_ref().unwrap().matched, file.linked.as_ref().unwrap().matched), (matched, matched), "{day}");
    }
    // a baseline said to be measured on one day and clocked on another is refused
    let (code, _) = oracle(&[
        "--no-roundtrip", "--baseline-out", out.to_str().unwrap(), "--measured-on", "2026-11-22", "--today", "2026-11-23",
        "--corpus", corpus.to_str().unwrap(),
    ]);
    assert_eq!(code, 2);
}
