//! One worksheet part, rewritten in a single pass. A row nothing touches is
//! emitted as its original bytes; inside a touched row, a cell nothing
//! touches is too; inside a touched cell, only the touched piece (its address,
//! its formula's references, its cached value, its style) changes.
//!
//! Shared formulas: a group is left exactly as written unless something would
//! make its children's derived formulas wrong — its master overwritten, a
//! member moved, or a reference in its master shifted — and then the whole
//! group is EXPANDED (every child gets its own formula, translated from the
//! master the way Excel derives it). Writing a literal over a master without
//! expanding orphans every child, a corruption Excel repairs on open.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::a1::{column_name, column_number, LAST_COLUMN, LAST_ROW};
use crate::book::Value;

use super::refs::{self, Shift};
use super::styles::{StyleBook, StyleOp};
use super::xml::{self, attr, decode_text, escape_text, next_tag, remove_attr, set_attr, Tag};
use super::PatchError;

/// What a write puts in a cell.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NewContent {
    Value(Value),
    /// A cell an array spills into that the file has no cell for: its value, written as Excel
    /// writes a spilled cell.
    Spilled(Value),
    /// File spelling (`_xlfn.`/`_xlpm.`), unescaped; `cached` is the engine's
    /// value for it; `cm`, a dynamic array of one cell, written as an array formula marked by
    /// that cell metadata entry.
    Formula {
        text: String,
        cached: Value,
        cm: Option<u32>,
    },
}

/// Everything the patch does to one cell (at its final position).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CellWrite {
    pub content: Option<NewContent>,
    pub style: Vec<StyleOp>,
}

/// A row's outline (final row numbers).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct RowOp {
    pub level: Option<u8>,
    pub hidden: Option<bool>,
}

/// Column changes (final column numbers).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ColOp {
    Width {
        first: i32,
        last: i32,
        width: f64,
    },
    Outline {
        first: i32,
        last: i32,
        level: u8,
        hidden: bool,
    },
}

pub(crate) struct SheetJob<'a> {
    pub name: &'a str,
    pub order: &'a [String],
    /// Every structural move of the patch, in order, workbook-wide.
    pub shifts: &'a [(String, Shift)],
    pub writes: &'a BTreeMap<(i32, i32), CellWrite>,
    pub row_ops: &'a BTreeMap<i32, RowOp>,
    pub col_ops: &'a [ColOp],
    /// The engine's value of the formula cell at a final position.
    pub value_at: &'a dyn Fn(i32, i32) -> Value,
    /// Whether the edits moved the engine's value of the formula that was at
    /// an ORIGINAL position and evaluates to the given value now — the only
    /// formulas whose cache the rewrite may touch.
    /// (Its final position given too: a cell unstable before the edits and after them is exempt.)
    pub moved: &'a dyn Fn((i32, i32), (i32, i32), &Value) -> bool,
    /// The dynamic array that was anchored at an ORIGINAL position and is at a final one: how many
    /// rows and columns it spilled over before the edits (`None` for one they made) and spills
    /// over now; `None` where the engine holds no dynamic array (a CSE array's `ref` moves with
    /// its cells).
    pub array_extent: &'a dyn Fn((i32, i32), (i32, i32)) -> Option<(Option<(i32, i32)>, (i32, i32))>,
    /// Whether the cell that was at an ORIGINAL position, the engine computing it then (a cell an
    /// array spilled into), is at a final one the engine computes nothing at now: an array no
    /// longer reaches it, volatile or not.
    pub left: &'a dyn Fn((i32, i32), (i32, i32)) -> bool,
    /// Whether the cell that was at an ORIGINAL position, the engine computing nothing there then
    /// (a cell the file holds empty), is at a final one an array spills into now.
    pub reached: &'a dyn Fn((i32, i32), (i32, i32)) -> bool,
}

#[derive(Debug, Default)]
pub(crate) struct SheetOutcome {
    /// The new part, `None` when not a byte changed.
    pub xml: Option<String>,
    /// Formula cells whose cached value was brought up to date.
    pub refreshed: usize,
    /// Shared-formula groups expanded.
    pub expanded: usize,
    pub rows_rewritten: usize,
    /// Cells holding a formula that are gone (deleted or overwritten).
    pub formulas_removed: usize,
}

struct Group {
    master: (i32, i32),
    text: String,
    bounds: Option<(i32, i32, i32, i32)>,
    /// Every cell of the group, the master among them.
    members: Vec<(i32, i32)>,
}

fn parse_addr(r: &str) -> Option<(i32, i32)> {
    let split = r.find(|c: char| c.is_ascii_digit())?;
    let col = column_number(&r[..split])?;
    let row: i32 = r[split..].parse().ok()?;
    Some((row, col))
}

fn addr(row: i32, col: i32) -> String {
    format!("{}{}", column_name(col), row)
}

/// A cell's cached value as the file states it.
fn cached_value(t: Option<&str>, v: Option<&str>) -> Value {
    match (t.unwrap_or("n"), v) {
        ("str", v) | ("inlineStr", v) => Value::Text(v.map(decode_text).unwrap_or_default()),
        ("b", Some(v)) => Value::Bool(v.trim() == "1"),
        ("e", Some(v)) => Value::Error(decode_text(v)),
        (_, None) => Value::Empty,
        (_, Some(v)) => v
            .trim()
            .parse::<f64>()
            .map(Value::Number)
            .unwrap_or(Value::Text(decode_text(v))),
    }
}

/// Whether the cache already says what the engine computed: values told apart as Excel tells
/// them (numbers to 15 significant digits, an empty cell neither 0 nor a text), save that an empty
/// value and the empty text a formula caches as `<v></v>` are one.
pub(crate) fn same_value(cached: &Value, live: &Value) -> bool {
    match (cached, live) {
        (Value::Number(a), Value::Number(b)) => a == b || format!("{a:.14e}") == format!("{b:.14e}"),
        (Value::Empty, Value::Text(t)) | (Value::Text(t), Value::Empty) => t.is_empty(),
        // a `#SPILL!` or `#CALC!` is cached as `#VALUE!` (see `cached_xml`)
        (Value::Error(a), Value::Error(b)) if a == "#VALUE!" && (b == "#SPILL!" || b == "#CALC!") => true,
        (a, b) => a == b,
    }
}

/// A number as the file spells it.
pub(crate) fn number(n: f64) -> String {
    if n == n.trunc() && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let a = n.abs();
    if !(1e-9..1e17).contains(&a) {
        return format!("{n:E}");
    }
    format!("{n}")
}

/// `<v>`/`t` for a formula's cached value: (t attribute or none, v element).
/// Why a value cannot be a file's cache: the errors only the engine has (a circular reference, a
/// function it does not implement, a formula it could not read), which no workbook file holds and
/// no reader takes. Excel would compute these cells; the patch is refused rather than write a
/// guess.
pub(crate) fn engine_only(v: &Value) -> Option<&'static str> {
    match v {
        Value::Error(e) if e == "#CIRC!" => Some("#CIRC! (a circular reference)"),
        Value::Error(e) if e == "#N/IMPL" => Some("#N/IMPL (a function the engine does not implement)"),
        Value::Error(e) if e == "#ERROR!" => Some("#ERROR! (a formula the engine could not read)"),
        _ => None,
    }
}

/// The refusal of a cache [`engine_only`] names, at `sheet!cell`.
pub(crate) fn engine_only_refusal(sheet: &str, cell: &str, what: &str) -> PatchError {
    PatchError::Unsupported(format!("{sheet}!{cell}: the engine computes {what}, an error no workbook file holds"))
}

fn cached_xml(v: &Value) -> (Option<&'static str>, String) {
    match v {
        Value::Number(n) => (None, format!("<v>{}</v>", number(*n))),
        Value::Text(s) => (Some("str"), text_element("v", s)),
        Value::Bool(b) => (Some("b"), format!("<v>{}</v>", u8::from(*b))),
        // `#SPILL!` and `#CALC!` Excel writes as `#VALUE!`, marking them with value metadata in
        // rich-data parts the patcher does not write: `#VALUE!` alone is the spelling every reader
        // takes, Excel's before it knew the two
        Value::Error(e) if e == "#SPILL!" || e == "#CALC!" => (Some("e"), "<v>#VALUE!</v>".to_string()),
        Value::Error(e) => (Some("e"), format!("<v>{}</v>", escape_text(e))),
        Value::Empty => (Some("str"), "<v></v>".to_string()),
    }
}

/// A text as a cell's `<t>` or `<v>` carries it ([`escape_text`] writes a control character and
/// protects a literal `_xHHHH_` as Excel does), a carriage return too written `_x000D_`: XML reads
/// a bare one as a line feed.
fn text_element(name: &str, s: &str) -> String {
    let preserve =
        s.starts_with(char::is_whitespace) || s.ends_with(char::is_whitespace) || s.contains('\n');
    format!(
        "<{name}{}>{}</{name}>",
        if preserve {
            " xml:space=\"preserve\""
        } else {
            ""
        },
        escape_text(s)
    )
}

fn set_or_remove(tag: &str, name: &str, value: Option<&str>) -> String {
    match value {
        Some(v) => set_attr(tag, name, v),
        None => remove_attr(tag, name),
    }
}

/// A cell an array spilled into, as Excel writes one: the value with no formula, a text as `str`
/// (an inline or shared string there is a constant, and blocks the array); `None` = no cell at all
/// (the array no longer reaches it, and it has no style).
fn spilled_cell(row: i32, col: i32, style: Option<&str>, value: &Value) -> Option<String> {
    let s = style.map(|s| format!(" s=\"{s}\"")).unwrap_or_default();
    let r = addr(row, col);
    if matches!(value, Value::Empty) {
        style?;
        return Some(format!("<c r=\"{r}\"{s}/>"));
    }
    let (t, v) = cached_xml(value);
    let t = t.map(|t| format!(" t=\"{t}\"")).unwrap_or_default();
    Some(format!("<c r=\"{r}\"{s}{t}>{v}</c>"))
}

/// A fresh cell for a content write; `None` = no cell at all (cleared and
/// unstyled).
fn fresh_cell(row: i32, col: i32, style: Option<&str>, content: &NewContent) -> Option<String> {
    let s = style.map(|s| format!(" s=\"{s}\"")).unwrap_or_default();
    let r = addr(row, col);
    Some(match content {
        NewContent::Value(Value::Empty) => {
            // cleared and unstyled: no cell at all
            style?;
            format!("<c r=\"{r}\"{s}/>")
        }
        NewContent::Value(Value::Number(n)) => format!("<c r=\"{r}\"{s}><v>{}</v></c>", number(*n)),
        NewContent::Value(Value::Text(t)) => {
            format!(
                "<c r=\"{r}\"{s} t=\"inlineStr\"><is>{}</is></c>",
                text_element("t", t)
            )
        }
        NewContent::Value(Value::Bool(b)) => {
            format!("<c r=\"{r}\"{s} t=\"b\"><v>{}</v></c>", u8::from(*b))
        }
        NewContent::Value(Value::Error(e)) => {
            format!("<c r=\"{r}\"{s} t=\"e\"><v>{}</v></c>", escape_text(e))
        }
        NewContent::Formula { text, cached, cm: None } => {
            let (t, v) = cached_xml(cached);
            let t = t.map(|t| format!(" t=\"{t}\"")).unwrap_or_default();
            format!("<c r=\"{r}\"{s}{t}><f>{}</f>{v}</c>", escape_text(text))
        }
        NewContent::Formula { text, cached, cm: Some(cm) } => {
            let (t, v) = cached_xml(cached);
            let t = t.map(|t| format!(" t=\"{t}\"")).unwrap_or_default();
            format!("<c r=\"{r}\"{s}{t} cm=\"{cm}\"><f t=\"array\" ref=\"{r}\">{}</f>{v}</c>", escape_text(text))
        }
        NewContent::Spilled(v) => return spilled_cell(row, col, style, v),
    })
}

fn map_index(shifts: &[Shift], rows: bool, mut v: i32) -> Option<i32> {
    for s in shifts.iter().filter(|s| s.is_rows() == rows) {
        v = s.index(v)?;
    }
    Some(v)
}

/// A deleted position clamps to where the deletion started (for selections
/// and panes, which must point somewhere).
fn clamp_index(shifts: &[Shift], rows: bool, mut v: i32) -> i32 {
    for s in shifts.iter().filter(|s| s.is_rows() == rows) {
        v = match s.index(v) {
            Some(x) => x,
            None => match *s {
                Shift::DeleteRows { at, .. } | Shift::DeleteCols { at, .. } => at.max(1),
                _ => v,
            },
        }
        .min(if rows { LAST_ROW } else { LAST_COLUMN });
    }
    v
}

fn shift_area_text(text: &str, own: &[Shift]) -> Option<Option<String>> {
    let mut area = refs::parse_area(text)?;
    for s in own {
        match s.area(area) {
            Some(a) => area = a,
            None => return Some(None),
        }
    }
    Some(Some(area.spell()))
}

fn shift_sqref_text(text: &str, own: &[Shift]) -> Option<String> {
    let mut cur = text.to_string();
    for s in own {
        cur = refs::shift_sqref(&cur, *s)?;
    }
    Some(cur)
}

/// Shifts a formula by every move of the patch; `None` when unchanged.
fn shift_text(text: &str, job: &SheetJob) -> Option<String> {
    let mut cur: Option<String> = None;
    for (sheet, s) in job.shifts {
        let base = cur.as_deref().unwrap_or(text);
        if let Some(n) = refs::shift_formula(base, sheet, *s, Some(job.name), job.order) {
            cur = Some(n);
        }
    }
    cur
}

pub(crate) fn rewrite(
    xml_text: &str,
    job: &SheetJob,
    styles: &mut StyleBook,
) -> Result<SheetOutcome, PatchError> {
    let bad = |m: &str| PatchError::Malformed(format!("sheet {:?}: {m}", job.name));
    let own: Vec<Shift> = job
        .shifts
        .iter()
        .filter(|(s, _)| s.eq_ignore_ascii_case(job.name))
        .map(|(_, s)| *s)
        .collect();
    let mut out = SheetOutcome::default();

    let sd = xml::find_open(xml_text, "sheetData", 0).ok_or_else(|| bad("no <sheetData>"))?;
    let (body_start, body_end, after) = if sd.self_closing {
        (sd.end, sd.end, sd.end)
    } else {
        let close = xml::find_close(xml_text, &sd).ok_or_else(|| bad("<sheetData> not closed"))?;
        (sd.end, close.start, close.end)
    };
    let head = &xml_text[..sd.start];
    let body = &xml_text[body_start..body_end];
    let tail = &xml_text[after..];

    // ---- head: cols first (new cells take their column's style) --------
    let new_head = rewrite_head(head, job, &own)?;
    let col_styles = column_styles(new_head.as_deref().unwrap_or(head));

    // ---- shared groups ---------------------------------------------------
    let mut groups: HashMap<String, Group> = HashMap::new();
    if body.contains("t=\"shared\"") {
        scan_shared(body, &mut groups);
    }
    let mut expand: HashSet<String> = HashSet::new();
    for (si, g) in &groups {
        let master_final = (
            map_index(&own, true, g.master.0),
            map_index(&own, false, g.master.1),
        );
        let overwritten = match master_final {
            (Some(r), Some(c)) => job.writes.get(&(r, c)).is_some_and(|w| w.content.is_some()),
            _ => true,
        };
        // a move that changes a member's place against the master's changes what its formula
        // derives to (`=A1` shared down B1:B3 reads A3 in B3, A4 once B3 sits a row further from
        // the master): expanded, unless the derived text is the same (an absolute `$Z$1`). A
        // group moved whole keeps its sharing, its `ref` moved (see `rewrite_cell`)
        let moved = match master_final {
            (Some(mr), Some(mc)) => g.members.iter().any(|&(r, c)| {
                let (Some(nr), Some(nc)) = (map_index(&own, true, r), map_index(&own, false, c)) else {
                    return false; // a member deleted goes with its row or column
                };
                let (was, now) = ((r - g.master.0, c - g.master.1), (nr - mr, nc - mc));
                was != now && refs::translate(&g.text, now.0, now.1) != refs::translate(&g.text, was.0, was.1)
            }),
            _ => false, // the master deleted: `overwritten` expands it
        } || (g.bounds.is_none() && !own.is_empty());
        // a move that reaches the formula of any member, the master's or a child's as it derives
        // from the master's (`=A10` shared down B1:B3 reads A12 in B3: an insert at row 11 moves
        // B3's reference and not the master's)
        let text_moves = !job.shifts.is_empty()
            && g.members.iter().any(|&(r, c)| {
                shift_text(&refs::translate(&g.text, r - g.master.0, c - g.master.1), job).is_some()
            });
        if overwritten || moved || text_moves {
            expand.insert(si.clone());
        }
    }
    out.expanded = expand.len();

    // ---- rows -----------------------------------------------------------
    let mut new_body = String::with_capacity(body.len() + 1024);
    let mut changed = false;
    let mut pending: Vec<(&(i32, i32), &CellWrite)> = job.writes.iter().collect();
    pending.reverse(); // pop from the end = ascending order
    let mut row_ops: Vec<(&i32, &RowOp)> = job.row_ops.iter().collect();
    row_ops.reverse();
    let mut pos = 0;
    let mut implicit_row = 0;
    let mut rows_given_r = 0;
    while let Some(t) = next_tag(body, pos) {
        new_body.push_str(&body[pos..t.start]);
        if t.closing || t.name != "row" {
            new_body.push_str(t.text);
            pos = t.end;
            continue;
        }
        let row_end = if t.self_closing {
            t.end
        } else {
            xml::find_close(body, &t)
                .ok_or_else(|| bad("<row> not closed"))?
                .end
        };
        let orig_r = attr(t.text, "r")
            .and_then(|r| r.parse().ok())
            .unwrap_or(implicit_row + 1);
        implicit_row = orig_r;
        let new_r = map_index(&own, true, orig_r);
        let Some(new_r) = new_r else {
            out.formulas_removed += body[t.start..row_end].matches("<f").count();
            changed = true;
            pos = row_end;
            continue;
        };
        // rows that exist only in the writes, before this one
        while pending.last().is_some_and(|(k, _)| k.0 < new_r)
            || row_ops.last().is_some_and(|(r, _)| **r < new_r)
        {
            let r = match (pending.last(), row_ops.last()) {
                (Some((k, _)), Some((ro, _))) => k.0.min(**ro),
                (Some((k, _)), None) => k.0,
                (None, Some((ro, _))) => **ro,
                (None, None) => break,
            };
            if let Some(s) = new_row(r, &mut pending, &mut row_ops, &col_styles, styles) {
                new_body.push_str(&s);
            }
            changed = true;
        }
        let row_xml = &body[t.start..row_end];
        let rewritten = rewrite_row(
            row_xml,
            &t,
            orig_r,
            new_r,
            &own,
            job,
            &groups,
            &expand,
            &mut pending,
            &mut row_ops,
            &col_styles,
            styles,
            &mut out,
        )?;
        match rewritten {
            Some(s) => {
                // a row whose only change is the `r` it lacked (rows may leave it implicit) is
                // written with it when the part is rewritten anyway, a new row before it shifting
                // what it would imply; on its own it is no reason to rewrite the part
                let r_only = attr(t.text, "r").is_none()
                    && new_r == orig_r
                    && s == format!("{}{}", set_attr(t.text, "r", &new_r.to_string()), &row_xml[t.text.len()..]);
                new_body.push_str(&s);
                if r_only {
                    rows_given_r += 1;
                } else {
                    out.rows_rewritten += 1;
                    changed = true;
                }
            }
            None => new_body.push_str(row_xml),
        }
        pos = row_end;
    }
    new_body.push_str(&body[pos..]);
    while !pending.is_empty() || !row_ops.is_empty() {
        let r = match (pending.last(), row_ops.last()) {
            (Some((k, _)), Some((ro, _))) => k.0.min(**ro),
            (Some((k, _)), None) => k.0,
            (None, Some((ro, _))) => **ro,
            (None, None) => break,
        };
        if let Some(s) = new_row(r, &mut pending, &mut row_ops, &col_styles, styles) {
            new_body.push_str(&s);
        }
        changed = true;
    }

    // ---- tail -----------------------------------------------------------
    let new_tail = if job.shifts.is_empty() {
        None
    } else {
        rewrite_tail(tail, job, &own)
    };

    // ---- dimension ------------------------------------------------------
    let mut head_text = new_head.clone().unwrap_or_else(|| head.to_string());
    if let Some(d) = xml::find_open(&head_text, "dimension", 0)
        && let Some(r) = attr(d.text, "ref") {
            let mut bounds = refs::parse_area(r).map(|a| a.bounds());
            if let Some(Some(s)) = shift_area_text(r, &own) {
                bounds = refs::parse_area(&s).map(|a| a.bounds());
            }
            if let Some((mut r1, mut c1, mut r2, mut c2)) = bounds {
                for (row, col) in job.writes.keys() {
                    r1 = r1.min(*row);
                    c1 = c1.min(*col);
                    r2 = r2.max(*row);
                    c2 = c2.max(*col);
                }
                let spelled = if (r1, c1) == (r2, c2) {
                    addr(r1, c1)
                } else {
                    format!("{}:{}", addr(r1, c1), addr(r2, c2))
                };
                if spelled != r {
                    let nd = set_attr(d.text, "ref", &spelled);
                    head_text = format!("{}{}{}", &head_text[..d.start], nd, &head_text[d.end..]);
                }
            }
        }
    let head_changed = head_text != head;

    if !changed && !head_changed && new_tail.is_none() {
        return Ok(out);
    }
    out.rows_rewritten += rows_given_r;
    let sheet_data = if new_body.is_empty() {
        "<sheetData/>".to_string()
    } else {
        let open = if sd.self_closing {
            "<sheetData>".to_string()
        } else {
            sd.text.to_string()
        };
        format!("{open}{new_body}</sheetData>")
    };
    out.xml = Some(format!(
        "{head_text}{sheet_data}{}",
        new_tail.as_deref().unwrap_or(tail)
    ));
    Ok(out)
}

fn scan_shared(body: &str, groups: &mut HashMap<String, Group>) {
    let mut pos = 0;
    let mut cell: Option<(i32, i32)> = None;
    let mut members: HashMap<String, Vec<(i32, i32)>> = HashMap::new();
    while let Some(t) = next_tag(body, pos) {
        pos = t.end;
        if t.closing {
            continue;
        }
        if t.name == "c" {
            cell = attr(t.text, "r").and_then(parse_addr);
        } else if t.name == "f" && attr(t.text, "t") == Some("shared") {
            let (Some(si), Some(at)) = (attr(t.text, "si"), cell) else {
                continue;
            };
            members.entry(si.to_string()).or_default().push(at);
            if t.self_closing {
                continue;
            }
            let Some(close) = xml::find_close(body, &t) else {
                continue;
            };
            let text = decode_text(&body[t.end..close.start]);
            let bounds = attr(t.text, "ref")
                .and_then(refs::parse_area)
                .map(|a| a.bounds());
            groups.insert(
                si.to_string(),
                Group {
                    master: at,
                    text,
                    bounds,
                    members: Vec::new(),
                },
            );
            pos = close.end;
        }
    }
    for (si, m) in members {
        if let Some(g) = groups.get_mut(&si) {
            g.members = m;
        }
    }
}

/// The style of each column, from `<cols>` (final numbering).
fn column_styles(head: &str) -> HashMap<i32, String> {
    let mut out = HashMap::new();
    let Some(cols) = xml::find_open(head, "cols", 0) else {
        return out;
    };
    let Some((a, b)) = xml::element_span(head, &cols) else {
        return out;
    };
    let mut pos = a;
    while let Some(t) = xml::find_open(&head[..b], "col", pos) {
        pos = t.end;
        let (Some(min), Some(max), Some(s)) = (
            attr(t.text, "min").and_then(|v| v.parse::<i32>().ok()),
            attr(t.text, "max").and_then(|v| v.parse::<i32>().ok()),
            attr(t.text, "style"),
        ) else {
            continue;
        };
        for c in min..=max.min(min + 16_384) {
            out.insert(c, s.to_string());
        }
    }
    out
}

/// A row that exists only in the writes / row ops.
fn new_row(
    r: i32,
    pending: &mut Vec<(&(i32, i32), &CellWrite)>,
    row_ops: &mut Vec<(&i32, &RowOp)>,
    col_styles: &HashMap<i32, String>,
    styles: &mut StyleBook,
) -> Option<String> {
    let mut cells = String::new();
    while let Some((k, w)) = pending.last().copied() {
        if k.0 != r {
            break;
        }
        pending.pop();
        if let Some(c) = cell_for_write(k.0, k.1, None, None, w, col_styles, styles) {
            cells.push_str(&c);
        }
    }
    let mut tag = format!("<row r=\"{r}\">");
    if row_ops.last().is_some_and(|(ro, _)| **ro == r) {
        let (_, op) = row_ops.pop()?;
        tag = apply_row_op(&tag, op);
    }
    if cells.is_empty() {
        if tag == format!("<row r=\"{r}\">") {
            return None;
        }
        return Some(tag.replacen('>', "/>", 1).replace("//>", "/>"));
    }
    Some(format!("{tag}{cells}</row>"))
}

fn apply_row_op(tag: &str, op: &RowOp) -> String {
    let mut t = tag.to_string();
    if let Some(level) = op.level {
        t = if level == 0 {
            remove_attr(&t, "outlineLevel")
        } else {
            set_attr(&t, "outlineLevel", &level.to_string())
        };
    }
    if let Some(h) = op.hidden {
        t = if h {
            set_attr(&t, "hidden", "1")
        } else {
            remove_attr(&t, "hidden")
        };
    }
    t
}

/// A cell built for a write where no cell existed (or replacing one whose style `old_style` it
/// keeps): a new cell takes its row's format where the row has one (`s` with `customFormat`), else
/// its column's, as Excel gives a cell typed into a formatted row or column.
fn cell_for_write(
    row: i32,
    col: i32,
    old_style: Option<&str>,
    row_style: Option<&str>,
    w: &CellWrite,
    col_styles: &HashMap<i32, String>,
    styles: &mut StyleBook,
) -> Option<String> {
    let base = old_style
        .or(row_style)
        .map(str::to_string)
        .or_else(|| col_styles.get(&col).cloned());
    let style = if w.style.is_empty() {
        base
    } else {
        let old: u32 = base.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
        Some(styles.apply(old, &w.style).to_string())
    };
    match &w.content {
        Some(c) => fresh_cell(row, col, style.as_deref(), c),
        None => style.map(|s| format!("<c r=\"{}\" s=\"{s}\"/>", addr(row, col))),
    }
}

#[allow(clippy::too_many_arguments)]
fn rewrite_row(
    row_xml: &str,
    row_tag: &Tag,
    orig_r: i32,
    new_r: i32,
    own: &[Shift],
    job: &SheetJob,
    groups: &HashMap<String, Group>,
    expand: &HashSet<String>,
    pending: &mut Vec<(&(i32, i32), &CellWrite)>,
    row_ops: &mut Vec<(&i32, &RowOp)>,
    col_styles: &HashMap<i32, String>,
    styles: &mut StyleBook,
    out: &mut SheetOutcome,
) -> Result<Option<String>, PatchError> {
    let bad = |m: String| PatchError::Malformed(format!("sheet {:?} row {orig_r}: {m}", job.name));
    // the row's own format, which a cell new in it takes (`customFormat` an xsd:boolean)
    let row_style = attr(row_tag.text, "s").filter(|_| matches!(attr(row_tag.text, "customFormat"), Some("1" | "true")));
    let has_cols = own.iter().any(|s| !s.is_rows());
    let mut changed = new_r != orig_r || attr(row_tag.text, "r").is_none();
    let mut cells_added_or_dropped = false;
    let mut cells = String::new();
    let inner_start = row_tag.end - row_tag.start;
    let inner_end = if row_tag.self_closing {
        inner_start
    } else {
        row_xml.rfind("</row>").unwrap_or(row_xml.len())
    };
    let inner = &row_xml[inner_start..inner_end];
    let mut pos = 0;
    let mut implicit_col = 0;
    while let Some(t) = next_tag(inner, pos) {
        cells.push_str(&inner[pos..t.start]);
        if t.closing || t.name != "c" {
            cells.push_str(t.text);
            pos = t.end;
            continue;
        }
        let cell_end = if t.self_closing {
            t.end
        } else {
            xml::find_close(inner, &t)
                .ok_or_else(|| bad("<c> not closed".into()))?
                .end
        };
        let cell_xml = &inner[t.start..cell_end];
        let orig_c = attr(t.text, "r")
            .and_then(parse_addr)
            .map(|a| a.1)
            .unwrap_or(implicit_col + 1);
        implicit_col = orig_c;
        let Some(new_c) = map_index(own, false, orig_c) else {
            out.formulas_removed += cell_xml.matches("<f").count();
            changed = true;
            cells_added_or_dropped = true;
            pos = cell_end;
            continue;
        };
        // new cells before this one
        while pending
            .last()
            .is_some_and(|(k, _)| k.0 == new_r && k.1 < new_c)
        {
            let (k, w) = pending.pop().ok_or_else(|| bad("pending".into()))?;
            if let Some(c) = cell_for_write(k.0, k.1, None, row_style, w, col_styles, styles) {
                cells.push_str(&c);
            }
            changed = true;
            cells_added_or_dropped = true;
        }
        let write = if pending
            .last()
            .is_some_and(|(k, _)| k.0 == new_r && k.1 == new_c)
        {
            pending.pop().map(|(_, w)| w)
        } else {
            None
        };
        let cell = rewrite_cell(
            cell_xml,
            &t,
            (orig_r, orig_c),
            (new_r, new_c),
            write,
            job,
            groups,
            expand,
            styles,
            out,
        )?;
        match cell {
            CellOut::Same => cells.push_str(cell_xml),
            CellOut::New(s) => {
                cells.push_str(&s);
                changed = true;
            }
            CellOut::Gone => {
                changed = true;
                cells_added_or_dropped = true;
            }
        }
        pos = cell_end;
    }
    cells.push_str(&inner[pos..]);
    while pending.last().is_some_and(|(k, _)| k.0 == new_r) {
        let (k, w) = pending.pop().ok_or_else(|| bad("pending".into()))?;
        if let Some(c) = cell_for_write(k.0, k.1, None, row_style, w, col_styles, styles) {
            cells.push_str(&c);
        }
        changed = true;
        cells_added_or_dropped = true;
    }
    let mut tag = row_tag.text.to_string();
    if row_ops.last().is_some_and(|(r, _)| **r == new_r)
        && let Some((_, op)) = row_ops.pop() {
            let t2 = apply_row_op(&tag, op);
            if t2 != tag {
                tag = t2;
                changed = true;
            }
        }
    if !changed {
        return Ok(None);
    }
    tag = set_attr(&tag, "r", &new_r.to_string());
    if cells_added_or_dropped || has_cols {
        // the hint would lie; Excel recomputes it
        tag = remove_attr(&tag, "spans");
    }
    if cells.trim().is_empty() && !cells.contains('<') {
        if tag.ends_with("/>") {
            return Ok(Some(tag));
        }
        return Ok(Some(format!("{}/>", tag.trim_end_matches('>'))));
    }
    let tag = if tag.ends_with("/>") {
        format!("{}>", tag.trim_end_matches("/>"))
    } else {
        tag
    };
    Ok(Some(format!("{tag}{cells}</row>")))
}

enum CellOut {
    Same,
    New(String),
    Gone,
}

#[allow(clippy::too_many_arguments)]
fn rewrite_cell(
    cell_xml: &str,
    tag: &Tag,
    orig: (i32, i32),
    new: (i32, i32),
    write: Option<&CellWrite>,
    job: &SheetJob,
    groups: &HashMap<String, Group>,
    expand: &HashSet<String>,
    styles: &mut StyleBook,
    out: &mut SheetOutcome,
) -> Result<CellOut, PatchError> {
    // Excel writes the cells a volatile array spills into with an empty `<f ca="1"/>`: a
    // placeholder, no formula of the cell's own (a shared child's `<f t="shared" …/>` is one)
    let placeholder = xml::find_open(cell_xml, "f", 0)
        .is_some_and(|f| f.self_closing && attr(f.text, "t") != Some("shared"));
    let had_formula = cell_xml.contains("<f") && !placeholder;
    let old_style = attr(tag.text, "s");
    if let Some(w) = write
        && w.content.is_some() {
            if had_formula {
                out.formulas_removed += 1;
            }
            let s = cell_for_write(new.0, new.1, old_style, None, w, &HashMap::new(), styles);
            // keep a style-only cell when the original had a style
            return Ok(match s {
                Some(s) => CellOut::New(s),
                None => CellOut::Gone,
            });
        }
    let inner_start = tag.end - tag.start;
    let inner_end = if tag.self_closing {
        inner_start
    } else {
        cell_xml.rfind("</c>").unwrap_or(cell_xml.len())
    };
    let inner = &cell_xml[inner_start..inner_end];
    let mut new_tag = tag.text.to_string();
    let mut changed = false;
    if new != orig || attr(tag.text, "r").is_none() {
        new_tag = set_attr(&new_tag, "r", &addr(new.0, new.1));
        changed = true;
    }
    if let Some(w) = write
        && !w.style.is_empty() {
            let old: u32 = old_style.and_then(|s| s.parse().ok()).unwrap_or(0);
            let s = styles.apply(old, &w.style);
            new_tag = set_attr(&new_tag, "s", &s.to_string());
            changed = true;
        }
    let mut new_inner = inner.to_string();
    if had_formula
        && let Some(f) = xml::find_open(inner, "f", 0) {
            let (f_start, f_end, f_text_raw) = if f.self_closing {
                (f.start, f.end, None)
            } else {
                let close = xml::find_close(inner, &f).ok_or_else(|| {
                    PatchError::Malformed(format!("{}: <f> not closed", addr(orig.0, orig.1)))
                })?;
                (f.start, close.end, Some(&inner[f.end..close.start]))
            };
            let shared_si = (attr(f.text, "t") == Some("shared"))
                .then(|| attr(f.text, "si"))
                .flatten();
            let mut f_tag = f.text.to_string();
            let mut text: Option<String> = None; // the new formula text, when it changes
            match shared_si {
                Some(si) if expand.contains(si) => {
                    let g = groups.get(si);
                    let derived = match g {
                        Some(g) => {
                            refs::translate(&g.text, orig.0 - g.master.0, orig.1 - g.master.1)
                        }
                        None => f_text_raw.map(decode_text).unwrap_or_default(),
                    };
                    let shifted = shift_text(&derived, job).unwrap_or(derived);
                    f_tag = remove_attr(&remove_attr(&remove_attr(&f_tag, "t"), "si"), "ref");
                    if f_tag.ends_with("/>") {
                        f_tag = format!("{}>", f_tag.trim_end_matches("/>"));
                    }
                    text = Some(shifted);
                }
                Some(si) => {
                    // a kept group: the master's text is unchanged by construction; its `ref`
                    // follows the moves (grown by an insertion inside it, cut by a deletion)
                    if groups.get(si).is_some_and(|g| g.master == orig)
                        && let Some(r) = attr(f.text, "ref")
                    {
                        let own: Vec<Shift> =
                            job.shifts.iter().filter(|(s, _)| s.eq_ignore_ascii_case(job.name)).map(|(_, s)| *s).collect();
                        if let Some(Some(nr)) = shift_area_text(r, &own)
                            && nr != r
                        {
                            f_tag = set_attr(&f_tag, "ref", &nr);
                            text = f_text_raw.map(decode_text);
                        }
                    }
                }
                None => {
                    if let Some(raw) = f_text_raw {
                        let decoded = decode_text(raw);
                        if let Some(s) = shift_text(&decoded, job) {
                            text = Some(s);
                        }
                    }
                    if attr(f.text, "t") == Some("array")
                        && let Some(r) = attr(f.text, "ref") {
                            let own: Vec<Shift> = job
                                .shifts
                                .iter()
                                .filter(|(s, _)| s.eq_ignore_ascii_case(job.name))
                                .map(|(_, s)| *s)
                                .collect();
                            // a dynamic array covers what it spills over now: it grows and shrinks
                            // with what it reads, and stays whole when rows or columns of it go
                            // (Excel spills it again). Where the engine sized it otherwise than
                            // the file before the edits, the file's size stands unless the edits
                            // changed the engine's; a CSE array covers what it covered, moved
                            let size = refs::parse_area(r).map(|a| {
                                let (r1, c1, r2, c2) = a.bounds();
                                (r2 - r1 + 1, c2 - c1 + 1)
                            });
                            let spell = |(rows, cols): (i32, i32)| {
                                if (rows, cols) == (1, 1) {
                                    addr(new.0, new.1)
                                } else {
                                    format!("{}:{}", addr(new.0, new.1), addr(new.0 + rows - 1, new.1 + cols - 1))
                                }
                            };
                            let wanted = match (job.array_extent)(orig, new) {
                                Some((then, now)) if then.is_none() || then == size || then != Some(now) => {
                                    Some(spell(now))
                                }
                                _ => shift_area_text(r, &own).flatten(),
                            };
                            if let Some(nr) = wanted
                                && nr != r {
                                    f_tag = set_attr(&f_tag, "ref", &nr);
                                    if text.is_none() {
                                        text = f_text_raw.map(decode_text);
                                    }
                                }
                        }
                }
            }
            if let Some(t) = text {
                let f_tag = if f_tag.ends_with("/>") {
                    format!("{}>", f_tag.trim_end_matches("/>"))
                } else {
                    f_tag
                };
                let name_end = "<f".len();
                let _ = name_end;
                new_inner = format!(
                    "{}{}{}</f>{}",
                    &inner[..f_start],
                    f_tag,
                    escape_text(&t),
                    &inner[f_end..]
                );
                changed = true;
            }
            // cached value — only where the edits moved the engine's value; a
            // formula the engine reads differently from the file for its own
            // reasons keeps the file's cache, whatever happened to its text
            let live = (job.value_at)(new.0, new.1);
            let moved = (job.moved)(orig, new, &live);
            let v_raw = xml::find_open(&new_inner, "v", 0).map(|v| {
                if v.self_closing {
                    (v.start, v.end, Some(""))
                } else {
                    let close = xml::find_close(&new_inner, &v);
                    match close {
                        Some(c) => (v.start, c.end, Some(&new_inner[v.end..c.start])),
                        None => (v.start, v.end, None),
                    }
                }
            });
            let cached = cached_value(attr(&new_tag, "t"), v_raw.and_then(|v| v.2));
            // a cell whose value moved and whose cache carries value metadata (a `#VALUE!` standing
            // for a `#SPILL!` or `#CALC!`) is rewritten whatever its cache says: the error may be
            // another now, or none
            if moved && (!same_value(&cached, &live) || attr(&new_tag, "vm").is_some()) {
                if let Some(what) = engine_only(&live) {
                    return Err(engine_only_refusal(job.name, &addr(new.0, new.1), what));
                }
                let (t, v) = cached_xml(&live);
                new_tag = set_or_remove(&new_tag, "t", t);
                // value metadata marks what a cached `#VALUE!` stands for (`#SPILL!`, `#CALC!`): a
                // refreshed cache is written without it (see `cached_xml`)
                new_tag = remove_attr(&new_tag, "vm");
                new_inner = match v_raw {
                    Some((a, b, _)) => format!("{}{}{}", &new_inner[..a], v, &new_inner[b..]),
                    None => {
                        // after </f> (or the self-closing <f/>)
                        let fe = xml::find_open(&new_inner, "f", 0)
                            .and_then(|f| {
                                if f.self_closing {
                                    Some(f.end)
                                } else {
                                    xml::find_close(&new_inner, &f).map(|c| c.end)
                                }
                            })
                            .unwrap_or(0);
                        format!("{}{}{}", &new_inner[..fe], v, &new_inner[fe..])
                    }
                };
                out.refreshed += 1;
                changed = true;
            }
        }
    if !had_formula {
        // a cell an array spilled into holds the array's value and no formula of its own: where
        // the edits moved that value (its anchor deleted, an input changed) it follows the engine
        // as a formula's cache does, and is empty where the array no longer reaches (left as a
        // value, Excel would read it as a constant); a plain value is never the engine's to move
        // (`moved` is false for a cell the engine did not compute)
        let live = (job.value_at)(new.0, new.1);
        if (job.moved)(orig, new, &live) || (job.left)(orig, new) || (job.reached)(orig, new) {
            if let Some(what) = engine_only(&live) {
                return Err(engine_only_refusal(job.name, &addr(new.0, new.1), what));
            }
            out.refreshed += 1;
            return Ok(match spilled_cell(new.0, new.1, attr(&new_tag, "s"), &live) {
                Some(s) => CellOut::New(s),
                None => CellOut::Gone,
            });
        }
    }
    if !changed {
        return Ok(CellOut::Same);
    }
    if new_inner.is_empty() {
        let t = if new_tag.ends_with("/>") {
            new_tag
        } else {
            format!("{}/>", new_tag.trim_end_matches('>'))
        };
        return Ok(CellOut::New(t));
    }
    let t = if new_tag.ends_with("/>") {
        format!("{}>", new_tag.trim_end_matches("/>"))
    } else {
        new_tag
    };
    Ok(CellOut::New(format!("{t}{new_inner}</c>")))
}

/// `<dimension>`, `<sheetViews>`, `<cols>`, `<sheetFormatPr>`; `None` when
/// unchanged.
fn rewrite_head(head: &str, job: &SheetJob, own: &[Shift]) -> Result<Option<String>, PatchError> {
    let mut s = head.to_string();
    // panes and selections follow the cells they point at
    if !own.is_empty() {
        let mut pos = 0;
        let mut rebuilt = String::with_capacity(s.len());
        let src = s.clone();
        while let Some(t) = next_tag(&src, pos) {
            rebuilt.push_str(&src[pos..t.start]);
            let mut text = t.text.to_string();
            if !t.closing && (t.name == "selection" || t.name == "pane") {
                for a in ["activeCell", "topLeftCell"] {
                    if let Some(v) = attr(&text, a)
                        && let Some((r, c)) = parse_addr(&v.replace('$', "")) {
                            let nv = addr(clamp_index(own, true, r), clamp_index(own, false, c));
                            text = set_attr(&text, a, &nv);
                        }
                }
                if let Some(v) = attr(&text, "sqref") {
                    let nv = shift_sqref_text(v, own).unwrap_or_else(|| v.to_string());
                    let nv = if nv.is_empty() {
                        attr(&text, "activeCell").unwrap_or("A1").to_string()
                    } else {
                        nv
                    };
                    text = set_attr(&text, "sqref", &nv);
                }
            }
            rebuilt.push_str(&text);
            pos = t.end;
        }
        rebuilt.push_str(&src[pos..]);
        s = rebuilt;
    }
    // columns
    let col_shifts: Vec<Shift> = own.iter().copied().filter(|x| !x.is_rows()).collect();
    if !col_shifts.is_empty() || !job.col_ops.is_empty() {
        s = rewrite_cols(&s, &col_shifts, job.col_ops);
    }
    // outline levels for the sheet's summary (Excel draws the +/- gutter wide
    // enough for the deepest level it is told of)
    let max_row_level = job
        .row_ops
        .values()
        .filter_map(|o| o.level)
        .max()
        .unwrap_or(0);
    let max_col_level = job
        .col_ops
        .iter()
        .filter_map(|o| match o {
            ColOp::Outline { level, .. } => Some(*level),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    if max_row_level > 0 || max_col_level > 0 {
        let fmt = xml::find_open(&s, "sheetFormatPr", 0);
        let (start, end, mut tag) = match fmt {
            Some(t) => (t.start, t.end, t.text.to_string()),
            None => {
                // CT_Worksheet order: … sheetViews, sheetFormatPr, cols …
                let at = xml::find_open(&s, "cols", 0)
                    .map(|t| t.start)
                    .unwrap_or(s.len());
                (
                    at,
                    at,
                    "<sheetFormatPr defaultRowHeight=\"15\"/>".to_string(),
                )
            }
        };
        for (name, level) in [
            ("outlineLevelRow", max_row_level),
            ("outlineLevelCol", max_col_level),
        ] {
            if level == 0 {
                continue;
            }
            let cur: u8 = attr(&tag, name).and_then(|v| v.parse().ok()).unwrap_or(0);
            if level > cur {
                tag = set_attr(&tag, name, &level.to_string());
            }
        }
        s = format!("{}{}{}", &s[..start], tag, &s[end..]);
    }
    Ok((s != head).then_some(s))
}

/// One `<col>` range with its attributes (the tag text, min/max replaced on
/// output).
#[derive(Clone)]
struct ColSpec {
    min: i32,
    max: i32,
    tag: String,
}

fn rewrite_cols(head: &str, shifts: &[Shift], ops: &[ColOp]) -> String {
    let cols_open = xml::find_open(head, "cols", 0);
    let (start, end, mut specs) = match cols_open {
        Some(open) => {
            let Some((a, b)) = xml::element_span(head, &open) else {
                return head.to_string();
            };
            let mut specs = Vec::new();
            let mut pos = open.end;
            while let Some(t) = xml::find_open(&head[..b], "col", pos) {
                pos = t.end;
                let (Some(min), Some(max)) = (
                    attr(t.text, "min").and_then(|v| v.parse().ok()),
                    attr(t.text, "max").and_then(|v| v.parse().ok()),
                ) else {
                    continue;
                };
                specs.push(ColSpec {
                    min,
                    max,
                    tag: t.text.to_string(),
                });
            }
            (a, b, specs)
        }
        None => {
            if ops.is_empty() {
                return head.to_string();
            }
            // <cols> goes right before <sheetData> (the end of the head)
            (head.len(), head.len(), Vec::new())
        }
    };
    // structural moves
    for sh in shifts {
        let mut next = Vec::new();
        for c in specs {
            if let Some((a, b)) = sh.span(c.min, c.max) {
                next.push(ColSpec {
                    min: a,
                    max: b,
                    ..c
                });
            }
        }
        specs = next;
    }
    // width / outline ops: split ranges at the op's edges, then set
    for op in ops {
        let (first, last) = match *op {
            ColOp::Width { first, last, .. } | ColOp::Outline { first, last, .. } => (first, last),
        };
        let mut next = Vec::new();
        let mut covered: Vec<(i32, i32)> = Vec::new();
        for c in specs {
            if c.max < first || c.min > last {
                next.push(c);
                continue;
            }
            if c.min < first {
                next.push(ColSpec {
                    min: c.min,
                    max: first - 1,
                    tag: c.tag.clone(),
                });
            }
            let (a, b) = (c.min.max(first), c.max.min(last));
            next.push(ColSpec {
                min: a,
                max: b,
                tag: col_op(&c.tag, op),
            });
            covered.push((a, b));
            if c.max > last {
                next.push(ColSpec {
                    min: last + 1,
                    max: c.max,
                    tag: c.tag.clone(),
                });
            }
        }
        // the parts of the op no <col> covered
        let mut cur = first;
        covered.sort();
        let mut gaps = Vec::new();
        for (a, b) in &covered {
            if *a > cur {
                gaps.push((cur, a - 1));
            }
            cur = cur.max(b + 1);
        }
        if cur <= last {
            gaps.push((cur, last));
        }
        for (a, b) in gaps {
            next.push(ColSpec {
                min: a,
                max: b,
                tag: col_op("<col/>", op),
            });
        }
        next.sort_by_key(|c| c.min);
        specs = next;
    }
    let body: String = specs
        .iter()
        .map(|c| {
            let t = set_attr(&c.tag, "min", &c.min.to_string());
            set_attr(&t, "max", &c.max.to_string())
        })
        .collect();
    let cols = if specs.is_empty() {
        String::new()
    } else {
        format!("<cols>{body}</cols>")
    };
    let original_open = cols_open.map(|t| t.text.to_string());
    let cols = match original_open {
        Some(open) if !specs.is_empty() => format!("{open}{body}</cols>"),
        _ => cols,
    };
    format!("{}{}{}", &head[..start], cols, &head[end..])
}

fn col_op(tag: &str, op: &ColOp) -> String {
    let tag = if tag == "<col/>" {
        "<col min=\"0\" max=\"0\"/>".to_string()
    } else {
        tag.to_string()
    };
    match *op {
        ColOp::Width { width, .. } => {
            let t = set_attr(&tag, "width", &number(width));
            set_attr(&t, "customWidth", "1")
        }
        ColOp::Outline { level, hidden, .. } => {
            let mut t = if level == 0 {
                remove_attr(&tag, "outlineLevel")
            } else {
                set_attr(&tag, "outlineLevel", &level.to_string())
            };
            t = if hidden {
                set_attr(&t, "hidden", "1")
            } else {
                remove_attr(&t, "hidden")
            };
            if attr(&t, "width").is_none() {
                // a <col> without a width is read as width 0 by Excel
                t = set_attr(&t, "width", "9.140625");
            }
            t
        }
    }
}

/// Everything after `</sheetData>`: merges, conditional formats, validations,
/// hyperlinks, auto filter, breaks, and the x14 extensions.
fn rewrite_tail(tail: &str, job: &SheetJob, own: &[Shift]) -> Option<String> {
    let out = walk_tail(tail, job, own, None);
    if out == tail {
        return None;
    }
    Some(fix_counts(&out))
}

const DROP_IF_GONE_TAG: &[&str] = &["mergeCell", "hyperlink", "ignoredError", "protectedRange", "sortCondition"];
const DROP_IF_GONE_ELEMENT: &[&str] = &["conditionalFormatting", "dataValidation", "autoFilter", "sortState"];
const FORMULA_TEXT: &[&str] = &["formula", "formula1", "formula2", "xm:f"];
const X14_ELEMENTS: &[&str] = &["x14:conditionalFormatting", "x14:dataValidation", "x14:protectedRange", "x14:ignoredError"];
/// A list and what it lists (`""`: any element): dropped when the moves leave it none.
const CONTAINERS: &[(&str, &str)] = &[
    ("mergeCells", "mergeCell"),
    ("hyperlinks", "hyperlink"),
    ("protectedRanges", "protectedRange"),
    ("ignoredErrors", "ignoredError"),
    ("dataValidations", "dataValidation"),
    ("x14:sparklineGroup", "x14:sparkline"),
    ("x14:sparklineGroups", "x14:sparklineGroup"),
    ("x14:conditionalFormattings", "x14:conditionalFormatting"),
    ("x14:dataValidations", "x14:dataValidation"),
    ("x14:protectedRanges", "x14:protectedRange"),
    ("x14:ignoredErrors", "x14:ignoredError"),
    ("ext", ""),
    ("extLst", "ext"),
];

/// Where a cell that is at `at` after `shift` was before it.
fn before_shift(shift: Shift, at: (i32, i32)) -> (i32, i32) {
    let back = |v: i32, from: i32, n: i32, delete: bool| match (v < from, delete) {
        (true, _) => v,
        (false, true) => v + n,
        (false, false) => v - n,
    };
    match shift {
        Shift::InsertRows { at: a, n } => (back(at.0, a, n, false), at.1),
        Shift::DeleteRows { at: a, n } => (back(at.0, a, n, true), at.1),
        Shift::InsertCols { at: a, n } => (at.0, back(at.1, a, n, false)),
        Shift::DeleteCols { at: a, n } => (at.0, back(at.1, a, n, true)),
    }
}

/// A conditional format's or a validation's formula, shifted by every move of the patch. Excel
/// keeps it relative to the top-left cell of its range (`sqref`'s first area): where a move
/// deletes that cell and the range survives, the formula is first re-anchored on the cell that
/// is the range's top-left after the move (`A2>5` on A2:A10, row 2 deleted, reads the old A3 as
/// `A3>5`), then shifted (`A2>5` again, on A2:A9). `None` when unchanged.
fn shift_anchored(text: &str, sqref: &str, job: &SheetJob) -> Option<String> {
    let mut areas: Vec<refs::RefArea> = sqref.split_whitespace().filter_map(refs::parse_area).collect();
    let mut cur = text.to_string();
    let mut changed = false;
    for (sheet, shift) in job.shifts {
        if sheet.eq_ignore_ascii_case(job.name) {
            let anchor = areas.first().map(|a| {
                let (r1, c1, _, _) = a.bounds();
                (r1, c1)
            });
            let after: Vec<refs::RefArea> = areas.iter().filter_map(|a| shift.area(*a)).collect();
            if let (Some(old), Some(first)) = (anchor, after.first()) {
                let (r1, c1, _, _) = first.bounds();
                let new = before_shift(*shift, (r1, c1));
                if new != old {
                    cur = refs::translate(&cur, new.0 - old.0, new.1 - old.1);
                    changed = true;
                }
            }
            areas = after;
        }
        if let Some(n) = refs::shift_formula(&cur, sheet, *shift, Some(job.name), job.order) {
            cur = n;
            changed = true;
        }
    }
    (changed && cur != text).then_some(cur)
}

fn walk_tail(s: &str, job: &SheetJob, own: &[Shift], anchor: Option<&str>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pos = 0;
    // the original `sqref` of the conditional format or validation being walked: its formulas are
    // relative to its top-left cell
    let mut local: Option<String> = None;
    // the autoFilter being walked: its first column before the moves and after them (a filter
    // column's `colId` counts from it)
    let mut filter: Option<(i32, i32)> = None;
    let row_shifts: Vec<Shift> = own.iter().copied().filter(|x| x.is_rows()).collect();
    let col_shifts: Vec<Shift> = own.iter().copied().filter(|x| !x.is_rows()).collect();
    let mut in_breaks: Option<bool> = None; // Some(true) = rowBreaks
    while let Some(t) = next_tag(s, pos) {
        out.push_str(&s[pos..t.start]);
        pos = t.end;
        if t.closing {
            if t.name == "rowBreaks" || t.name == "colBreaks" {
                in_breaks = None;
            }
            if t.name == "conditionalFormatting" || t.name == "dataValidation" {
                local = None;
            }
            if t.name == "autoFilter" {
                filter = None;
            }
            out.push_str(t.text);
            continue;
        }
        let name = t.name;
        // a sparkline goes with its cell, as in Excel (its location deleted)
        if name == "x14:sparkline"
            && let Some((a, b)) = xml::element_span(s, &t)
            && xml::inner(&s[a..b], "xm:sqref").is_some_and(|sq| shift_sqref_text(sq.trim(), own).is_some_and(|n| n.is_empty()))
        {
            pos = b;
            continue;
        }
        // a container whose last member went goes too (an empty list is no list the schema
        // takes): a sparkline group, the groups, an x14 list of formats or validations, an
        // extension left with nothing, the extension list
        if let Some((_, member)) = CONTAINERS.iter().find(|(c, _)| *c == name)
            && !t.self_closing
            && let Some((a, b)) = xml::element_span(s, &t)
        {
            let inner = &s[t.end..b - format!("</{name}>").len()];
            let walked = walk_tail(inner, job, own, anchor);
            let holds = |x: &str| if member.is_empty() { next_tag(x, 0).is_some() } else { xml::find_open(x, member, 0).is_some() };
            if holds(inner) && !holds(&walked) {
                pos = b;
                continue;
            }
            out.push_str(t.text);
            out.push_str(&walked);
            out.push_str(&s[b - format!("</{name}>").len()..b]);
            pos = b;
            let _ = a;
            continue;
        }
        if X14_ELEMENTS.contains(&name) {
            let Some((a, b)) = xml::element_span(s, &t) else {
                out.push_str(t.text);
                continue;
            };
            let el = &s[a..b];
            let sq = xml::inner(el, "xm:sqref");
            if let Some(sq) = sq
                && let Some(n) = shift_sqref_text(sq.trim(), own)
                    && n.is_empty() {
                        pos = b;
                        continue;
                    }
            // transform inside (formulas, the sqref element itself)
            let inner_start = t.end - a;
            let inner = &el[inner_start..el.len() - format!("</{name}>").len()];
            out.push_str(t.text);
            out.push_str(&walk_tail(inner, job, own, sq.map(str::trim)));
            out.push_str(&format!("</{name}>"));
            pos = b;
            continue;
        }
        if DROP_IF_GONE_TAG.contains(&name) || DROP_IF_GONE_ELEMENT.contains(&name) {
            let key = if attr(t.text, "sqref").is_some() {
                "sqref"
            } else {
                "ref"
            };
            let mut text = t.text.to_string();
            if let Some(v) = attr(t.text, key) {
                let new = if key == "sqref" {
                    shift_sqref_text(v, own).map(Some)
                } else {
                    shift_area_text(v, own)
                };
                // a merge cut down to one cell is no merge: Excel unmerges it
                let one_cell = |n: &str| {
                    name == "mergeCell" && refs::parse_area(n).is_some_and(|a| {
                        let (r1, c1, r2, c2) = a.bounds();
                        r1 == r2 && c1 == c2
                    })
                };
                match new {
                    Some(Some(n)) if !n.is_empty() && !one_cell(&n) => text = set_attr(&text, key, &n),
                    Some(_) => {
                        // gone: drop the element (and nothing of it lives on in what follows)
                        pos = xml::element_span(s, &t).map(|(_, b)| b).unwrap_or(t.end);
                        continue;
                    }
                    None => {}
                }
            }
            if !t.self_closing {
                if name == "conditionalFormatting" || name == "dataValidation" {
                    local = attr(t.text, "sqref").map(str::to_string);
                }
                if name == "autoFilter" {
                    filter = attr(t.text, "ref").and_then(refs::parse_area).map(|a| {
                        let (_, c1, _, _) = a.bounds();
                        let now = attr(&text, "ref").and_then(refs::parse_area).map(|b| b.bounds().1).unwrap_or(c1);
                        (c1, now)
                    });
                }
            }
            out.push_str(&text);
            continue;
        }
        if name == "filterColumn"
            && let (Some((was, now)), Some(id)) = (filter, attr(t.text, "colId").and_then(|v| v.parse::<i32>().ok()))
        {
            // a filter on a column counts from the filter's first column; one on a deleted column goes
            match map_index(&col_shifts, false, was + id) {
                Some(col) => out.push_str(&set_attr(t.text, "colId", &(col - now).to_string())),
                None => {
                    pos = if t.self_closing { t.end } else { xml::element_span(s, &t).map(|(_, b)| b).unwrap_or(t.end) };
                }
            }
            continue;
        }
        if name == "rowBreaks" || name == "colBreaks" {
            in_breaks = Some(name == "rowBreaks");
            out.push_str(t.text);
            continue;
        }
        if name == "brk"
            && let (Some(rows), Some(id)) = (
                in_breaks,
                attr(t.text, "id").and_then(|v| v.parse::<i32>().ok()),
            ) {
                let shifts = if rows { &row_shifts } else { &col_shifts };
                // a break on a deleted row or column goes with it
                if let Some(n) = map_index(shifts, rows, id) {
                    out.push_str(&set_attr(t.text, "id", &n.to_string()));
                }
                continue;
            }
        // a conditional format's threshold (`<cfvo val>`): a number, or a formula moved as the
        // format's own are
        if name == "cfvo"
            && let Some(val) = attr(t.text, "val")
        {
            let decoded = xml::decode_attr(val);
            let shifted = match local.as_deref().or(anchor) {
                Some(sqref) => shift_anchored(&decoded, sqref, job),
                None => shift_text(&decoded, job),
            };
            match shifted {
                Some(n) => out.push_str(&set_attr(t.text, "val", &crate::patch::xml::escape_attr(&n))),
                None => out.push_str(t.text),
            }
            continue;
        }
        if FORMULA_TEXT.contains(&name) && !t.self_closing
            && let Some(close) = xml::find_close(s, &t) {
                let raw = &s[t.end..close.start];
                let decoded = decode_text(raw);
                let shifted = match local.as_deref().or(anchor) {
                    Some(sqref) => shift_anchored(&decoded, sqref, job),
                    None => shift_text(&decoded, job),
                };
                let new = shifted.map(|n| escape_text(&n)).unwrap_or_else(|| raw.to_string());
                out.push_str(t.text);
                out.push_str(&new);
                out.push_str(close.text);
                pos = close.end;
                continue;
            }
        if name == "xm:sqref" && !t.self_closing
            && let Some(close) = xml::find_close(s, &t) {
                let raw = &s[t.end..close.start];
                let new = shift_sqref_text(raw.trim(), own).unwrap_or_else(|| raw.to_string());
                out.push_str(t.text);
                out.push_str(&new);
                out.push_str(close.text);
                pos = close.end;
                continue;
            }
        out.push_str(t.text);
    }
    out.push_str(&s[pos..]);
    out
}

/// After elements were dropped: the `count` attributes that state them.
fn fix_counts(s: &str) -> String {
    let mut out = s.to_string();
    for (parent, child, attrs) in [
        ("mergeCells", "mergeCell", &["count"][..]),
        ("dataValidations", "dataValidation", &["count"][..]),
        ("x14:dataValidations", "x14:dataValidation", &["count"][..]),
        ("rowBreaks", "brk", &["count", "manualBreakCount"][..]),
        ("colBreaks", "brk", &["count", "manualBreakCount"][..]),
    ] {
        let Some(open) = xml::find_open(&out, parent, 0) else {
            continue;
        };
        let Some((a, b)) = xml::element_span(&out, &open) else {
            continue;
        };
        let n = {
            let mut n = 0;
            let mut p = open.end;
            while let Some(t) = xml::find_open(&out[..b], child, p) {
                n += 1;
                p = t.end;
            }
            n
        };
        if n == 0 {
            out = format!("{}{}", &out[..a], &out[b..]);
            continue;
        }
        let mut tag = open.text.to_string();
        for at in attrs {
            if attr(&tag, at).is_some() {
                tag = set_attr(&tag, at, &n.to_string());
            }
        }
        out = format!("{}{}{}", &out[..open.start], tag, &out[open.end..]);
    }
    out
}
