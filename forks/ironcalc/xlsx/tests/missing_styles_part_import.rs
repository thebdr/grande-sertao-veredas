#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

use std::io::{Cursor, Read, Write};

use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::types::Styles;
use ironcalc_base::Model;

// The styles part itself is optional in OOXML: a package may have no
// `xl/styles.xml` at all (minimal writers leave it out, and so does a file
// whose formats were stripped). The importer used to refuse such a package
// ("specified file not found in archive"); it reads as the default styles.

/// tests/calc_tests/arithmetic.xlsx (Excel-saved) without its styles part:
/// the part, its relationship, its content-type override and every cell's
/// `s="N"` removed.
fn arithmetic_without_styles() -> Vec<u8> {
    let bytes = std::fs::read("tests/calc_tests/arithmetic.xlsx").unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut out));
        for index in 0..archive.len() {
            let mut file = archive.by_index(index).unwrap();
            let name = file.name().unwrap().into_owned();
            if name == "xl/styles.xml" {
                continue;
            }
            let mut data = Vec::new();
            file.read_to_end(&mut data).unwrap();
            if name.ends_with(".xml") || name.ends_with(".rels") {
                let mut text = String::from_utf8(data).unwrap();
                for (open, close) in [
                    ("<Override PartName=\"/xl/styles.xml\"", "/>"),
                    ("<Relationship Id=\"rId4\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\"", "/>"),
                ] {
                    if let Some(start) = text.find(open) {
                        let end = text[start..].find(close).unwrap() + start + close.len();
                        text.replace_range(start..end, "");
                    }
                }
                if name.starts_with("xl/worksheets/") {
                    for n in 0..10 {
                        text = text.replace(&format!(" s=\"{n}\""), "");
                    }
                }
                data = text.into_bytes();
            }
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(&data).unwrap();
        }
        writer.finish().unwrap();
    }
    out
}

#[test]
fn a_package_without_a_styles_part_loads_with_the_default_styles() {
    let bytes = arithmetic_without_styles();
    // the premise: the part and every pointer to it are gone
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
    assert!(archive.by_name("xl/styles.xml").is_err());
    let mut rels = String::new();
    archive
        .by_name("xl/_rels/workbook.xml.rels")
        .unwrap()
        .read_to_string(&mut rels)
        .unwrap();
    assert!(!rels.contains("/styles\""), "{rels}");

    let workbook = load_from_xlsx_bytes(&bytes, "arithmetic", "en", "UTC").unwrap();
    assert_eq!(workbook.styles, Styles::default());
    let mut model = Model::from_workbook(workbook, "en").unwrap();
    model.evaluate();
    // Sheet1!E2 = C2+D2 over 1 and 2, as Excel cached it
    assert_eq!(
        model.get_formatted_cell_value(0, 2, 5).unwrap(),
        "3".to_string()
    );
    assert_eq!(model.get_cell_style_index(0, 2, 5).unwrap(), 0);
}
