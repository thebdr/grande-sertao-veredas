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
//! empty, on either side. A cell the live side cannot give (the linked workbook is missing and read
//! from the cache, or lacks the sheet) is not a difference: the link's status already says what
//! happened to that book.

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
    /// What the linked workbook holds now; `None` when empty.
    pub live: Option<ExternalValue>,
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

/// `[1]ZONES!D23: cached 4 → live 5`, the log line.
impl fmt::Display for LinkDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: cached {} → live {}",
            self.address(),
            format_value(self.cached.as_ref()),
            format_value(self.live.as_ref())
        )
    }
}

/// A value as the log spells it: a number as Excel turns it into text (15 significant digits,
/// `1E+21` from an exponent of 15 on, `0.0000001` below, `0` for a negative zero); text between
/// double quotes, a quote doubled as in a formula, every character as it is but a line feed, a
/// carriage return and a tab, written `\n`, `\r` and `\t`, and any other control character,
/// written `\u{..}`, so the line stays one line; a boolean upper-case, an error as itself, an empty
/// cell as `(empty)`.
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
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
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
        if let Some((cached, live)) = differing(cached, live) {
            diffs.push(LinkDiff {
                book: reference.book,
                sheet: reference.sheet,
                row: reference.row,
                column: reference.column,
                cached,
                live,
            });
        }
    }
    diffs
}

impl OpenedWorkbook {
    /// The link difference log of this opening: empty unless the links were resolved against the
    /// linked workbooks ([`LinkOutcome::Linked`]); books that could not be opened are left out,
    /// their status already saying so.
    pub fn link_diffs(&mut self) -> Vec<LinkDiff> {
        let LinkOutcome::Linked(statuses) = &self.links else {
            return Vec::new();
        };
        let skip: Vec<u32> = statuses
            .iter()
            .filter(|s| !matches!(s.state, ironcalc::import::LinkState::Loaded))
            .map(|s| s.index)
            .collect();
        link_diffs(&mut self.model, &skip)
    }
}
