//! Which cells are dynamic arrays: a cell's `cm` attribute is a 1-based index into the cell
//! metadata of the workbook's metadata part (`xl/metadata.xml`, the `sheetMetadata`
//! relationship). Each entry (`<cellMetadata><bk><rc t=".." v=".."/></bk>`) names a metadata type
//! (`t`, 1-based, of `<metadataTypes>`) and a value of it (`v`, 0-based, of that type's
//! `<futureMetadata>`). A dynamic array's type is `XLDAPR`, its value one whose
//! `<xda:dynamicArrayProperties>` says `fDynamic` (`1` or `true`).
//!
//! Excel 365 writes the dynamic array's entry first, so `cm="1"` is what most files hold; a
//! workbook whose metadata part already held cell metadata (an OLAP formula's, say) gets its
//! dynamic array entry at another index. A file with no metadata part, or one this reader cannot
//! read, keeps the reading that `cm="1"` is a dynamic array.

use std::collections::{HashMap, HashSet};
use std::io::{BufReader, Read};

use super::worksheets::Relationship;
use super::xml::XmlNode;

/// The `cm` values that mark a dynamic array in this workbook.
pub(super) fn dynamic_array_indices<R: Read + std::io::Seek>(
    archive: &mut zip::read::ZipArchive<R>,
    rels: &HashMap<String, Relationship>,
) -> HashSet<u32> {
    let fallback = || HashSet::from([1]);
    let Some(target) = rels
        .values()
        .find(|r| r.rel_type.ends_with("/sheetMetadata"))
        .map(|r| r.target.clone())
    else {
        return fallback();
    };
    let path = match target.strip_prefix('/') {
        Some(absolute) => absolute.to_string(),
        None => format!("xl/{target}"),
    };
    let Ok(file) = archive.by_name(&path) else {
        return fallback();
    };
    match XmlNode::parse(BufReader::new(file)) {
        Ok(doc) => indices_of(&doc),
        Err(_) => fallback(),
    }
}

/// The dynamic-array `cm` values a metadata part states.
fn indices_of(doc: &XmlNode) -> HashSet<u32> {
    // the 1-based index of the XLDAPR type
    let Some(types) = doc.child("metadataTypes") else {
        return HashSet::new();
    };
    let Some(type_index) = types
        .children()
        .filter(|n| n.has_tag_name("metadataType"))
        .position(|n| n.attribute("name") == Some("XLDAPR"))
        .map(|i| i + 1)
    else {
        return HashSet::new();
    };
    // the 0-based values of that type that are dynamic
    let dynamic_values: HashSet<usize> = doc
        .children()
        .filter(|n| n.has_tag_name("futureMetadata") && n.attribute("name") == Some("XLDAPR"))
        .flat_map(|list| {
            list.children()
                .filter(|n| n.has_tag_name("bk"))
                .enumerate()
                .filter(|(_, bk)| {
                    bk.descendants().any(|n| {
                        n.has_tag_name("dynamicArrayProperties")
                            && matches!(n.attribute("fDynamic"), Some("1") | Some("true"))
                    })
                })
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        })
        .collect();
    let Some(cells) = doc.child("cellMetadata") else {
        return HashSet::new();
    };
    cells
        .children()
        .filter(|n| n.has_tag_name("bk"))
        .enumerate()
        .filter(|(_, bk)| {
            bk.children().any(|rc| {
                rc.has_tag_name("rc")
                    && rc.attribute("t").and_then(|t| t.parse::<usize>().ok()) == Some(type_index)
                    && rc
                        .attribute("v")
                        .and_then(|v| v.parse::<usize>().ok())
                        .is_some_and(|v| dynamic_values.contains(&v))
            })
        })
        .map(|(i, _)| i as u32 + 1)
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const NS: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:xda="http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray""#;

    fn indices(xml: &str) -> Vec<u32> {
        let mut v: Vec<u32> = indices_of(&XmlNode::parse_str(xml).unwrap()).into_iter().collect();
        v.sort();
        v
    }

    #[test]
    fn excel_365s_own_part_marks_cm_1() {
        let xml = format!(
            r#"<metadata {NS}><metadataTypes count="1"><metadataType name="XLDAPR" minSupportedVersion="120000" cellMeta="1"/></metadataTypes><futureMetadata name="XLDAPR" count="1"><bk><extLst><ext uri="{{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}}"><xda:dynamicArrayProperties fDynamic="1" fCollapsed="0"/></ext></extLst></bk></futureMetadata><cellMetadata count="1"><bk><rc t="1" v="0"/></bk></cellMetadata></metadata>"#
        );
        assert_eq!(indices(&xml), [1]);
    }

    #[test]
    fn an_entry_after_another_types_is_found_where_it_is() {
        // an OLAP formula's cell metadata first: the dynamic array's entry is the second
        let xml = format!(
            r#"<metadata {NS}><metadataTypes count="2"><metadataType name="XLMDX" minSupportedVersion="120000" cellMeta="1"/><metadataType name="XLDAPR" minSupportedVersion="120000" cellMeta="1"/></metadataTypes><metadataStrings count="1"><s v="x"/></metadataStrings><mdxMetadata count="1"><mdx n="0" f="m"><t c="1"><n x="0" s="1"/></t></mdx></mdxMetadata><futureMetadata name="XLDAPR" count="2"><bk><extLst><ext uri="{{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}}"><xda:dynamicArrayProperties fDynamic="0" fCollapsed="0"/></ext></extLst></bk><bk><extLst><ext uri="{{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}}"><xda:dynamicArrayProperties fDynamic="true" fCollapsed="0"/></ext></extLst></bk></futureMetadata><cellMetadata count="3"><bk><rc t="1" v="0"/></bk><bk><rc t="2" v="1"/></bk><bk><rc t="2" v="0"/></bk></cellMetadata></metadata>"#
        );
        assert_eq!(indices(&xml), [2]);
    }

    #[test]
    fn a_part_without_the_dynamic_array_type_marks_none() {
        let xml = format!(
            r#"<metadata {NS}><metadataTypes count="1"><metadataType name="XLRICHVALUE" minSupportedVersion="120000"/></metadataTypes><valueMetadata count="1"><bk><rc t="1" v="0"/></bk></valueMetadata></metadata>"#
        );
        assert!(indices(&xml).is_empty());
    }
}
