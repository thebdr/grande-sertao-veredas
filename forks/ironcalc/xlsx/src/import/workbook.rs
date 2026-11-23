use std::io::{BufReader, Read};

use super::xml::XmlNode;
use ironcalc_base::types::{DefinedName, SheetState};

use crate::error::XlsxError;

use super::shared_strings::decode_xlsx_escapes;
use super::{
    util::get_attribute,
    worksheets::{Sheet, WorkbookXML},
};

pub(super) fn load_workbook<R: Read + std::io::Seek>(
    archive: &mut zip::read::ZipArchive<R>,
) -> Result<WorkbookXML, XlsxError> {
    let file = archive.by_name("xl/workbook.xml")?;
    let doc = XmlNode::parse(BufReader::new(file))?;
    let mut defined_names = Vec::new();
    let mut sheets = Vec::new();
    // Get the sheets
    let sheet_nodes: Vec<&XmlNode> = doc
        .descendants()
        .filter(|n| n.has_tag_name("sheet"))
        .collect();
    for sheet in sheet_nodes {
        let name = get_attribute(sheet, "name")?.to_string();
        let sheet_id = get_attribute(sheet, "sheetId")?.to_string();
        let sheet_id = sheet_id.parse::<u32>()?;
        let id = sheet
            .attribute_ns(
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
                "id",
            )
            .ok_or_else(|| XlsxError::Xml("Missing \"r:id\" XML attribute".to_string()))
            .map(super::collapse)?;
        let state = match sheet.attribute("state") {
            Some("visible") | None => SheetState::Visible,
            Some("hidden") => SheetState::Hidden,
            Some("veryHidden") => SheetState::VeryHidden,
            Some(state) => return Err(XlsxError::Xml(format!("Unknown sheet state: {state}"))),
        };
        sheets.push(Sheet {
            name,
            sheet_id,
            id,
            state,
        });
    }
    // Get the defined names
    let name_nodes: Vec<&XmlNode> = doc
        .descendants()
        .filter(|n| n.has_tag_name("definedName"))
        .collect();
    for node in name_nodes {
        let name = get_attribute(node, "name")?.to_string();
        // a name's formula is escaped text as a cell's is: `_xHHHH_` decoded
        let formula = decode_xlsx_escapes(node.text().unwrap_or(""));
        // NOTE: In Excel the `localSheetId` is just the index of the worksheet and unrelated to the sheetId
        let sheet_id = match node.attribute("localSheetId") {
            Some(s) => {
                let index = s.parse::<usize>()?;
                Some(sheets[index].sheet_id)
            }
            None => None,
        };
        // `hidden` is an xsd:boolean: "1"/"true" or "0"/"false", default false
        let hidden = matches!(node.attribute("hidden"), Some("1") | Some("true"));
        let comment = node.attribute("comment").unwrap_or("").to_string();
        defined_names.push(DefinedName {
            name,
            formula,
            sheet_id,
            hidden,
            comment,
        })
    }
    // The linked workbooks, in the order the formulas count them ([1], [2], …)
    let mut external_references = Vec::new();
    for node in doc
        .descendants()
        .filter(|n| n.has_tag_name("externalReference"))
    {
        let id = node
            .attribute_ns(
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
                "id",
            )
            .ok_or_else(|| {
                XlsxError::Xml("Missing \"r:id\" XML attribute in externalReference".to_string())
            })?;
        external_references.push(id.to_string());
    }
    Ok(WorkbookXML {
        worksheets: sheets,
        defined_names,
        external_references,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::io::Write;

    use super::*;

    #[test]
    fn a_names_formula_decodes_the_ooxml_escape_like_text() {
        let xml = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets><definedNames><definedName name="Lit">"_x005F_x0041_"</definedName><definedName name="Ch">"_x0041_"</definedName></definedNames></workbook>"#;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file("xl/workbook.xml", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let wb = load_workbook(&mut archive).unwrap();
        let formulas: Vec<&str> = wb.defined_names.iter().map(|n| n.formula.as_str()).collect();
        assert_eq!(formulas, [r#""_x0041_""#, r#""A""#]);
    }
}
