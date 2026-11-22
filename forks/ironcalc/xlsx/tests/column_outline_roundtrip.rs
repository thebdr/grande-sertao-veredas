#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

// Veredas fork change 11: column outline groups (Excel's `outlineLevel` and
// `collapsed` on `<col>`) survive load and save, and setting a column's width
// or hidden flag keeps its level. The importer used to skip both attributes.

use std::io::{Cursor, Read};

use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::Model;

fn sheet_xml(bytes: &[u8]) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut text).unwrap();
    text
}

fn save(model: &Model) -> Vec<u8> {
    save_xlsx_to_writer(model, Cursor::new(Vec::new())).unwrap().into_inner()
}

#[test]
fn outline_levels_round_trip() {
    let mut model = Model::new_empty("outline", "en", "UTC", "en").unwrap();
    let ws = model.workbook.worksheet_mut(0).unwrap();
    for c in 3..=5 {
        ws.set_column_outline(c, 1, false).unwrap();
    }
    ws.set_column_hidden(4, true).unwrap();
    ws.set_column_width(5, 120.0).unwrap();
    ws.set_column_outline(7, 2, true).unwrap();
    // the width/hidden edits kept the levels
    assert_eq!(ws.column_outline(4), (1, false));
    assert_eq!(ws.column_outline(5), (1, false));
    assert_eq!(ws.column_outline(6), (0, false));
    assert!(ws.is_column_hidden(4).unwrap());

    let bytes = save(&model);
    let xml = sheet_xml(&bytes);
    assert!(xml.contains("<sheetFormatPr defaultRowHeight=\"15\" outlineLevelCol=\"2\"/>"), "{xml}");
    assert!(xml.contains("<col min=\"3\" max=\"3\""), "{xml}");
    assert!(xml.contains("hidden=\"1\" outlineLevel=\"1\"/>"), "{xml}");
    assert!(xml.contains("outlineLevel=\"2\" collapsed=\"1\"/>"), "{xml}");

    let back = load_from_xlsx_bytes(&bytes, "outline", "en", "UTC").unwrap();
    let ws = &back.worksheets[0];
    let level = |c: i32| {
        ws.cols.iter().find(|k| k.min <= c && c <= k.max).map(|k| (k.outline_level, k.collapsed)).unwrap_or((0, false))
    };
    assert_eq!(level(3), (1, false));
    assert_eq!(level(4), (1, false));
    assert_eq!(level(5), (1, false));
    assert_eq!(level(6), (0, false));
    assert_eq!(level(7), (2, true));
}

#[test]
fn a_sheet_without_groups_writes_no_format_element() {
    let model = Model::new_empty("plain", "en", "UTC", "en").unwrap();
    let xml = sheet_xml(&save(&model));
    assert!(!xml.contains("sheetFormatPr"), "{xml}");
    assert!(!xml.contains("outlineLevel"), "{xml}");
}

#[test]
fn levels_above_seven_are_refused() {
    let mut model = Model::new_empty("bad", "en", "UTC", "en").unwrap();
    let ws = model.workbook.worksheet_mut(0).unwrap();
    assert!(ws.set_column_outline(2, 8, false).is_err());
    assert_eq!(ws.column_outline(2), (0, false));
}
