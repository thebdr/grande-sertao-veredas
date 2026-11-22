//! The zip package around the sheets: reading entries in order, writing the
//! patched package with every untouched entry RAW-copied (its compressed
//! bytes, CRC and all — not even recompressed), and the small XML parts that
//! tie the package together (`[Content_Types].xml`, the rels,
//! `workbook.xml`), plus the parts that point into a sheet's grid and must
//! follow a structural move (comments and their VML shapes, drawings,
//! charts, tables).

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::refs::{self, Shift};
use super::xml::{self, attr, decode_text, escape_text, next_tag, set_attr};
use super::PatchError;

/// The source package: its entries in order, read lazily.
pub(crate) struct Package {
    archive: ZipArchive<Cursor<Vec<u8>>>,
    pub names: Vec<String>,
}

impl Package {
    pub fn open(bytes: Vec<u8>) -> Result<Package, PatchError> {
        let archive = ZipArchive::new(Cursor::new(bytes))
            .map_err(|e| PatchError::Malformed(format!("not a zip package: {e}")))?;
        let mut names = Vec::with_capacity(archive.len());
        for i in 0..archive.len() {
            let name = archive
                .name_for_index(i)
                .ok_or_else(|| PatchError::Malformed(format!("zip entry {i} has no name")))?
                .map_err(|e| PatchError::Malformed(format!("zip entry {i}: {e}")))?;
            names.push(name.to_string());
        }
        Ok(Package { archive, names })
    }

    pub fn has(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    pub fn read(&mut self, name: &str) -> Result<Vec<u8>, PatchError> {
        let mut f = self
            .archive
            .by_name(name)
            .map_err(|e| PatchError::Malformed(format!("{name}: {e}")))?;
        let mut out = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut out)
            .map_err(|e| PatchError::Malformed(format!("{name}: {e}")))?;
        Ok(out)
    }

    pub fn read_text(&mut self, name: &str) -> Result<String, PatchError> {
        String::from_utf8(self.read(name)?)
            .map_err(|_| PatchError::Malformed(format!("{name}: not UTF-8")))
    }

    /// Writes the package: entries in their original order (replaced ones
    /// with their new bytes, removed ones left out, the rest raw-copied),
    /// then the added ones.
    pub fn write(
        mut self,
        replaced: &BTreeMap<String, Vec<u8>>,
        removed: &[String],
        added: &[(String, Vec<u8>)],
    ) -> Result<Vec<u8>, PatchError> {
        let io =
            |e: zip::result::ZipError| PatchError::Malformed(format!("writing the package: {e}"));
        let mut w = ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for i in 0..self.names.len() {
            let name = self.names[i].clone();
            if removed.contains(&name) {
                continue;
            }
            if let Some(bytes) = replaced.get(&name) {
                w.start_file(name.as_str(), opts).map_err(io)?;
                w.write_all(bytes)
                    .map_err(|e| PatchError::Malformed(e.to_string()))?;
            } else {
                let f = self.archive.by_index_raw(i).map_err(io)?;
                w.raw_copy_file(f).map_err(io)?;
            }
        }
        for (name, bytes) in added {
            w.start_file(name.as_str(), opts).map_err(io)?;
            w.write_all(bytes)
                .map_err(|e| PatchError::Malformed(e.to_string()))?;
        }
        Ok(w.finish().map_err(io)?.into_inner())
    }
}

/// `xl/worksheets/sheet1.xml` → `xl/worksheets/_rels/sheet1.xml.rels`.
pub(crate) fn rels_of(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// Resolves a relationship target against the part that owns the rels.
pub(crate) fn resolve(owner: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = owner.split('/').collect();
    parts.pop();
    for seg in target.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// One relationship: id, type (its last path segment: `worksheet`,
/// `comments`, …), resolved target, external flag.
#[derive(Debug, Clone)]
pub(crate) struct Rel {
    pub id: String,
    pub kind: String,
    pub target: String,
    pub external: bool,
}

pub(crate) fn rels(owner: &str, rels_xml: &str) -> Vec<Rel> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(t) = xml::find_open(rels_xml, "Relationship", pos) {
        pos = t.end;
        let (Some(id), Some(ty), Some(target)) = (
            attr(t.text, "Id"),
            attr(t.text, "Type"),
            attr(t.text, "Target"),
        ) else {
            continue;
        };
        let external = attr(t.text, "TargetMode") == Some("External");
        let target = xml::unescape(target).into_owned();
        out.push(Rel {
            id: id.to_string(),
            kind: ty.rsplit('/').next().unwrap_or_default().to_string(),
            target: if external {
                target
            } else {
                resolve(owner, &target)
            },
            external,
        });
    }
    out
}

/// A sheet as `workbook.xml` lists it.
#[derive(Debug, Clone)]
pub(crate) struct SheetEntry {
    pub name: String,
    pub sheet_id: u32,
    #[allow(dead_code)]
    pub rid: String,
    pub part: String,
    /// What the `<sheet>` is, by its relationship: `worksheet`, or a `chartsheet` (a chart on
    /// its own tab, no cells), a `dialogsheet` or a `macrosheet`. Every kind counts in the
    /// `<sheets>` order a defined name's `localSheetId` indexes; only a worksheet holds cells.
    pub kind: String,
}

impl SheetEntry {
    pub fn is_worksheet(&self) -> bool {
        self.kind == "worksheet"
    }
}

pub(crate) fn sheets(workbook_xml: &str, wb_rels: &[Rel]) -> Result<Vec<SheetEntry>, PatchError> {
    let mut out = Vec::new();
    let sheets = xml::find_open(workbook_xml, "sheets", 0)
        .ok_or_else(|| PatchError::Malformed("workbook.xml: no <sheets>".into()))?;
    let (_, end) = xml::element_span(workbook_xml, &sheets)
        .ok_or_else(|| PatchError::Malformed("workbook.xml: <sheets> not closed".into()))?;
    let mut pos = sheets.end;
    while let Some(t) = xml::find_open(&workbook_xml[..end], "sheet", pos) {
        pos = t.end;
        let name = attr(t.text, "name").map(decode_text).unwrap_or_default();
        let sheet_id = attr(t.text, "sheetId")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let rid = rid_attr(t.text).unwrap_or_default().to_string();
        let rel = wb_rels.iter().find(|r| r.id == rid).ok_or_else(|| {
            PatchError::Malformed(format!("sheet {name:?}: relationship {rid} not found"))
        })?;
        out.push(SheetEntry {
            name,
            sheet_id,
            rid,
            part: rel.target.clone(),
            kind: rel.kind.clone(),
        });
    }
    Ok(out)
}

/// The value of the `…:id` attribute (the relationships namespace prefix is
/// usually `r`, but a file may bind another).
fn rid_attr(tag: &str) -> Option<&str> {
    attr(tag, "r:id").or_else(|| {
        let i = tag.find(":id=\"")?;
        let s = i + 5;
        let e = s + tag[s..].find('"')?;
        Some(&tag[s..e])
    })
}

/// `[Content_Types].xml` without the override for `part`.
pub(crate) fn remove_override(ct: &str, part: &str) -> String {
    let needle = format!("PartName=\"/{part}\"");
    let mut pos = 0;
    while let Some(t) = xml::find_open(ct, "Override", pos) {
        if t.text.contains(&needle) {
            return format!("{}{}", &ct[..t.start], &ct[t.end..]);
        }
        pos = t.end;
    }
    ct.to_string()
}

pub(crate) fn add_override(ct: &str, part: &str, content_type: &str) -> String {
    let tag = format!("<Override PartName=\"/{part}\" ContentType=\"{content_type}\"/>");
    match ct.rfind("</Types>") {
        Some(i) => format!("{}{}{}", &ct[..i], tag, &ct[i..]),
        None => ct.to_string(),
    }
}

/// The rels without the relationship whose resolved target is `part`.
pub(crate) fn remove_rel_to(owner: &str, rels_xml: &str, part: &str) -> String {
    let mut pos = 0;
    while let Some(t) = xml::find_open(rels_xml, "Relationship", pos) {
        if let Some(target) = attr(t.text, "Target")
            && resolve(owner, &xml::unescape(target)) == part {
                return format!("{}{}", &rels_xml[..t.start], &rels_xml[t.end..]);
            }
        pos = t.end;
    }
    rels_xml.to_string()
}

pub(crate) fn add_rel(rels_xml: &str, id: &str, kind_uri: &str, target: &str) -> String {
    let tag = format!(
        "<Relationship Id=\"{id}\" Type=\"{kind_uri}\" Target=\"{}\"/>",
        escape_text(target)
    );
    match rels_xml.rfind("</Relationships>") {
        Some(i) => format!("{}{}{}", &rels_xml[..i], tag, &rels_xml[i..]),
        None => rels_xml.to_string(),
    }
}

/// The first `rIdN` not used in the rels.
pub(crate) fn free_rid(rels: &[Rel]) -> String {
    let max = rels
        .iter()
        .filter_map(|r| r.id.strip_prefix("rId").and_then(|n| n.parse::<u32>().ok()))
        .max()
        .unwrap_or(0);
    format!("rId{}", max + 1)
}

// ---- parts that point into a sheet's grid --------------------------------

fn map_index(shifts: &[Shift], rows: bool, mut v: i32) -> Option<i32> {
    for s in shifts.iter().filter(|s| s.is_rows() == rows) {
        v = s.index(v)?;
    }
    Some(v)
}

/// `commentsN.xml` / threaded comments: each `ref` follows its cell; a
/// comment on a deleted cell goes. Returns the new part and the (0-based)
/// cells whose notes were dropped, for the VML.
pub(crate) fn shift_comments(
    s: &str,
    own: &[Shift],
    tag_name: &str,
) -> (Option<String>, Vec<(i32, i32)>) {
    let mut out = String::with_capacity(s.len());
    let mut dropped = Vec::new();
    let mut pos = 0;
    while let Some(t) = next_tag(s, pos) {
        out.push_str(&s[pos..t.start]);
        pos = t.end;
        if !t.closing && t.name == tag_name
            && let Some(r) = attr(t.text, "ref")
                && let Some(area) = refs::parse_area(r) {
                    let (row, col, _, _) = area.bounds();
                    match (map_index(own, true, row), map_index(own, false, col)) {
                        (Some(nr), Some(nc)) => {
                            let a = format!("{}{}", crate::a1::column_name(nc), nr);
                            out.push_str(&set_attr(t.text, "ref", &a));
                        }
                        _ => {
                            dropped.push((row - 1, col - 1));
                            pos = xml::element_span(s, &t).map(|(_, b)| b).unwrap_or(t.end);
                        }
                    }
                    continue;
                }
        out.push_str(t.text);
    }
    out.push_str(&s[pos..]);
    ((out != s).then_some(out), dropped)
}

/// The legacy VML of notes: `<x:Row>`/`<x:Column>` (0-based) and the
/// `<x:Anchor>` box follow the cell; a shape whose cell is gone goes.
pub(crate) fn shift_vml(s: &str, own: &[Shift]) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut pos = 0;
    while let Some(t) = next_tag(s, pos) {
        if t.closing || t.name != "v:shape" {
            out.push_str(&s[pos..t.end]);
            pos = t.end;
            continue;
        }
        out.push_str(&s[pos..t.start]);
        let Some((a, b)) = xml::element_span(s, &t) else {
            out.push_str(t.text);
            pos = t.end;
            continue;
        };
        let shape = &s[a..b];
        let row = xml::inner(shape, "x:Row").and_then(|v| v.trim().parse::<i32>().ok());
        let col = xml::inner(shape, "x:Column").and_then(|v| v.trim().parse::<i32>().ok());
        match (row, col) {
            (Some(r), Some(c)) => {
                // a note whose cell is gone goes with it
                if let (Some(nr), Some(nc)) =
                    (map_index(own, true, r + 1), map_index(own, false, c + 1))
                {
                    let mut sh = replace_inner(shape, "x:Row", &(nr - 1).to_string());
                    sh = replace_inner(&sh, "x:Column", &(nc - 1).to_string());
                    if let Some(anchor) = xml::inner(&sh, "x:Anchor") {
                        let mut nums: Vec<i32> = anchor
                            .split(',')
                            .filter_map(|n| n.trim().parse().ok())
                            .collect();
                        if nums.len() == 8 {
                            for (i, rows) in [(0, false), (2, true), (4, false), (6, true)] {
                                nums[i] = clamp(own, rows, nums[i] + 1) - 1;
                            }
                            let text = nums
                                .iter()
                                .map(|n| n.to_string())
                                .collect::<Vec<_>>()
                                .join(", ");
                            sh = replace_inner(&sh, "x:Anchor", &text);
                        }
                    }
                    out.push_str(&sh);
                }
            }
            _ => out.push_str(shape),
        }
        pos = b;
    }
    out.push_str(&s[pos..]);
    (out != s).then_some(out)
}

fn clamp(own: &[Shift], rows: bool, mut v: i32) -> i32 {
    for sh in own.iter().filter(|x| x.is_rows() == rows) {
        v = match sh.index(v) {
            Some(n) => n,
            None => match *sh {
                Shift::DeleteRows { at, .. } | Shift::DeleteCols { at, .. } => at.max(1),
                _ => v,
            },
        };
    }
    v
}

fn replace_inner(s: &str, name: &str, value: &str) -> String {
    let Some(open) = xml::find_open(s, name, 0) else {
        return s.to_string();
    };
    let Some(close) = xml::find_close(s, &open) else {
        return s.to_string();
    };
    format!("{}{}{}", &s[..open.end], value, &s[close.start..])
}

/// Drawing anchors (`xdr:from`/`xdr:to`, 0-based col/row) follow the grid.
pub(crate) fn shift_drawing(s: &str, own: &[Shift]) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut pos = 0;
    while let Some(t) = next_tag(s, pos) {
        out.push_str(&s[pos..t.start]);
        pos = t.end;
        let rows = match t.name {
            "xdr:row" => Some(true),
            "xdr:col" => Some(false),
            _ => None,
        };
        if let (Some(rows), false) = (rows, t.closing || t.self_closing)
            && let Some(close) = xml::find_close(s, &t)
                && let Ok(v) = s[t.end..close.start].trim().parse::<i32>() {
                    let n = clamp(own, rows, v + 1) - 1;
                    out.push_str(t.text);
                    out.push_str(&n.to_string());
                    out.push_str(close.text);
                    pos = close.end;
                    continue;
                }
        out.push_str(t.text);
    }
    out.push_str(&s[pos..]);
    (out != s).then_some(out)
}

/// Formula text in elements named `name` (`c:f` in charts,
/// `calculatedColumnFormula` in tables), shifted by every move.
pub(crate) fn shift_formula_elements(
    s: &str,
    name: &str,
    shifts: &[(String, Shift)],
    home: Option<&str>,
    order: &[String],
) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut pos = 0;
    while let Some(t) = next_tag(s, pos) {
        out.push_str(&s[pos..t.start]);
        pos = t.end;
        if !t.closing && !t.self_closing && t.name == name
            && let Some(close) = xml::find_close(s, &t) {
                let raw = &s[t.end..close.start];
                let mut cur = decode_text(raw);
                let mut changed = false;
                for (sheet, sh) in shifts {
                    if let Some(n) = refs::shift_formula(&cur, sheet, *sh, home, order) {
                        cur = n;
                        changed = true;
                    }
                }
                out.push_str(t.text);
                out.push_str(&if changed {
                    escape_text(&cur)
                } else {
                    raw.to_string()
                });
                out.push_str(close.text);
                pos = close.end;
                continue;
            }
        out.push_str(t.text);
    }
    out.push_str(&s[pos..]);
    (out != s).then_some(out)
}

/// A table on a moved sheet: its `ref` and its filter's `ref`.
pub(crate) fn shift_table(s: &str, own: &[Shift]) -> Result<Option<String>, PatchError> {
    let mut out = s.to_string();
    for name in ["table", "autoFilter"] {
        let Some(t) = xml::find_open(&out, name, 0) else {
            continue;
        };
        let Some(r) = attr(t.text, "ref") else {
            continue;
        };
        let Some(mut area) = refs::parse_area(r) else {
            continue;
        };
        for sh in own {
            area = sh.area(area).ok_or_else(|| {
                PatchError::Unsupported(format!("a structural edit deletes the whole table at {r}"))
            })?;
        }
        let n = set_attr(t.text, "ref", &area.spell());
        out = format!("{}{}{}", &out[..t.start], n, &out[t.end..]);
    }
    Ok((out != s).then_some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rels_resolve_relative_targets() {
        assert_eq!(
            resolve("xl/workbook.xml", "worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(
            resolve("xl/worksheets/sheet1.xml", "../comments1.xml"),
            "xl/comments1.xml"
        );
        assert_eq!(
            resolve("xl/workbook.xml", "/xl/calcChain.xml"),
            "xl/calcChain.xml"
        );
        assert_eq!(
            rels_of("xl/worksheets/sheet2.xml"),
            "xl/worksheets/_rels/sheet2.xml.rels"
        );
    }

    #[test]
    fn comments_and_vml_follow_their_cells() {
        let c = r#"<comments><commentList><comment ref="B5" authorId="0"><text>x</text></comment><comment ref="B9" authorId="0"><text>y</text></comment></commentList></comments>"#;
        let own = [Shift::DeleteRows { at: 5, n: 1 }];
        let (n, dropped) = shift_comments(c, &own, "comment");
        assert_eq!(
            n.unwrap(),
            r#"<comments><commentList><comment ref="B8" authorId="0"><text>y</text></comment></commentList></comments>"#
        );
        assert_eq!(dropped, vec![(4, 1)]);
        let v = r#"<xml><v:shape id="a"><x:ClientData ObjectType="Note"><x:Anchor>2, 15, 3, 2, 4, 15, 7, 4</x:Anchor><x:Row>4</x:Row><x:Column>1</x:Column></x:ClientData></v:shape><v:shape id="b"><x:ClientData ObjectType="Note"><x:Anchor>2, 15, 7, 2, 4, 15, 11, 4</x:Anchor><x:Row>8</x:Row><x:Column>1</x:Column></x:ClientData></v:shape></xml>"#;
        let n = shift_vml(v, &own).unwrap();
        assert_eq!(
            n,
            r#"<xml><v:shape id="b"><x:ClientData ObjectType="Note"><x:Anchor>2, 15, 6, 2, 4, 15, 10, 4</x:Anchor><x:Row>7</x:Row><x:Column>1</x:Column></x:ClientData></v:shape></xml>"#
        );
    }

    #[test]
    fn content_types_and_rels_edits() {
        let ct = r#"<Types><Override PartName="/xl/calcChain.xml" ContentType="x"/><Override PartName="/xl/workbook.xml" ContentType="y"/></Types>"#;
        assert_eq!(
            remove_override(ct, "xl/calcChain.xml"),
            r#"<Types><Override PartName="/xl/workbook.xml" ContentType="y"/></Types>"#
        );
        let r = r#"<Relationships><Relationship Id="rId3" Type="http://x/calcChain" Target="calcChain.xml"/><Relationship Id="rId1" Type="http://x/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
        let parsed = rels("xl/workbook.xml", r);
        assert_eq!(parsed[1].target, "xl/worksheets/sheet1.xml");
        assert_eq!(free_rid(&parsed), "rId4");
        assert_eq!(
            remove_rel_to("xl/workbook.xml", r, "xl/calcChain.xml"),
            r#"<Relationships><Relationship Id="rId1" Type="http://x/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#
        );
    }

    #[test]
    fn drawing_anchors_move_and_clamp() {
        let d = "<xdr:from><xdr:col>1</xdr:col><xdr:row>9</xdr:row></xdr:from><xdr:to><xdr:col>3</xdr:col><xdr:row>4</xdr:row></xdr:to>";
        let n = shift_drawing(d, &[Shift::InsertRows { at: 6, n: 2 }]).unwrap();
        assert_eq!(
            n,
            "<xdr:from><xdr:col>1</xdr:col><xdr:row>11</xdr:row></xdr:from><xdr:to><xdr:col>3</xdr:col><xdr:row>4</xdr:row></xdr:to>"
        );
    }
}
