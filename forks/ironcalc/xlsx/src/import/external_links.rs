//! The workbooks this one links to: `xl/externalLinks/externalLinkN.xml`.
//!
//! `xl/workbook.xml` lists them in `<externalReferences>`, one `r:id` each, in
//! the order the formulas count them: `[1]Sheet!A1` is the first. Each part
//! holds an `<externalBook r:id>` whose relationship (in the part's own
//! `.rels`, `TargetMode="External"`) is the path of the linked file, the names
//! of its sheets, its defined names, and a `<sheetDataSet>` with the last
//! value Excel saw of every referenced cell — what the formulas show while the
//! linked workbook is closed. All of it is kept on the workbook
//! (`external_books`) so that a save carries the links it was opened with.

use std::{
    collections::HashMap,
    io::{BufReader, Read},
};

use ironcalc_base::{
    expressions::{token::get_error_by_english_name, utils::parse_reference_a1},
    external::{ExternalBook, ExternalDefinedName, ExternalSheet, ExternalValue},
};

use crate::error::XlsxError;

use super::{util::get_attribute, worksheets::Relationship, xml::XmlNode};

const RELATIONSHIPS_NS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// The relationships of one part: `xl/externalLinks/externalLink1.xml` →
/// `xl/externalLinks/_rels/externalLink1.xml.rels`. Empty if there is none.
fn load_part_relationships<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    part_path: &str,
) -> Result<HashMap<String, Relationship>, XlsxError> {
    let (dir, file_name) = match part_path.rsplit_once('/') {
        Some((dir, file_name)) => (dir, file_name),
        None => ("", part_path),
    };
    let rels_path = if dir.is_empty() {
        format!("_rels/{file_name}.rels")
    } else {
        format!("{dir}/_rels/{file_name}.rels")
    };
    let Ok(file) = archive.by_name(&rels_path) else {
        return Ok(HashMap::new());
    };
    let doc = XmlNode::parse(BufReader::new(file))?;
    let mut rels = HashMap::new();
    for node in doc.descendants().filter(|n| n.has_tag_name("Relationship")) {
        rels.insert(
            get_attribute(node, "Id")?.to_string(),
            Relationship {
                rel_type: get_attribute(node, "Type")?.to_string(),
                target: get_attribute(node, "Target")?.to_string(),
            },
        );
    }
    Ok(rels)
}

/// A relationship target as a path inside the package: `/xl/a.xml` is
/// absolute, `externalLinks/a.xml` is relative to `xl/`.
fn part_path_of(target: &str) -> String {
    match target.strip_prefix('/') {
        Some(absolute) => absolute.to_string(),
        None => format!("xl/{target}"),
    }
}

/// The value of one `<cell>` of a `<sheetData>`: `t="str"` text, `t="b"`
/// boolean, `t="e"` error, a number otherwise. `None` when there is no `<v>`
/// (an empty cell Excel still listed).
fn cell_value(cell: &XmlNode) -> Option<ExternalValue> {
    let v = cell.child("v")?;
    let text = v.text().unwrap_or("");
    Some(match cell.attribute("t") {
        Some("str") | Some("inlineStr") | Some("s") => ExternalValue::String(text.to_string()),
        Some("b") => ExternalValue::Boolean(text.trim() == "1" || text.trim() == "TRUE"),
        Some("e") => match get_error_by_english_name(text.trim()) {
            Some(error) => ExternalValue::Error(error),
            None => ExternalValue::String(text.to_string()),
        },
        _ => match text.trim().parse::<f64>() {
            Ok(number) => ExternalValue::Number(number),
            // Not a number where one was expected: keep it as the text it is
            Err(_) => ExternalValue::String(text.to_string()),
        },
    })
}

/// Reads one `<externalLink>` document. `rels` are the part's relationships;
/// the `r:id` of `<externalBook>` names the linked file.
pub(super) fn parse_external_link(
    doc: &XmlNode,
    rels: &HashMap<String, Relationship>,
) -> Result<ExternalBook, XlsxError> {
    let mut book = ExternalBook::default();
    let Some(external_book) = doc.child("externalBook") else {
        // A DDE or OLE link: not a workbook. The index is kept so that the
        // formulas' `[n]` still count right; there is nothing to read in it.
        return Ok(book);
    };
    if let Some(id) = external_book.attribute_ns(RELATIONSHIPS_NS, "id") {
        if let Some(rel) = rels.get(id) {
            book.target = rel.target.clone();
        }
    }
    // Excel 2021+ records an absolute URL beside the relative target
    // (<xxl21:alternateUrls><xxl21:absoluteUrl r:id="rId2"/></xxl21:alternateUrls>)
    if let Some(alternate) = external_book.child("alternateUrls") {
        if let Some(absolute) = alternate.child("absoluteUrl") {
            if let Some(id) = absolute.attribute_ns(RELATIONSHIPS_NS, "id") {
                if let Some(rel) = rels.get(id) {
                    book.absolute_url = Some(rel.target.clone());
                }
            }
        }
    }
    if let Some(sheet_names) = external_book.child("sheetNames") {
        for node in sheet_names
            .children()
            .filter(|n| n.has_tag_name("sheetName"))
        {
            book.sheets.push(ExternalSheet {
                name: get_attribute(node, "val")?.to_string(),
                cells: HashMap::new(),
            });
        }
    }
    if let Some(defined_names) = external_book.child("definedNames") {
        for node in defined_names
            .children()
            .filter(|n| n.has_tag_name("definedName"))
        {
            let sheet_id = match node.attribute("sheetId") {
                Some(s) => Some(s.parse::<u32>()?),
                None => None,
            };
            book.defined_names.push(ExternalDefinedName {
                name: get_attribute(node, "name")?.to_string(),
                refers_to: node.attribute("refersTo").unwrap_or("").to_string(),
                sheet_id,
            });
        }
    }
    if let Some(data_set) = external_book.child("sheetDataSet") {
        for sheet_data in data_set.children().filter(|n| n.has_tag_name("sheetData")) {
            // sheetId here is the position in sheetNames, 0-based
            let index = get_attribute(sheet_data, "sheetId")?.parse::<usize>()?;
            let Some(sheet) = book.sheets.get_mut(index) else {
                return Err(XlsxError::Xml(format!(
                    "External link: sheetData sheetId {index} has no sheetName"
                )));
            };
            for row in sheet_data.children().filter(|n| n.has_tag_name("row")) {
                for cell in row.children().filter(|n| n.has_tag_name("cell")) {
                    let reference = get_attribute(cell, "r")?;
                    let Some(parsed) = parse_reference_a1(reference) else {
                        return Err(XlsxError::Xml(format!(
                            "External link: invalid cell reference {reference}"
                        )));
                    };
                    if let Some(value) = cell_value(cell) {
                        sheet.cells.insert((parsed.row, parsed.column), value);
                    }
                }
            }
        }
    }
    Ok(book)
}

/// Every external book of the workbook, in `<externalReferences>` order.
/// `reference_ids` are the `r:id`s of that list; `rels` the workbook's
/// relationships.
pub(super) fn load_external_links<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    reference_ids: &[String],
    rels: &HashMap<String, Relationship>,
) -> Result<Vec<ExternalBook>, XlsxError> {
    let mut books = Vec::with_capacity(reference_ids.len());
    for id in reference_ids {
        let Some(rel) = rels.get(id) else {
            return Err(XlsxError::Xml(format!(
                "externalReference {id} has no relationship"
            )));
        };
        let part_path = part_path_of(&rel.target);
        let part_rels = load_part_relationships(archive, &part_path)?;
        let doc = {
            let file = archive.by_name(&part_path)?;
            XmlNode::parse(BufReader::new(file))?
        };
        books.push(parse_external_link(&doc, &part_rels)?);
    }
    Ok(books)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironcalc_base::expressions::token::Error;

    // The shape Excel writes (taken from a real workbook's externalLink1.xml), with
    // the xxl21 alternate URL, a string, numbers, an empty listed cell and
    // a refreshError flag.
    const LINK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<externalLink xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="x14 xxl21" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xxl21="http://schemas.microsoft.com/office/spreadsheetml/2021/extlinks2021"><externalBook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="rId1"><xxl21:alternateUrls><xxl21:absoluteUrl r:id="rId2"/></xxl21:alternateUrls><sheetNames><sheetName val="Cover"/><sheetName val="ZONES"/></sheetNames><definedNames><definedName name="Zones" refersTo="=ZONES!$A$23:$A$30"/><definedName name="Local" refersTo="=ZONES!$B$4" sheetId="1"/></definedNames><sheetDataSet><sheetData sheetId="0"/><sheetData sheetId="1" refreshError="1"><row r="4"><cell r="B4"><v>2</v></cell></row><row r="6"><cell r="B6" t="str"><v>L1.SEG%1.ZONE%2.STATE</v></cell><cell r="C6" t="b"><v>1</v></cell><cell r="D6" t="e"><v>#N/A</v></cell></row><row r="43"><cell r="O43"/><cell r="P43" t="str"><v xml:space="preserve"> </v></cell></row></sheetData></sheetDataSet></externalBook></externalLink>"#;

    fn rels() -> HashMap<String, Relationship> {
        let mut rels = HashMap::new();
        rels.insert(
            "rId1".to_string(),
            Relationship {
                rel_type: "http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLinkPath".to_string(),
                target: "Linked_02.xlsx".to_string(),
            },
        );
        rels.insert(
            "rId2".to_string(),
            Relationship {
                rel_type: "http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLinkPath".to_string(),
                target: "https://example.invalid/shared/Linked_02.xlsx".to_string(),
            },
        );
        rels
    }

    #[test]
    fn reads_excel_shape() {
        let doc = XmlNode::parse_str(LINK_XML).unwrap();
        let book = parse_external_link(&doc, &rels()).unwrap();
        assert_eq!(book.target, "Linked_02.xlsx");
        assert_eq!(
            book.absolute_url.as_deref(),
            Some("https://example.invalid/shared/Linked_02.xlsx")
        );
        assert_eq!(book.sheets.len(), 2);
        assert_eq!(book.sheets[0].name, "Cover");
        assert!(book.sheets[0].cells.is_empty());
        let zone = &book.sheets[1];
        assert_eq!(zone.name, "ZONES");
        assert_eq!(zone.cells.get(&(4, 2)), Some(&ExternalValue::Number(2.0)));
        assert_eq!(
            zone.cells.get(&(6, 2)),
            Some(&ExternalValue::String("L1.SEG%1.ZONE%2.STATE".to_string()))
        );
        assert_eq!(zone.cells.get(&(6, 3)), Some(&ExternalValue::Boolean(true)));
        assert_eq!(
            zone.cells.get(&(6, 4)),
            Some(&ExternalValue::Error(Error::NA))
        );
        // a listed cell without a value is an empty cell: not stored
        assert_eq!(zone.cells.get(&(43, 15)), None);
        // whitespace-only text is text
        assert_eq!(
            zone.cells.get(&(43, 16)),
            Some(&ExternalValue::String(" ".to_string()))
        );
        assert_eq!(zone.cells.len(), 5);
        assert_eq!(book.defined_names.len(), 2);
        assert_eq!(book.defined_names[0].name, "Zones");
        assert_eq!(book.defined_names[0].refers_to, "=ZONES!$A$23:$A$30");
        assert_eq!(book.defined_names[0].sheet_id, None);
        assert_eq!(book.defined_names[1].sheet_id, Some(1));
    }

    #[test]
    fn a_link_without_rels_keeps_its_data_and_no_target() {
        let doc = XmlNode::parse_str(LINK_XML).unwrap();
        let book = parse_external_link(&doc, &HashMap::new()).unwrap();
        assert_eq!(book.target, "");
        assert_eq!(book.absolute_url, None);
        assert_eq!(book.sheets.len(), 2);
    }

    #[test]
    fn a_dde_link_is_an_empty_placeholder() {
        let xml = r#"<externalLink xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><ddeLink ddeService="Excel" ddeTopic="x"/></externalLink>"#;
        let doc = XmlNode::parse_str(xml).unwrap();
        let book = parse_external_link(&doc, &HashMap::new()).unwrap();
        assert_eq!(book, ExternalBook::default());
    }

    #[test]
    fn part_paths() {
        assert_eq!(
            part_path_of("externalLinks/externalLink1.xml"),
            "xl/externalLinks/externalLink1.xml"
        );
        assert_eq!(
            part_path_of("/xl/externalLinks/a.xml"),
            "xl/externalLinks/a.xml"
        );
    }
}
