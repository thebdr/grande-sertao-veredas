//! Fork change 27: a package's relationships read as OPC reads them. `Id` (an xsd:ID), `Type` and
//! `Target` (xsd:anyURI) with their whitespace collapsed, a target resolved against its part's
//! folder with its dot segments, a part named in another letter case: each loads the workbook the
//! original does. A sheet whose relationship is missing is an error, never a panic.
#![allow(clippy::unwrap_used)]

use std::io::{Cursor, Read, Write};

use ironcalc::import::load_from_xlsx_bytes;

/// The package at `path` with its part `part` passed through `edit`, which must change it.
fn rewritten(path: &str, part: &str, edit: impl Fn(&str) -> String) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut changed = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let name = entry.name().unwrap().to_string();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        if name == part {
            let text = String::from_utf8(bytes).unwrap();
            let edited = edit(&text);
            assert_ne!(edited, text, "{part} unchanged");
            bytes = edited.into_bytes();
            changed += 1;
        }
        out.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        out.write_all(&bytes).unwrap();
    }
    assert_eq!(changed, 1, "{part}");
    out.finish().unwrap().into_inner()
}

fn load(bytes: &[u8]) -> Result<ironcalc_base::types::Workbook, String> {
    load_from_xlsx_bytes(bytes, "book", "en", "UTC").map_err(|e| e.to_string())
}

#[test]
fn a_workbook_s_relationships_are_read_as_opc_reads_them() {
    let path = "tests/calc_tests/MATH_AND_TRIGONOMETRY/ABS.xlsx";
    let original = load(&std::fs::read(path).unwrap()).unwrap();
    let rels = "xl/_rels/workbook.xml.rels";
    let sheet = r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml""#;
    for (what, part, from, to) in [
        ("a Target padded", rels, sheet, sheet.replace(r#"Target="worksheets"#, r#"Target=" worksheets"#).replace(r#"sheet1.xml""#, "sheet1.xml \n\"")),
        ("a Target in another case", rels, sheet, sheet.replace("worksheets/sheet1.xml", "Worksheets/Sheet1.XML")),
        ("a Target with a dot segment", rels, sheet, sheet.replace("worksheets/sheet1.xml", "./worksheets/sheet1.xml")),
        ("a Target up and back", rels, sheet, sheet.replace("worksheets/sheet1.xml", "../xl/worksheets/sheet1.xml")),
        ("a Target absolute", rels, sheet, sheet.replace("worksheets/sheet1.xml", "/xl/worksheets/sheet1.xml")),
        ("a Type padded", rels, sheet, sheet.replace(r#"/worksheet""#, r#"/worksheet ""#)),
        ("the relationship's Id padded", rels, r#"Id="rId1""#, r#"Id=" rId1 ""#.to_string()),
        ("the sheet's r:id padded", "xl/workbook.xml", r#"r:id="rId1""#, "r:id=\"\trId1 \"".to_string()),
        (
            "the styles' Target with a dot segment",
            rels,
            r#"Target="styles.xml""#,
            r#"Target="./styles.xml""#.to_string(),
        ),
    ] {
        let bytes = rewritten(path, part, |t| t.replacen(from, &to, 1));
        assert_eq!(load(&bytes).as_ref(), Ok(&original), "{what}");
    }
}

#[test]
fn a_sheet_s_relationships_are_read_as_opc_reads_them() {
    // example.xlsx's seventh sheet holds comments, through `../comments2.xml`
    let path = "tests/example.xlsx";
    let original = load(&std::fs::read(path).unwrap()).unwrap();
    let rels = "xl/worksheets/_rels/sheet7.xml.rels";
    let comments = r#"Target="../comments2.xml""#;
    for (what, to) in [
        ("padded", r#"Target=" ../comments2.xml ""#),
        ("absolute", r#"Target="/xl/comments2.xml""#),
        ("in another case", r#"Target="../Comments2.XML""#),
        ("with a dot segment", r#"Target="./../comments2.xml""#),
    ] {
        let bytes = rewritten(path, rels, |t| t.replacen(comments, to, 1));
        assert_eq!(load(&bytes).as_ref(), Ok(&original), "comments' target {what}");
    }
    assert!(original.worksheets.iter().any(|w| !w.comments.is_empty()));
}

#[test]
fn a_sheet_without_its_relationship_is_an_error_not_a_panic() {
    let path = "tests/calc_tests/MATH_AND_TRIGONOMETRY/ABS.xlsx";
    for (what, part, from, to) in [
        ("an Id no relationship has", "xl/workbook.xml", r#"r:id="rId1""#, r#"r:id="rId99""#),
        ("a target no part has", "xl/_rels/workbook.xml.rels", "worksheets/sheet1.xml", "worksheets/sheet9.xml"),
        ("a one-letter target", "xl/_rels/workbook.xml.rels", "worksheets/sheet1.xml", "x"),
    ] {
        let bytes = rewritten(path, part, |t| t.replacen(from, to, 1));
        let outcome = std::panic::catch_unwind(|| load(&bytes));
        assert!(matches!(outcome, Ok(Err(_))), "{what}: {outcome:?}");
    }
}
