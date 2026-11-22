//! References into other workbooks: `[1]Sheet!A1`.
//!
//! A workbook may read cells of other workbooks. The file keeps, for every
//! such *external book*, the path it was linked from, the names of its sheets
//! and the last value seen of every cell that is referenced (the
//! `sheetDataSet` of `xl/externalLinks/externalLinkN.xml`): what Excel shows
//! while the linked workbook is closed. The model keeps that as
//! [`ExternalBook`]s, in the order of the workbook's `externalReferences`, so
//! that `[n]` in a formula is the `n`-th book (1-based) and a saved file
//! carries the links it was opened with.
//!
//! *How* a referenced cell is read is a strategy, the [`ExternalResolver`]
//! of the model:
//!
//! * [`CachedResolver`] reads the values the file carries — Excel's own
//!   behaviour with the linked book closed. The default.
//! * a *linked* resolver opens the linked workbook, evaluates it and reads
//!   live values. It needs a file loader, so it lives with the xlsx layer
//!   (`ironcalc::import::LinkedResolver`).
//! * [`HostResolver`] hands the question to the application (a host that
//!   can ask another program — Excel — to compute the linked workbook).
//!
//! Whatever the strategy, a cell it cannot resolve is `#REF!`: the formula
//! is well formed, it is the data that is missing.

use std::collections::{BTreeSet, HashMap};

use bitcode::{Decode, Encode};

use crate::constants::{LAST_COLUMN, LAST_ROW};
use crate::expressions::parser::Node;
use crate::expressions::token::Error;

/// The value an external cell holds.
#[derive(Encode, Decode, Debug, PartialEq, Clone)]
pub enum ExternalValue {
    Number(f64),
    String(String),
    Boolean(bool),
    Error(Error),
}

/// One sheet of an external book: its name and the cells the file carries a
/// value for, keyed by `(row, column)`, both 1-based.
#[derive(Encode, Decode, Debug, PartialEq, Clone, Default)]
pub struct ExternalSheet {
    pub name: String,
    pub cells: HashMap<(i32, i32), ExternalValue>,
}

/// A defined name of an external book, kept as the file spells it so that it
/// survives a save. Formulas cannot use it yet (`[1]Book!Name` is a parse
/// error).
#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct ExternalDefinedName {
    pub name: String,
    /// The `refersTo` attribute, verbatim.
    pub refers_to: String,
    /// The index, in `ExternalBook::sheets`, of the sheet the name is local
    /// to; `None` for a workbook-level name.
    pub sheet_id: Option<u32>,
}

/// Another workbook this one references.
#[derive(Encode, Decode, Debug, PartialEq, Clone, Default)]
pub struct ExternalBook {
    /// The link target as the file spells it: a path relative to the
    /// workbook's folder (`Linked_01.xlsx`), an absolute path,
    /// or a URL.
    pub target: String,
    /// An absolute URL Excel recorded beside the relative target
    /// (`xxl21:absoluteUrl`), if the file carries one.
    pub absolute_url: Option<String>,
    /// The sheets, in the order of the file's `sheetNames`.
    pub sheets: Vec<ExternalSheet>,
    pub defined_names: Vec<ExternalDefinedName>,
}

impl ExternalBook {
    /// The sheet called `name`. Sheet names compare case-insensitively, as in Excel.
    pub fn sheet(&self, name: &str) -> Option<&ExternalSheet> {
        self.sheets.iter().find(|s| same_sheet_name(&s.name, name))
    }

    /// The sheet called `name`, mutable. See [`ExternalBook::sheet`].
    pub fn sheet_mut(&mut self, name: &str) -> Option<&mut ExternalSheet> {
        self.sheets
            .iter_mut()
            .find(|s| same_sheet_name(&s.name, name))
    }
}

/// Excel compares sheet names without case. The ASCII comparison allocates
/// nothing, which matters: a large workbook asks this for every one of its
/// hundreds of thousands of external references.
pub fn same_sheet_name(a: &str, b: &str) -> bool {
    if a.is_ascii() && b.is_ascii() {
        a.eq_ignore_ascii_case(b)
    } else {
        a.to_lowercase() == b.to_lowercase()
    }
}

/// A cell of another workbook that a formula reads: `[book]sheet!row,column`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExternalCellRef {
    /// 1-based, as written in the formula.
    pub book: u32,
    pub sheet: String,
    pub row: i32,
    pub column: i32,
}

/// The most cells one external range contributes to a listing; the same
/// bound the evaluation puts on materialising such a range.
pub(crate) const MAX_EXTERNAL_RANGE_CELLS: i64 = 4_000_000;

/// Adds to `out` every cell of another workbook that `node` reads, with the
/// formula sitting at (`row`, `column`) so relative references resolve.
pub(crate) fn collect_external_references(
    node: &Node,
    row: i32,
    column: i32,
    out: &mut BTreeSet<ExternalCellRef>,
) {
    match node {
        Node::ExternalReferenceKind {
            book,
            sheet_name,
            absolute_row,
            absolute_column,
            row: r,
            column: c,
        } => {
            let r = if *absolute_row { *r } else { *r + row };
            let c = if *absolute_column { *c } else { *c + column };
            if r >= 1 && c >= 1 && r <= LAST_ROW && c <= LAST_COLUMN {
                out.insert(ExternalCellRef {
                    book: *book,
                    sheet: sheet_name.clone(),
                    row: r,
                    column: c,
                });
            }
        }
        Node::ExternalRangeKind {
            book,
            sheet_name,
            absolute_row1,
            absolute_column1,
            row1,
            column1,
            absolute_row2,
            absolute_column2,
            row2,
            column2,
        } => {
            let r1 = if *absolute_row1 { *row1 } else { *row1 + row };
            let r2 = if *absolute_row2 { *row2 } else { *row2 + row };
            let c1 = if *absolute_column1 {
                *column1
            } else {
                *column1 + column
            };
            let c2 = if *absolute_column2 {
                *column2
            } else {
                *column2 + column
            };
            let (r1, r2) = (r1.min(r2).max(1), r1.max(r2).min(LAST_ROW));
            let (c1, c2) = (c1.min(c2).max(1), c1.max(c2).min(LAST_COLUMN));
            if r1 > r2 || c1 > c2 {
                return;
            }
            let cells = (r2 - r1 + 1) as i64 * (c2 - c1 + 1) as i64;
            if cells > MAX_EXTERNAL_RANGE_CELLS {
                return;
            }
            for r in r1..=r2 {
                for c in c1..=c2 {
                    out.insert(ExternalCellRef {
                        book: *book,
                        sheet: sheet_name.clone(),
                        row: r,
                        column: c,
                    });
                }
            }
        }
        Node::OpRangeKind { left, right }
        | Node::OpConcatenateKind { left, right }
        | Node::OpSumKind { left, right, .. }
        | Node::OpProductKind { left, right, .. }
        | Node::OpPowerKind { left, right }
        | Node::CompareKind { left, right, .. } => {
            collect_external_references(left, row, column, out);
            collect_external_references(right, row, column, out);
        }
        Node::UnaryKind { right, .. } => collect_external_references(right, row, column, out),
        Node::ImplicitIntersection { child, .. } | Node::SpillRangeOperator { child } => {
            collect_external_references(child, row, column, out)
        }
        Node::FunctionKind { args, .. } | Node::NamedFunctionKind { args, .. } => {
            for arg in args {
                collect_external_references(arg, row, column, out);
            }
        }
        Node::LambdaDefKind { body, .. } => collect_external_references(body, row, column, out),
        Node::LambdaCallKind { lambda, args } => {
            collect_external_references(lambda, row, column, out);
            for arg in args {
                collect_external_references(arg, row, column, out);
            }
        }
        // Leaves, and nodes whose text is not parsed here (a defined name's
        // formula is its own model entry)
        Node::BooleanKind(_)
        | Node::NumberKind(_)
        | Node::StringKind(_)
        | Node::ReferenceKind { .. }
        | Node::RangeKind { .. }
        | Node::WrongReferenceKind { .. }
        | Node::WrongRangeKind { .. }
        | Node::ArrayKind(_)
        | Node::DefinedNameKind { .. }
        | Node::TableNameKind(_)
        | Node::NamedVariableKind { .. }
        | Node::ErrorKind(_)
        | Node::ParseErrorKind { .. }
        | Node::EmptyArgKind => {}
    }
}

/// What a resolver found for one external cell.
#[derive(Debug, PartialEq, Clone)]
pub enum ExternalLookup {
    /// The cell holds this value.
    Value(ExternalValue),
    /// The sheet is known and the cell is empty.
    Empty,
    /// The book or the sheet is unknown to this strategy, or the strategy
    /// has no data (a host that cannot answer). Evaluates to `#REF!`.
    NotAvailable,
}

/// How the model reads a cell of another workbook. See the module docs.
pub trait ExternalResolver {
    /// The value of cell `(row, column)` of sheet `sheet` of the `book`-th
    /// external book (1-based, as written in the formula). `books` are the
    /// workbook's external books, for the strategies that read the cache or
    /// need the link target.
    fn resolve(
        &mut self,
        books: &[ExternalBook],
        book: u32,
        sheet: &str,
        row: i32,
        column: i32,
    ) -> ExternalLookup;

    /// A short name for diagnostics: `cached`, `linked`, `host`.
    fn strategy(&self) -> &'static str;
}

/// Reads the values the file carries (`sheetDataSet`): what Excel shows while
/// the linked workbook is closed. A cell the cache does not hold is empty —
/// Excel caches every referenced cell, so an absent one was empty when the
/// file was saved. A book or sheet the cache does not know is not available.
#[derive(Debug, Default, Clone, Copy)]
pub struct CachedResolver;

impl ExternalResolver for CachedResolver {
    fn resolve(
        &mut self,
        books: &[ExternalBook],
        book: u32,
        sheet: &str,
        row: i32,
        column: i32,
    ) -> ExternalLookup {
        let Some(book) = book.checked_sub(1).and_then(|i| books.get(i as usize)) else {
            return ExternalLookup::NotAvailable;
        };
        let Some(sheet) = book.sheet(sheet) else {
            return ExternalLookup::NotAvailable;
        };
        match sheet.cells.get(&(row, column)) {
            Some(value) => ExternalLookup::Value(value.clone()),
            None => ExternalLookup::Empty,
        }
    }

    fn strategy(&self) -> &'static str {
        "cached"
    }
}

/// The signature of a host's hook: the external book (for its target), the
/// sheet name, the row and the column.
pub type HostHook = Box<dyn FnMut(&ExternalBook, &str, i32, i32) -> ExternalLookup>;

/// Hands every lookup to the application. The host decides how the linked
/// workbook is computed (for instance by having Excel recalculate it) and
/// answers cell by cell; a host without an answer says `NotAvailable` and the
/// cell is `#REF!`.
pub struct HostResolver {
    hook: HostHook,
}

impl HostResolver {
    pub fn new(hook: HostHook) -> Self {
        HostResolver { hook }
    }

    /// A host that is not there: every cell is `NotAvailable`. The stand-in
    /// until an application wires its own hook.
    pub fn not_available() -> Self {
        HostResolver {
            hook: Box::new(|_, _, _, _| ExternalLookup::NotAvailable),
        }
    }
}

impl ExternalResolver for HostResolver {
    fn resolve(
        &mut self,
        books: &[ExternalBook],
        book: u32,
        sheet: &str,
        row: i32,
        column: i32,
    ) -> ExternalLookup {
        match book.checked_sub(1).and_then(|i| books.get(i as usize)) {
            Some(book) => (self.hook)(book, sheet, row, column),
            None => ExternalLookup::NotAvailable,
        }
    }

    fn strategy(&self) -> &'static str {
        "host"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn books() -> Vec<ExternalBook> {
        let mut cells = HashMap::new();
        cells.insert((6, 2), ExternalValue::String("L1.SEG%1".to_string()));
        cells.insert((4, 2), ExternalValue::Number(2.0));
        vec![ExternalBook {
            target: "Linked.xlsx".to_string(),
            absolute_url: None,
            sheets: vec![
                ExternalSheet {
                    name: "Cover".to_string(),
                    cells: HashMap::new(),
                },
                ExternalSheet {
                    name: "ZONES".to_string(),
                    cells,
                },
            ],
            defined_names: vec![],
        }]
    }

    #[test]
    fn cached_reads_values_empties_and_unknowns() {
        let books = books();
        let mut r = CachedResolver;
        assert_eq!(
            r.resolve(&books, 1, "ZONES", 4, 2),
            ExternalLookup::Value(ExternalValue::Number(2.0))
        );
        // sheet names are case-insensitive
        assert_eq!(
            r.resolve(&books, 1, "zones", 6, 2),
            ExternalLookup::Value(ExternalValue::String("L1.SEG%1".to_string()))
        );
        // a cell the cache does not hold is empty
        assert_eq!(
            r.resolve(&books, 1, "ZONES", 88, 6),
            ExternalLookup::Empty
        );
        // an unknown sheet, book 0 and a book past the end are not available
        assert_eq!(
            r.resolve(&books, 1, "Nope", 1, 1),
            ExternalLookup::NotAvailable
        );
        assert_eq!(
            r.resolve(&books, 0, "Cover", 1, 1),
            ExternalLookup::NotAvailable
        );
        assert_eq!(
            r.resolve(&books, 2, "Cover", 1, 1),
            ExternalLookup::NotAvailable
        );
        assert_eq!(r.strategy(), "cached");
    }

    #[test]
    fn host_hook_sees_the_book_and_absent_host_is_not_available() {
        let books = books();
        let mut r = HostResolver::new(Box::new(|book, sheet, row, column| {
            assert_eq!(book.target, "Linked.xlsx");
            ExternalLookup::Value(ExternalValue::String(format!("{sheet}:{row}:{column}")))
        }));
        assert_eq!(
            r.resolve(&books, 1, "ZONES", 3, 4),
            ExternalLookup::Value(ExternalValue::String("ZONES:3:4".to_string()))
        );
        assert_eq!(
            r.resolve(&books, 7, "ZONES", 3, 4),
            ExternalLookup::NotAvailable
        );
        let mut none = HostResolver::not_available();
        assert_eq!(
            none.resolve(&books, 1, "ZONES", 4, 2),
            ExternalLookup::NotAvailable
        );
        assert_eq!(none.strategy(), "host");
    }
}
