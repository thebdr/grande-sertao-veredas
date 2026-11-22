//! Cell formatting for the patcher: a fill or a border on a cell is a NEW
//! cell format (`<xf>`) appended to `styles.xml`, cloned from the cell's own
//! so its font, number format and alignment stay; the fill/border it points
//! at is appended too. Nothing already in `styles.xml` is changed or
//! reordered — style index N means what it meant before, in every sheet.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::xml::{self, find_close, find_open, set_attr};

/// A border line style Excel knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineStyle {
    Thin,
    Medium,
    Thick,
    Dashed,
    Dotted,
    Double,
    Hair,
}

impl LineStyle {
    fn xml(self) -> &'static str {
        match self {
            LineStyle::Thin => "thin",
            LineStyle::Medium => "medium",
            LineStyle::Thick => "thick",
            LineStyle::Dashed => "dashed",
            LineStyle::Dotted => "dotted",
            LineStyle::Double => "double",
            LineStyle::Hair => "hair",
        }
    }
}

/// One formatting change on a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StyleOp {
    /// A solid interior colour (`0xRRGGBB`), or no fill.
    Fill { rgb: Option<u32> },
    /// The same line on all four edges, or no border.
    Border {
        rgb: Option<u32>,
        style: Option<LineStyle>,
    },
}

#[derive(Debug, thiserror::Error)]
#[error("styles.xml: {0}")]
pub struct StylesError(pub String);

/// `styles.xml` plus what this patch appends to it.
pub struct StyleBook {
    xml: String,
    /// The raw `<xf …>` element text of every existing cell format.
    xfs: Vec<String>,
    fills: usize,
    borders: usize,
    new_fills: Vec<String>,
    new_borders: Vec<String>,
    new_xfs: Vec<String>,
    fill_ids: HashMap<String, usize>,
    border_ids: HashMap<String, usize>,
    memo: HashMap<(u32, Vec<StyleOp>), u32>,
}

fn children(s: &str, parent: &str, child: &str) -> Result<(Vec<String>, usize), StylesError> {
    let open = find_open(s, parent, 0).ok_or_else(|| StylesError(format!("no <{parent}>")))?;
    if open.self_closing {
        return Ok((Vec::new(), open.end));
    }
    let close =
        find_close(s, &open).ok_or_else(|| StylesError(format!("<{parent}> not closed")))?;
    let mut out = Vec::new();
    let mut from = open.end;
    while let Some(t) = find_open(&s[..close.start], child, from) {
        let (a, b) =
            xml::element_span(s, &t).ok_or_else(|| StylesError(format!("<{child}> not closed")))?;
        out.push(s[a..b].to_string());
        from = b;
    }
    Ok((out, close.start))
}

/// The styles part a package without one is given when an edit needs a
/// format: what every cell index 0 already means when the part is absent —
/// one font, the two fills OOXML reserves (`none`, `gray125`), one empty
/// border, one cell format, the Normal style. No theme references, so it is
/// valid whether or not the package has a theme.
pub const MINIMAL: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font><sz val="11"/><name val="Calibri"/><family val="2"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles><dxfs count="0"/><tableStyles count="0"/></styleSheet>"#;

impl StyleBook {
    pub fn new(styles_xml: String) -> Result<StyleBook, StylesError> {
        let (xfs, _) = children(&styles_xml, "cellXfs", "xf")?;
        let (fills, _) = children(&styles_xml, "fills", "fill")?;
        let (borders, _) = children(&styles_xml, "borders", "border")?;
        Ok(StyleBook {
            fills: fills.len(),
            borders: borders.len(),
            xfs,
            xml: styles_xml,
            new_fills: Vec::new(),
            new_borders: Vec::new(),
            new_xfs: Vec::new(),
            fill_ids: HashMap::new(),
            border_ids: HashMap::new(),
            memo: HashMap::new(),
        })
    }

    pub fn changed(&self) -> bool {
        !self.new_xfs.is_empty()
    }

    fn fill_id(&mut self, rgb: Option<u32>) -> usize {
        let Some(rgb) = rgb else { return 0 };
        let xml = format!(
            "<fill><patternFill patternType=\"solid\"><fgColor rgb=\"FF{rgb:06X}\"/><bgColor indexed=\"64\"/></patternFill></fill>"
        );
        if let Some(id) = self.fill_ids.get(&xml) {
            return *id;
        }
        let id = self.fills + self.new_fills.len();
        self.new_fills.push(xml.clone());
        self.fill_ids.insert(xml, id);
        id
    }

    fn border_id(&mut self, rgb: Option<u32>, style: Option<LineStyle>) -> usize {
        let (Some(style), rgb) = (style, rgb) else {
            return 0;
        };
        let color = match rgb {
            Some(rgb) => format!("<color rgb=\"FF{rgb:06X}\"/>"),
            None => "<color indexed=\"64\"/>".to_string(),
        };
        let side = |n: &str| format!("<{n} style=\"{}\">{color}</{n}>", style.xml());
        let xml = format!(
            "<border>{}{}{}{}<diagonal/></border>",
            side("left"),
            side("right"),
            side("top"),
            side("bottom")
        );
        if let Some(id) = self.border_ids.get(&xml) {
            return *id;
        }
        let id = self.borders + self.new_borders.len();
        self.new_borders.push(xml.clone());
        self.border_ids.insert(xml, id);
        id
    }

    /// The style index a cell styled `old` gets after `ops`.
    pub fn apply(&mut self, old: u32, ops: &[StyleOp]) -> u32 {
        if ops.is_empty() {
            return old;
        }
        let key = (old, ops.to_vec());
        if let Some(s) = self.memo.get(&key) {
            return *s;
        }
        let base = self
            .xfs
            .get(old as usize)
            .or_else(|| self.xfs.first())
            .cloned()
            .unwrap_or_else(|| {
                "<xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>".into()
            });
        // edit the opening tag only; children (alignment, protection) stay
        let tag_end = base.find('>').map(|i| i + 1).unwrap_or(base.len());
        let mut tag = base[..tag_end].to_string();
        let rest = base[tag_end..].to_string();
        for op in ops {
            match *op {
                StyleOp::Fill { rgb } => {
                    let id = self.fill_id(rgb);
                    tag = set_attr(&tag, "fillId", &id.to_string());
                    tag = set_attr(&tag, "applyFill", "1");
                }
                StyleOp::Border { rgb, style } => {
                    let id = self.border_id(rgb, style);
                    tag = set_attr(&tag, "borderId", &id.to_string());
                    tag = set_attr(&tag, "applyBorder", "1");
                }
            }
        }
        let index = (self.xfs.len() + self.new_xfs.len()) as u32;
        self.new_xfs.push(format!("{tag}{rest}"));
        self.memo.insert(key, index);
        index
    }

    /// The new `styles.xml`, or `None` when nothing was added.
    pub fn finish(self) -> Result<Option<String>, StylesError> {
        if !self.changed() {
            return Ok(None);
        }
        let mut s = self.xml;
        s = append(
            &s,
            "fills",
            &self.new_fills.concat(),
            self.fills + self.new_fills.len(),
        )?;
        s = append(
            &s,
            "borders",
            &self.new_borders.concat(),
            self.borders + self.new_borders.len(),
        )?;
        s = append(
            &s,
            "cellXfs",
            &self.new_xfs.concat(),
            self.xfs.len() + self.new_xfs.len(),
        )?;
        Ok(Some(s))
    }
}

/// Appends `content` as last children of `<parent>` and sets its `count`.
fn append(s: &str, parent: &str, content: &str, count: usize) -> Result<String, StylesError> {
    if content.is_empty() {
        return Ok(s.to_string());
    }
    let open = find_open(s, parent, 0).ok_or_else(|| StylesError(format!("no <{parent}>")))?;
    let new_open = set_attr(open.text, "count", &count.to_string());
    if open.self_closing {
        let tag = new_open.trim_end_matches("/>").to_string() + ">";
        return Ok(format!(
            "{}{tag}{content}</{parent}>{}",
            &s[..open.start],
            &s[open.end..]
        ));
    }
    let close =
        find_close(s, &open).ok_or_else(|| StylesError(format!("<{parent}> not closed")))?;
    Ok(format!(
        "{}{}{}{}{}",
        &s[..open.start],
        new_open,
        &s[open.end..close.start],
        content,
        &s[close.start..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLES: &str = r#"<styleSheet><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="49" fontId="3" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"><alignment wrapText="1"/></xf></cellXfs></styleSheet>"#;

    #[test]
    fn appends_fill_border_and_a_cloned_format_without_touching_the_old_ones() {
        let mut book = StyleBook::new(STYLES.to_string()).unwrap();
        let fill = [StyleOp::Fill {
            rgb: Some(0xB4D2F0),
        }];
        let s = book.apply(1, &fill);
        assert_eq!(s, 2);
        assert_eq!(book.apply(1, &fill), 2, "memoised");
        let both = [
            StyleOp::Fill {
                rgb: Some(0xB4D2F0),
            },
            StyleOp::Border {
                rgb: Some(0x0070C0),
                style: Some(LineStyle::Thin),
            },
        ];
        assert_eq!(book.apply(0, &both), 3);
        let out = book.finish().unwrap().unwrap();
        assert!(out.contains(r#"<fills count="3">"#), "{out}");
        assert!(out.contains(r#"<fgColor rgb="FFB4D2F0"/>"#));
        assert!(out.contains(r#"<borders count="2">"#));
        assert!(out.contains(r#"<cellXfs count="4">"#));
        // the clone keeps font 3, number format 49 and the alignment child
        assert!(out.contains(r#"<xf numFmtId="49" fontId="3" fillId="2" borderId="0" xfId="0" applyNumberFormat="1" applyFill="1"><alignment wrapText="1"/></xf>"#), "{out}");
        assert!(out.contains(r#"<xf numFmtId="0" fontId="0" fillId="2" borderId="1" xfId="0" applyFill="1" applyBorder="1"/>"#), "{out}");
        // the original formats are byte-identical
        assert!(out.starts_with(r#"<styleSheet><fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill>"#));
    }

    #[test]
    fn nothing_applied_is_no_rewrite() {
        let book = StyleBook::new(STYLES.to_string()).unwrap();
        assert!(book.finish().unwrap().is_none());
    }
}
