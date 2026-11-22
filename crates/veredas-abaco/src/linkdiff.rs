//! The link difference log.
//!
//! When a workbook is resolved against its linked workbooks (the person answered *open and
//! calculate*, or the policy is *always recalculate linked spreadsheets*), Ábaco compares, for
//! every cell of another workbook a formula reads, the value the file cached (what Excel last saw)
//! with the value the linked workbook holds now, and writes **one log line per cell that differs**:
//! the address, the cached value, the live value. Nothing else. The architect's ruling for
//! Veredas 1's Ábaco (2026-11-04, its Q4): "a log line per row differing showing before and after
//! values is enough."
//!
//! Values are compared by kind, never by their display: a number against a number at Excel's 15
//! significant digits (what Excel can show of it, so `0.1+0.2` is `0.3`), text against text exactly
//! (its case included: a name whose case changed has changed), a boolean or an error against its
//! like; a number never equals a text. A cell the cache does not hold (Excel's self-closing
//! `<cell/>`, an empty cell) differs only when the live value is not empty; an empty text is
//! empty, on either side. A cell of a linked workbook that could not be opened is not a difference
//! (it is answered from the cache, and the link's status says why). A cell of a linked workbook that
//! opened but has no sheet of that name is one: the formula reads `#REF!` where the cache held a
//! value, and the line says so (`[1]ZONES!B2: cached 999 → live (no sheet ZONES)`).

use std::fmt;

use ironcalc_base::Model;
use ironcalc_base::external::{CachedResolver, ExternalLookup, ExternalResolver, ExternalValue};

use crate::links::{LinkOutcome, OpenedWorkbook};

/// One external cell whose live value differs from the cached one.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkDiff {
    /// 1-based, as the formulas write it.
    pub book: u32,
    pub sheet: String,
    pub row: i32,
    pub column: i32,
    /// What the file cached; `None` when the cache holds no value (empty).
    pub cached: Option<ExternalValue>,
    /// What the linked workbook holds now; `None` when empty, or when it has no such sheet.
    pub live: Option<ExternalValue>,
    /// The linked workbook opened but has no sheet of that name.
    pub live_sheet_missing: bool,
}

impl LinkDiff {
    /// The cell as a formula would name it: `[1]ZONES!D23`, `'[1]Zone list'!D23`.
    pub fn address(&self) -> String {
        let column = ironcalc_base::expressions::utils::number_to_column(self.column)
            .unwrap_or_else(|| format!("C{}", self.column));
        format!(
            "{}!{column}{}",
            ironcalc_base::expressions::utils::quote_external_name(self.book, &self.sheet),
            self.row
        )
    }
}

/// `[1]ZONES!D23: cached 4 → live 5`, the log line; `live (no sheet ZONES)` when the linked
/// workbook has no such sheet. Between two texts of which one holds a character beyond ASCII, the
/// line ends with the first character where they differ, by its code point
/// (`(first difference at character 5: U+2126 against U+03A9)`): an ohm sign and a Greek omega, a
/// non-breaking hyphen and a hyphen, or any two characters drawn alike, never read the same.
impl fmt::Display for LinkDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let live = if self.live_sheet_missing { format!("(no sheet {})", self.sheet) } else { format_value(self.live.as_ref()) };
        write!(f, "{}: cached {} → live {live}", self.address(), format_value(self.cached.as_ref()))?;
        if let (Some(ExternalValue::String(a)), Some(ExternalValue::String(b))) = (&self.cached, &self.live)
            && !(a.is_ascii() && b.is_ascii())
            && let Some(at) = first_difference(a, b)
        {
            write!(f, " {at}")?;
        }
        Ok(())
    }
}

/// `(first difference at character N: U+XXXX against U+YYYY)`, counting characters from 1, `end`
/// for the side that stopped; `None` when the two are equal.
fn first_difference(a: &str, b: &str) -> Option<String> {
    let code = |c: Option<char>| c.map(|c| format!("U+{:04X}", c as u32)).unwrap_or_else(|| "end".to_string());
    let (mut x, mut y) = (a.chars(), b.chars());
    let mut at = 1;
    loop {
        let (p, q) = (x.next(), y.next());
        if p.is_none() && q.is_none() {
            return None;
        }
        if p != q {
            return Some(format!("(first difference at character {at}: {} against {})", code(p), code(q)));
        }
        at += 1;
    }
}

/// A value as the log spells it: a number as Excel turns it into text (15 significant digits,
/// `1E+21` from an exponent of 15 on, `0.0000001` below, `0` for a negative zero); text between
/// double quotes, a quote doubled as in a formula, a backslash doubled, a line feed, a carriage
/// return and a tab written `\n`, `\r` and `\t`, and written `\u{..}`: any other control
/// character, every space but the plain one (a no-break space, a thin space), Unicode's line and
/// paragraph separators, the listed invisible format characters (a zero-width space, a soft
/// hyphen, a byte-order mark, the bidirectional controls) and the combining marks Latin, Greek
/// and Cyrillic letters decompose into (so a decomposed accent reads apart from a composed one), so
/// the line stays one line. Every other character as it is; a boolean upper-case, an error as itself, an empty
/// cell as `(empty)`. What else could read alike, a line tells apart by code point
/// (`LinkDiff`'s display).
pub fn format_value(value: Option<&ExternalValue>) -> String {
    match value {
        None => "(empty)".to_string(),
        Some(ExternalValue::Number(n)) => ironcalc_base::number_format::number_to_text(*n),
        Some(ExternalValue::String(s)) => quote(s),
        Some(ExternalValue::Boolean(true)) => "TRUE".to_string(),
        Some(ExternalValue::Boolean(false)) => "FALSE".to_string(),
        Some(ExternalValue::Error(e)) => e.to_string(),
    }
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\"\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if escaped(c) => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A character the log writes `\u{..}`: one that is invisible, or drawn like another, on a line.
fn escaped(c: char) -> bool {
    c.is_control()
        // every space but the plain one, and Unicode's line and paragraph separators
        || (c.is_whitespace() && c != ' ')
        // the invisible format characters: the soft hyphen, the Mongolian selectors, the
        // zero-width space, joiners and the left-to-right and right-to-left marks, the
        // bidirectional embeddings, overrides and isolates, the word joiner and the invisible
        // operators, the variation selectors, the byte-order mark
        || matches!(
            c,
            '\u{00ad}'
                | '\u{061c}'
                | '\u{180b}'..='\u{180f}'
                | '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206f}'
                | '\u{fe00}'..='\u{fe0f}'
                | '\u{feff}'
        )
        // the combining marks Latin, Greek and Cyrillic letters decompose into
        || matches!(c, '\u{0300}'..='\u{036f}' | '\u{1ab0}'..='\u{1aff}' | '\u{1dc0}'..='\u{1dff}' | '\u{20d0}'..='\u{20ff}' | '\u{fe20}'..='\u{fe2f}')
}

/// A lookup as a comparable value: `None` when the side cannot answer, `Some(None)` for an empty
/// cell or an empty text, `Some(Some(v))` for a value.
fn comparable(lookup: ExternalLookup) -> Option<Option<ExternalValue>> {
    match lookup {
        ExternalLookup::NotAvailable => None,
        ExternalLookup::Empty => Some(None),
        ExternalLookup::Value(ExternalValue::String(s)) if s.is_empty() => Some(None),
        ExternalLookup::Value(v) => Some(Some(v)),
    }
}

/// Two values the same by the module's rules: numbers at 15 significant digits, the rest exactly.
fn same(a: &Option<ExternalValue>, b: &Option<ExternalValue>) -> bool {
    use ironcalc_base::number_format::to_excel_precision;
    match (a, b) {
        (Some(ExternalValue::Number(x)), Some(ExternalValue::Number(y))) => to_excel_precision(*x, 15) == to_excel_precision(*y, 15),
        _ => a == b,
    }
}

/// Whether `cached` and `live` differ, and the pair to log when they do. A side that cannot answer
/// is never a difference.
pub fn differing(cached: ExternalLookup, live: ExternalLookup) -> Option<(Option<ExternalValue>, Option<ExternalValue>)> {
    let cached = comparable(cached)?;
    let live = comparable(live)?;
    if same(&cached, &live) {
        return None;
    }
    Some((cached, live))
}

/// Every external cell the model's formulas read whose value under the model's current strategy
/// differs from the cached one, in the order the engine lists them. `skip_books` are 1-based book
/// indexes left out (a linked workbook that could not be opened). It reads; it does not evaluate.
pub fn link_diffs(model: &mut Model, skip_books: &[u32]) -> Vec<LinkDiff> {
    link_diffs_of(model, skip_books, &[])
}

/// [`link_diffs`], where `opened_books` are the 1-based indexes of linked workbooks that opened:
/// for those, a cell the live side cannot give is one of a sheet the workbook does not have, and is
/// a line.
pub fn link_diffs_of(model: &mut Model, skip_books: &[u32], opened_books: &[u32]) -> Vec<LinkDiff> {
    let mut diffs = Vec::new();
    let references = model.external_references();
    let mut cache = CachedResolver;
    for reference in references {
        if skip_books.contains(&reference.book) {
            continue;
        }
        let cached = cache.resolve(
            &model.workbook.external_books,
            reference.book,
            &reference.sheet,
            reference.row,
            reference.column,
        );
        let live = model.resolve_external(reference.book, &reference.sheet, reference.row, reference.column);
        if matches!(live, ExternalLookup::NotAvailable) && opened_books.contains(&reference.book) {
            let cached = comparable(cached).flatten();
            diffs.push(LinkDiff {
                book: reference.book,
                sheet: reference.sheet,
                row: reference.row,
                column: reference.column,
                cached,
                live: None,
                live_sheet_missing: true,
            });
            continue;
        }
        if let Some((cached, live)) = differing(cached, live) {
            diffs.push(LinkDiff {
                book: reference.book,
                sheet: reference.sheet,
                row: reference.row,
                column: reference.column,
                cached,
                live,
                live_sheet_missing: false,
            });
        }
    }
    diffs
}

impl OpenedWorkbook {
    /// The link difference log of this opening: empty unless the links were resolved against the
    /// linked workbooks ([`LinkOutcome::Linked`]); books that could not be opened are left out,
    /// their status already saying so; a sheet missing from a book that opened is a line.
    pub fn link_diffs(&mut self) -> Vec<LinkDiff> {
        let LinkOutcome::Linked(statuses) = &self.links else {
            return Vec::new();
        };
        let (opened, skip): (Vec<_>, Vec<_>) =
            statuses.iter().partition(|s| matches!(s.state, ironcalc::import::LinkState::Loaded));
        let opened: Vec<u32> = opened.iter().map(|s| s.index).collect();
        let skip: Vec<u32> = skip.iter().map(|s| s.index).collect();
        link_diffs_of(&mut self.model, &skip, &opened)
    }
}
