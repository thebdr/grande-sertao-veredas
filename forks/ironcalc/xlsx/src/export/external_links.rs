//! Writes the workbooks this one links to: one
//! `xl/externalLinks/externalLinkN.xml` per external book, with its `.rels`
//! (the linked file's path, `TargetMode="External"`), the content-type
//! override, the relationship from `workbook.xml.rels`, and the
//! `<externalReferences>` list in `workbook.xml` that gives the formulas'
//! `[n]` their order. See `import::external_links` for the shape.

use ironcalc_base::{
    expressions::utils::number_to_column,
    external::{ExternalBook, ExternalValue},
    types::Workbook,
};

use super::escape::escape_xml;
use super::xml_constants::XML_DECLARATION;

const EXTERNAL_LINK_REL: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLink";
const EXTERNAL_LINK_PATH_REL: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLinkPath";
pub(crate) const EXTERNAL_LINK_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.externalLink+xml";

/// The relationship ids of `workbook.xml.rels` are `rId1..n` for the
/// worksheets, then styles, sharedStrings, metadata and theme: the first
/// external link is the next one.
pub(crate) fn first_external_link_rel_id(workbook: &Workbook) -> usize {
    workbook.worksheets.len() + 5
}

/// `<Relationship …/>` entries for `workbook.xml.rels`, one per external book.
pub(crate) fn workbook_relationships(workbook: &Workbook) -> Vec<String> {
    let first = first_external_link_rel_id(workbook);
    (0..workbook.external_books.len())
        .map(|index| {
            format!(
                "<Relationship Id=\"rId{}\" Type=\"{EXTERNAL_LINK_REL}\" Target=\"externalLinks/externalLink{}.xml\"/>",
                first + index,
                index + 1
            )
        })
        .collect()
}

/// `<externalReferences>…</externalReferences>` for `workbook.xml`, or an
/// empty string when there are no links.
pub(crate) fn external_references_xml(workbook: &Workbook) -> String {
    if workbook.external_books.is_empty() {
        return String::new();
    }
    let first = first_external_link_rel_id(workbook);
    let references: Vec<String> = (0..workbook.external_books.len())
        .map(|index| format!("<externalReference r:id=\"rId{}\"/>", first + index))
        .collect();
    format!(
        "<externalReferences>{}</externalReferences>",
        references.join("")
    )
}

/// `<Override …/>` entries for `[Content_Types].xml`.
pub(crate) fn content_type_overrides(workbook: &Workbook) -> Vec<String> {
    (0..workbook.external_books.len())
        .map(|index| {
            format!(
                "<Override PartName=\"/xl/externalLinks/externalLink{}.xml\" ContentType=\"{EXTERNAL_LINK_CONTENT_TYPE}\"/>",
                index + 1
            )
        })
        .collect()
}

/// The `.rels` of one external link part: `rId1` is the linked file, `rId2`
/// its absolute URL when the book carries one.
pub(crate) fn external_link_rels_xml(book: &ExternalBook) -> String {
    let mut rels = vec![format!(
        "<Relationship Id=\"rId1\" Type=\"{EXTERNAL_LINK_PATH_REL}\" Target=\"{}\" TargetMode=\"External\"/>",
        escape_xml(&book.target)
    )];
    if let Some(url) = &book.absolute_url {
        rels.push(format!(
            "<Relationship Id=\"rId2\" Type=\"{EXTERNAL_LINK_PATH_REL}\" Target=\"{}\" TargetMode=\"External\"/>",
            escape_xml(url)
        ));
    }
    format!(
        "{XML_DECLARATION}\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{}</Relationships>",
        rels.join("")
    )
}

fn cell_xml(reference: &str, value: &ExternalValue) -> String {
    match value {
        ExternalValue::Number(n) => {
            format!("<cell r=\"{reference}\"><v>{}</v></cell>", n)
        }
        ExternalValue::String(s) => {
            // leading/trailing whitespace must be declared significant
            let space = if s.starts_with(char::is_whitespace) || s.ends_with(char::is_whitespace) {
                " xml:space=\"preserve\""
            } else {
                ""
            };
            format!(
                "<cell r=\"{reference}\" t=\"str\"><v{space}>{}</v></cell>",
                escape_xml(s)
            )
        }
        ExternalValue::Boolean(b) => {
            format!(
                "<cell r=\"{reference}\" t=\"b\"><v>{}</v></cell>",
                if *b { 1 } else { 0 }
            )
        }
        ExternalValue::Error(e) => {
            format!(
                "<cell r=\"{reference}\" t=\"e\"><v>{}</v></cell>",
                escape_xml(&e.to_string())
            )
        }
    }
}

/// One `externalLinkN.xml`: the book's sheets, defined names and cached
/// cells, rows and cells in order. A book that is not a workbook (a DDE or
/// OLE link read as a placeholder) is written as an external book with no
/// sheets, which keeps the `[n]` numbering of the formulas.
pub(crate) fn external_link_xml(book: &ExternalBook) -> String {
    let mut xml = String::with_capacity(1024);
    xml.push_str(XML_DECLARATION);
    xml.push('\n');
    let has_alternate = book.absolute_url.is_some();
    if has_alternate {
        xml.push_str(
            "<externalLink xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
             xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"xxl21\" \
             xmlns:xxl21=\"http://schemas.microsoft.com/office/spreadsheetml/2021/extlinks2021\">",
        );
    } else {
        xml.push_str(
            "<externalLink xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">",
        );
    }
    xml.push_str(
        "<externalBook xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:id=\"rId1\">",
    );
    if has_alternate {
        xml.push_str(
            "<xxl21:alternateUrls><xxl21:absoluteUrl r:id=\"rId2\"/></xxl21:alternateUrls>",
        );
    }
    xml.push_str("<sheetNames>");
    for sheet in &book.sheets {
        xml.push_str(&format!("<sheetName val=\"{}\"/>", escape_xml(&sheet.name)));
    }
    xml.push_str("</sheetNames>");
    if !book.defined_names.is_empty() {
        xml.push_str("<definedNames>");
        for name in &book.defined_names {
            let sheet_id = match name.sheet_id {
                Some(id) => format!(" sheetId=\"{id}\""),
                None => String::new(),
            };
            xml.push_str(&format!(
                "<definedName name=\"{}\" refersTo=\"{}\"{sheet_id}/>",
                escape_xml(&name.name),
                escape_xml(&name.refers_to)
            ));
        }
        xml.push_str("</definedNames>");
    }
    xml.push_str("<sheetDataSet>");
    for (index, sheet) in book.sheets.iter().enumerate() {
        if sheet.cells.is_empty() {
            xml.push_str(&format!("<sheetData sheetId=\"{index}\"/>"));
            continue;
        }
        xml.push_str(&format!("<sheetData sheetId=\"{index}\">"));
        let mut cells: Vec<(&(i32, i32), &ExternalValue)> = sheet.cells.iter().collect();
        cells.sort_by_key(|(position, _)| **position);
        let mut current_row: Option<i32> = None;
        for ((row, column), value) in cells {
            if current_row != Some(*row) {
                if current_row.is_some() {
                    xml.push_str("</row>");
                }
                xml.push_str(&format!("<row r=\"{row}\">"));
                current_row = Some(*row);
            }
            let column_name = number_to_column(*column).unwrap_or_default();
            xml.push_str(&cell_xml(&format!("{column_name}{row}"), value));
        }
        if current_row.is_some() {
            xml.push_str("</row>");
        }
        xml.push_str("</sheetData>");
    }
    xml.push_str("</sheetDataSet></externalBook></externalLink>");
    xml
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ironcalc_base::{
        expressions::token::Error,
        external::{ExternalDefinedName, ExternalSheet},
    };

    use super::*;

    fn book() -> ExternalBook {
        let mut cells = HashMap::new();
        cells.insert((6, 2), ExternalValue::String("S1 & <2>".to_string()));
        cells.insert((4, 2), ExternalValue::Number(2.0));
        cells.insert((4, 1), ExternalValue::Boolean(false));
        cells.insert((9, 3), ExternalValue::Error(Error::NA));
        cells.insert((9, 2), ExternalValue::String(" ".to_string()));
        ExternalBook {
            target: "Linked.xlsx".to_string(),
            absolute_url: Some("https://example.invalid/a%20b/Linked.xlsx".to_string()),
            sheets: vec![
                ExternalSheet {
                    name: "Cover".to_string(),
                    cells: HashMap::new(),
                },
                ExternalSheet {
                    name: "ZONE SET".to_string(),
                    cells,
                },
            ],
            defined_names: vec![ExternalDefinedName {
                name: "Zones".to_string(),
                refers_to: "=ZONES!$A$23:$A$30".to_string(),
                sheet_id: Some(1),
            }],
        }
    }

    #[test]
    fn link_xml_is_ordered_and_escaped() {
        let xml = external_link_xml(&book());
        let expected = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
            "<externalLink xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" ",
            "xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"xxl21\" ",
            "xmlns:xxl21=\"http://schemas.microsoft.com/office/spreadsheetml/2021/extlinks2021\">",
            "<externalBook xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:id=\"rId1\">",
            "<xxl21:alternateUrls><xxl21:absoluteUrl r:id=\"rId2\"/></xxl21:alternateUrls>",
            "<sheetNames><sheetName val=\"Cover\"/><sheetName val=\"ZONE SET\"/></sheetNames>",
            "<definedNames><definedName name=\"Zones\" refersTo=\"=ZONES!$A$23:$A$30\" sheetId=\"1\"/></definedNames>",
            "<sheetDataSet><sheetData sheetId=\"0\"/><sheetData sheetId=\"1\">",
            "<row r=\"4\"><cell r=\"A4\" t=\"b\"><v>0</v></cell><cell r=\"B4\"><v>2</v></cell></row>",
            "<row r=\"6\"><cell r=\"B6\" t=\"str\"><v>S1 &amp; &lt;2&gt;</v></cell></row>",
            "<row r=\"9\"><cell r=\"B9\" t=\"str\"><v xml:space=\"preserve\"> </v></cell><cell r=\"C9\" t=\"e\"><v>#N/A</v></cell></row>",
            "</sheetData></sheetDataSet></externalBook></externalLink>"
        );
        assert_eq!(xml, expected);
    }

    #[test]
    fn rels_and_workbook_entries() {
        let rels = external_link_rels_xml(&book());
        assert!(rels.contains("Id=\"rId1\""));
        assert!(rels.contains("Target=\"Linked.xlsx\" TargetMode=\"External\""));
        assert!(rels.contains("Id=\"rId2\""));
        assert!(rels.contains(
            "Target=\"https://example.invalid/a%20b/Linked.xlsx\" TargetMode=\"External\""
        ));

        let mut workbook = ironcalc_base::Model::new_empty("m", "en", "UTC", "en")
            .unwrap()
            .workbook;
        assert_eq!(external_references_xml(&workbook), "");
        assert!(workbook_relationships(&workbook).is_empty());
        assert!(content_type_overrides(&workbook).is_empty());
        workbook.external_books.push(book());
        workbook.external_books.push(ExternalBook::default());
        // one sheet: rId1 sheet, rId2..5 styles/strings/metadata/theme, rId6 first link
        assert_eq!(
            external_references_xml(&workbook),
            "<externalReferences><externalReference r:id=\"rId6\"/><externalReference r:id=\"rId7\"/></externalReferences>"
        );
        let rels = workbook_relationships(&workbook);
        assert_eq!(rels.len(), 2);
        assert!(
            rels[0].contains("Id=\"rId6\"")
                && rels[0].contains("Target=\"externalLinks/externalLink1.xml\"")
        );
        assert!(
            rels[1].contains("Id=\"rId7\"")
                && rels[1].contains("Target=\"externalLinks/externalLink2.xml\"")
        );
        let overrides = content_type_overrides(&workbook);
        assert_eq!(overrides.len(), 2);
        assert!(overrides[1].contains("PartName=\"/xl/externalLinks/externalLink2.xml\""));
        // the placeholder is a book with no sheets
        let xml = external_link_xml(&ExternalBook::default());
        assert!(xml.contains("<sheetNames></sheetNames><sheetDataSet></sheetDataSet>"));
    }
}
