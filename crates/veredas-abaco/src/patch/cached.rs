//! What a workbook file SAYS, read straight from its XML: every cell's cached
//! value (shared strings, inline strings and rich-text runs resolved) and its
//! formula text as the file spells it. No engine involved — this is the
//! expected side of every comparison the patcher's tests and the B3 exit make
//! (the oracle's method, `docs/abaco-oracle.md`).

use std::collections::BTreeMap;
use std::path::Path;

use crate::book::Value;

use super::package::{self, Package};
use super::xml::{self, attr, decode_text, next_tag};
use super::PatchError;

/// One cell as the file states it.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedCell {
    pub value: Value,
    /// The formula text (file spelling, unescaped); shared children carry
    /// `None` here and `shared` set.
    pub formula: Option<String>,
    pub shared: bool,
    pub style: Option<u32>,
    /// A shared-formula cell's OWN formula as Excel shows it: the master's
    /// text, translated by the cell's offset from the master (relative
    /// references move, `$` ones do not). `None` off a shared group, and
    /// for a child whose master is missing or comes later in the file.
    pub resolved: Option<String>,
}

/// `(sheet name, row, column)` → cell, for every `<c>` of every sheet.
pub type Cells = BTreeMap<(String, i32, i32), CachedCell>;

/// One sheet of a file as it states it: `(row, column)` → cell.
pub type SheetCells = BTreeMap<(i32, i32), CachedCell>;

fn shared_strings(pkg: &mut Package, part: Option<&str>) -> Result<Vec<String>, PatchError> {
    let Some(part) = part else {
        return Ok(Vec::new());
    };
    if !pkg.has(part) {
        return Ok(Vec::new());
    }
    let s = pkg.read_text(part)?;
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(si) = xml::find_open(&s, "si", pos) {
        let (a, b) = xml::element_span(&s, &si)
            .ok_or_else(|| PatchError::Malformed("<si> not closed".into()))?;
        out.push(runs_text(&s[a..b]));
        pos = b;
    }
    Ok(out)
}

/// The text of `<t>` elements in a string item, phonetic runs (`<rPh>`) left
/// out, as Excel shows it.
fn runs_text(item: &str) -> String {
    let mut out = String::new();
    let mut pos = 0;
    while let Some(t) = next_tag(item, pos) {
        pos = t.end;
        if t.closing {
            continue;
        }
        if t.name == "rPh" {
            if let Some((_, b)) = xml::element_span(item, &t) {
                pos = b;
            }
            continue;
        }
        if t.name == "t" && !t.self_closing
            && let Some(c) = xml::find_close(item, &t) {
                out.push_str(&decode_text(&item[t.end..c.start]));
                pos = c.end;
            }
    }
    out
}

fn parse_addr(r: &str) -> Option<(i32, i32)> {
    let split = r.find(|c: char| c.is_ascii_digit())?;
    Some((
        r[split..].parse().ok()?,
        crate::a1::column_number(&r[..split])?,
    ))
}

/// Every cell of every sheet of `path`.
pub fn read(path: &Path) -> Result<Cells, PatchError> {
    let mut out = Cells::new();
    for (name, cells) in read_sheets(path, |_| true)? {
        for ((r, c), cell) in cells {
            out.insert((name.clone(), r, c), cell);
        }
    }
    Ok(out)
}

/// Every sheet of `path` in WORKBOOK order (`<sheets>` order, what the tab
/// strip shows), each with its cells; a sheet `want` refuses is listed with no
/// cells and its XML is never parsed. What a tool needs that reads the file
/// the way a file-level tool reads it: cached values, formulas as spelled,
/// shared flags.
pub fn read_sheets(
    path: &Path,
    mut want: impl FnMut(&str) -> bool,
) -> Result<Vec<(String, SheetCells)>, PatchError> {
    let bytes = std::fs::read(path).map_err(|e| PatchError::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    let mut pkg = Package::open(bytes)?;
    let wb_part = "xl/workbook.xml";
    let wb_xml = pkg.read_text(wb_part)?;
    let rels_xml = pkg.read_text(&package::rels_of(wb_part))?;
    let rels = package::rels(wb_part, &rels_xml);
    let sst_part = rels
        .iter()
        .find(|r| r.kind == "sharedStrings")
        .map(|r| r.target.clone());
    let sst = shared_strings(&mut pkg, sst_part.as_deref())?;
    let mut out = Vec::new();
    for sheet in package::sheets(&wb_xml, &rels)?.into_iter().filter(|s| s.is_worksheet()) {
        let mut cells = SheetCells::new();
        if !want(&sheet.name) {
            out.push((sheet.name.clone(), cells));
            continue;
        }
        let s = pkg.read_text(&sheet.part)?;
        let mut pos = 0;
        let mut row = 0;
        let mut col = 0;
        // shared groups: si → (master row, master column, master text)
        let mut masters: std::collections::HashMap<String, (i32, i32, String)> = std::collections::HashMap::new();
        while let Some(t) = next_tag(&s, pos) {
            pos = t.end;
            if t.closing {
                continue;
            }
            if t.name == "row" {
                row = attr(t.text, "r")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(row + 1);
                col = 0;
                continue;
            }
            if t.name != "c" {
                continue;
            }
            let (r, c) = attr(t.text, "r")
                .and_then(parse_addr)
                .unwrap_or((row, col + 1));
            col = c;
            let end = if t.self_closing {
                t.end
            } else {
                xml::find_close(&s, &t).map(|x| x.end).unwrap_or(t.end)
            };
            let body = &s[t.end.min(end)..end];
            pos = end;
            let kind = attr(t.text, "t").unwrap_or("n");
            let v = xml::find_open(body, "v", 0).and_then(|v| {
                if v.self_closing {
                    Some(String::new())
                } else {
                    xml::find_close(body, &v).map(|c| body[v.end..c.start].to_string())
                }
            });
            let mut resolved: Option<String> = None;
            let (formula, shared) = match xml::find_open(body, "f", 0) {
                Some(f) => {
                    let shared = attr(f.text, "t") == Some("shared");
                    if shared {
                        let si = attr(f.text, "si").unwrap_or("").to_string();
                        let own = if f.self_closing {
                            None
                        } else {
                            xml::find_close(body, &f).map(|c| decode_text(&body[f.end..c.start]))
                        };
                        match own.filter(|t| !t.is_empty()) {
                            Some(text) => {
                                masters.insert(si, (r, c, text.clone()));
                                resolved = Some(text);
                            }
                            None => {
                                resolved = masters
                                    .get(&si)
                                    .map(|(mr, mc, text)| super::refs::translate(text, r - mr, c - mc));
                            }
                        }
                    }
                    let text = if f.self_closing {
                        None
                    } else {
                        xml::find_close(body, &f).map(|c| decode_text(&body[f.end..c.start]))
                    };
                    (
                        text.or(if shared { None } else { Some(String::new()) }),
                        shared,
                    )
                }
                None => (None, false),
            };
            let value = match kind {
                "s" => v
                    .as_deref()
                    .and_then(|i| i.trim().parse::<usize>().ok())
                    .and_then(|i| sst.get(i).cloned())
                    .map(Value::Text)
                    .unwrap_or(Value::Empty),
                "inlineStr" => Value::Text(
                    xml::find_open(body, "is", 0)
                        .and_then(|is| xml::element_span(body, &is))
                        .map(|(a, b)| runs_text(&body[a..b]))
                        .unwrap_or_default(),
                ),
                // No `<v>` at all is "no cached value" (Empty), not empty text.
                "str" => v.as_deref().map(|t| Value::Text(decode_text(t))).unwrap_or(Value::Empty),
                "b" => Value::Bool(v.as_deref().map(str::trim) == Some("1")),
                "e" => Value::Error(v.as_deref().map(decode_text).unwrap_or_default()),
                _ => match v.as_deref() {
                    None | Some("") => Value::Empty,
                    Some(n) => n
                        .trim()
                        .parse()
                        .map(Value::Number)
                        .unwrap_or_else(|_| Value::Text(decode_text(n))),
                },
            };
            let style = attr(t.text, "s").and_then(|v| v.parse().ok());
            cells.insert(
                (r, c),
                CachedCell {
                    value,
                    formula,
                    shared,
                    style,
                    resolved,
                },
            );
        }
        out.push((sheet.name.clone(), cells));
    }
    Ok(out)
}

/// One sheet's `<c>` elements exactly as the file has them: `(row, column)`
/// → the element's bytes, from `<c` to its end.
pub type SheetElements = BTreeMap<(i32, i32), String>;

/// Every sheet of a package in workbook order — `(name, part, elements)` —
/// with each `<c>` element as written. The patcher's per-cell comparison: a cell
/// the patch had no reason to touch must come out as these bytes.
pub fn read_elements(bytes: &[u8]) -> Result<Vec<(String, String, SheetElements)>, PatchError> {
    let mut pkg = Package::open(bytes.to_vec())?;
    let wb_part = "xl/workbook.xml";
    let wb_xml = pkg.read_text(wb_part)?;
    let rels_xml = pkg.read_text(&package::rels_of(wb_part))?;
    let rels = package::rels(wb_part, &rels_xml);
    let mut out = Vec::new();
    for sheet in package::sheets(&wb_xml, &rels)?.into_iter().filter(|s| s.is_worksheet()) {
        let s = pkg.read_text(&sheet.part)?;
        let mut cells = SheetElements::new();
        let mut pos = 0;
        let (mut row, mut col) = (0, 0);
        while let Some(t) = next_tag(&s, pos) {
            pos = t.end;
            if t.closing {
                continue;
            }
            if t.name == "row" {
                row = attr(t.text, "r")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(row + 1);
                col = 0;
                continue;
            }
            if t.name != "c" {
                continue;
            }
            let (r, c) = attr(t.text, "r")
                .and_then(parse_addr)
                .unwrap_or((row, col + 1));
            col = c;
            let end = if t.self_closing {
                t.end
            } else {
                xml::find_close(&s, &t).map(|x| x.end).unwrap_or(t.end)
            };
            cells.insert((r, c), s[t.start..end].to_string());
            pos = end;
        }
        out.push((sheet.name.clone(), sheet.part.clone(), cells));
    }
    Ok(out)
}

/// The value cells differ in, `Value::Empty` for a cell absent on one side.
pub fn same(a: &Value, b: &Value) -> bool {
    super::sheet::same_value(a, b) && super::sheet::same_value(b, a)
}
