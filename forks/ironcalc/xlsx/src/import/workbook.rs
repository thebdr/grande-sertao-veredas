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
    // The workbook's own lists, where the schema puts them (Veredas fork change 28): `sheets/sheet`,
    // `definedNames/definedName`, `externalReferences/externalReference`, each in SpreadsheetML's
    // namespace; an element of the same local name in an extension, or of another namespace, is
    // none of them
    let list = |list: &'static str, item: &'static str| -> Vec<&XmlNode> {
        doc.main_children(list).flat_map(|l| l.main_children(item)).collect()
    };
    // Get the sheets
    let sheet_nodes = list("sheets", "sheet");
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
    let name_nodes = list("definedNames", "definedName");
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
    for node in list("externalReferences", "externalReference") {
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

    fn workbook_of(xml: &str) -> WorkbookXML {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file("xl/workbook.xml", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        load_workbook(&mut zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap()).unwrap()
    }

    #[test]
    fn the_workbook_s_lists_are_read_where_the_schema_puts_them() {
        // a sheet, a name and a link in an extension (SpreadsheetML's namespace under a prefix, or
        // another), and an element of another namespace in a list: none of the workbook's
        // (fork change 28)
        let main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
        let r = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
        let xml = format!(
            r#"<workbook xmlns="{main}" xmlns:r="{r}" xmlns:x="{main}" xmlns:foo="urn:example:foo"><sheets><sheet name="S" sheetId="1" r:id="rId1"/><foo:sheet name="F" sheetId="5" r:id="rId5"/></sheets><definedNames><definedName name="Real">S!$A$1</definedName></definedNames><extLst><ext uri="{{00000000-0000-0000-0000-0000000000ab}}"><x:sheets><x:sheet name="S" sheetId="2" r:id="rId9"/></x:sheets><foo:sheet name="S" sheetId="3" r:id="rId8"/><x:definedNames><x:definedName name="Decoy">S!$B$1</x:definedName></x:definedNames><x:externalReferences><x:externalReference r:id="rId7"/></x:externalReferences></ext></extLst></workbook>"#
        );
        let wb = workbook_of(&xml);
        let sheets: Vec<(&str, u32, &str)> = wb.worksheets.iter().map(|s| (s.name.as_str(), s.sheet_id, s.id.as_str())).collect();
        assert_eq!(sheets, [("S", 1, "rId1")]);
        let names: Vec<&str> = wb.defined_names.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["Real"]);
        assert!(wb.external_references.is_empty());
        // SpreadsheetML under a prefix (the .NET SDK's spelling) and the strict namespace read alike
        for ns in [main, "http://purl.oclc.org/ooxml/spreadsheetml/main"] {
            let xml = format!(r#"<x:workbook xmlns:x="{ns}" xmlns:r="{r}"><x:sheets><x:sheet name="S" sheetId="1" r:id="rId1"/></x:sheets></x:workbook>"#);
            assert_eq!(workbook_of(&xml).worksheets.len(), 1, "{ns}");
        }
    }
}
