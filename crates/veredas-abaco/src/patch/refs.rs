//! References inside formula TEXT as the file spells it (`_xlfn.`, `[1]Sheet!`,
//! `'Sheet name'!$A$1:B2`), found and rewritten token by token.
//!
//! The patcher never re-stringifies a formula it did not write: when rows or
//! columns move, only the reference tokens that point into the moved region
//! change, and every other byte of the formula — spacing, case, number
//! spelling, the `_xlfn.` prefixes — stays as Excel wrote it. That is what
//! "re-spelled as Excel would" means for a formula Excel already spelled: Excel
//! itself rewrites the references and nothing else.
//!
//! Recognised: `A1`, `$A$1`, `A1:B2`, `A:C`, `3:7`, each with an optional sheet
//! prefix (`Sheet1!`, `'My sheet'!`, 3-D `S1:S3!`), and the external prefix
//! `[1]Sheet!` / `'[1]Sheet name'!` (never shifted: it points into another
//! workbook — but translated like any other reference when a shared formula is
//! expanded). Skipped: string literals, structured references (`T[[#This
//! Row],[A]]`), error literals, function names (`LOG10(`), defined names and
//! LET/LAMBDA parameters (`_xlpm.x` — the file spelling makes them
//! unambiguous).

use crate::a1::{column_name, column_number, LAST_COLUMN, LAST_ROW};

/// One coordinate of a reference: its number and whether it is `$`-anchored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coord {
    pub v: i32,
    pub abs: bool,
}

/// The area a reference token names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefArea {
    Cell {
        col: Coord,
        row: Coord,
    },
    Range {
        c1: Coord,
        r1: Coord,
        c2: Coord,
        r2: Coord,
    },
    /// `A:C` — whole columns.
    Cols {
        c1: Coord,
        c2: Coord,
    },
    /// `3:7` — whole rows.
    Rows {
        r1: Coord,
        r2: Coord,
    },
}

impl RefArea {
    /// The A1 spelling, `$` kept.
    pub fn spell(&self) -> String {
        fn col(c: Coord) -> String {
            format!("{}{}", if c.abs { "$" } else { "" }, column_name(c.v))
        }
        fn row(r: Coord) -> String {
            format!("{}{}", if r.abs { "$" } else { "" }, r.v)
        }
        match *self {
            RefArea::Cell { col: c, row: r } => format!("{}{}", col(c), row(r)),
            RefArea::Range { c1, r1, c2, r2 } => {
                format!("{}{}:{}{}", col(c1), row(r1), col(c2), row(r2))
            }
            RefArea::Cols { c1, c2 } => format!("{}:{}", col(c1), col(c2)),
            RefArea::Rows { r1, r2 } => format!("{}:{}", row(r1), row(r2)),
        }
    }

    /// `(first row, first col, last row, last col)`; whole rows/columns span
    /// the sheet.
    pub fn bounds(&self) -> (i32, i32, i32, i32) {
        match *self {
            RefArea::Cell { col, row } => (row.v, col.v, row.v, col.v),
            RefArea::Range { c1, r1, c2, r2 } => (r1.v, c1.v, r2.v, c2.v),
            RefArea::Cols { c1, c2 } => (1, c1.v, LAST_ROW, c2.v),
            RefArea::Rows { r1, r2 } => (r1.v, 1, r2.v, LAST_COLUMN),
        }
    }
}

/// A structural move of one sheet's grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shift {
    InsertRows { at: i32, n: i32 },
    DeleteRows { at: i32, n: i32 },
    InsertCols { at: i32, n: i32 },
    DeleteCols { at: i32, n: i32 },
}

impl Shift {
    pub fn is_rows(&self) -> bool {
        matches!(self, Shift::InsertRows { .. } | Shift::DeleteRows { .. })
    }

    /// Where a single row (or column, by axis) lands; `None` when deleted or
    /// pushed off the sheet.
    pub fn index(&self, v: i32) -> Option<i32> {
        match *self {
            Shift::InsertRows { at, n } | Shift::InsertCols { at, n } => {
                let last = if self.is_rows() {
                    LAST_ROW
                } else {
                    LAST_COLUMN
                };
                let out = if v >= at { v + n } else { v };
                (out <= last).then_some(out)
            }
            Shift::DeleteRows { at, n } | Shift::DeleteCols { at, n } => {
                if v < at {
                    Some(v)
                } else if v < at + n {
                    None
                } else {
                    Some(v - n)
                }
            }
        }
    }

    /// Where a span `[a, b]` of rows (or columns) lands, as Excel moves a
    /// range: an insertion inside it grows it, a deletion shrinks it, a span
    /// wholly deleted is `None`.
    pub fn span(&self, a: i32, b: i32) -> Option<(i32, i32)> {
        match *self {
            Shift::InsertRows { at, n } | Shift::InsertCols { at, n } => {
                let last = if self.is_rows() {
                    LAST_ROW
                } else {
                    LAST_COLUMN
                };
                let a2 = if a >= at { a + n } else { a };
                let b2 = if b >= at { b + n } else { b };
                (a2 <= last).then_some((a2, b2.min(last)))
            }
            Shift::DeleteRows { at, n } | Shift::DeleteCols { at, n } => {
                let e = at + n - 1;
                if a >= at && b <= e {
                    return None;
                }
                let a2 = if a < at {
                    a
                } else if a <= e {
                    at
                } else {
                    a - n
                };
                let b2 = if b < at {
                    b
                } else if b <= e {
                    at - 1
                } else {
                    b - n
                };
                Some((a2, b2))
            }
        }
    }

    /// The area after the move; `None` = the area is gone (`#REF!`).
    pub fn area(&self, area: RefArea) -> Option<RefArea> {
        let rows = self.is_rows();
        let mv = |c: Coord| -> Option<Coord> {
            Some(Coord {
                v: self.index(c.v)?,
                abs: c.abs,
            })
        };
        match area {
            RefArea::Cell { col, row } => Some(if rows {
                RefArea::Cell { col, row: mv(row)? }
            } else {
                RefArea::Cell { col: mv(col)?, row }
            }),
            RefArea::Range { c1, r1, c2, r2 } => {
                if rows {
                    let (a, b) = self.span(r1.v, r2.v)?;
                    Some(RefArea::Range {
                        c1,
                        c2,
                        r1: Coord { v: a, abs: r1.abs },
                        r2: Coord { v: b, abs: r2.abs },
                    })
                } else {
                    let (a, b) = self.span(c1.v, c2.v)?;
                    Some(RefArea::Range {
                        r1,
                        r2,
                        c1: Coord { v: a, abs: c1.abs },
                        c2: Coord { v: b, abs: c2.abs },
                    })
                }
            }
            RefArea::Cols { c1, c2 } => {
                if rows {
                    Some(area)
                } else {
                    let (a, b) = self.span(c1.v, c2.v)?;
                    Some(RefArea::Cols {
                        c1: Coord { v: a, abs: c1.abs },
                        c2: Coord { v: b, abs: c2.abs },
                    })
                }
            }
            RefArea::Rows { r1, r2 } => {
                if rows {
                    let (a, b) = self.span(r1.v, r2.v)?;
                    Some(RefArea::Rows {
                        r1: Coord { v: a, abs: r1.abs },
                        r2: Coord { v: b, abs: r2.abs },
                    })
                } else {
                    Some(area)
                }
            }
        }
    }
}

/// A reference token found in a formula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefToken {
    /// Byte span of the area part (after any `Sheet!` prefix).
    pub start: usize,
    pub end: usize,
    /// The sheet the prefix names (unquoted), `None` without a prefix.
    pub sheet: Option<String>,
    /// The last sheet of a 3-D prefix (`S1:S3!`).
    pub sheet_last: Option<String>,
    /// `[n]…!` — a reference into another workbook.
    pub external: bool,
    pub area: RefArea,
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'\\' || b == b'?' || b >= 0x80
}

fn is_word(b: u8) -> bool {
    is_ident(b) || b == b'$'
}

/// `$?LETTERS` → a column coordinate.
fn parse_col(s: &str) -> Option<Coord> {
    let (abs, rest) = match s.strip_prefix('$') {
        Some(r) => (true, r),
        None => (false, s),
    };
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    Some(Coord {
        v: column_number(rest)?,
        abs,
    })
}

/// `$?DIGITS` → a row coordinate.
fn parse_row(s: &str) -> Option<Coord> {
    let (abs, rest) = match s.strip_prefix('$') {
        Some(r) => (true, r),
        None => (false, s),
    };
    if rest.is_empty() || rest.len() > 7 || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let v: i32 = rest.parse().ok()?;
    (1..=LAST_ROW).contains(&v).then_some(Coord { v, abs })
}

/// `$?LETTERS$?DIGITS` → a cell.
fn parse_cell(s: &str) -> Option<(Coord, Coord)> {
    let body = s.strip_prefix('$').unwrap_or(s);
    let split = body.find(|c: char| !c.is_ascii_alphabetic())?;
    let cut = s.len() - body.len() + split;
    Some((parse_col(&s[..cut])?, parse_row(&s[cut..])?))
}

/// Reads a word (`[A-Za-z0-9_.$\\?]` and non-ASCII) starting at `i`.
fn word_end(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && is_word(b[i]) {
        i += 1;
    }
    i
}

/// Parses the area at `i` (after a prefix or bare). Returns the area and its
/// end, or `None` when the text there is not a reference.
fn area_at(text: &str, i: usize) -> Option<(RefArea, usize)> {
    let b = text.as_bytes();
    let e1 = word_end(b, i);
    if e1 == i {
        return None;
    }
    let w1 = &text[i..e1];
    // A function call (`LOG10(`) or a name followed by `(` is not a reference.
    if b.get(e1) == Some(&b'(') {
        return None;
    }
    let second = |from: usize| -> Option<(&str, usize)> {
        if b.get(from) != Some(&b':') {
            return None;
        }
        let e2 = word_end(b, from + 1);
        if e2 == from + 1 || b.get(e2) == Some(&b'(') || b.get(e2) == Some(&b'!') {
            return None;
        }
        Some((&text[from + 1..e2], e2))
    };
    if let Some((col, row)) = parse_cell(w1) {
        if let Some((w2, e2)) = second(e1)
            && let Some((c2, r2)) = parse_cell(w2) {
                return Some((
                    RefArea::Range {
                        c1: col,
                        r1: row,
                        c2,
                        r2,
                    },
                    e2,
                ));
            }
        return Some((RefArea::Cell { col, row }, e1));
    }
    if let Some((w2, e2)) = second(e1) {
        if let (Some(c1), Some(c2)) = (parse_col(w1), parse_col(w2)) {
            return Some((RefArea::Cols { c1, c2 }, e2));
        }
        if let (Some(r1), Some(r2)) = (parse_row(w1), parse_row(w2)) {
            return Some((RefArea::Rows { r1, r2 }, e2));
        }
    }
    None
}

/// Splits a sheet prefix's name into (first, last) for 3-D references.
fn split_3d(name: &str) -> (String, Option<String>) {
    match name.split_once(':') {
        Some((a, b)) => (a.to_string(), Some(b.to_string())),
        None => (name.to_string(), None),
    }
}

/// Every reference token in `formula`, in order.
pub fn tokens(formula: &str) -> Vec<RefToken> {
    let b = formula.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'"' => {
                i += 1;
                while i < b.len() {
                    if b[i] == b'"' {
                        if b.get(i + 1) == Some(&b'"') {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'\'' => {
                // A quoted sheet prefix: 'name'! (quotes doubled inside).
                let mut j = i + 1;
                let mut name = String::new();
                while j < b.len() {
                    if b[j] == b'\'' {
                        if b.get(j + 1) == Some(&b'\'') {
                            name.push('\'');
                            j += 2;
                            continue;
                        }
                        break;
                    }
                    let ch_len = utf8_len(b[j]);
                    name.push_str(&formula[j..j + ch_len]);
                    j += ch_len;
                }
                if j < b.len() && b.get(j + 1) == Some(&b'!') {
                    let (external, name) = strip_book(&name);
                    let (first, last) = split_3d(&name);
                    let at = j + 2;
                    if let Some((area, end)) = area_at(formula, at) {
                        out.push(RefToken {
                            start: at,
                            end,
                            sheet: Some(first),
                            sheet_last: last,
                            external,
                            area,
                        });
                        i = end;
                        continue;
                    }
                    i = at;
                } else {
                    i = j + 1;
                }
            }
            b'[' => {
                // `[1]Sheet!A1` (external) or a structured reference.
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                if j > i + 1 && b.get(j) == Some(&b']') {
                    let ns = j + 1;
                    let ne = word_end_sheet(b, ns);
                    if b.get(ne) == Some(&b'!') && ne > ns {
                        let (first, last) = split_3d(&formula[ns..ne]);
                        let at = ne + 1;
                        if let Some((area, end)) = area_at(formula, at) {
                            out.push(RefToken {
                                start: at,
                                end,
                                sheet: Some(first),
                                sheet_last: last,
                                external: true,
                                area,
                            });
                            i = end;
                            continue;
                        }
                        i = at;
                        continue;
                    }
                }
                // Structured reference: skip the balanced brackets.
                let mut depth = 0;
                while i < b.len() {
                    match b[i] {
                        b'[' => depth += 1,
                        b']' => {
                            depth -= 1;
                            if depth == 0 {
                                i += 1;
                                break;
                            }
                        }
                        b'\'' => i += 1, // escape inside a structured ref
                        _ => {}
                    }
                    i += 1;
                }
            }
            b'#' => {
                // #REF!, #N/A, #DIV/0!, a spill marker after a reference.
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'/' || b[i] == b'_')
                {
                    i += 1;
                }
                if i < b.len() && (b[i] == b'!' || b[i] == b'?') {
                    i += 1;
                }
            }
            _ if is_word(c) => {
                let e = word_end(b, i);
                // `Sheet!` or 3-D `S1:S3!`
                let mut prefix_end = None;
                if b.get(e) == Some(&b'!') {
                    prefix_end = Some(e);
                } else if b.get(e) == Some(&b':') {
                    let e2 = word_end(b, e + 1);
                    if e2 > e + 1 && b.get(e2) == Some(&b'!') {
                        prefix_end = Some(e2);
                    }
                }
                if let Some(pe) = prefix_end {
                    let (first, last) = split_3d(&formula[i..pe]);
                    let at = pe + 1;
                    if let Some((area, end)) = area_at(formula, at) {
                        out.push(RefToken {
                            start: at,
                            end,
                            sheet: Some(first),
                            sheet_last: last,
                            external: false,
                            area,
                        });
                        i = end;
                        continue;
                    }
                    i = at;
                    continue;
                }
                if let Some((area, end)) = area_at(formula, i) {
                    out.push(RefToken {
                        start: i,
                        end,
                        sheet: None,
                        sheet_last: None,
                        external: false,
                        area,
                    });
                    i = end;
                } else {
                    i = e;
                }
            }
            _ => i += 1,
        }
    }
    out
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/// A sheet name after `[n]`: anything up to `!` that is a word char.
fn word_end_sheet(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && (is_word(b[i]) || b[i] == b':') {
        i += 1;
    }
    i
}

/// `[1]Sheet name` → (external, `Sheet name`).
fn strip_book(name: &str) -> (bool, String) {
    if let Some(rest) = name.strip_prefix('[')
        && let Some(close) = rest.find(']')
            && rest[..close].bytes().all(|b| b.is_ascii_digit()) && close > 0 {
                return (true, rest[close + 1..].to_string());
            }
    (false, name.to_string())
}

/// What a rewrite does to one token.
pub enum Rewrite {
    Keep,
    Area(RefArea),
    /// The area is gone: the token becomes `#REF!` (its sheet prefix stays,
    /// as Excel writes `Sheet1!#REF!`).
    Gone,
}

/// Applies `f` to every reference token; `None` when nothing changed.
pub fn rewrite(formula: &str, mut f: impl FnMut(&RefToken) -> Rewrite) -> Option<String> {
    let toks = tokens(formula);
    let mut out = String::with_capacity(formula.len() + 8);
    let mut last = 0;
    let mut changed = false;
    for t in &toks {
        let replacement = match f(t) {
            Rewrite::Keep => continue,
            Rewrite::Area(a) => {
                if a == t.area {
                    continue;
                }
                a.spell()
            }
            Rewrite::Gone => "#REF!".to_string(),
        };
        out.push_str(&formula[last..t.start]);
        out.push_str(&replacement);
        last = t.end;
        changed = true;
    }
    if !changed {
        return None;
    }
    out.push_str(&formula[last..]);
    Some(out)
}

/// Does the token point at `sheet`? `home` is the sheet a bare reference
/// belongs to (`None`: a bare reference belongs to no known sheet). `order`
/// is the workbook's sheet order, for 3-D spans.
pub fn targets(t: &RefToken, sheet: &str, home: Option<&str>, order: &[String]) -> bool {
    if t.external {
        return false;
    }
    match (&t.sheet, &t.sheet_last) {
        (None, _) => home.is_some_and(|h| h.eq_ignore_ascii_case(sheet)),
        (Some(s), None) => s.eq_ignore_ascii_case(sheet),
        (Some(a), Some(z)) => {
            let pos = |n: &str| order.iter().position(|o| o.eq_ignore_ascii_case(n));
            match (pos(a), pos(z), pos(sheet)) {
                (Some(a), Some(z), Some(s)) => a.min(z) <= s && s <= a.max(z),
                _ => false,
            }
        }
    }
}

/// The formula after `shift` moved `sheet`'s grid; `None` when unchanged.
pub fn shift_formula(
    formula: &str,
    sheet: &str,
    shift: Shift,
    home: Option<&str>,
    order: &[String],
) -> Option<String> {
    rewrite(formula, |t| {
        if !targets(t, sheet, home, order) {
            return Rewrite::Keep;
        }
        match shift.area(t.area) {
            Some(a) => Rewrite::Area(a),
            None => Rewrite::Gone,
        }
    })
}

/// A shared formula's text as seen from a cell `dr` rows and `dc` columns
/// away from its master: relative coordinates move, `$` ones do not (every
/// reference, external included — Excel's shared-formula rule).
pub fn translate(formula: &str, dr: i32, dc: i32) -> String {
    if dr == 0 && dc == 0 {
        return formula.to_string();
    }
    let mv = |c: Coord, d: i32, last: i32| -> Option<Coord> {
        if c.abs {
            return Some(c);
        }
        let v = c.v + d;
        (1..=last).contains(&v).then_some(Coord { v, abs: false })
    };
    rewrite(formula, |t| {
        let a = match t.area {
            RefArea::Cell { col, row } => (|| {
                Some(RefArea::Cell {
                    col: mv(col, dc, LAST_COLUMN)?,
                    row: mv(row, dr, LAST_ROW)?,
                })
            })(),
            RefArea::Range { c1, r1, c2, r2 } => (|| {
                Some(RefArea::Range {
                    c1: mv(c1, dc, LAST_COLUMN)?,
                    r1: mv(r1, dr, LAST_ROW)?,
                    c2: mv(c2, dc, LAST_COLUMN)?,
                    r2: mv(r2, dr, LAST_ROW)?,
                })
            })(),
            RefArea::Cols { c1, c2 } => (|| {
                Some(RefArea::Cols {
                    c1: mv(c1, dc, LAST_COLUMN)?,
                    c2: mv(c2, dc, LAST_COLUMN)?,
                })
            })(),
            RefArea::Rows { r1, r2 } => (|| {
                Some(RefArea::Rows {
                    r1: mv(r1, dr, LAST_ROW)?,
                    r2: mv(r2, dr, LAST_ROW)?,
                })
            })(),
        };
        match a {
            Some(a) => Rewrite::Area(a),
            None => Rewrite::Gone,
        }
    })
    .unwrap_or_else(|| formula.to_string())
}

/// Parses a bare area (`A1`, `B2:C9`, `A:A`, `3:3`) as used in `ref`/`sqref`
/// attributes.
pub fn parse_area(text: &str) -> Option<RefArea> {
    let (a, end) = area_at(text, 0)?;
    (end == text.len()).then_some(a)
}

/// Moves a `ref` attribute's area; `None` = gone.
pub fn shift_ref(text: &str, shift: Shift) -> Option<Option<String>> {
    let area = parse_area(text)?;
    Some(shift.area(area).map(|a| a.spell()))
}

/// Moves a space-separated `sqref` list; areas that vanish are dropped. The
/// outer `None` = not parseable (leave it); `Some("")` = every area gone.
pub fn shift_sqref(text: &str, shift: Shift) -> Option<String> {
    let mut parts = Vec::new();
    for piece in text.split_whitespace() {
        let area = parse_area(piece)?;
        if let Some(a) = shift.area(area) {
            parts.push(a.spell());
        }
    }
    Some(parts.join(" "))
}


/// Every function call of a formula as the file spells it: its prefix (`_xlfn.`, `_xlfn._xlws.`,
/// none) and its name in upper case, in order, outside strings and quoted sheet names. Two
/// formulas the engine reads alike (`IFNA(…)`, `_xlfn.IFNA(…)`) differ here: Excel reads the first
/// as an unknown function.
pub fn function_calls(formula: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = formula.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' || c == '\'' {
            // to the closing quote, a doubled one kept inside
            i += 1;
            while i < chars.len() {
                if chars[i] == c && chars.get(i + 1) == Some(&c) {
                    i += 2;
                } else if chars[i] == c {
                    break;
                } else {
                    i += 1;
                }
            }
            i += 1;
        } else if c.is_alphabetic() || c == '_' || c == '\\' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '.' | '\\')) {
                i += 1;
            }
            if chars.get(i) == Some(&'(') {
                let word: String = chars[start..i].iter().collect::<String>().to_uppercase();
                let mut name = word.as_str();
                let mut prefix = String::new();
                for p in ["_XLFN.", "_XLWS."] {
                    if let Some(rest) = name.strip_prefix(p) {
                        prefix.push_str(p);
                        name = rest;
                    }
                }
                out.push((prefix, name.to_string()));
            }
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_are_read_with_their_prefix_outside_strings_and_sheet_names() {
        let pair = |p: &str, n: &str| (p.to_string(), n.to_string());
        assert_eq!(
            function_calls(r#"_xlfn.DAYS(A1,B1)+'My SUM(x'!A1&"CEILING(""x"")"&_xlfn._xlws.filter(A:A,B:B)+iso.ceiling(1)+\x(2)"#),
            [pair("_XLFN.", "DAYS"), pair("_XLFN._XLWS.", "FILTER"), pair("", "ISO.CEILING"), pair("", "\\X")]
        );
        assert_ne!(function_calls("IFNA(A1,0)"), function_calls("_xlfn.IFNA(A1,0)"));
        assert_eq!(function_calls("SUM(A1)"), function_calls("sum(A1)"));
    }

    fn order() -> Vec<String> {
        ["Cover", "CABINET", "S 2", "Last"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn ins_rows(f: &str, at: i32, n: i32) -> String {
        shift_formula(
            f,
            "CABINET",
            Shift::InsertRows { at, n },
            Some("CABINET"),
            &order(),
        )
        .unwrap_or_else(|| f.to_string())
    }

    fn del_rows(f: &str, at: i32, n: i32) -> String {
        shift_formula(
            f,
            "CABINET",
            Shift::DeleteRows { at, n },
            Some("CABINET"),
            &order(),
        )
        .unwrap_or_else(|| f.to_string())
    }

    #[test]
    fn finds_every_reference_shape_and_nothing_else() {
        let f = r#"SUM($A$1:B2)+Sheet1!C3+'S 2'!$D4&"A1 not a ref"&[1]DB_X!$K$57+LOG10(E5)+A:C+3:7+T[[#This Row],[B1]]+#REF!+1E5+_xlpm.t1"#;
        let got: Vec<(Option<String>, bool, String)> = tokens(f)
            .into_iter()
            .map(|t| (t.sheet.clone(), t.external, f[t.start..t.end].to_string()))
            .collect();
        assert_eq!(
            got,
            vec![
                (None, false, "$A$1:B2".into()),
                (Some("Sheet1".into()), false, "C3".into()),
                (Some("S 2".into()), false, "$D4".into()),
                (Some("DB_X".into()), true, "$K$57".into()),
                (None, false, "E5".into()),
                (None, false, "A:C".into()),
                (None, false, "3:7".into()),
            ]
        );
    }

    #[test]
    fn quoted_external_and_3d_prefixes() {
        let f = "'[1]DB name'!A1+'O''Brien'!B2+Cover:Last!C3";
        let t = tokens(f);
        assert_eq!(t.len(), 3);
        assert!(t[0].external);
        assert_eq!(t[0].sheet.as_deref(), Some("DB name"));
        assert_eq!(t[1].sheet.as_deref(), Some("O'Brien"));
        assert_eq!(t[2].sheet.as_deref(), Some("Cover"));
        assert_eq!(t[2].sheet_last.as_deref(), Some("Last"));
    }

    #[test]
    fn insert_rows_moves_only_what_lies_at_or_below() {
        assert_eq!(ins_rows("A4+A5+$A$6+A3", 5, 2), "A4+A7+$A$8+A3");
        // a range grows when the insertion falls inside it, moves when at its top
        assert_eq!(ins_rows("SUM(B2:B10)", 5, 3), "SUM(B2:B13)");
        assert_eq!(ins_rows("SUM(B5:B10)", 5, 1), "SUM(B6:B11)");
        assert_eq!(ins_rows("SUM(B2:B4)", 5, 1), "SUM(B2:B4)");
        // whole columns never move with rows; whole rows do
        assert_eq!(
            ins_rows("COUNTA(A:A)+SUM(5:6)", 5, 1),
            "COUNTA(A:A)+SUM(6:7)"
        );
    }

    #[test]
    fn other_sheets_and_other_books_do_not_move() {
        assert_eq!(
            ins_rows("Cover!A9+CABINET!A9+[1]CABINET!A9", 1, 1),
            "Cover!A9+CABINET!A10+[1]CABINET!A9"
        );
        // a bare reference on another sheet's formula does not point here
        let f = "A9";
        assert_eq!(
            shift_formula(
                f,
                "CABINET",
                Shift::InsertRows { at: 1, n: 1 },
                Some("Cover"),
                &order()
            ),
            None
        );
        // 3-D spans that include the sheet move
        assert_eq!(ins_rows("SUM(Cover:Last!A9)", 1, 1), "SUM(Cover:Last!A10)");
    }

    #[test]
    fn delete_rows_shrinks_ranges_and_refs_into_the_gap_become_ref_errors() {
        assert_eq!(del_rows("A4+A5+A6+A9", 5, 2), "A4+#REF!+#REF!+A7");
        assert_eq!(del_rows("CABINET!A5", 5, 1), "CABINET!#REF!");
        assert_eq!(del_rows("SUM(B2:B10)", 5, 3), "SUM(B2:B7)");
        assert_eq!(del_rows("SUM(B5:B10)", 4, 3), "SUM(B4:B7)");
        assert_eq!(del_rows("SUM(B5:B6)", 5, 2), "SUM(#REF!)");
    }

    #[test]
    fn columns_shift_by_letters() {
        let f = "B1+$Z$3+SUM(A1:C1)+B:D";
        assert_eq!(
            shift_formula(
                f,
                "CABINET",
                Shift::InsertCols { at: 2, n: 2 },
                Some("CABINET"),
                &order()
            )
            .unwrap(),
            "D1+$AB$3+SUM(A1:E1)+D:F"
        );
        assert_eq!(
            shift_formula(
                f,
                "CABINET",
                Shift::DeleteCols { at: 2, n: 1 },
                Some("CABINET"),
                &order()
            )
            .unwrap(),
            "#REF!+$Y$3+SUM(A1:B1)+B:C"
        );
    }

    #[test]
    fn strings_functions_names_and_numbers_are_left_alone() {
        let f =
            r#"_xlfn.LET(_xlpm.t,[1]DB!$K$57,SUBSTITUTE(_xlpm.t,"A5",LOG10(2))&" - "&T5)+1.5E+5"#;
        assert_eq!(
            ins_rows(f, 1, 1),
            r#"_xlfn.LET(_xlpm.t,[1]DB!$K$57,SUBSTITUTE(_xlpm.t,"A5",LOG10(2))&" - "&T6)+1.5E+5"#
        );
        assert_eq!(ins_rows("MyName+XFE1+ZZZZ9", 1, 1), "MyName+XFE1+ZZZZ9");
    }

    #[test]
    fn translate_moves_relative_parts_of_every_reference() {
        // a shared formula's child: relative parts move, $ parts and other books' $ do not
        assert_eq!(translate("$B$4&\".Alarm1\"", 3, 0), "$B$4&\".Alarm1\"");
        assert_eq!(translate("D1378+1", 1, 0), "D1379+1");
        assert_eq!(translate("[1]DB!$K57+B$2+C3", 2, 1), "[1]DB!$K59+C$2+D5");
        assert_eq!(translate("A1", -1, 0), "#REF!");
    }

    #[test]
    fn sqref_lists_drop_vanished_areas() {
        assert_eq!(
            shift_sqref("A1 B5:C6 D9", Shift::DeleteRows { at: 5, n: 2 }).unwrap(),
            "A1 D7"
        );
        assert_eq!(
            shift_sqref("B5:C6", Shift::DeleteRows { at: 5, n: 2 }).unwrap(),
            ""
        );
        assert_eq!(
            shift_ref("A1:X2339", Shift::InsertRows { at: 100, n: 1 }),
            Some(Some("A1:X2340".into()))
        );
        assert_eq!(
            shift_sqref("not an area", Shift::InsertRows { at: 1, n: 1 }),
            None
        );
    }
}
