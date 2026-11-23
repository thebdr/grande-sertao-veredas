//! Fork change 25: every function the Excel-saved workbooks among these tests call is written as
//! Excel wrote it there, `_xlfn.` or plain. The base crate's `test_xlsx_function_spelling` holds
//! the functions no such workbook calls to Microsoft's grammar and XlsxWriter's list.
#![allow(clippy::unwrap_used)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};

use ironcalc_base::expressions::parser::stringify::to_excel_string;
use ironcalc_base::expressions::parser::{new_parser_english, Node};
use ironcalc_base::expressions::types::CellReferenceRC;
use quick_xml::events::Event;

/// The `<Application>` of the workbooks Excel saved (docProps/app.xml).
const EXCEL: [&str; 4] = ["Microsoft Excel", "Microsoft Macintosh Excel", "Microsoft Excel Online", "Excel Android"];

/// How many of the engine's functions those workbooks call, today.
const CALLED: usize = 422;

fn workbooks(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            workbooks(&path, out);
        } else if path.extension().is_some_and(|e| e == "xlsx") {
            out.push(path);
        }
    }
}

/// The text of every element named `local` of a part, its entities resolved.
fn texts(xml: &str, local: &str) -> Vec<String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let (mut out, mut inside) = (Vec::new(), None::<String>);
    loop {
        match reader.read_event().unwrap() {
            Event::Eof => break,
            Event::Start(e) if e.local_name().as_ref() == local => inside = Some(String::new()),
            Event::End(e) if e.local_name().as_ref() == local => out.extend(inside.take()),
            Event::Text(t) => {
                if let Some(s) = inside.as_mut() {
                    s.push_str(&t.xml_content(quick_xml::XmlVersion::Implicit1_0));
                }
            }
            Event::GeneralRef(r) => {
                if let Some(s) = inside.as_mut() {
                    let c = match r.resolve_char_ref().unwrap() {
                        Some(c) => c.to_string(),
                        None => quick_xml::escape::resolve_predefined_entity(&r).unwrap().to_string(),
                    };
                    s.push_str(&c);
                }
            }
            _ => {}
        }
    }
    out
}

fn part(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    let mut s = String::new();
    zip.by_name(name).ok()?.read_to_string(&mut s).ok()?;
    Some(s)
}

/// Every call of a formula, its prefix (`_xlfn.`, `_xlfn._xlws.` or none) and its name, outside
/// its strings and quoted sheet names.
fn calls(formula: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = formula.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' || c == '\'' {
            // to the closing quote, a doubled one kept inside
            i += 1;
            while i < chars.len() {
                if chars[i] == c && chars.get(i + 1) == Some(&c) {
                    i += 2;
                } else if chars[i] == c {
                    break;
                } else {
                    i += 1;
                }
            }
            i += 1;
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                i += 1;
            }
            if chars.get(i) == Some(&'(') {
                let word: String = chars[start..i].iter().collect();
                let mut name = word.as_str();
                let mut prefix = String::new();
                for p in ["_xlfn.", "_xlws."] {
                    if let Some(rest) = name.strip_prefix(p) {
                        prefix.push_str(p);
                        name = rest;
                    }
                }
                out.push((prefix, name.to_ascii_uppercase()));
            }
        } else {
            i += 1;
        }
    }
    out
}

#[test]
fn every_function_excel_s_own_files_call_is_written_as_they_spell_it() {
    let mut files = Vec::new();
    workbooks(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests"), &mut files);
    files.sort();
    // each function's spellings, with a file that holds each
    let mut spelled: BTreeMap<String, BTreeMap<String, PathBuf>> = BTreeMap::new();
    let mut saved_by_excel = 0;
    for path in &files {
        let Ok(mut zip) = zip::ZipArchive::new(std::fs::File::open(path).unwrap()) else { continue };
        let app = part(&mut zip, "docProps/app.xml").map(|x| texts(&x, "Application")).unwrap_or_default();
        if !app.iter().any(|a| EXCEL.contains(&a.as_str())) {
            continue;
        }
        saved_by_excel += 1;
        let names: Vec<String> = zip.file_names().map(|n| n.unwrap().to_string()).collect();
        let mut formulas = Vec::new();
        for name in names {
            if name.starts_with("xl/worksheets/") && name.ends_with(".xml") {
                formulas.extend(texts(&part(&mut zip, &name).unwrap(), "f"));
            } else if name == "xl/workbook.xml" {
                formulas.extend(texts(&part(&mut zip, &name).unwrap(), "definedName"));
            }
        }
        for f in formulas {
            for (prefix, name) in calls(&f) {
                spelled.entry(name).or_default().entry(prefix).or_insert_with(|| path.clone());
            }
        }
    }
    assert!(saved_by_excel >= 250, "{saved_by_excel} workbooks saved by Excel");
    let sheets = vec!["Sheet1".to_string()];
    let ctx = CellReferenceRC { sheet: "Sheet1".to_string(), row: 1, column: 1 };
    let (mut called, mut wrong) = (BTreeSet::new(), Vec::new());
    for (name, spellings) in &spelled {
        // a function the engine knows; the spill and intersection operators are not functions
        let mut parser = new_parser_english(sheets.clone(), vec![], HashMap::new());
        let node = parser.parse(&format!("{name}(1)"), &ctx);
        if !matches!(node, Node::FunctionKind { .. }) {
            continue;
        }
        called.insert(name.clone());
        assert_eq!(spellings.len(), 1, "{name} spelled two ways by Excel: {spellings:?}");
        let (prefix, file) = spellings.iter().next().unwrap();
        let written = to_excel_string(&node, &ctx);
        if written != format!("{prefix}{name}(1)") {
            wrong.push(format!("{name}: written {written}, Excel wrote {prefix}{name} in {}", file.display()));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    assert_eq!(called.len(), CALLED, "{called:?}");
}

#[test]
fn a_call_is_found_outside_strings_and_sheet_names() {
    assert_eq!(
        calls(r#"_xlfn.DAYS(A1,B1)+'My SUM(x'!A1&"CEILING(""x"")"&_xlfn._xlws.FILTER(A:A,B:B)+iso.ceiling(1)"#),
        [
            ("_xlfn.".to_string(), "DAYS".to_string()),
            ("_xlfn._xlws.".to_string(), "FILTER".to_string()),
            (String::new(), "ISO.CEILING".to_string()),
        ]
    );
}
