mod cell_metadata;
mod conditional_formatting;
mod external_links;
pub mod linked;
mod metadata;
pub(crate) mod shared_strings;
mod sheet_data;
mod styles;
mod tables;
mod theme;
mod util;
mod workbook;
mod worksheets;
mod xml;

use std::{
    collections::HashMap,
    fs,
    io::{BufReader, Cursor, Read},
};

use xml::XmlNode;

use ironcalc_base::{
    expressions::{
        parser::{new_parser_english, stringify::to_english_string},
        types::CellReferenceRC,
    },
    types::{Metadata, Workbook, WorkbookSettings},
    Model,
};

use crate::error::XlsxError;

use shared_strings::{read_shared_strings, SharedStringTable};

use external_links::load_external_links;
pub use linked::{LinkState, LinkStatus, LinkedResolver, DEFAULT_LINK_DEPTH};
use metadata::load_metadata;
use styles::load_styles;
use util::get_attribute;
use workbook::load_workbook;
use worksheets::{load_sheets, Relationship};

fn load_relationships<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<HashMap<String, Relationship>, XlsxError> {
    let file = archive.by_name("xl/_rels/workbook.xml.rels")?;
    let doc = XmlNode::parse(BufReader::new(file))?;
    let nodes: Vec<&XmlNode> = doc
        .descendants()
        .filter(|n| n.has_tag_name("Relationship"))
        .collect();
    let names = part_names(archive);
    let mut rels = HashMap::new();
    for node in nodes {
        // `Id` an xsd:ID, `Type` and `Target` xsd:anyURI: their whitespace collapses (Veredas fork
        // change 27); an internal target is the part it names, as below
        let target = collapse(get_attribute(node, "Target")?);
        let target = if get_attribute(node, "TargetMode").ok().map(collapse).as_deref() == Some("External") {
            target
        } else {
            part_target(&target, &names)
        };
        rels.insert(
            collapse(get_attribute(node, "Id")?),
            Relationship {
                rel_type: collapse(get_attribute(node, "Type")?),
                target,
            },
        );
    }
    Ok(rels)
}

/// XSD's whitespace collapsed: the ends trimmed, every inner run one space.
pub(crate) fn collapse(s: &str) -> String {
    s.split([' ', '\t', '\n', '\r'])
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The names of the package's entries.
pub(crate) fn part_names<R: Read + std::io::Seek>(archive: &zip::ZipArchive<R>) -> Vec<String> {
    archive
        .file_names()
        .filter_map(|n| n.ok().map(|n| n.to_string()))
        .collect()
}

/// The part a target of the workbook's relationships names ([`resolve_part`] from `xl`), written
/// absolute (`/xl/worksheets/sheet1.xml`), which every reader of a target here takes as it is.
fn part_target(target: &str, names: &[String]) -> String {
    format!("/{}", resolve_part("xl", target, names))
}

/// The part a relationship's target names, as OPC resolves it: against the folder of the part the
/// relationship belongs to, or the package's root when it opens with `/`, its `.` and `..` segments
/// removed, and spelled as the package's entry of that name in any letter case (OPC compares part
/// names so).
pub(crate) fn resolve_part(folder: &str, target: &str, names: &[String]) -> String {
    let joined = match target.strip_prefix('/') {
        Some(absolute) => absolute.to_string(),
        None => format!("{folder}/{target}"),
    };
    let mut segments: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    let resolved = segments.join("/");
    names
        .iter()
        .find(|n| **n == resolved)
        .or_else(|| names.iter().find(|n| n.eq_ignore_ascii_case(&resolved)))
        .cloned()
        .unwrap_or(resolved)
}

fn resolve_theme_path(rels: &HashMap<String, Relationship>) -> Option<String> {
    let target = rels
        .values()
        .find(|r| r.rel_type.ends_with("/theme"))?
        .target
        .clone();
    Some(if let Some(absolute) = target.strip_prefix('/') {
        absolute.to_string()
    } else {
        format!("xl/{target}")
    })
}

// FIXME: This is a bit of a HACK. We basically re-parse all the defined names assuming the context is `A1`
// and there is no tables or defined names.
fn reparse_formula_hack(formula: &str, worksheets: &[String]) -> Result<String, XlsxError> {
    let defined_names = Vec::new();
    let tables = HashMap::new();
    let mut parser = new_parser_english(worksheets.to_owned(), defined_names, tables);
    let cell_reference = CellReferenceRC {
        sheet: worksheets[0].clone(),
        column: 1,
        row: 1,
    };
    let t = parser.parse(formula, &cell_reference);

    Ok(to_english_string(&t, &cell_reference))
}

fn load_xlsx_from_reader<R: Read + std::io::Seek>(
    name: String,
    reader: R,
    locale: &str,
    tz: &str,
) -> Result<Workbook, XlsxError> {
    let mut archive = zip::ZipArchive::new(reader)?;

    let mut shared_strings = SharedStringTable::new(read_shared_strings(&mut archive)?);
    let mut workbook = load_workbook(&mut archive)?;
    let rels = load_relationships(&mut archive)?;
    let theme_path = resolve_theme_path(&rels);
    let theme = theme::load(&mut archive, theme_path.as_deref());
    let mut tables = HashMap::new();
    // Styles must be loaded before the worksheets: conditional-formatting rules
    // stored in x14 `extLst` extensions carry inline `<x14:dxf>` formats that we
    // append to `styles.dxfs`, referencing them back by index from the rule.
    let mut styles = load_styles(&mut archive, &theme)?;
    // which cells' `cm` marks a dynamic array, read from the metadata part
    let dynamic_cm = cell_metadata::dynamic_array_indices(&mut archive, &rels);
    let worksheets = load_sheets(
        &mut archive,
        &rels,
        &workbook,
        &mut tables,
        &mut shared_strings,
        &theme,
        &mut styles.dxfs,
        &dynamic_cm,
    )?;
    // reparse formulas in defined names, since they may refer to sheets and tables that have been loaded
    let worksheet_names = worksheets
        .iter()
        .map(|s| s.name.clone())
        .collect::<Vec<_>>();
    for dn in &mut workbook.defined_names {
        dn.formula = reparse_formula_hack(&dn.formula, &worksheet_names)?;
    }
    // The workbooks this one links to, in the order the formulas count them
    let external_books = load_external_links(&mut archive, &workbook.external_references, &rels)?;
    let metadata = match load_metadata(&mut archive) {
        Ok(metadata) => metadata,
        Err(_) => {
            // In case there is no metadata, add some
            Metadata {
                application: "Unknown application".to_string(),
                app_version: "".to_string(),
                creator: "".to_string(),
                last_modified_by: "".to_string(),
                created: "".to_string(),
                last_modified: "".to_string(),
            }
        }
    };
    Ok(Workbook {
        shared_strings: shared_strings.into_strings(),
        defined_names: workbook.defined_names,
        worksheets,
        styles,
        name,
        settings: WorkbookSettings {
            tz: tz.to_string(),
            locale: locale.to_string(),
        },
        metadata,
        tables,
        theme,
        external_books,
    })
}

// Imports a file from disk into an internal representation
fn load_from_excel(file_name: &str, locale: &str, tz: &str) -> Result<Workbook, XlsxError> {
    let file_path = std::path::Path::new(file_name);
    let file = fs::File::open(file_path)?;
    let reader = BufReader::new(file);
    let name = file_path
        .file_stem()
        .ok_or_else(|| XlsxError::IO("Could not extract workbook name".to_string()))?
        .to_string_lossy()
        .to_string();
    load_xlsx_from_reader(name, reader, locale, tz)
}

/// Loads a [Workbook] from the bytes of an xlsx file.
/// This is useful, for instance, when bytes are transferred over the network
pub fn load_from_xlsx_bytes(
    bytes: &[u8],
    name: &str,
    locale: &str,
    tz: &str,
) -> Result<Workbook, XlsxError> {
    let cursor = Cursor::new(bytes);
    let reader = BufReader::new(cursor);
    load_xlsx_from_reader(name.to_string(), reader, locale, tz)
}

/// Loads a [Model] from an xlsx file
pub fn load_from_xlsx<'a>(
    file_name: &str,
    locale: &str,
    tz: &str,
    language: &'a str,
) -> Result<Model<'a>, XlsxError> {
    let workbook = load_from_excel(file_name, locale, tz)?;
    Model::from_workbook(workbook, language).map_err(XlsxError::Workbook)
}

/// Loads a [Model] from an `ic` file (a file in the IronCalc internal representation)
pub fn load_from_icalc<'a>(file_name: &str, language_id: &'a str) -> Result<Model<'a>, XlsxError> {
    let contents = fs::read(file_name)
        .map_err(|e| XlsxError::IO(format!("Could not extract workbook name: {e}")))?;
    let workbook: Workbook = bitcode::decode(&contents)
        .map_err(|e| XlsxError::IO(format!("Failed to decode file: {e}")))?;
    Model::from_workbook(workbook, language_id).map_err(XlsxError::Workbook)
}

#[cfg(test)]
mod zip_input_tests {
    use super::{load_from_xlsx_bytes, XlsxError};

    #[test]
    fn malformed_zip_inputs_remain_zip_errors() {
        for bytes in [b"".as_slice(), b"not a ZIP archive", b"PK\x03\x04"] {
            let result = load_from_xlsx_bytes(bytes, "invalid", "en", "UTC");
            assert!(matches!(result, Err(XlsxError::Zip(_))));
        }
    }
}
