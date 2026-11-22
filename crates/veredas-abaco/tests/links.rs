//! A workbook that links to other workbooks, opened under the link policy, and the link
//! difference log (contract: Ábaco's link clause; `src/links.rs`, `src/linkdiff.rs`).
//!
//! Real workbooks: each is written to a scratch folder by the fork's own writer with its external
//! links and their cache, and opened back through `veredas_abaco::links::open`, so the reader of
//! the links, both resolvers and the evaluation are the ones a person's file meets. The log's
//! comparison rules are pinned on a model whose live side is the engine's host hook.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ironcalc::export::save_to_xlsx;
use ironcalc::import::LinkState;
use ironcalc_base::Model;
use ironcalc_base::expressions::token::Error;
use ironcalc_base::external::{
    CachedResolver, ExternalBook, ExternalLookup, ExternalSheet, ExternalValue, HostResolver,
};
use veredas_abaco::linkdiff::{differing, format_value, link_diffs};
use veredas_abaco::links::{LinkChoice, LinkOutcome, LinkPolicy, OpenError, OpenedWorkbook, open};

/// A workbook saved to `dir`: cells on Sheet1 (values or formulas), and its external books.
fn save_book(dir: &Path, name: &str, cells: &[(&str, &str)], books: Vec<ExternalBook>) -> PathBuf {
    let mut model = Model::new_empty(name, "en", "UTC", "en").unwrap();
    model.workbook.external_books = books;
    for (reference, value) in cells {
        let r = model.parse_reference(&format!("Sheet1!{reference}")).unwrap();
        model.set_user_input(r.sheet, r.row, r.column, value.to_string()).unwrap();
    }
    model.evaluate();
    let path = dir.join(name);
    save_to_xlsx(&model, path.to_str().unwrap()).unwrap();
    path
}

/// A link to `target` whose cache holds `cached_b2` at Sheet1!B2.
fn link_to(target: &str, cached_b2: f64) -> ExternalBook {
    let mut cells = HashMap::new();
    cells.insert((2, 2), ExternalValue::Number(cached_b2));
    ExternalBook {
        target: target.to_string(),
        absolute_url: None,
        sheets: vec![ExternalSheet { name: "Sheet1".to_string(), cells }],
        defined_names: vec![],
    }
}

fn cell(opened: &OpenedWorkbook, row: i32) -> String {
    opened.model.get_formatted_cell_value(0, row, 1).unwrap()
}

#[test]
fn the_policies_answer_as_named_and_ask_by_default() {
    assert_eq!(LinkPolicy::default(), LinkPolicy::AlwaysAsk);
    assert_eq!(LinkPolicy::AlwaysAsk.choice(), None);
    assert_eq!(LinkPolicy::AlwaysRecalculateLinked.choice(), Some(LinkChoice::OpenAndCalculate));
    assert_eq!(LinkPolicy::AlwaysUseCached.choice(), Some(LinkChoice::UseCached));
    assert_eq!(LinkPolicy::AlwaysAsk.label(), "always ask");
    assert_eq!(LinkPolicy::AlwaysRecalculateLinked.label(), "always recalculate linked spreadsheets");
    assert_eq!(LinkPolicy::AlwaysUseCached.label(), "always use linked spreadsheets' cached values");
    assert_eq!(LinkChoice::UseCached.label(), "use cached values");
    assert_eq!(LinkChoice::OpenAndCalculate.label(), "open and calculate");
}

#[test]
fn a_file_without_links_never_asks_and_is_evaluated() {
    let dir = tempfile::tempdir().unwrap();
    let path = save_book(dir.path(), "plain.xlsx", &[("A1", "=1+1")], vec![]);
    for policy in [LinkPolicy::AlwaysAsk, LinkPolicy::AlwaysRecalculateLinked, LinkPolicy::AlwaysUseCached] {
        let opened = open(&path, policy, |_| panic!("asked about a file without links")).unwrap();
        assert_eq!(opened.links, LinkOutcome::NoLinks);
        assert_eq!(opened.choice, None);
        assert_eq!(cell(&opened, 1), "2");
        assert_eq!(opened.path, path);
    }
}

#[test]
fn only_always_ask_asks_and_the_answer_decides_the_values() {
    let dir = tempfile::tempdir().unwrap();
    save_book(dir.path(), "companion.xlsx", &[("B2", "10")], vec![]);
    let main = save_book(dir.path(), "main.xlsx", &[("A1", "=[1]Sheet1!$B$2")], vec![link_to("companion.xlsx", 999.0)]);

    // always use cached: no question, the cache answers
    let opened = open(&main, LinkPolicy::AlwaysUseCached, |_| panic!("asked")).unwrap();
    assert_eq!(opened.choice, Some(LinkChoice::UseCached));
    assert_eq!(opened.links, LinkOutcome::Cached { books: 1 });
    assert_eq!(cell(&opened, 1), "999");
    assert_eq!(opened.model.external_strategy(), "cached");

    // always recalculate: no question, the linked workbook is opened and read live
    let opened = open(&main, LinkPolicy::AlwaysRecalculateLinked, |_| panic!("asked")).unwrap();
    assert_eq!(opened.choice, Some(LinkChoice::OpenAndCalculate));
    match &opened.links {
        LinkOutcome::Linked(statuses) => {
            assert_eq!(statuses.len(), 1);
            assert_eq!(statuses[0].index, 1);
            assert_eq!(statuses[0].target, "companion.xlsx");
            assert_eq!(statuses[0].state, LinkState::Loaded);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(cell(&opened, 1), "10");
    assert_eq!(opened.model.external_strategy(), "linked");

    // always ask: asked once, shown the books, and its answer applied
    let asked = Cell::new(0);
    let opened = open(&main, LinkPolicy::AlwaysAsk, |books| {
        asked.set(asked.get() + 1);
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].target, "companion.xlsx");
        assert_eq!(books[0].sheets[0].name, "Sheet1");
        assert_eq!(books[0].sheets[0].cells.get(&(2, 2)), Some(&ExternalValue::Number(999.0)));
        LinkChoice::OpenAndCalculate
    })
    .unwrap();
    assert_eq!(asked.get(), 1);
    assert_eq!(opened.choice, Some(LinkChoice::OpenAndCalculate));
    assert_eq!(cell(&opened, 1), "10");
    let opened = open(&main, LinkPolicy::AlwaysAsk, |_| LinkChoice::UseCached).unwrap();
    assert_eq!(opened.choice, Some(LinkChoice::UseCached));
    assert_eq!(cell(&opened, 1), "999");
}

#[test]
fn a_missing_linked_workbook_is_reported_and_the_cache_answers_for_it() {
    let dir = tempfile::tempdir().unwrap();
    save_book(dir.path(), "companion.xlsx", &[("B2", "10")], vec![]);
    let main = save_book(
        dir.path(),
        "main.xlsx",
        &[("A1", "=[1]Sheet1!$B$2"), ("A2", "=[2]Sheet1!$B$2")],
        vec![link_to("companion.xlsx", 999.0), link_to("missing.xlsx", 7.0)],
    );
    let opened = open(&main, LinkPolicy::AlwaysRecalculateLinked, |_| panic!("asked")).unwrap();
    let LinkOutcome::Linked(statuses) = &opened.links else { panic!("{:?}", opened.links) };
    assert_eq!(statuses.len(), 2);
    assert_eq!(statuses[0].state, LinkState::Loaded);
    assert_eq!(statuses[1].target, "missing.xlsx");
    let missing = dir.path().join("missing.xlsx");
    assert_eq!(statuses[1].state, LinkState::Unavailable(format!("{} not found", missing.display())));
    assert_eq!(cell(&opened, 1), "10");
    assert_eq!(cell(&opened, 2), "7");
}

#[test]
fn a_linked_workbook_that_cannot_be_read_is_reported_with_its_reason_and_logs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("broken.xlsx"), b"not a zip").unwrap();
    let main = save_book(dir.path(), "main.xlsx", &[("A1", "=[1]Sheet1!$B$2")], vec![link_to("broken.xlsx", 7.0)]);
    let mut opened = open(&main, LinkPolicy::AlwaysRecalculateLinked, |_| panic!("asked")).unwrap();
    let LinkOutcome::Linked(statuses) = &opened.links else { panic!("{:?}", opened.links) };
    let broken = dir.path().join("broken.xlsx");
    match &statuses[0].state {
        LinkState::Unavailable(reason) => {
            assert!(reason.starts_with(&format!("{}: ", broken.display())), "{reason}");
            assert!(reason.len() > broken.display().to_string().len() + 2, "{reason}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(cell(&opened, 1), "7");
    assert!(opened.link_diffs().is_empty());
}

#[test]
fn a_target_written_as_excel_writes_it_opens_the_workbook_it_names() {
    let dir = tempfile::tempdir().unwrap();
    // a folder and a name with spaces, the target percent-encoded as Excel writes it
    std::fs::create_dir(dir.path().join("sub dir")).unwrap();
    save_book(&dir.path().join("sub dir"), "Zone list.xlsx", &[("B2", "11")], vec![]);
    // a file: URL holding the absolute path
    save_book(dir.path(), "companion.xlsx", &[("B2", "10")], vec![]);
    let url = format!("file://{}", dir.path().join("companion.xlsx").display()).replace('\\', "/");
    let url = if url.starts_with("file:///") { url } else { url.replacen("file://", "file:///", 1) };
    let main = save_book(
        dir.path(),
        "main.xlsx",
        &[("A1", "=[1]Sheet1!$B$2"), ("A2", "=[2]Sheet1!$B$2")],
        vec![link_to("sub%20dir/Zone%20list.xlsx", 999.0), link_to(&url, 998.0)],
    );
    let opened = open(&main, LinkPolicy::AlwaysRecalculateLinked, |_| panic!("asked")).unwrap();
    let LinkOutcome::Linked(statuses) = &opened.links else { panic!("{:?}", opened.links) };
    assert_eq!(statuses[0].state, LinkState::Loaded, "{:?}", statuses[0]);
    assert_eq!(statuses[0].path, dir.path().join("sub dir/Zone list.xlsx"));
    assert_eq!(statuses[1].state, LinkState::Loaded, "{:?}", statuses[1]);
    assert_eq!(cell(&opened, 1), "11");
    assert_eq!(cell(&opened, 2), "10");
}

#[test]
fn the_values_shown_are_computed_at_opening_not_the_ones_the_file_stored() {
    let dir = tempfile::tempdir().unwrap();
    // a workbook whose stored values are not what its formulas and its cache give: A1 stored 555
    // (computed under a host's answer), the link's cache holding 999; A2 = B2*2 stored 2 while B2
    // holds 5
    let mut model = Model::new_empty("main.xlsx", "en", "UTC", "en").unwrap();
    model.workbook.external_books = vec![link_to("companion.xlsx", 999.0)];
    model.set_user_input(0, 1, 1, "=[1]Sheet1!$B$2".to_string()).unwrap();
    model.set_user_input(0, 2, 2, "1".to_string()).unwrap();
    model.set_user_input(0, 2, 1, "=B2*2".to_string()).unwrap();
    model.set_external_resolver(Box::new(HostResolver::new(Box::new(|_, _, _, _| {
        ExternalLookup::Value(ExternalValue::Number(555.0))
    }))));
    model.evaluate();
    model.set_user_input(0, 2, 2, "5".to_string()).unwrap();
    let main = dir.path().join("main.xlsx");
    save_to_xlsx(&model, main.to_str().unwrap()).unwrap();
    // read back without evaluating, the stored values show
    let stored = ironcalc::import::load_from_xlsx(main.to_str().unwrap(), "en", "UTC", "en").unwrap();
    assert_eq!(stored.get_formatted_cell_value(0, 1, 1).unwrap(), "555");
    assert_eq!(stored.get_formatted_cell_value(0, 2, 1).unwrap(), "2");
    // opened, the cache answers the link and the formulas are computed
    let opened = open(&main, LinkPolicy::AlwaysUseCached, |_| panic!("asked")).unwrap();
    assert_eq!(cell(&opened, 1), "999");
    assert_eq!(cell(&opened, 2), "10");
    // a file without links is computed too
    let mut plain = Model::new_empty("plain.xlsx", "en", "UTC", "en").unwrap();
    plain.set_user_input(0, 1, 2, "1".to_string()).unwrap();
    plain.set_user_input(0, 1, 1, "=B1*2".to_string()).unwrap();
    plain.evaluate();
    plain.set_user_input(0, 1, 2, "5".to_string()).unwrap();
    let path = dir.path().join("plain.xlsx");
    save_to_xlsx(&plain, path.to_str().unwrap()).unwrap();
    let stored = ironcalc::import::load_from_xlsx(path.to_str().unwrap(), "en", "UTC", "en").unwrap();
    assert_eq!(stored.get_formatted_cell_value(0, 1, 1).unwrap(), "2");
    let opened = open(&path, LinkPolicy::AlwaysAsk, |_| panic!("asked")).unwrap();
    assert_eq!(cell(&opened, 1), "10");
}

#[test]
fn the_log_lists_changed_linked_cells_only_under_a_linked_resolution() {
    let dir = tempfile::tempdir().unwrap();
    // the linked workbook moved on: B2 is 10 live where the file cached 999; B3 is the same text
    // on both sides; a second book is missing
    save_book(dir.path(), "companion.xlsx", &[("B2", "10"), ("B3", "same")], vec![]);
    let mut book = link_to("companion.xlsx", 999.0);
    book.sheets[0].cells.insert((3, 2), ExternalValue::String("same".to_string()));
    let main = save_book(
        dir.path(),
        "main.xlsx",
        &[("A1", "=[1]Sheet1!$B$2"), ("A2", "=[1]Sheet1!$B$3"), ("A3", "=[2]Sheet1!$B$2")],
        vec![book, link_to("missing.xlsx", 7.0)],
    );
    let mut opened = open(&main, LinkPolicy::AlwaysUseCached, |_| panic!("asked")).unwrap();
    assert!(opened.link_diffs().is_empty());
    let mut opened = open(&main, LinkPolicy::AlwaysRecalculateLinked, |_| panic!("asked")).unwrap();
    let lines: Vec<String> = opened.link_diffs().iter().map(|d| d.to_string()).collect();
    assert_eq!(lines, vec!["[1]Sheet1!B2: cached 999 → live 10"]);
    // a file without links has no log
    let plain = save_book(dir.path(), "plain.xlsx", &[("A1", "1")], vec![]);
    let mut opened = open(&plain, LinkPolicy::AlwaysRecalculateLinked, |_| panic!("asked")).unwrap();
    assert!(opened.link_diffs().is_empty());
}

#[test]
fn a_file_that_cannot_be_read_is_an_error_naming_it() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nothing.xlsx");
    let err = open(&missing, LinkPolicy::AlwaysUseCached, |_| LinkChoice::UseCached).err().unwrap();
    assert!(matches!(err, OpenError::Load { ref path, .. } if *path == missing));
    assert!(err.to_string().starts_with(&format!("cannot open {}: ", missing.display())), "{err}");
    let garbage = dir.path().join("garbage.xlsx");
    std::fs::write(&garbage, b"not a zip").unwrap();
    let err = open(&garbage, LinkPolicy::AlwaysUseCached, |_| LinkChoice::UseCached).err().unwrap();
    assert!(err.to_string().contains("garbage.xlsx"), "{err}");
}

// The log's rules, on a model whose live side is the engine's host hook.

/// A model with one external book cached as given, its formulas reading B4, B6 (text), B7 (absent
/// from the cache), B8 (an error) and the range D1:D2.
fn model_with_cache(cells: &[((i32, i32), ExternalValue)]) -> Model<'static> {
    let mut model = Model::new_empty("m", "en", "UTC", "en").unwrap();
    model.workbook.external_books.push(ExternalBook {
        target: "Linked.xlsx".to_string(),
        absolute_url: None,
        sheets: vec![ExternalSheet { name: "ZONES".to_string(), cells: cells.iter().cloned().collect() }],
        defined_names: vec![],
    });
    for (row, formula) in [
        (1, "=[1]ZONES!$B$4"),
        (2, "=[1]ZONES!$B$6"),
        (3, "=[1]ZONES!$B$7"),
        (4, "=[1]ZONES!$B$8"),
        (5, "=SUM([1]ZONES!$D$1:$D$2)"),
    ] {
        model.set_user_input(0, row, 1, formula.to_string()).unwrap();
    }
    model.evaluate();
    model
}

fn cache() -> Vec<((i32, i32), ExternalValue)> {
    vec![
        ((4, 2), ExternalValue::Number(4.0)),
        ((6, 2), ExternalValue::String("L1.SEG%1".to_string())),
        ((8, 2), ExternalValue::Error(Error::NA)),
        ((1, 4), ExternalValue::Number(10.0)),
        ((2, 4), ExternalValue::Number(20.0)),
    ]
}

/// A live side answering from `live`, empty where it has nothing.
fn live(live: Vec<((i32, i32), ExternalValue)>) -> HostResolver {
    let map: HashMap<(i32, i32), ExternalValue> = live.into_iter().collect();
    HostResolver::new(Box::new(move |_, sheet, row, column| {
        assert_eq!(sheet, "ZONES");
        match map.get(&(row, column)) {
            Some(v) => ExternalLookup::Value(v.clone()),
            None => ExternalLookup::Empty,
        }
    }))
}

fn lines(model: &mut Model, skip: &[u32]) -> Vec<String> {
    link_diffs(model, skip).iter().map(|d| d.to_string()).collect()
}

#[test]
fn no_change_is_no_line() {
    let mut model = model_with_cache(&cache());
    model.set_external_resolver(Box::new(live(cache())));
    model.evaluate();
    assert!(link_diffs(&mut model, &[]).is_empty());
    model.set_external_resolver(Box::new(CachedResolver));
    assert!(link_diffs(&mut model, &[]).is_empty());
}

#[test]
fn one_changed_cell_is_one_line_with_both_values() {
    let mut model = model_with_cache(&cache());
    let mut l = cache();
    l[0] = ((4, 2), ExternalValue::Number(5.0));
    model.set_external_resolver(Box::new(live(l)));
    model.evaluate();
    let diffs = link_diffs(&mut model, &[]);
    assert_eq!(diffs.len(), 1);
    assert_eq!(diffs[0].to_string(), "[1]ZONES!B4: cached 4 → live 5");
    assert_eq!((diffs[0].book, diffs[0].sheet.as_str(), diffs[0].row, diffs[0].column), (1, "ZONES", 4, 2));
    assert_eq!(diffs[0].cached, Some(ExternalValue::Number(4.0)));
    assert_eq!(diffs[0].live, Some(ExternalValue::Number(5.0)));
}

#[test]
fn a_cell_absent_from_the_cache_differs_only_when_live_is_not_empty() {
    let mut model = model_with_cache(&cache());
    model.set_external_resolver(Box::new(live(cache())));
    model.evaluate();
    assert!(link_diffs(&mut model, &[]).is_empty());
    let mut l = cache();
    l.push(((7, 2), ExternalValue::String("new".to_string())));
    model.set_external_resolver(Box::new(live(l)));
    model.evaluate();
    assert_eq!(lines(&mut model, &[]), vec!["[1]ZONES!B7: cached (empty) → live \"new\""]);
    let mut l = cache();
    l.push(((7, 2), ExternalValue::String(String::new())));
    model.set_external_resolver(Box::new(live(l)));
    assert!(link_diffs(&mut model, &[]).is_empty());
}

#[test]
fn values_compare_by_kind_and_a_range_is_read_cell_by_cell() {
    let mut model = model_with_cache(&cache());
    let mut l = cache();
    l[1] = ((6, 2), ExternalValue::String("L1.SEG%2".to_string()));
    l[2] = ((8, 2), ExternalValue::Error(Error::REF));
    l[3] = ((1, 4), ExternalValue::String("10".to_string())); // a number become the text "10"
    model.set_external_resolver(Box::new(live(l)));
    model.evaluate();
    assert_eq!(
        lines(&mut model, &[]),
        vec![
            "[1]ZONES!D1: cached 10 → live \"10\"",
            "[1]ZONES!B6: cached \"L1.SEG%1\" → live \"L1.SEG%2\"",
            "[1]ZONES!B8: cached #N/A → live #REF!",
        ]
    );
}

#[test]
fn a_side_that_cannot_answer_or_a_skipped_book_yields_no_line() {
    let mut model = model_with_cache(&cache());
    model.set_external_resolver(Box::new(HostResolver::not_available()));
    model.evaluate();
    assert!(link_diffs(&mut model, &[]).is_empty());
    let mut l = cache();
    l[0] = ((4, 2), ExternalValue::Number(5.0));
    model.set_external_resolver(Box::new(live(l)));
    assert_eq!(link_diffs(&mut model, &[]).len(), 1);
    assert!(link_diffs(&mut model, &[1]).is_empty());
}

#[test]
fn a_sheet_name_that_needs_quotes_is_quoted_in_the_line() {
    let mut model = Model::new_empty("m", "en", "UTC", "en").unwrap();
    let mut cells = HashMap::new();
    cells.insert((1, 1), ExternalValue::Number(1.0));
    model.workbook.external_books.push(ExternalBook {
        target: "Linked.xlsx".to_string(),
        absolute_url: None,
        sheets: vec![ExternalSheet { name: "Zone list".to_string(), cells }],
        defined_names: vec![],
    });
    model.set_user_input(0, 1, 1, "='[1]Zone list'!A1".to_string()).unwrap();
    model.set_external_resolver(Box::new(HostResolver::new(Box::new(|_, _, _, _| {
        ExternalLookup::Value(ExternalValue::Boolean(true))
    }))));
    model.evaluate();
    assert_eq!(lines(&mut model, &[]), vec!["'[1]Zone list'!A1: cached 1 → live TRUE"]);
}

#[test]
fn the_comparison_rules_and_the_spelling_of_values() {
    use ExternalLookup as L;
    let n = |x: f64| L::Value(ExternalValue::Number(x));
    let s = |x: &str| L::Value(ExternalValue::String(x.to_string()));
    assert_eq!(differing(n(1.0), n(1.0)), None);
    assert_eq!(differing(n(1.0), n(2.0)), Some((Some(ExternalValue::Number(1.0)), Some(ExternalValue::Number(2.0)))));
    // numbers at Excel's 15 significant digits
    assert_eq!(differing(n(0.1 + 0.2), n(0.3)), None);
    assert_eq!(differing(n(1.0), n(1.000000000000001)), None);
    assert!(differing(n(1.0), n(1.00000000000001)).is_some());
    assert_eq!(differing(n(-0.0), n(0.0)), None);
    // text exactly, its case included; a number never equals a text
    assert!(differing(s("Abc"), s("ABC")).is_some());
    assert!(differing(n(10.0), s("10")).is_some());
    // an empty text is empty, on either side
    assert_eq!(differing(L::Empty, L::Empty), None);
    assert_eq!(differing(L::Empty, s("")), None);
    assert_eq!(differing(s(""), L::Empty), None);
    assert_eq!(differing(L::NotAvailable, n(1.0)), None);
    assert_eq!(differing(n(1.0), L::NotAvailable), None);
    assert_eq!(differing(n(1.0), L::Empty), Some((Some(ExternalValue::Number(1.0)), None)));
    let f = |v: ExternalValue| format_value(Some(&v));
    assert_eq!(format_value(None), "(empty)");
    // numbers as Excel turns them into text
    assert_eq!(f(ExternalValue::Number(0.1 + 0.2)), "0.3");
    assert_eq!(f(ExternalValue::Number(1e21)), "1E+21");
    assert_eq!(f(ExternalValue::Number(1e16)), "1E+16");
    assert_eq!(f(ExternalValue::Number(1e-7)), "0.0000001");
    assert_eq!(f(ExternalValue::Number(123456789012345678.0)), "1.23456789012346E+17");
    assert_eq!(f(ExternalValue::Number(-0.0)), "0");
    assert_eq!(f(ExternalValue::Number(-2.5)), "-2.5");
    // text quoted as in a formula, one line, every other character as it is
    assert_eq!(f(ExternalValue::String("a\"b".to_string())), "\"a\"\"b\"");
    assert_eq!(f(ExternalValue::String("one\ntwo\r\tx\u{7}".to_string())), "\"one\\ntwo\\r\\tx\\u{7}\"");
    assert_eq!(f(ExternalValue::String("a\u{a0}b".to_string())), "\"a\u{a0}b\"");
    let decomposed = "ac\u{327}a\u{303}o";
    assert_eq!(f(ExternalValue::String(decomposed.to_string())), format!("\"{decomposed}\""));
    assert_eq!(f(ExternalValue::String("C:\\x".to_string())), "\"C:\\x\"");
    assert_eq!(f(ExternalValue::Boolean(true)), "TRUE");
    assert_eq!(f(ExternalValue::Boolean(false)), "FALSE");
    assert_eq!(f(ExternalValue::Error(Error::DIV)), "#DIV/0!");
}
