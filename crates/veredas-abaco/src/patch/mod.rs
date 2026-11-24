//! The byte-faithful patcher for FOREIGN workbooks (`docs/abaco-plan.md`, phase 2).
//!
//! A workbook Ábaco did not create (a plant's engineering workbooks, the ones the other components
//! read) is never written through the fork's writer. It is patched: a list of
//! [`Edit`]s (cell values, formulas, inserted/deleted rows and columns, new
//! sheets, defined names, column widths, outline groups, fills and borders)
//! is applied to the engine's model, in order, and the result is spliced into
//! the ORIGINAL package:
//!
//! - every zip entry no edit touched is raw-copied — the same compressed
//!   bytes, so the same part, byte for byte (comments, custom properties,
//!   printer settings, the VBA project, drawings, the external-link cache);
//! - inside a touched part, untouched rows and cells keep their bytes; a
//!   moved reference is rewritten token by token ([`refs`]), wherever it
//!   points from (every sheet, defined names, validations, conditional
//!   formats, charts);
//! - a formula is spelled for the FILE — `_xlfn.` on post-2007 functions,
//!   `_xlpm.` on LET/LAMBDA parameters — from the engine's own parse of what
//!   the edit said, whatever spelling it came in (a missing `_xlfn.` reads as
//!   `#NAME?` in Excel);
//! - cached values are the engine's where the edits reached: the edited cells
//!   and every formula whose engine value the edits MOVED (evaluated before
//!   and after them — the engine has no dependency graph to ask, so the
//!   dependency is measured; `Before`) carry what the engine computed (it
//!   matches Excel's cache, the oracle's parity, C-222),
//!   so the file reads right before Excel recalculates. Every other formula
//!   keeps the file's cache byte for byte, also where the engine would say
//!   something else (a link with no `externalLink` part, a function it lacks,
//!   a shared child with no master): inside a touched part, a `<c>` element
//!   changes only when an edit wrote it, a move rewrote its formula or
//!   position, its shared group had to be expanded, or the edits moved its
//!   value. Text is written inline, so `sharedStrings.xml` is never touched;
//! - a package with no styles part (OOXML allows it) is patched without one;
//!   a minimal part ([`styles::MINIMAL`]) is added, with its relationship and
//!   content type, only when an edit creates a format (a fill, a border);
//! - every cell edit may carry what it expects to find ([`Expect`]), a
//!   three-way check: the file already holding the new content is
//!   "already there", holding something else is a conflict and the edit is
//!   skipped;
//! - the write is atomic: the new file is written beside the destination,
//!   [`PatchOptions::verify`] (on by default) re-opens THAT file through the
//!   engine and compares every cell (content and value) with the model the
//!   edits produced, and every cache the edits moved with the value written,
//!   the source is re-read (it must not have moved since it was read), and
//!   only then is the file renamed over the destination, its previous version
//!   kept as `.bak`; a failed verification or a moved source leaves the
//!   destination as it was.

pub mod cached;
pub mod package;
pub mod refs;
pub mod sheet;
pub mod styles;
pub mod xml;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ironcalc_base::expressions::parser::stringify::to_excel_string;
use ironcalc_base::expressions::parser::{new_parser_english, Node};
use ironcalc_base::expressions::types::CellReferenceRC;
use ironcalc_base::types::Cell;
use serde::{Deserialize, Serialize};

use crate::a1::{column_number, Addr, Area};
use crate::links::{self, LinkChoice, LinkPolicy};
use crate::book::{Content, Name, Value, Workbook};

pub use refs::Shift;
pub use styles::{LineStyle, StyleOp};

use sheet::{CellWrite, ColOp, NewContent, RowOp, SheetJob};

const NS_WORKSHEET: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
const CT_WORKSHEET: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
const NS_STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
const CT_STYLES: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml";
const NS_METADATA: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata";
const CT_METADATA: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheetMetadata+xml";
/// The metadata part Excel 365 writes for a workbook with dynamic arrays: one cell metadata
/// entry (`cm="1"`) pointing at the dynamic-array property.
const METADATA: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:xda="http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray"><metadataTypes count="1"><metadataType name="XLDAPR" minSupportedVersion="120000" copy="1" pasteAll="1" pasteValues="1" merge="1" splitFirst="1" rowColShift="1" clearFormats="1" clearComments="1" assign="1" coerce="1" cellMeta="1"/></metadataTypes><futureMetadata name="XLDAPR" count="1"><bk><extLst><ext uri="{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}"><xda:dynamicArrayProperties fDynamic="1" fCollapsed="0"/></ext></extLst></bk></futureMetadata><cellMetadata count="1"><bk><rc t="1" v="0"/></bk></cellMetadata></metadata>"#;

#[derive(Debug, thiserror::Error)]
pub enum PatchError {
    #[error("{path}: {message}")]
    Io { path: PathBuf, message: String },
    #[error("the package is not what a workbook should be: {0}")]
    Malformed(String),
    #[error("edit {index} ({op}): {message}")]
    Edit {
        index: usize,
        op: String,
        message: String,
    },
    #[error("not supported by the patcher yet: {0}")]
    Unsupported(String),
    #[error("{0} changed on disk while it was being patched — nothing was written")]
    SourceMoved(PathBuf),
    #[error("verification failed: {0}")]
    Verify(String),
    #[error(transparent)]
    Open(#[from] links::OpenError),
}

fn io_err(path: &Path, e: impl std::fmt::Display) -> PatchError {
    PatchError::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    }
}

/// What a cell edit expects the file to hold now (the three-way base).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Expect {
    /// A formula, in any spelling (`=`, `_xlfn.` and `_xlpm.` optional).
    Formula { formula: String },
    /// A value (`null` = empty).
    Value { value: serde_json::Value },
}

/// One change to a foreign workbook. Addresses are A1 (`B4`, `B4:C9`), rows
/// and columns 1-based, sheets by name; each edit sees the workbook as the
/// edits before it left it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Edit {
    /// A value: number, text (stored as text, never parsed), bool, `null`
    /// (clear), or `{"error": "#N/A"}`.
    Value {
        sheet: String,
        cell: String,
        value: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expect: Option<Expect>,
    },
    /// A formula in user (`=LET(t,1,t)`) or file spelling.
    Formula {
        sheet: String,
        cell: String,
        formula: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expect: Option<Expect>,
    },
    /// Clears the contents (not the formats) of an area.
    Clear {
        sheet: String,
        area: String,
    },
    InsertRows {
        sheet: String,
        at: i32,
        count: i32,
    },
    DeleteRows {
        sheet: String,
        at: i32,
        count: i32,
    },
    InsertColumns {
        sheet: String,
        at: String,
        count: i32,
    },
    DeleteColumns {
        sheet: String,
        at: String,
        count: i32,
    },
    AddSheet {
        name: String,
    },
    /// A defined name (formula in any spelling); `scope` = a sheet for a
    /// local name.
    SetName {
        name: String,
        formula: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
    },
    DeleteName {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
    },
    /// `columns`: `B` or `B:D`; width in Excel's character units.
    ColumnWidth {
        sheet: String,
        columns: String,
        width: f64,
    },
    /// Outline level 0–7 on rows `first..=last` (`level` 0 removes it).
    OutlineRows {
        sheet: String,
        first: i32,
        last: i32,
        level: u8,
        #[serde(default)]
        hidden: bool,
    },
    OutlineColumns {
        sheet: String,
        columns: String,
        level: u8,
        #[serde(default)]
        hidden: bool,
    },
    /// A solid interior colour `RRGGBB` (hex) on an area; `null` removes it.
    Fill {
        sheet: String,
        area: String,
        rgb: Option<String>,
    },
    /// The same line on the four edges of every cell of an area; `style`
    /// `null` removes the border.
    Border {
        sheet: String,
        area: String,
        #[serde(default)]
        rgb: Option<String>,
        #[serde(default)]
        style: Option<LineStyle>,
    },
}

impl Edit {
    fn op(&self) -> &'static str {
        match self {
            Edit::Value { .. } => "value",
            Edit::Formula { .. } => "formula",
            Edit::Clear { .. } => "clear",
            Edit::InsertRows { .. } => "insert_rows",
            Edit::DeleteRows { .. } => "delete_rows",
            Edit::InsertColumns { .. } => "insert_columns",
            Edit::DeleteColumns { .. } => "delete_columns",
            Edit::AddSheet { .. } => "add_sheet",
            Edit::SetName { .. } => "set_name",
            Edit::DeleteName { .. } => "delete_name",
            Edit::ColumnWidth { .. } => "column_width",
            Edit::OutlineRows { .. } => "outline_rows",
            Edit::OutlineColumns { .. } => "outline_columns",
            Edit::Fill { .. } => "fill",
            Edit::Border { .. } => "border",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Applied,
    /// The file already holds what the edit writes (nothing written).
    AlreadyThere,
    /// The file holds neither what the edit expects nor what it writes:
    /// skipped.
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub index: usize,
    pub status: Status,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fate {
    Identical,
    Rewritten,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartFate {
    pub name: String,
    pub fate: Fate,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PatchReport {
    pub outcomes: Vec<Outcome>,
    pub parts: Vec<PartFate>,
    /// Formula cells whose cached value was brought up to the engine's.
    pub refreshed: usize,
    /// Shared-formula groups expanded into plain formulas.
    pub expanded: usize,
    pub rows_rewritten: usize,
    pub calc_chain_dropped: bool,
    /// Cells compared by [`PatchOptions::verify`] (0 = not run).
    pub verified_cells: usize,
}

impl PatchReport {
    pub fn applied(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| o.status == Status::Applied)
            .count()
    }

    pub fn count(&self, fate: Fate) -> usize {
        self.parts.iter().filter(|p| p.fate == fate).count()
    }

    /// The report as log lines (the workspace's `_Log`).
    pub fn log_lines(&self, file: &str) -> Vec<String> {
        let mut out = vec![format!(
            "patch {file}: {} applied, {} already there, {} conflict(s); parts {} identical, {} rewritten, {} added, {} removed; {} cached value(s) refreshed, {} shared group(s) expanded{}",
            self.applied(),
            self.outcomes
                .iter()
                .filter(|o| o.status == Status::AlreadyThere)
                .count(),
            self.outcomes
                .iter()
                .filter(|o| o.status == Status::Conflict)
                .count(),
            self.count(Fate::Identical),
            self.count(Fate::Rewritten),
            self.count(Fate::Added),
            self.count(Fate::Removed),
            self.refreshed,
            self.expanded,
            if self.verified_cells > 0 {
                format!("; verified {} cells", self.verified_cells)
            } else {
                String::new()
            }
        )];
        for p in &self.parts {
            if p.fate != Fate::Identical {
                let fate = match p.fate {
                    Fate::Rewritten => "rewritten",
                    Fate::Added => "added",
                    Fate::Removed => "removed",
                    Fate::Identical => "identical",
                };
                out.push(format!("patch {file}: {fate} {}", p.name));
            }
        }
        for o in &self.outcomes {
            if o.status == Status::Conflict {
                out.push(format!(
                    "patch {file}: edit {} conflict — {}",
                    o.index, o.note
                ));
            }
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct PatchOptions {
    /// Re-open the written file, before it replaces the destination, and compare every cell with
    /// the model and every moved value with its written cache. On by default; off, a patch writes
    /// what it computed unchecked.
    pub verify: bool,
    /// Set `fullCalcOnLoad` (Excel recalculates everything on open). Off by
    /// default: the patcher writes the engine's values.
    pub full_calc_on_load: bool,
}

impl Default for PatchOptions {
    fn default() -> PatchOptions {
        PatchOptions { verify: true, full_calc_on_load: false }
    }
}

/// An identifier of a formula in A1 spelling: its text, the first character after it that is not
/// a space (`None` at the end), and the sheet that qualifies it (`Data!R`, `'My data'!R`).
#[derive(Debug, PartialEq)]
struct Token<'a> {
    text: &'a str,
    next: Option<char>,
    sheet: Option<String>,
}

/// The identifiers of a formula in A1 spelling, outside string literals and quoted sheet names.
fn tokens(formula: &str) -> Vec<Token<'_>> {
    let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '.' || c == '\\';
    let chars: Vec<(usize, char)> = formula.char_indices().collect();
    let next_after = |i: usize| formula[i..].chars().find(|c| !c.is_whitespace());
    let mut out = Vec::new();
    // the name a `!` qualifies with (the identifier or quoted name just before it) and where the `!` is
    let mut qualifier: Option<(String, usize)> = None;
    // the last name read and the index just past it
    let mut last: Option<(String, usize)> = None;
    let mut k = 0;
    while k < chars.len() {
        let (i, c) = chars[k];
        if c == '"' || c == '\'' {
            // a literal, `""` / `''` inside it standing for the quote itself
            let mut text = String::new();
            k += 1;
            while k < chars.len() {
                let (_, d) = chars[k];
                if d == c {
                    if chars.get(k + 1).is_some_and(|(_, e)| *e == c) {
                        text.push(c);
                        k += 2;
                        continue;
                    }
                    break;
                }
                text.push(d);
                k += 1;
            }
            let end = chars.get(k).map(|(j, _)| j + 1).unwrap_or(formula.len());
            last = (c == '\'').then_some((text, end));
            k += 1;
            continue;
        }
        if c == '!' {
            qualifier = last.take().filter(|(_, end)| *end == i).map(|(name, _)| (name, i));
            k += 1;
            continue;
        }
        if ident(c) {
            let start = i;
            while k < chars.len() && ident(chars[k].1) {
                k += 1;
            }
            let end = chars.get(k).map(|(j, _)| *j).unwrap_or(formula.len());
            let sheet = qualifier.take().filter(|(_, bang)| bang + 1 == start).map(|(name, _)| name);
            let text = &formula[start..end];
            out.push(Token { text, next: next_after(end), sheet });
            last = Some((text.to_string(), end));
            continue;
        }
        if !c.is_whitespace() {
            last = None;
        }
        k += 1;
    }
    out
}

/// Whether a formula (A1 spelling) calls a function whose value changes from one evaluation to the
/// next, by name (`_xlfn.` allowed): `GRANDTOTAL(` or a text `"RAND()"` is no call.
fn calls_volatile(formula: &str) -> bool {
    tokens(formula).iter().any(|t| {
        let name = t.text.trim_start_matches("_xlfn.").trim_start_matches("_xlws.");
        t.next == Some('(')
            && ["RAND", "RANDBETWEEN", "RANDARRAY", "NOW", "TODAY"].iter().any(|v| name.eq_ignore_ascii_case(v))
    })
}

/// The defined name (its place in `names`) a token of a formula on `sheet` reads (`None` for a
/// name's formula with no sheet of its own), as Excel resolves it: `S!N` is S's local N, or the
/// workbook's N; `N` is the formula's sheet's local N, or the workbook's N. A sheet (`N!`) or a
/// table (`N[`) is no name; a name called (`N(`) is one (a LAMBDA).
fn resolve(names: &[Name], token: &Token, sheet: Option<&str>) -> Option<usize> {
    if matches!(token.next, Some('!' | '[')) {
        return None;
    }
    let same = |a: &str, b: &str| a.to_lowercase() == b.to_lowercase();
    let scope = token.sheet.as_deref().or(sheet);
    scope
        .and_then(|sc| {
            names.iter().position(|n| same(&n.name, token.text) && n.scope.as_deref().is_some_and(|ns| same(ns, sc)))
        })
        .or_else(|| names.iter().position(|n| same(&n.name, token.text) && n.scope.is_none()))
}

/// Which defined names (by their place in `names`) call a volatile function, directly or through
/// other names, each name's formula read on its own sheet.
fn volatile_names(names: &[Name]) -> Vec<bool> {
    let mut volatile: Vec<bool> = names.iter().map(|n| calls_volatile(&n.formula)).collect();
    loop {
        let more: Vec<usize> = (0..names.len())
            .filter(|&i| !volatile[i])
            .filter(|&i| {
                tokens(&names[i].formula)
                    .iter()
                    .any(|t| resolve(names, t, names[i].scope.as_deref()).is_some_and(|j| volatile[j]))
            })
            .collect();
        if more.is_empty() {
            return volatile;
        }
        for i in more {
            volatile[i] = true;
        }
    }
}

/// Whether a formula of `sheet` (A1 spelling) calls a volatile function or reads a volatile name.
fn formula_volatile(formula: &str, sheet: &str, names: &[Name], volatile: &[bool]) -> bool {
    calls_volatile(formula)
        || tokens(formula).iter().any(|t| resolve(names, t, Some(sheet)).is_some_and(|j| volatile[j]))
}

/// Every formula of every sheet in A1 spelling, by sheet and by the engine's formula index (the
/// engine keeps them in R1C1, where `R[0]C[-2]` would read as a name `R`).
fn a1_formulas(wb: &Workbook) -> Vec<Vec<String>> {
    let model = wb.model();
    model
        .workbook
        .worksheets
        .iter()
        .enumerate()
        .map(|(i, ws)| {
            let ctx = CellReferenceRC { sheet: ws.get_name(), row: 1, column: 1 };
            model
                .parsed_formulas
                .get(i)
                .map(|parsed| parsed.iter().map(|p| to_excel_string(p.0.as_ref(), &ctx)).collect())
                .unwrap_or_default()
        })
        .collect()
}

/// Every dynamic array of a sheet: its anchor and how many rows and columns it spills over (the
/// engine keeps the size as width by height).
fn array_extents(ws: &ironcalc_base::types::Worksheet) -> std::collections::HashMap<(i32, i32), (i32, i32)> {
    ws.sheet_data
        .cells()
        .filter_map(|(row, col, c)| match &*c {
            Cell::ArrayFormula { r: (width, height), kind: ironcalc_base::types::ArrayKind::Dynamic, .. } => {
                Some(((row, col), (*height, *width)))
            }
            _ => None,
        })
        .collect()
}

/// Whether the engine computes the cell at `at` of `ws`: a formula, or a cell an array spills
/// into that its anchor's array covers now (a move can leave a spilled cell behind that its array
/// no longer reaches: the engine reads it empty).
fn computed_at(ws: &ironcalc_base::types::Worksheet, at: (i32, i32)) -> bool {
    match ws.cell(at.0, at.1).map(|c| c.as_ref().clone()) {
        Some(Cell::SpillCell { a, .. }) => match ws.cell(a.0, a.1).map(|c| c.as_ref().clone()) {
            Some(Cell::ArrayFormula { r: (width, height), .. }) => {
                (a.0..a.0 + height).contains(&at.0) && (a.1..a.1 + width).contains(&at.1)
            }
            _ => false,
        },
        Some(c) => c.get_formula().is_some(),
        None => false,
    }
}

/// How many evaluations, each with its own seed and day, the measure of what moves with a volatile
/// function compares with the reference one.
const OTHER_EVALUATIONS: u64 = 4;

/// The random functions' seed of the reference evaluation.
const REFERENCE_SEED: u64 = 1;

/// While the patcher evaluates: the engine's random functions seeded and its clock pinned, so the
/// evaluation before the edits, the one after them and verification's all draw the same numbers
/// and read the same time (the engine evaluates in a fixed order), and a cell that only reads a
/// random one is moved by an edit or by nothing. The measure of what moves with a volatile
/// function evaluates with other seeds, on other days. When it drops, the thread's own clock pin
/// comes back, and the random functions go back to chance unless the host had seeded them.
struct Fixed {
    host_clock: Option<i64>,
    host_seeded: bool,
    now: i64,
}

impl Fixed {
    fn new() -> Fixed {
        let host_clock = ironcalc_base::pinned_clock();
        let host_seeded = ironcalc_base::random_is_seeded();
        let now = host_clock.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)
        });
        let fixed = Fixed { host_clock, host_seeded, now };
        fixed.reference();
        fixed
    }

    /// The reference evaluation, everything is compared at.
    fn reference(&self) {
        ironcalc_base::seed_random(Some(REFERENCE_SEED));
        ironcalc_base::pin_clock(Some(self.now));
    }

    /// The `k`-th other evaluation: another seed, `k` days later.
    fn other(&self, k: u64) {
        ironcalc_base::seed_random(Some(REFERENCE_SEED + k));
        ironcalc_base::pin_clock(Some(self.now + k as i64 * 86_400_000));
    }
}

impl Drop for Fixed {
    fn drop(&mut self) {
        ironcalc_base::pin_clock(self.host_clock);
        if !self.host_seeded {
            ironcalc_base::seed_random(None);
        }
    }
}

/// The engine's value of every formula cell of the UNEDITED workbook, by
/// sheet index and original position — what decides which caches the patch
/// may re-write (an untouched cell comes out as it went in).
///
/// The engine is a full-recalculation engine with no dependency graph to ask,
/// so "depends on an edited cell" is measured, not inferred: the model is
/// evaluated before the edits (here) and after them (`recalc`), and a
/// formula's cached value is brought up to the engine's only when the two
/// evaluations differ — the edits moved it, directly or through any chain of
/// references, names, `INDIRECT`/`OFFSET` included. A formula the engine
/// evaluates differently from the file's cache for its own reasons (a link
/// with no `externalLink` part, a function it lacks, a shared child with no
/// master) evaluates the same both times, so its cache is kept, byte for byte.
///
/// Volatile functions (`RAND`, `RANDBETWEEN`, `RANDARRAY`, `NOW`, `TODAY`)
/// would read as "moved" on every evaluation. Every evaluation the patcher
/// compares is made alike ([`Fixed`]: one seed, one time), so a draw and the
/// cells that read it are the same before and after unless an edit moved
/// them; and when the workbook calls one, a cell whose formula calls it,
/// directly or through defined names (resolved by scope), the cells it spills
/// into, and every cell whose value differs in evaluations with other seeds on
/// other days are `unstable`: its cache is kept unless an edit wrote the cell
/// itself. Excel recalculates volatile cells on every recalculation anyway.
struct Before {
    values: Vec<std::collections::HashMap<(i32, i32), Value>>,
    unstable: Vec<std::collections::HashSet<(i32, i32)>>,
    /// Each dynamic array's spill (rows, columns), by its anchor's original position: an anchor's
    /// `ref` follows the engine only where the edits changed its spill (the engine and Excel may
    /// size one differently, and the file keeps Excel's then).
    extents: Vec<std::collections::HashMap<(i32, i32), (i32, i32)>>,
}

impl Before {
    fn snapshot(wb: &Workbook) -> Vec<std::collections::HashMap<(i32, i32), Value>> {
        let model = wb.model();
        model
            .workbook
            .worksheets
            .iter()
            .enumerate()
            .map(|(i, ws)| {
                // every computed cell: a formula, and a cell a formula spilled into (its value
                // moves with the formula's, a random array's included)
                ws.sheet_data
                    .cells()
                    .filter(|(_, _, c)| c.get_formula().is_some() || matches!(&**c, Cell::SpillCell { .. }))
                    .map(|(row, col, _)| {
                        let v = wb
                            .value_now(i as u32, Addr::new(row, col))
                            .unwrap_or(Value::Empty);
                        ((row, col), v)
                    })
                    .collect()
            })
            .collect()
    }

    /// Whether any formula of `wb`, a cell's or a defined name's, calls a volatile function (a
    /// text test: a false positive costs one more evaluation, nothing else).
    fn volatile(wb: &Workbook) -> bool {
        a1_formulas(wb).iter().flatten().any(|f| calls_volatile(f)) || volatile_names(&wb.names()).contains(&true)
    }

    /// The cells whose own formula calls a volatile function, directly or through a defined name
    /// that does, and the cells such a formula spills into: unstable whatever two evaluations
    /// happen to give (a random draw can repeat).
    fn volatile_cells(wb: &Workbook) -> Vec<std::collections::HashSet<(i32, i32)>> {
        let names = wb.names();
        let volatile_name = volatile_names(&names);
        let formulas = a1_formulas(wb);
        wb.model()
            .workbook
            .worksheets
            .iter()
            .zip(formulas.iter())
            .map(|(ws, texts)| {
                let sheet = ws.get_name();
                let volatile: Vec<bool> =
                    texts.iter().map(|f| formula_volatile(f, &sheet, &names, &volatile_name)).collect();
                let mut cells: std::collections::HashSet<(i32, i32)> = ws
                    .sheet_data
                    .cells()
                    .filter(|(_, _, c)| c.get_formula().and_then(|i| volatile.get(i as usize)).is_some_and(|v| *v))
                    .map(|(row, col, _)| (row, col))
                    .collect();
                let spilled: Vec<(i32, i32)> = ws
                    .sheet_data
                    .cells()
                    .filter_map(|(row, col, c)| match &*c {
                        Cell::SpillCell { a, .. } if cells.contains(a) => Some((row, col)),
                        _ => None,
                    })
                    .collect();
                cells.extend(spilled);
                cells
            })
            .collect()
    }

    /// The cells of an evaluated `wb` whose value a second evaluation
    /// changes (empty, and nothing evaluated, without volatile functions).
    fn unstable(
        wb: &mut Workbook,
        values: &[std::collections::HashMap<(i32, i32), Value>],
        fixed: &Fixed,
    ) -> Vec<std::collections::HashSet<(i32, i32)>> {
        if !Before::volatile(wb) {
            return vec![std::collections::HashSet::new(); values.len()];
        }
        // by text: the volatile formulas and what they spill into; measured: what moves with them,
        // over evaluations with other seeds and on other days than the reference one
        let mut out = Before::volatile_cells(wb);
        out.resize(values.len(), std::collections::HashSet::new());
        for k in 1..=OTHER_EVALUATIONS {
            fixed.other(k);
            wb.recalc();
            let again = Before::snapshot(wb);
            for ((a, b), u) in values.iter().zip(again.iter()).zip(out.iter_mut()) {
                u.extend(a.iter().filter(|(k, v)| !b.get(*k).is_some_and(|w| same(v, w))).map(|(k, _)| *k));
            }
        }
        // back to the reference evaluation the values were taken at
        fixed.reference();
        wb.recalc();
        out
    }

    /// `wb` as `links::open` left it under `fixed`'s reference: evaluated, unedited.
    fn take(wb: &mut Workbook, fixed: &Fixed) -> Before {
        let values = Before::snapshot(wb);
        let extents = wb.model().workbook.worksheets.iter().map(array_extents).collect();
        let unstable = Before::unstable(wb, &values, fixed);
        Before { values, unstable, extents }
    }

    /// Whether the edits moved the engine's value of the formula that was at
    /// `orig` on sheet `sheet` and evaluates to `live` now.
    /// `unstable_now`: whether the cell, where it is after the edits, is unstable in the edited
    /// workbook. A cell is exempt only when it is unstable both before and after: one the edits
    /// made deterministic (its random input replaced, deleted) has a value to bring its cache to.
    fn moved(&self, sheet: u32, orig: (i32, i32), live: &Value, unstable_now: bool) -> bool {
        let s = sheet as usize;
        if unstable_now && self.unstable.get(s).is_some_and(|u| u.contains(&orig)) {
            return false;
        }
        match self.values.get(s).and_then(|m| m.get(&orig)) {
            Some(was) => !same(was, live),
            // the file says formula, the engine did not read one (a shared
            // child whose master is missing): it has no value to bring the
            // cache to, so the file's stays
            None => false,
        }
    }
}

/// A cell write recorded while the edits run, at its position THEN.
struct PendingWrite {
    sheet: String,
    row: i32,
    col: i32,
    /// How many shifts had happened when it was made.
    after: usize,
    content: Option<ContentKind>,
    style: Vec<StyleOp>,
}

enum ContentKind {
    Value(Value),
    Formula,
}

struct PendingRows {
    sheet: String,
    first: i32,
    last: i32,
    after: usize,
    op: RowOp,
}

struct PendingCols {
    sheet: String,
    first: i32,
    last: i32,
    after: usize,
    op: ColOp,
}

fn json_value(v: &serde_json::Value) -> Result<Value, String> {
    Ok(match v {
        serde_json::Value::Null => Value::Empty,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().ok_or("not a number")?),
        serde_json::Value::String(s) => Value::Text(s.clone()),
        serde_json::Value::Object(o) => match o.get("error").and_then(|e| e.as_str()) {
            // the errors a file holds as a cell's value (`#SPILL!` and `#CALC!` are a formula's
            // only, spelled `#VALUE!` with metadata)
            Some(e) if ["#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A", "#GETTING_DATA"].contains(&e) => {
                Value::Error(e.to_string())
            }
            Some(e) => return Err(format!("not an error a cell holds as a value: {e}")),
            None => return Err(format!("not a value: {v}")),
        },
        serde_json::Value::Array(_) => return Err(format!("not a value: {v}")),
    })
}

fn columns(spec: &str) -> Result<(i32, i32), String> {
    let s = spec.replace('$', "");
    let (a, b) = s.split_once(':').unwrap_or((&s, &s));
    let a = column_number(a.trim()).ok_or_else(|| format!("not a column: {spec}"))?;
    let b = column_number(b.trim()).ok_or_else(|| format!("not a column: {spec}"))?;
    Ok((a.min(b), a.max(b)))
}

fn rgb(spec: &Option<String>) -> Result<Option<u32>, String> {
    match spec {
        None => Ok(None),
        Some(s) => u32::from_str_radix(s.trim().trim_start_matches('#'), 16)
            .ok()
            .filter(|v| *v <= 0xFF_FFFF)
            .map(Some)
            .ok_or_else(|| format!("not an RRGGBB colour: {s}")),
    }
}

/// The parsed formula of the cell, if it holds one.
fn formula_node(wb: &Workbook, sheet: u32, a: Addr) -> Option<Node> {
    let model = wb.model();
    let ws = model.workbook.worksheet(sheet).ok()?;
    let index = ws.cell(a.row, a.column)?.get_formula()?;
    Some(
        model
            .parsed_formulas
            .get(sheet as usize)?
            .get(index as usize)?
            .0
            .as_ref()
            .clone(),
    )
}

/// The table whose header row holds the cell (`sheet`, `row`, `col`), if one does: a header names
/// its table's column in the table's part, which the patcher does not rename.
fn table_header_at(wb: &Workbook, sheet: &str, row: i32, col: i32) -> Option<String> {
    wb.model().workbook.tables.values().find_map(|t| {
        if !t.sheet_name.eq_ignore_ascii_case(sheet) || t.header_row_count == 0 {
            return None;
        }
        let (r1, c1, _, c2) = refs::parse_area(&t.reference)?.bounds();
        (row == r1 && (c1..=c2).contains(&col)).then(|| format!("{} at {}!{}", t.name, t.sheet_name, t.reference))
    })
}

/// The formula `formula` parsed as at `a`, `None` when it does not parse.
fn parsed_at(wb: &Workbook, sheet: u32, a: Addr, formula: &str) -> Option<Node> {
    let model = wb.model();
    let names = model.workbook.get_worksheet_names();
    let mut parser = new_parser_english(names.clone(), model.workbook.get_defined_names_with_scope(), model.workbook.tables.clone());
    let ctx = CellReferenceRC { sheet: names.get(sheet as usize)?.clone(), row: a.row, column: a.column };
    let body = formula.trim();
    let node = parser.parse(body.strip_prefix('=').unwrap_or(body), &ctx);
    (!matches!(node, Node::ParseErrorKind { .. })).then_some(node)
}

/// Whether a formula an edit writes at `a` would be written as an array (see `array_under_an_operator`).
fn writes_as_array(wb: &Workbook, sheet: u32, a: Addr, formula: &str) -> bool {
    use ironcalc_base::expressions::parser::static_analysis::{run_static_analysis_on_node, StaticResult};
    parsed_at(wb, sheet, a, formula)
        .is_some_and(|n| !matches!(run_static_analysis_on_node(&n), StaticResult::Scalar) || array_under_an_operator(&n))
}

/// Whether the cell holds an array formula (dynamic or legacy), which Excel reads otherwise than
/// a plain one of the same text.
fn holds_an_array(wb: &Workbook, sheet: u32, a: Addr) -> bool {
    matches!(wb.content(sheet, a), Ok(Content::DynamicFormula(_) | Content::ArrayFormula { .. }))
}

/// Where a cell at `(row, col)` of `sheet` lands through the shifts from the `from`-th on; `None`
/// when one deletes it.
fn forward(shifts: &[(String, Shift)], from: usize, sheet: &str, mut row: i32, mut col: i32) -> Option<(i32, i32)> {
    for (s, sh) in shifts.get(from..).unwrap_or_default() {
        if s.eq_ignore_ascii_case(sheet) {
            if sh.is_rows() {
                row = sh.index(row)?;
            } else {
                col = sh.index(col)?;
            }
        }
    }
    Some((row, col))
}

/// Where the cell now at `(row, col)` of `sheet` was in the source, back through every shift so
/// far; `None` for a cell an insertion made.
fn backward(shifts: &[(String, Shift)], sheet: &str, mut row: i32, mut col: i32) -> Option<(i32, i32)> {
    for (s, sh) in shifts.iter().rev() {
        if !s.eq_ignore_ascii_case(sheet) {
            continue;
        }
        let v = if sh.is_rows() { &mut row } else { &mut col };
        *v = match *sh {
            Shift::InsertRows { at, n } | Shift::InsertCols { at, n } => {
                if *v < at {
                    *v
                } else if *v < at + n {
                    return None;
                } else {
                    *v - n
                }
            }
            Shift::DeleteRows { at, n } | Shift::DeleteCols { at, n } => {
                if *v < at { *v } else { *v + n }
            }
        };
    }
    Some((row, col))
}

/// Whether the source's own text of the formula now at `a` calls its functions as `want` spells
/// them (`_xlfn.IFNA`, never `IFNA`): the engine reads both alike, Excel does not. A cell an earlier
/// edit wrote, or one the source holds no formula text for, has no other spelling.
fn spelled_as_in_the_source(
    source: &cached::Cells,
    shifts: &[(String, Shift)],
    sheet: &str,
    a: Addr,
    want: &str,
) -> bool {
    let Some((row, col)) = backward(shifts, sheet, a.row, a.column) else { return true };
    let Some(cell) = source.iter().find(|((s, r, c), _)| s.eq_ignore_ascii_case(sheet) && *r == row && *c == col).map(|(_, c)| c)
    else {
        return true;
    };
    match cell.formula.as_deref().filter(|f| !f.is_empty()).or(cell.resolved.as_deref()) {
        Some(text) => refs::function_calls(text) == refs::function_calls(want),
        None => true,
    }
}

/// The first function a formula calls that the engine does not compute: not one of its own, nor a
/// name of the workbook (a LAMBDA) nor a parameter a LET or a LAMBDA around it binds. Such a call
/// would be cached as `#NAME?` and spelled as the engine prints an unknown name, so an edit
/// writing one is refused.
fn unknown_call(node: &Node, bound: &mut Vec<String>, names: &[String]) -> Option<String> {
    fn any<'n>(nodes: impl IntoIterator<Item = &'n Node>, bound: &mut Vec<String>, names: &[String]) -> Option<String> {
        nodes.into_iter().find_map(|n| unknown_call(n, bound, names))
    }
    // a LAMBDA parameter's name, which the engine keeps private but prints: `NamedVariable { name: "x", … }`
    fn parameter_name(p: &impl std::fmt::Debug) -> Option<String> {
        let text = format!("{p:?}");
        let start = text.find("name: \"")? + "name: \"".len();
        Some(text[start..start + text[start..].find('"')?].to_string())
    }
    match node {
        Node::NamedFunctionKind { name, args, .. } => {
            let bare = name.trim_start_matches("_xlfn.").trim_start_matches("_xlpm.");
            let known = bound.iter().any(|b| b.eq_ignore_ascii_case(bare)) || names.iter().any(|n| n.eq_ignore_ascii_case(bare));
            if !known {
                return Some(bare.to_uppercase());
            }
            any(args, bound, names)
        }
        Node::LambdaDefKind { parameters, body } => {
            let depth = bound.len();
            bound.extend(parameters.iter().filter_map(parameter_name));
            let found = unknown_call(body, bound, names);
            bound.truncate(depth);
            found
        }
        // the engine keeps its function list private: LET is told by its name
        Node::FunctionKind { kind, args } if format!("{kind:?}") == "Let" && args.len() >= 3 && args.len() % 2 == 1 => {
            let depth = bound.len();
            let pairs = (args.len() - 1) / 2;
            let mut found = None;
            for i in 0..pairs {
                if let Node::NamedVariableKind { name, .. } = &args[2 * i] {
                    bound.push(name.clone());
                }
                found = found.or_else(|| unknown_call(&args[2 * i + 1], bound, names));
            }
            let found = found.or_else(|| unknown_call(&args[2 * pairs], bound, names));
            bound.truncate(depth);
            found
        }
        Node::FunctionKind { args, .. } => any(args, bound, names),
        Node::LambdaCallKind { lambda, args } => any(std::iter::once(lambda.as_ref()).chain(args.iter()), bound, names),
        Node::OpRangeKind { left, right }
        | Node::OpConcatenateKind { left, right }
        | Node::OpSumKind { left, right, .. }
        | Node::OpProductKind { left, right, .. }
        | Node::OpPowerKind { left, right }
        | Node::CompareKind { left, right, .. } => any([left.as_ref(), right.as_ref()], bound, names),
        Node::UnaryKind { right, .. } => unknown_call(right, bound, names),
        Node::ImplicitIntersection { child, .. } | Node::SpillRangeOperator { child } => unknown_call(child, bound, names),
        _ => None,
    }
}

/// [`unknown_call`] of a formula an edit writes, parsed at `a` of `sheet`, where the names it sees
/// are the workbook's global ones and those local to `scope` (the cell's sheet, a name's own scope):
/// a LAMBDA name local to another sheet is none it can call.
fn calls_unknown(wb: &Workbook, sheet: u32, a: Addr, formula: &str, scope: Option<&str>) -> Option<String> {
    let node = parsed_at(wb, sheet, a, formula)?;
    let names: Vec<String> = wb
        .names()
        .into_iter()
        .filter(|n| match (&n.scope, scope) {
            (None, _) => true,
            (Some(local), Some(here)) => local.eq_ignore_ascii_case(here),
            (Some(_), None) => false,
        })
        .map(|n| n.name)
        .collect();
    unknown_call(&node, &mut Vec::new(), &names)
}

/// Whether an earlier edit of the list wrote the cell now at `a` of `sheet` (through the moves so
/// far): the file will hold it as the patcher writes its text.
fn written_by_an_edit(shifts: &[(String, Shift)], writes: &[PendingWrite], sheet: &str, a: Addr) -> bool {
    let here = (a.row, a.column);
    writes
        .iter()
        .any(|w| w.content.is_some() && w.sheet.eq_ignore_ascii_case(sheet) && forward(shifts, w.after, sheet, w.row, w.col) == Some(here))
}

/// Whether a name's definition is one cell (`S!$A$1`) or a constant: no range to intersect.
fn names_one_value(formula: &str) -> bool {
    let body = formula.trim().trim_start_matches('=');
    // an array constant of more than one element (`{1,2,3}`) is several values
    if body.starts_with('{') {
        return !body.contains([',', ';']);
    }
    let cell = body.rsplit('!').next().unwrap_or(body).replace('$', "");
    let letters = cell.chars().take_while(char::is_ascii_alphabetic).count();
    let reference = letters > 0 && letters < cell.len() && cell[letters..].chars().all(|c| c.is_ascii_digit());
    reference || !body.contains(':') && !body.contains('(') && !body.chars().next().is_some_and(char::is_alphabetic)
}

/// Whether a part of a formula is, before any evaluation, one value: a cell, a one-cell range, a
/// name of one cell or of a constant, a literal, an operation on such, or a function the engine's
/// static analysis finds scalar.
fn one_value(node: &Node) -> bool {
    use ironcalc_base::expressions::parser::static_analysis::{run_static_analysis_on_node, StaticResult};
    match node {
        Node::ReferenceKind { .. } | Node::NumberKind(_) | Node::StringKind(_) | Node::BooleanKind(_) | Node::ErrorKind(_) => true,
        Node::RangeKind { row1, column1, row2, column2, .. } => row1 == row2 && column1 == column2,
        Node::DefinedNameKind { formula, .. } => names_one_value(formula),
        Node::OpSumKind { left, right, .. }
        | Node::OpProductKind { left, right, .. }
        | Node::OpPowerKind { left, right }
        | Node::OpConcatenateKind { left, right }
        | Node::CompareKind { left, right, .. } => one_value(left) && one_value(right),
        Node::UnaryKind { right, .. } => one_value(right),
        // the engine's analysis cannot see a name's extent: a name or a range of one value is read
        // as the value it is before the analysis runs
        _ => matches!(run_static_analysis_on_node(&as_values(node)), StaticResult::Scalar),
    }
}

/// A formula with every name of one value and every one-cell range read as a number, the rest as
/// it is.
fn as_values(node: &Node) -> Node {
    let b = |n: &Node| Box::new(as_values(n));
    match node {
        Node::DefinedNameKind { formula, .. } if names_one_value(formula) => Node::NumberKind(0.0),
        Node::RangeKind { row1, column1, row2, column2, .. } if row1 == row2 && column1 == column2 => Node::NumberKind(0.0),
        Node::OpSumKind { kind, left, right } => Node::OpSumKind { kind: kind.clone(), left: b(left), right: b(right) },
        Node::OpProductKind { kind, left, right } => Node::OpProductKind { kind: kind.clone(), left: b(left), right: b(right) },
        Node::OpPowerKind { left, right } => Node::OpPowerKind { left: b(left), right: b(right) },
        Node::OpConcatenateKind { left, right } => Node::OpConcatenateKind { left: b(left), right: b(right) },
        Node::CompareKind { kind, left, right } => Node::CompareKind { kind: kind.clone(), left: b(left), right: b(right) },
        Node::UnaryKind { kind, right } => Node::UnaryKind { kind: kind.clone(), right: b(right) },
        Node::FunctionKind { kind, args } => Node::FunctionKind { kind: kind.clone(), args: args.iter().map(as_values).collect() },
        other => other.clone(),
    }
}

/// Whether every operator of a formula, at any depth, has single-valued operands: where one does
/// not, a legacy formula intersects it implicitly and an array formula lifts it.
fn operators_on_one_value(node: &Node) -> bool {
    match node {
        Node::OpSumKind { left, right, .. }
        | Node::OpProductKind { left, right, .. }
        | Node::OpPowerKind { left, right }
        | Node::OpConcatenateKind { left, right }
        | Node::CompareKind { left, right, .. } => {
            one_value(left) && one_value(right) && operators_on_one_value(left) && operators_on_one_value(right)
        }
        Node::UnaryKind { right, .. } => one_value(right) && operators_on_one_value(right),
        Node::FunctionKind { args, .. } | Node::NamedFunctionKind { args, .. } => args.iter().all(operators_on_one_value),
        Node::LambdaCallKind { lambda, args } => operators_on_one_value(lambda) && args.iter().all(operators_on_one_value),
        Node::ImplicitIntersection { child, .. } | Node::SpillRangeOperator { child } => operators_on_one_value(child),
        _ => true,
    }
}

/// Whether a plain formula reads in Excel 365 as it is written, with no `@` of the implicit
/// intersection Excel shows on a legacy formula where it applies (Microsoft's rule: a range or a
/// function that can return one where a single value goes, `=@INDEX(A1:A10,B1)`): its result one
/// value, every operator's operands one value, and the engine's own pass adding no intersection to
/// a function's single-valued arguments. Such a formula is, to Excel, the array an edit writes of
/// the same text, whatever the values it reads; another is not, whatever they are.
fn reads_without_intersection(node: &Node) -> bool {
    // the engine's pass on the formula with its one-value names and ranges read as values (it
    // cannot see a name's extent)
    let base = as_values(node);
    let mut with = base.clone();
    ironcalc_base::expressions::parser::static_analysis::add_implicit_intersection(&mut with, true);
    with == base && one_value(node) && operators_on_one_value(node)
}

/// Whether the formula the cell holds (its text already found equal to `formula`'s) is, to Excel,
/// the formula an edit writes as `formula`: both arrays, a formula the engine reads as one value
/// (written plain, and plain or an array alike to Excel), one an earlier edit wrote (the patcher
/// writes one text one way), or a plain one Excel 365 reads with no implicit intersection
/// ([`reads_without_intersection`]), judged on the formula as it stands after the edits so far.
/// The text is parsed afresh: the engine stores a plain formula with the intersections its own
/// import added, where it cannot see a name's extent (`IF(Flag,1,0)` as `@IF(Flag,1,0)`).
fn same_kind(wb: &Workbook, sheet: u32, a: Addr, formula: &str, edited: bool) -> bool {
    // a legacy array (`t="array"`, its size fixed) is the dynamic array an edit writes only where
    // the formula's result is that size: `{=A1:A3*2}` over one cell shows one value, the edit spills
    if let Ok(Content::ArrayFormula { columns, rows, .. }) = wb.content(sheet, a)
        && !edited
    {
        use ironcalc_base::expressions::parser::static_analysis::{run_static_analysis_on_node, StaticResult};
        return match parsed_at(wb, sheet, a, formula).map(|n| run_static_analysis_on_node(&n)) {
            Some(StaticResult::Scalar) => columns == 1 && rows == 1,
            Some(StaticResult::Array(r, c) | StaticResult::Range(r, c)) => (r, c) == (rows, columns),
            _ => false,
        };
    }
    if holds_an_array(wb, sheet, a) || !writes_as_array(wb, sheet, a, formula) || edited {
        return true;
    }
    parsed_at(wb, sheet, a, formula).is_some_and(|n| reads_without_intersection(&n))
}

/// A formula in file spelling as `wb` would write it at `a` — `None` when
/// it does not parse.
fn file_spelling(wb: &Workbook, sheet: u32, a: Addr, formula: &str) -> Option<String> {
    let model = wb.model();
    let names = model.workbook.get_worksheet_names();
    let mut parser = new_parser_english(
        names.clone(),
        model.workbook.get_defined_names_with_scope(),
        model.workbook.tables.clone(),
    );
    let ctx = CellReferenceRC {
        sheet: names.get(sheet as usize)?.clone(),
        row: a.row,
        column: a.column,
    };
    let body = formula.trim();
    let body = body.strip_prefix('=').unwrap_or(body);
    let node = parser.parse(body, &ctx);
    if matches!(node, Node::ParseErrorKind { .. }) {
        return None;
    }
    Some(to_excel_string(&node, &ctx))
}

/// Whether legacy Excel would read a formula otherwise than a dynamic array does: a part that may
/// not be a single value (a range, a name, a table, a function that may return an array, as the
/// engine's static analysis judges it) where an operator, a comparison, `&` or a unary sign
/// expects one. A plain formula intersects it implicitly (`SUM(A1:A3*B1:B3)` reads one row), an
/// array formula takes it whole; a range given to a function as such (`SUM(A1:A3)`) reads alike.
fn array_under_an_operator(node: &Node) -> bool {
    use ironcalc_base::expressions::parser::static_analysis::{run_static_analysis_on_node, StaticResult};
    let not_single = |n: &Node| !matches!(run_static_analysis_on_node(n), StaticResult::Scalar);
    match node {
        Node::OpSumKind { left, right, .. }
        | Node::OpProductKind { left, right, .. }
        | Node::OpPowerKind { left, right }
        | Node::OpConcatenateKind { left, right }
        | Node::CompareKind { left, right, .. } => {
            not_single(left) || not_single(right) || array_under_an_operator(left) || array_under_an_operator(right)
        }
        Node::UnaryKind { right, .. } => not_single(right) || array_under_an_operator(right),
        // a function inside another whose result may not be single (`ABS(A1:A3)` in `SUM(…)`): a
        // plain formula intersects its argument, an array formula lifts it
        Node::FunctionKind { args, .. } | Node::NamedFunctionKind { args, .. } => args.iter().any(|arg| {
            (matches!(arg, Node::FunctionKind { .. } | Node::NamedFunctionKind { .. } | Node::LambdaCallKind { .. }) && not_single(arg))
                || array_under_an_operator(arg)
        }),
        Node::LambdaCallKind { lambda, args } => array_under_an_operator(lambda) || args.iter().any(array_under_an_operator),
        Node::ImplicitIntersection { child, .. } | Node::SpillRangeOperator { child } => array_under_an_operator(child),
        Node::OpRangeKind { left, right } => array_under_an_operator(left) || array_under_an_operator(right),
        _ => false,
    }
}

/// The cell's formula as the file spells it, `dynamic` for an array formula (no `@` of its implied:
/// every one written, as `_xlfn.SINGLE`).
fn file_formula(wb: &Workbook, sheet: u32, a: Addr, dynamic: bool) -> Option<String> {
    if !dynamic {
        return current_file_formula(wb, sheet, a);
    }
    let node = formula_node(wb, sheet, a)?;
    let ctx = CellReferenceRC { sheet: wb.sheet_name(sheet).ok()?, row: a.row, column: a.column };
    Some(ironcalc_base::expressions::parser::stringify::to_excel_array_formula_string(&node, &ctx))
}

/// The cell's formula in file spelling, `None` for a value cell.
fn current_file_formula(wb: &Workbook, sheet: u32, a: Addr) -> Option<String> {
    let node = formula_node(wb, sheet, a)?;
    let ctx = CellReferenceRC {
        sheet: wb.sheet_name(sheet).ok()?,
        row: a.row,
        column: a.column,
    };
    Some(to_excel_string(&node, &ctx))
}

/// Two values as Excel tells them apart: of one kind, numbers equal to 15 significant digits
/// (Excel's precision), texts, booleans and errors exactly; an empty cell is neither 0 nor `""`.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x == y || format!("{x:.14e}") == format!("{y:.14e}"),
        _ => a == b,
    }
}

/// Whether a cache read from the file holds `want`: as [`same`], save that an empty value and the
/// empty text a formula caches as `<v></v>` are one ([`sheet::same_value`], the rule the rewrite
/// refreshes a cache by).
fn cache_holds(cached: &Value, want: &Value) -> bool {
    sheet::same_value(cached, want)
}

fn describe(wb: &Workbook, sheet: u32, a: Addr) -> String {
    match current_file_formula(wb, sheet, a) {
        Some(f) => format!("={f}"),
        None => match wb.value_now(sheet, a) {
            Ok(Value::Text(t)) => format!("{t:?}"),
            Ok(Value::Number(n)) => sheet::number(n),
            Ok(Value::Bool(b)) => b.to_string().to_uppercase(),
            Ok(Value::Error(e)) => e,
            Ok(Value::Empty) | Err(_) => "(empty)".into(),
        },
    }
}

/// Whether the cell holds what `expect` says; a formula of the same text judged by [`same_kind`].
fn holds(wb: &Workbook, sheet: u32, a: Addr, expect: &Expect, edited: bool) -> Result<bool, String> {
    let current = current_file_formula(wb, sheet, a);
    match expect {
        Expect::Formula { formula } => {
            let want = file_spelling(wb, sheet, a, formula)
                .ok_or_else(|| format!("the expected formula does not parse: {formula}"))?;
            if current.as_deref() != Some(want.as_str()) {
                return Ok(false);
            }
            Ok(same_kind(wb, sheet, a, formula, edited))
        }
        Expect::Value { value } => {
            if current.is_some() {
                return Ok(false);
            }
            let want = json_value(value)?;
            let now = wb.value_now(sheet, a).map_err(|e| e.to_string())?;
            Ok(same(&now, &want))
        }
    }
}

/// Patches `source` into `dest` (a new file, or `source` itself — its
/// previous version then kept as `.bak`).
pub fn patch(
    source: &Path,
    dest: &Path,
    edits: &[Edit],
    options: &PatchOptions,
) -> Result<PatchReport, PatchError> {
    let bytes = std::fs::read(source).map_err(|e| io_err(source, e))?;
    // what the source held when it was read: it must hold the same before the rename
    let fingerprint = bytes.clone();
    let mut pkg = package::Package::open(bytes)?;
    // every sheet the file names, chart sheets included (the engine knows worksheets only)
    let file_sheets: Vec<String> = {
        let xml = pkg.read_text("xl/workbook.xml")?;
        let rels_part = package::rels_of("xl/workbook.xml");
        let rels = package::rels("xl/workbook.xml", &pkg.read_text(&rels_part)?);
        package::sheets(&xml, &rels)?.into_iter().map(|e| e.name).collect()
    };

    // ---- the engine's model, links from the cache (Excel's closed-book view)
    let fixed = Fixed::new();
    let opened = links::open(source, LinkPolicy::AlwaysUseCached, |_| {
        LinkChoice::UseCached
    })?;
    let mut wb = Workbook::from_model(opened.model, crate::book::Origin::New);
    // the workbook the model computes is the destination (`CELL("filename")` names it), from the
    // first evaluation the patch compares
    if let Some(stem) = dest.file_stem().map(|s| s.to_string_lossy().to_string())
        && stem != wb.model().workbook.name
    {
        wb.model_mut().workbook.name = stem;
        wb.recalc();
    }
    if crate::files::contents(source).map_err(|e| io_err(source, e))? != fingerprint {
        return Err(PatchError::SourceMoved(source.to_path_buf()));
    }
    // the engine's value of every formula BEFORE the edits: a cache is
    // re-written only where the edits moved it (see `Before`)
    let before = Before::take(&mut wb, &fixed);

    let mut report = PatchReport::default();
    let mut shifts: Vec<(String, Shift)> = Vec::new();
    let mut writes: Vec<PendingWrite> = Vec::new();
    let mut rows_ops: Vec<PendingRows> = Vec::new();
    let mut cols_ops: Vec<PendingCols> = Vec::new();
    let mut new_sheets: Vec<String> = Vec::new();
    let mut names_touched = false;
    // the source's cells as its file spells them, read when a formula edit needs them
    let mut source_cells: Option<cached::Cells> = None;
    let mut set_names: Vec<(String, Option<String>)> = Vec::new();
    // the names an edit deleted and none set again since: gone from the file, a built-in one too
    let mut deleted_names: Vec<(String, Option<String>)> = Vec::new();

    for (index, edit) in edits.iter().enumerate() {
        let fail = |message: String| PatchError::Edit {
            index,
            op: edit.op().to_string(),
            message,
        };
        let sheet_ix =
            |wb: &Workbook, name: &str| wb.sheet_index(name).map_err(|e| fail(e.to_string()));
        let mut outcome = Outcome {
            index,
            status: Status::Applied,
            note: String::new(),
        };
        match edit {
            Edit::Value {
                sheet,
                cell,
                expect,
                ..
            }
            | Edit::Formula {
                sheet,
                cell,
                expect,
                ..
            } => {
                let s = sheet_ix(&wb, sheet)?;
                let a = Addr::parse(cell).map_err(|e| fail(e.to_string()))?;
                let canonical = wb.sheet_name(s).map_err(|e| fail(e.to_string()))?;
                if let Some(Cell::ArrayFormula { .. } | Cell::SpillCell { .. }) = wb
                    .model()
                    .workbook
                    .worksheet(s)
                    .ok()
                    .and_then(|w| w.cell(a.row, a.column))
                    .map(|c| c.as_ref().clone())
                {
                    // a CSE/dynamic array is one object; a cell of it is not editable alone (a
                    // legacy array of one cell is the cell: Excel overwrites it)
                    let one_cell = matches!(
                        wb.model().workbook.worksheet(s).ok().and_then(|w| w.cell(a.row, a.column)).map(|c| c.as_ref().clone()),
                        Some(Cell::ArrayFormula { r: (1, 1), .. })
                    );
                    if !one_cell
                        && (formula_node(&wb, s, a).is_none() || matches!(wb.content(s, a), Ok(Content::ArrayFormula { .. })))
                    {
                        outcome.status = Status::Conflict;
                        outcome.note = format!("{canonical}!{a} is part of an array formula");
                        report.outcomes.push(outcome);
                        continue;
                    }
                }
                let (target, is_formula) = value_or_formula(edit);
                // already there?
                let already = if is_formula {
                    let want = file_spelling(&wb, s, a, &target)
                        .ok_or_else(|| fail(format!("the formula does not parse: {target}")))?;
                    let edited = written_by_an_edit(&shifts, &writes, &canonical, a);
                    current_file_formula(&wb, s, a).as_deref() == Some(want.as_str())
                        && same_kind(&wb, s, a, &target, edited)
                        && (edited || {
                            if source_cells.is_none() {
                                source_cells = Some(cached::read(source)?);
                            }
                            source_cells.as_ref().is_some_and(|c| spelled_as_in_the_source(c, &shifts, &canonical, a, &want))
                        })
                } else {
                    let v = json_value(match edit {
                        Edit::Value { value, .. } => value,
                        _ => &serde_json::Value::Null,
                    })
                    .map_err(fail)?;
                    current_file_formula(&wb, s, a).is_none()
                        && same(&wb.value_now(s, a).map_err(|e| fail(e.to_string()))?, &v)
                };
                if already {
                    outcome.status = Status::AlreadyThere;
                    outcome.note = format!("{canonical}!{a} already holds it");
                    report.outcomes.push(outcome);
                    continue;
                }
                if let Some(expect) = expect
                    && !holds(&wb, s, a, expect, written_by_an_edit(&shifts, &writes, &canonical, a))
                    .map_err(fail)?
                {
                        outcome.status = Status::Conflict;
                        outcome.note = format!("{canonical}!{a} now holds {}", describe(&wb, s, a));
                        report.outcomes.push(outcome);
                        continue;
                    }
                if let Some(table) = table_header_at(&wb, &canonical, a.row, a.column) {
                    return Err(PatchError::Unsupported(format!(
                        "{canonical}!{a} is a header of table {table} (a header names its column, and the patcher does not rename table columns yet)"
                    )));
                }
                let content = if is_formula {
                    if let Some(name) = calls_unknown(&wb, s, a, &target, Some(&canonical)) {
                        return Err(PatchError::Unsupported(format!(
                            "{canonical}!{a}: {name} is a function the engine does not compute (the patcher writes no formula it would cache as #NAME?)"
                        )));
                    }
                    wb.set_formula(s, a, &target)
                        .map_err(|e| fail(e.to_string()))?;
                    ContentKind::Formula
                } else {
                    let v = json_value(match edit {
                        Edit::Value { value, .. } => value,
                        _ => &serde_json::Value::Null,
                    })
                    .map_err(fail)?;
                    wb.set_value(s, a, &v).map_err(|e| fail(e.to_string()))?;
                    ContentKind::Value(v)
                };
                writes.push(PendingWrite {
                    sheet: canonical,
                    row: a.row,
                    col: a.column,
                    after: shifts.len(),
                    content: Some(content),
                    style: Vec::new(),
                });
            }
            Edit::Clear { sheet, area } => {
                let s = sheet_ix(&wb, sheet)?;
                let ar = Area::parse(area).map_err(|e| fail(e.to_string()))?;
                let canonical = wb.sheet_name(s).map_err(|e| fail(e.to_string()))?;
                let inside = |row: i32, col: i32| {
                    row >= ar.first.row && row <= ar.last.row && col >= ar.first.column && col <= ar.last.column
                };
                // a cell an array spills into from outside the area is the array's: Excel's clear
                // leaves it, as it leaves a value edit (an anchor inside clears the whole array)
                let spilled_from_outside = |a: &Addr| {
                    wb.model().workbook.worksheet(s).ok().and_then(|ws| ws.cell(a.row, a.column)).is_some_and(|c| {
                        matches!(c.as_ref(), Cell::SpillCell { a: (r, c), .. } if !inside(*r, *c))
                    })
                };
                let (left, existing): (Vec<Addr>, Vec<Addr>) = wb
                    .cells(s)
                    .map_err(|e| fail(e.to_string()))?
                    .into_iter()
                    .map(|(a, _)| a)
                    .filter(|a| inside(a.row, a.column))
                    .partition(|a| spilled_from_outside(a));
                if !left.is_empty() {
                    outcome.note = format!("{canonical}!{area}: {} cell(s) an array spills into left to it", left.len());
                }
                if let Some(a) = existing.iter().find(|a| table_header_at(&wb, &canonical, a.row, a.column).is_some()) {
                    let table = table_header_at(&wb, &canonical, a.row, a.column).unwrap_or_default();
                    return Err(PatchError::Unsupported(format!(
                        "{canonical}!{a} is a header of table {table} (a header names its column, and the patcher does not rename table columns yet)"
                    )));
                }
                wb.clear(s, ar).map_err(|e| fail(e.to_string()))?;
                for a in existing {
                    writes.push(PendingWrite {
                        sheet: canonical.clone(),
                        row: a.row,
                        col: a.column,
                        after: shifts.len(),
                        content: Some(ContentKind::Value(Value::Empty)),
                        style: Vec::new(),
                    });
                }
            }
            Edit::InsertRows { sheet, at, count } | Edit::DeleteRows { sheet, at, count } => {
                let s = sheet_ix(&wb, sheet)?;
                let canonical = wb.sheet_name(s).map_err(|e| fail(e.to_string()))?;
                let insert = matches!(edit, Edit::InsertRows { .. });
                let r = if insert {
                    wb.model_mut().insert_rows(s, *at, *count)
                } else {
                    wb.model_mut().delete_rows(s, *at, *count)
                };
                r.map_err(fail)?;
                wb.mark_stale();
                let sh = if insert {
                    Shift::InsertRows { at: *at, n: *count }
                } else {
                    Shift::DeleteRows { at: *at, n: *count }
                };
                move_model_names(&mut wb, &canonical, sh).map_err(fail)?;
                move_model_tables(&mut wb, &canonical, sh)?;
                shifts.push((canonical, sh));
            }
            Edit::InsertColumns { sheet, at, count } | Edit::DeleteColumns { sheet, at, count } => {
                let s = sheet_ix(&wb, sheet)?;
                let canonical = wb.sheet_name(s).map_err(|e| fail(e.to_string()))?;
                let c = columns(at).map_err(fail)?.0;
                let insert = matches!(edit, Edit::InsertColumns { .. });
                let r = if insert {
                    wb.model_mut().insert_columns(s, c, *count)
                } else {
                    wb.model_mut().delete_columns(s, c, *count)
                };
                r.map_err(fail)?;
                wb.mark_stale();
                let sh = if insert {
                    Shift::InsertCols { at: c, n: *count }
                } else {
                    Shift::DeleteCols { at: c, n: *count }
                };
                move_model_names(&mut wb, &canonical, sh).map_err(fail)?;
                move_model_tables(&mut wb, &canonical, sh)?;
                shifts.push((canonical, sh));
            }
            Edit::AddSheet { name } => {
                let taken = wb.sheet_index(name).is_ok()
                    || file_sheets.iter().chain(new_sheets.iter()).any(|n| n.to_lowercase() == name.to_lowercase());
                if taken {
                    return Err(fail(format!("a sheet named {name:?} exists")));
                }
                // Excel keeps the name for its change history
                if name.eq_ignore_ascii_case("History") {
                    return Err(fail(format!("{name:?} is a name Excel reserves")));
                }
                wb.add_sheet(name).map_err(|e| fail(e.to_string()))?;
                new_sheets.push(name.clone());
            }
            Edit::SetName {
                name,
                formula,
                scope,
            } => {
                let body = formula.trim().trim_start_matches('=');
                let at = scope.as_deref().and_then(|sc| wb.sheet_index(sc).ok()).unwrap_or(0);
                if let Some(called) = calls_unknown(&wb, at, Addr::new(1, 1), body, scope.as_deref()) {
                    return Err(PatchError::Unsupported(format!(
                        "name {name}: {called} is a function the engine does not compute (the patcher writes no formula it cannot read)"
                    )));
                }
                // the engine stores user spelling; give it that
                let user = strip_file_prefixes(body);
                let exists = wb.names().iter().any(|n| {
                    n.name.to_uppercase() == name.to_uppercase()
                        && n.scope.as_deref().map(str::to_lowercase)
                            == scope.as_deref().map(str::to_lowercase)
                });
                if exists {
                    wb.delete_name(name, scope.as_deref())
                        .map_err(|e| fail(e.to_string()))?;
                }
                wb.set_name(name, &user, scope.as_deref())
                    .map_err(|e| fail(e.to_string()))?;
                let key = (name.to_lowercase(), scope.as_ref().map(|s| s.to_lowercase()));
                deleted_names.retain(|k| *k != key);
                set_names.push(key);
                names_touched = true;
            }
            Edit::DeleteName { name, scope } => {
                wb.delete_name(name, scope.as_deref())
                    .map_err(|e| fail(e.to_string()))?;
                let key = (name.to_lowercase(), scope.as_ref().map(|s| s.to_lowercase()));
                set_names.retain(|k| *k != key);
                deleted_names.push(key);
                names_touched = true;
            }
            Edit::ColumnWidth {
                sheet,
                columns: spec,
                width,
            } => {
                let s = sheet_ix(&wb, sheet)?;
                let (first, last) = columns(spec).map_err(fail)?;
                if !(0.0..=255.0).contains(width) {
                    return Err(fail(format!("width {width} is outside 0–255")));
                }
                cols_ops.push(PendingCols {
                    sheet: wb.sheet_name(s).map_err(|e| fail(e.to_string()))?,
                    first,
                    last,
                    after: shifts.len(),
                    op: ColOp::Width {
                        first,
                        last,
                        width: *width,
                    },
                });
            }
            Edit::OutlineRows {
                sheet,
                first,
                last,
                level,
                hidden,
            } => {
                let s = sheet_ix(&wb, sheet)?;
                if *level > 7 || first > last || *first < 1 || *last > 1_048_576 {
                    return Err(fail("level 0–7 on rows first ≤ last".into()));
                }
                rows_ops.push(PendingRows {
                    sheet: wb.sheet_name(s).map_err(|e| fail(e.to_string()))?,
                    first: *first,
                    last: *last,
                    after: shifts.len(),
                    op: RowOp {
                        level: Some(*level),
                        hidden: Some(*hidden),
                    },
                });
            }
            Edit::OutlineColumns {
                sheet,
                columns: spec,
                level,
                hidden,
            } => {
                let s = sheet_ix(&wb, sheet)?;
                let (first, last) = columns(spec).map_err(fail)?;
                if *level > 7 {
                    return Err(fail("level 0–7".into()));
                }
                cols_ops.push(PendingCols {
                    sheet: wb.sheet_name(s).map_err(|e| fail(e.to_string()))?,
                    first,
                    last,
                    after: shifts.len(),
                    op: ColOp::Outline {
                        first,
                        last,
                        level: *level,
                        hidden: *hidden,
                    },
                });
            }
            Edit::Fill {
                sheet,
                area,
                rgb: color,
            }
            | Edit::Border {
                sheet,
                area,
                rgb: color,
                ..
            } => {
                let s = sheet_ix(&wb, sheet)?;
                let ar = Area::parse(area).map_err(|e| fail(e.to_string()))?;
                if ar.rows() as i64 * ar.columns() as i64 > 1_000_000 {
                    return Err(fail("more than a million cells".into()));
                }
                let color = rgb(color).map_err(fail)?;
                let op = match edit {
                    Edit::Border { style, .. } => StyleOp::Border {
                        rgb: color,
                        style: *style,
                    },
                    _ => StyleOp::Fill { rgb: color },
                };
                let canonical = wb.sheet_name(s).map_err(|e| fail(e.to_string()))?;
                for row in ar.first.row..=ar.last.row {
                    for col in ar.first.column..=ar.last.column {
                        writes.push(PendingWrite {
                            sheet: canonical.clone(),
                            row,
                            col,
                            after: shifts.len(),
                            content: None,
                            style: vec![op],
                        });
                    }
                }
            }
        }
        report.outcomes.push(outcome);
    }

    if shifts.iter().any(|_| true) && pkg.names.iter().any(|n| n.starts_with("xl/pivotTables/")) {
        return Err(PatchError::Unsupported(
            "rows/columns moved in a workbook with pivot tables (their caches and locations are not rippled yet)".into(),
        ));
    }

    fixed.reference();
    wb.recalc();
    // what is unstable after the edits, by final position, measured as before them
    let unstable_after = {
        let values = Before::snapshot(&wb);
        Before::unstable(&mut wb, &values, &fixed)
    };

    // ---- the plan at final positions -------------------------------------
    let order: Vec<String> = wb.sheet_names();
    let map_pos = |sheet: &str, after: usize, mut row: i32, mut col: i32| -> Option<(i32, i32)> {
        for (s, sh) in &shifts[after..] {
            if s.eq_ignore_ascii_case(sheet) {
                if sh.is_rows() {
                    row = sh.index(row)?;
                } else {
                    col = sh.index(col)?;
                }
            }
        }
        Some((row, col))
    };
    let mut final_writes: BTreeMap<String, BTreeMap<(i32, i32), CellWrite>> = BTreeMap::new();
    // a cell several edits wrote holds the last one's content: only that one is computed (an
    // earlier formula's text is gone from the model)
    let mut last_content: BTreeMap<(String, i32, i32), usize> = BTreeMap::new();
    for (i, w) in writes.iter().enumerate() {
        if w.content.is_some()
            && let Some((row, col)) = map_pos(&w.sheet, w.after, w.row, w.col)
        {
            last_content.insert((w.sheet.to_lowercase(), row, col), i);
        }
    }
    for (index, w) in writes.iter().enumerate() {
        let Some((row, col)) = map_pos(&w.sheet, w.after, w.row, w.col) else {
            continue;
        };
        let entry = final_writes
            .entry(w.sheet.clone())
            .or_default()
            .entry((row, col))
            .or_default();
        entry.style.extend(w.style.iter().copied());
        let last = last_content.get(&(w.sheet.to_lowercase(), row, col)) == Some(&index);
        if let Some(c) = w.content.as_ref().filter(|_| last) {
            let s = wb
                .sheet_index(&w.sheet)
                .map_err(|e| PatchError::Malformed(e.to_string()))?;
            let a = Addr::new(row, col);
            let computes_now = wb.model().workbook.worksheet(s).is_ok_and(|ws| computed_at(ws, (row, col)));
            entry.content = Some(match c {
                // a cell an edit wrote that an array spills into now (the edit cleared what
                // blocked it): the array's value, as a spilled cell
                ContentKind::Value(_) if computes_now => {
                    NewContent::Spilled(wb.value_now(s, a).map_err(|e| PatchError::Malformed(e.to_string()))?)
                }
                ContentKind::Value(v) => NewContent::Value(v.clone()),
                ContentKind::Formula => {
                    if let Ok(Content::DynamicFormula(_) | Content::ArrayFormula { .. }) =
                        wb.content(s, a)
                    {
                        let spill = wb
                            .model()
                            .workbook
                            .worksheet(s)
                            .ok()
                            .and_then(|ws| ws.cell(row, col))
                            .and_then(|c| match c.as_ref() {
                                Cell::ArrayFormula { r, .. } => Some(*r),
                                _ => None,
                            });
                        if spill.is_some_and(|r| r != (1, 1)) {
                            return Err(PatchError::Unsupported(format!(
                                "{}!{a}: a formula that spills over several cells (dynamic arrays are not patched yet)",
                                w.sheet
                            )));
                        }
                    }
                    // a formula the engine evaluates as an array is written as Excel 365 writes
                    // one: an array formula of its one cell, marked dynamic (its `cm` set once the
                    // metadata part is known); a plain `<f>` would be read with an implicit
                    // intersection (`SUM(A1:A3*B1:B3)` reading 10, not 140)
                    let dynamic = matches!(wb.content(s, a), Ok(Content::DynamicFormula(_)))
                        || formula_node(&wb, s, a).is_some_and(|n| array_under_an_operator(&n));
                    let text = file_formula(&wb, s, a, dynamic).ok_or_else(|| {
                        PatchError::Malformed(format!(
                            "{}!{a}: the formula vanished from the model",
                            w.sheet
                        ))
                    })?;
                    let cached = wb
                        .value_now(s, a)
                        .map_err(|e| PatchError::Malformed(e.to_string()))?;
                    NewContent::Formula { text, cached, cm: dynamic.then_some(0) }
                }
            });
        }
    }
    // a cache the writes would hold that no workbook file holds refuses the patch (see
    // `sheet::engine_only`); the refresh of a moved cache refuses it the same way
    for (name, cells) in &final_writes {
        for (&(row, col), w) in cells {
            if let Some(NewContent::Formula { cached: v, .. } | NewContent::Spilled(v)) = &w.content
                && let Some(what) = sheet::engine_only(v)
            {
                return Err(sheet::engine_only_refusal(name, &Addr::new(row, col).to_string(), what));
            }
        }
    }
    // the cells an array spills into that the file has no cell for: written as Excel writes a
    // spilled cell where the array reaches them because of the edits (grown, moved into empty
    // cells), left out where it reached them already and their value did not move; and every cell
    // the engine computes whose value the edits moved, with that value, where the file holds it:
    // what verification must read back from the written caches
    let mut expected: Vec<(String, i32, i32, Value)> = Vec::new();
    let spills_now = wb.model().workbook.worksheets.iter().any(|ws| {
        ws.sheet_data.cells().any(|(_, _, c)| matches!(&*c, Cell::SpillCell { .. }))
    });
    // before the edits, a computed cell that is no formula is a spilled one
    let spills_before = before
        .values
        .iter()
        .zip(wb.model().workbook.worksheets.iter())
        .any(|(v, ws)| v.len() > ws.sheet_data.cells().filter(|(_, _, c)| c.get_formula().is_some()).count());
    let in_file: Option<BTreeMap<String, std::collections::HashSet<(i32, i32)>>> = if spills_now || spills_before {
        Some(
            cached::read_elements(&fingerprint)?
                .into_iter()
                .map(|(name, _, cells)| {
                    let at = cells.keys().filter_map(|&(r, c)| map_pos(&name, 0, r, c)).collect();
                    (name.to_lowercase(), at)
                })
                .collect(),
        )
    } else {
        None
    };
    let holds_cell = |name: &str, at: (i32, i32)| {
        in_file.as_ref().is_none_or(|f| f.get(&name.to_lowercase()).is_some_and(|s| s.contains(&at)))
    };
    // exempt: unstable before the edits and, where it is now, after them
    let unstable = |i: usize, orig: &(i32, i32), now: (i32, i32)| {
        before.unstable.get(i).is_some_and(|u| u.contains(orig)) && unstable_after.get(i).is_some_and(|u| u.contains(&now))
    };
    if spills_now {
        for (i, ws) in wb.model().workbook.worksheets.iter().enumerate() {
            let name = order.get(i).cloned().unwrap_or_default();
            // what the engine computed before the edits, by where it is now
            let was: std::collections::HashMap<(i32, i32), ((i32, i32), Value)> = before
                .values
                .get(i)
                .map(|m| m.iter().filter_map(|(o, v)| map_pos(&name, 0, o.0, o.1).map(|p| (p, (*o, v.clone())))).collect())
                .unwrap_or_default();
            let covered: Vec<(i32, i32)> = ws
                .sheet_data
                .cells()
                .filter(|(_, _, c)| matches!(&**c, Cell::SpillCell { .. }))
                .map(|(r, c, _)| (r, c))
                .filter(|p| computed_at(ws, *p))
                .collect();
            for (row, col) in covered {
                let v = wb.value_now(i as u32, Addr::new(row, col)).map_err(|e| PatchError::Malformed(e.to_string()))?;
                if holds_cell(&name, (row, col)) {
                    // a cell the file holds that the engine did not compute before (an empty
                    // element the array grew into): the rewrite writes it, verification expects it
                    if !was.contains_key(&(row, col)) {
                        expected.push((name.clone(), row, col, v));
                    }
                    continue;
                }
                if let Some((orig, then)) = was.get(&(row, col))
                    && (same(then, &v) || unstable(i, orig, (row, col)))
                {
                    continue;
                }
                let entry = final_writes.entry(name.clone()).or_default().entry((row, col)).or_default();
                if entry.content.is_none() {
                    if let Some(what) = sheet::engine_only(&v) {
                        return Err(sheet::engine_only_refusal(&name, &Addr::new(row, col).to_string(), what));
                    }
                    entry.content = Some(NewContent::Spilled(v.clone()));
                    expected.push((name.clone(), row, col, v));
                }
            }
        }
    }
    let computes = |i: usize, at: (i32, i32)| wb.model().workbook.worksheet(i as u32).is_ok_and(|ws| computed_at(ws, at));
    // a cell an edit wrote a value into that the engine computes now (an array spilling into it)
    // must say what the engine says
    for (name, cells) in &final_writes {
        let Some(i) = order.iter().position(|o| o.eq_ignore_ascii_case(name)) else { continue };
        for (&(row, col), w) in cells {
            if matches!(w.content, Some(NewContent::Value(_))) && computes(i, (row, col)) {
                let live = wb.value_now(i as u32, Addr::new(row, col)).unwrap_or(Value::Empty);
                expected.push((name.clone(), row, col, live));
            }
        }
    }
    for (i, values) in before.values.iter().enumerate() {
        let Some(name) = order.get(i) else { continue };
        for (orig, was) in values {
            if map_pos(name, 0, orig.0, orig.1).is_some_and(|p| unstable(i, orig, p) && computes(i, p)) {
                continue;
            }
            let Some((row, col)) = map_pos(name, 0, orig.0, orig.1) else { continue };
            if !holds_cell(name, (row, col))
                || final_writes.get(name).is_some_and(|w| w.get(&(row, col)).is_some_and(|w| w.content.is_some()))
            {
                continue;
            }
            let live = wb.value_now(i as u32, Addr::new(row, col)).unwrap_or(Value::Empty);
            if !same(was, &live) {
                expected.push((name.clone(), row, col, live));
            }
        }
    }

    let mut final_rows: BTreeMap<String, BTreeMap<i32, RowOp>> = BTreeMap::new();
    for r in &rows_ops {
        for row in r.first..=r.last {
            if let Some((nr, _)) = map_pos(&r.sheet, r.after, row, 1) {
                final_rows
                    .entry(r.sheet.clone())
                    .or_default()
                    .insert(nr, r.op);
            }
        }
    }
    let mut final_cols: BTreeMap<String, Vec<ColOp>> = BTreeMap::new();
    for c in &cols_ops {
        // the range as Excel moves one: what survives of it, grown by an insertion inside
        let mut span = Some((c.first, c.last));
        for (sheet, sh) in &shifts[c.after..] {
            if sheet.eq_ignore_ascii_case(&c.sheet) && !sh.is_rows() {
                span = span.and_then(|(a, b)| sh.span(a, b));
            }
        }
        let Some((a, b)) = span else {
            continue;
        };
        let op = match c.op {
            ColOp::Width { width, .. } => ColOp::Width {
                first: a,
                last: b,
                width,
            },
            ColOp::Outline { level, hidden, .. } => ColOp::Outline {
                first: a,
                last: b,
                level,
                hidden,
            },
        };
        final_cols.entry(c.sheet.clone()).or_default().push(op);
    }

    // ---- the parts -------------------------------------------------------
    let mut replaced: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut removed: Vec<String> = Vec::new();
    let mut added: Vec<(String, Vec<u8>)> = Vec::new();

    let wb_part = "xl/workbook.xml".to_string();
    let wb_rels_part = package::rels_of(&wb_part);
    let mut workbook_xml = pkg.read_text(&wb_part)?;
    let mut wb_rels_xml = pkg.read_text(&wb_rels_part)?;
    let mut ct_xml = pkg.read_text("[Content_Types].xml")?;
    let wb_rels = package::rels(&wb_part, &wb_rels_xml);
    let entries = package::sheets(&workbook_xml, &wb_rels)?;
    // The styles part. OOXML lets a package have none (every cell is then
    // format 0): such a package is patched without one, and a minimal part is
    // synthesized — with its relationship and content type — only when an
    // edit creates a format (a fill, a border).
    let (styles_part, mut styles, styles_synthesized) =
        match wb_rels.iter().find(|r| r.kind == "styles") {
            Some(r) => (
                r.target.clone(),
                styles::StyleBook::new(pkg.read_text(&r.target)?)
                    .map_err(|e| PatchError::Malformed(e.to_string()))?,
                false,
            ),
            None => {
                let mut n = 0;
                let mut part = "xl/styles.xml".to_string();
                while pkg.has(&part) {
                    n += 1;
                    part = format!("xl/styles{n}.xml");
                }
                (
                    part,
                    styles::StyleBook::new(styles::MINIMAL.to_string())
                        .map_err(|e| PatchError::Malformed(e.to_string()))?,
                    true,
                )
            }
        };

    let empty_writes = BTreeMap::new();
    let empty_rows = BTreeMap::new();
    let mut formulas_removed = 0;
    let any_formula_write = final_writes.values().any(|m| {
        m.values()
            .any(|w| matches!(w.content, Some(NewContent::Formula { .. })))
    });

    let mut sheet_jobs: Vec<(String, String, Option<String>)> = Vec::new(); // (name, part, xml when new)
    // worksheets only: a chart sheet holds no cells, and its part is left as it is
    for e in entries.iter().filter(|e| e.is_worksheet()) {
        sheet_jobs.push((e.name.clone(), e.part.clone(), None));
    }
    // new sheets: parts, rels, content types, <sheet> entries
    let mut next_sheet_id = entries.iter().map(|e| e.sheet_id).max().unwrap_or(0);
    let mut rels_now = wb_rels.clone();
    for name in &new_sheets {
        let mut n = 1;
        // part names are told apart without case (the package's rule)
        while pkg.names.iter().any(|p| p.eq_ignore_ascii_case(&format!("xl/worksheets/sheet{n}.xml")))
            || sheet_jobs
                .iter()
                .any(|(_, p, _)| p.eq_ignore_ascii_case(&format!("xl/worksheets/sheet{n}.xml")))
        {
            n += 1;
        }
        let part = format!("xl/worksheets/sheet{n}.xml");
        let rid = package::free_rid(&rels_now);
        rels_now.push(package::Rel {
            id: rid.clone(),
            kind: "worksheet".into(),
            target: part.clone(),
            external: false,
        });
        wb_rels_xml = package::add_rel(
            &wb_rels_xml,
            &rid,
            NS_WORKSHEET,
            &format!("worksheets/sheet{n}.xml"),
        );
        ct_xml = package::add_override(&ct_xml, &part, CT_WORKSHEET);
        next_sheet_id += 1;
        let prefix = rid_prefix(&workbook_xml);
        let tag = format!(
            "<sheet name=\"{}\" sheetId=\"{next_sheet_id}\" {prefix}:id=\"{rid}\"/>",
            xml::escape_attr(name)
        );
        let close = workbook_xml
            .find("</sheets>")
            .ok_or_else(|| PatchError::Malformed("workbook.xml: no </sheets>".into()))?;
        workbook_xml.insert_str(close, &tag);
        let blank = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><dimension ref=\"A1\"/><sheetViews><sheetView workbookViewId=\"0\"/></sheetViews><sheetFormatPr defaultRowHeight=\"15\"/><sheetData/><pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/></worksheet>";
        sheet_jobs.push((name.clone(), part, Some(blank.to_string())));
    }

    // ---- the dynamic-array metadata a written array formula points at (`cm`) ----
    if final_writes.values().flat_map(|m| m.values()).any(|w| matches!(w.content, Some(NewContent::Formula { cm: Some(_), .. }))) {
        let cm = match wb_rels.iter().find(|r| r.kind == "sheetMetadata") {
            Some(r) => {
                let part = pkg.read_text(&r.target)?;
                match dynamic_cell_metadata(&part) {
                    Some(cm) => cm,
                    None => {
                        // its rich values kept; the dynamic-array entry appended
                        let (grown, cm) = add_dynamic_entry(&part).ok_or_else(|| {
                            PatchError::Malformed("the workbook's metadata part has no <metadata> to add a dynamic-array entry to".into())
                        })?;
                        replaced.insert(r.target.clone(), grown.into_bytes());
                        cm
                    }
                }
            }
            None => {
                let mut n = 0;
                let mut part = "xl/metadata.xml".to_string();
                while pkg.has(&part) {
                    n += 1;
                    part = format!("xl/metadata{n}.xml");
                }
                let rid = package::free_rid(&rels_now);
                let target = part.trim_start_matches("xl/").to_string();
                rels_now.push(package::Rel { id: rid.clone(), kind: "sheetMetadata".into(), target: part.clone(), external: false });
                wb_rels_xml = package::add_rel(&wb_rels_xml, &rid, NS_METADATA, &target);
                ct_xml = package::add_override(&ct_xml, &part, CT_METADATA);
                added.push((part, METADATA.as_bytes().to_vec()));
                1
            }
        };
        for w in final_writes.values_mut().flat_map(|m| m.values_mut()) {
            if let Some(NewContent::Formula { cm: Some(c), .. }) = &mut w.content {
                *c = cm;
            }
        }
    }

    for (name, part, fresh) in &sheet_jobs {
        let sheet_index = wb
            .sheet_index(name)
            .map_err(|e| PatchError::Malformed(e.to_string()))?;
        let text = match fresh {
            Some(t) => t.clone(),
            None => pkg.read_text(part)?,
        };
        let value_at = |row: i32, col: i32| -> Value {
            wb.value_now(sheet_index, Addr::new(row, col))
                .unwrap_or(Value::Empty)
        };
        let moved = |orig: (i32, i32), new: (i32, i32), live: &Value| -> bool {
            let unstable_now = unstable_after.get(sheet_index as usize).is_some_and(|u| u.contains(&new));
            fresh.is_none() && before.moved(sheet_index, orig, live, unstable_now)
        };
        let extents_now = wb
            .model()
            .workbook
            .worksheet(sheet_index)
            .map(array_extents)
            .unwrap_or_default();
        let array_extent = |orig: (i32, i32), new: (i32, i32)| -> Option<(Option<(i32, i32)>, (i32, i32))> {
            let now = *extents_now.get(&new)?;
            let then = if fresh.is_some() {
                None
            } else {
                before.extents.get(sheet_index as usize).and_then(|e| e.get(&orig)).copied()
            };
            Some((then, now))
        };
        let key = order
            .iter()
            .find(|n| n.eq_ignore_ascii_case(name))
            .cloned()
            .unwrap_or_else(|| name.clone());
        let left = |orig: (i32, i32), new: (i32, i32)| -> bool {
            fresh.is_none()
                && before.values.get(sheet_index as usize).is_some_and(|v| v.contains_key(&orig))
                && !wb.model().workbook.worksheet(sheet_index).is_ok_and(|ws| computed_at(ws, new))
        };
        let reached = |orig: (i32, i32), new: (i32, i32)| -> bool {
            fresh.is_none()
                && !before.values.get(sheet_index as usize).is_some_and(|v| v.contains_key(&orig))
                && wb.model().workbook.worksheet(sheet_index).is_ok_and(|ws| {
                    computed_at(ws, new) && matches!(ws.cell(new.0, new.1).map(|c| c.as_ref().clone()), Some(Cell::SpillCell { .. }))
                })
        };
        let job = SheetJob {
            name: &key,
            order: &order,
            shifts: &shifts,
            writes: final_writes.get(&key).unwrap_or(&empty_writes),
            row_ops: final_rows.get(&key).unwrap_or(&empty_rows),
            col_ops: final_cols.get(&key).map(|v| v.as_slice()).unwrap_or(&[]),
            value_at: &value_at,
            moved: &moved,
            array_extent: &array_extent,
            left: &left,
            reached: &reached,
        };
        let out = sheet::rewrite(&text, &job, &mut styles)?;
        report.refreshed += out.refreshed;
        report.expanded += out.expanded;
        report.rows_rewritten += out.rows_rewritten;
        formulas_removed += out.formulas_removed;
        match (fresh, out.xml) {
            (Some(_), Some(x)) => added.push((part.clone(), x.into_bytes())),
            (Some(t), None) => added.push((part.clone(), t.clone().into_bytes())),
            (None, Some(x)) => {
                replaced.insert(part.clone(), x.into_bytes());
            }
            (None, None) => {}
        }

        // parts that point into this sheet's grid
        let own: Vec<Shift> = shifts
            .iter()
            .filter(|(s, _)| s.eq_ignore_ascii_case(name))
            .map(|(_, s)| *s)
            .collect();
        if fresh.is_none() && !shifts.is_empty() {
            let rels_part = package::rels_of(part);
            if pkg.has(&rels_part) {
                let rels_xml = pkg.read_text(&rels_part)?;
                for r in package::rels(part, &rels_xml) {
                    let handled = [
                        "comments",
                        "threadedComment",
                        "vmlDrawing",
                        "drawing",
                        "table",
                        "ctrlProp",
                    ];
                    if r.external || !pkg.has(&r.target) || !handled.contains(&r.kind.as_str()) {
                        continue;
                    }
                    let content = pkg.read_text(&r.target)?;
                    let new = match r.kind.as_str() {
                        "comments" if !own.is_empty() => {
                            package::shift_comments(&content, &own, "comment").0
                        }
                        "threadedComment" if !own.is_empty() => {
                            package::shift_comments(&content, &own, "threadedComment").0
                        }
                        "vmlDrawing" => {
                            // notes and controls follow this sheet's moves; a control's link and
                            // list range (the VML copy of its ctrlProp) any move
                            let anchored = if own.is_empty() { None } else { package::shift_vml(&content, &own) };
                            let mut base = anchored.clone().unwrap_or(content.clone());
                            let mut linked = false;
                            for element in ["x:FmlaLink", "x:FmlaRange", "x:FmlaTxbx"] {
                                if let Some(n) = package::shift_formula_elements(&base, element, &shifts, Some(name), &order) {
                                    base = n;
                                    linked = true;
                                }
                            }
                            if linked { Some(base) } else { anchored }
                        }
                        "drawing" => {
                            // charts hang off the drawing; their series formulas
                            // may point at any moved sheet
                            shift_charts(&mut pkg, &r.target, &shifts, &order, &mut replaced)?;
                            // the anchors follow this sheet's moves; a shape's text link any move
                            let anchored = if own.is_empty() { None } else { package::shift_drawing(&content, &own) };
                            let base = anchored.clone().unwrap_or(content.clone());
                            package::shift_textlinks(&base, &shifts, Some(name), &order).or(anchored)
                        }
                        // a form control's linked cell and its list's range
                        "ctrlProp" => package::shift_link_attributes(&content, &["fmlaLink", "fmlaRange", "fmlaTxbx"], &shifts, Some(name), &order),
                        "table" => {
                            let t = if own.is_empty() {
                                None
                            } else {
                                package::shift_table(&content, &own)?
                            };
                            let base = t.clone().unwrap_or(content.clone());
                            // a column's formula and its totals row's
                            let calculated = package::shift_formula_elements(
                                &base,
                                "calculatedColumnFormula",
                                &shifts,
                                Some(name),
                                &order,
                            );
                            let base = calculated.clone().unwrap_or(base);
                            package::shift_formula_elements(&base, "totalsRowFormula", &shifts, Some(name), &order)
                                .or(calculated)
                                .or(t)
                        }
                        _ => None,
                    };
                    if let Some(n) = new {
                        replaced.insert(r.target.clone(), n.into_bytes());
                    }
                }
            }
        }
    }

    // a chart sheet's part is left as it is, but the chart on it reads the grid like any other:
    // its series formulas follow the moves
    if !shifts.is_empty() {
        for e in entries.iter().filter(|e| !e.is_worksheet()) {
            let rels_part = package::rels_of(&e.part);
            if !pkg.has(&rels_part) {
                continue;
            }
            let rels_xml = pkg.read_text(&rels_part)?;
            for r in package::rels(&e.part, &rels_xml) {
                if r.kind == "drawing" && !r.external && pkg.has(&r.target) {
                    shift_charts(&mut pkg, &r.target, &shifts, &order, &mut replaced)?;
                    // a shape's text link on the chart sheet (its references name their sheet)
                    let drawing = pkg.read_text(&r.target)?;
                    if let Some(n) = package::shift_textlinks(&drawing, &shifts, None, &order) {
                        replaced.insert(r.target.clone(), n.into_bytes());
                    }
                }
            }
        }
    }

    // ---- workbook.xml: defined names, calcPr ------------------------------
    if !shifts.is_empty() || names_touched {
        workbook_xml =
            rewrite_defined_names(&workbook_xml, &wb, &shifts, &order, &entries, &set_names, &deleted_names)?;
    }
    if options.full_calc_on_load
        && let Some(t) = xml::find_open(&workbook_xml, "calcPr", 0) {
            let n = xml::set_attr(t.text, "fullCalcOnLoad", "1");
            workbook_xml = format!(
                "{}{}{}",
                &workbook_xml[..t.start],
                n,
                &workbook_xml[t.end..]
            );
        }

    // ---- the calculation chain --------------------------------------------
    // It lists formula cells by address: moved, added or removed formulas make
    // it lie, and a lying chain is a repair on open. Excel rebuilds it.
    if let Some(chain) = wb_rels.iter().find(|r| r.kind == "calcChain")
        && (!shifts.is_empty() || formulas_removed > 0 || any_formula_write || report.expanded > 0) {
            removed.push(chain.target.clone());
            wb_rels_xml = package::remove_rel_to(&wb_part, &wb_rels_xml, &chain.target);
            ct_xml = package::remove_override(&ct_xml, &chain.target);
            report.calc_chain_dropped = true;
        }

    // ---- styles -----------------------------------------------------------
    if let Some(x) = styles
        .finish()
        .map_err(|e| PatchError::Malformed(e.to_string()))?
    {
        if styles_synthesized {
            let rid = package::free_rid(&rels_now);
            let target = styles_part.trim_start_matches("xl/").to_string();
            wb_rels_xml = package::add_rel(&wb_rels_xml, &rid, NS_STYLES, &target);
            ct_xml = package::add_override(&ct_xml, &styles_part, CT_STYLES);
            added.push((styles_part, x.into_bytes()));
        } else {
            replaced.insert(styles_part, x.into_bytes());
        }
    }

    // the small parts, only when they changed
    for (part, text) in [
        (wb_part.clone(), workbook_xml),
        (wb_rels_part.clone(), wb_rels_xml),
        ("[Content_Types].xml".to_string(), ct_xml),
    ] {
        let before = pkg.read(&part)?;
        if before != text.as_bytes() {
            replaced.insert(part, text.into_bytes());
        }
    }
    // a "rewritten" part with the same bytes is not rewritten
    let mut identical_rewrites = Vec::new();
    for (name, bytes) in &replaced {
        if pkg.has(name) && pkg.read(name)? == *bytes {
            identical_rewrites.push(name.clone());
        }
    }
    for n in identical_rewrites {
        replaced.remove(&n);
    }

    for n in &pkg.names {
        let fate = if removed.contains(n) {
            Fate::Removed
        } else if replaced.contains_key(n) {
            Fate::Rewritten
        } else {
            Fate::Identical
        };
        report.parts.push(PartFate {
            name: n.clone(),
            fate,
        });
    }
    for (n, _) in &added {
        report.parts.push(PartFate {
            name: n.clone(),
            fate: Fate::Added,
        });
    }

    let out = pkg.write(&replaced, &removed, &added)?;

    // ---- written beside the destination, verified there, then renamed ---------
    commit(source, &fingerprint, dest, &out, |written| {
        if options.verify {
            report.verified_cells = verify(written, &mut wb, &fixed, &expected)?;
        }
        Ok(())
    })?;
    Ok(report)
}

/// Writes `bytes` as `dest`'s next version. `check` runs first on a copy of them in a folder of its
/// own under the system's temporary folder (a short UTF-8 path, the engine's loader taking no other
/// whatever `dest`'s folder and name); then, the source still holding `fingerprint`, the bytes go
/// into a temporary file of their own beside `dest` (`.veredas-<process>-<n>.tmp`, made new), a
/// `dest` already there is kept as `<name>.bak` (its name as the system gives it) and the new file
/// is renamed over it. A failing check, a source changed since it was read, or any failure after
/// leaves `dest` as it was and nothing beside it.
fn commit(
    source: &Path,
    fingerprint: &[u8],
    dest: &Path,
    bytes: &[u8],
    check: impl FnOnce(&Path) -> Result<(), PatchError>,
) -> Result<(), PatchError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let next = || format!("{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    let dir = dest.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let file_name = dest.file_name().ok_or_else(|| PatchError::Malformed(format!("{}: not a file path", dest.display())))?;
    // the check, on a copy at a short UTF-8 path
    let probe = loop {
        let probe = std::env::temp_dir().join(format!("veredas-patch-{}", next()));
        match std::fs::create_dir(&probe) {
            Ok(()) => break probe,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io_err(&probe, e)),
        }
    };
    let copy = probe.join("patched.xlsx");
    let checked = std::fs::write(&copy, bytes).map_err(|e| io_err(&copy, e)).and_then(|()| check(&copy));
    let _ = std::fs::remove_dir_all(&probe);
    checked?;
    if crate::files::contents(source).map_err(|e| io_err(source, e))? != fingerprint {
        return Err(PatchError::SourceMoved(source.to_path_buf()));
    }
    // then beside the destination
    let (tmp, mut file) = loop {
        let tmp = dir.join(format!(".veredas-{}.tmp", next()));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io_err(&tmp, e)),
        }
    };
    let placed = (|| {
        use std::io::Write as _;
        file.write_all(bytes).and_then(|()| file.sync_all()).map_err(|e| io_err(&tmp, e))?;
        drop(file);
        // the destination's previous version kept as `.bak` (a previous `.bak` replaced, read-only
        // or not), the destination's own permissions kept on its new version
        let permissions = dest.exists().then(|| std::fs::metadata(dest).map(|m| m.permissions())).transpose().map_err(|e| io_err(dest, e))?;
        if dest.exists() {
            let mut bak = file_name.to_os_string();
            bak.push(".bak");
            let bak = dir.join(bak);
            if bak.exists() {
                std::fs::remove_file(&bak).map_err(|e| io_err(&bak, e))?;
            }
            std::fs::copy(dest, &bak).map_err(|e| io_err(&bak, e))?;
        }
        std::fs::rename(&tmp, dest).map_err(|e| io_err(dest, e))?;
        if let Some(p) = permissions {
            std::fs::set_permissions(dest, p).map_err(|e| io_err(dest, e))?;
        }
        Ok(())
    })();
    if placed.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    placed
}

/// The cell metadata entry (its 1-based index, a cell's `cm`) of a metadata part that marks a
/// formula a dynamic array: the entry whose record points at the `XLDAPR` type's future metadata
/// with `fDynamic="1"`.
fn dynamic_cell_metadata(part: &str) -> Option<u32> {
    let bks = |s: &str| -> Vec<String> {
        let mut out = Vec::new();
        let mut pos = 0;
        while let Some(t) = xml::find_open(s, "bk", pos) {
            let Some((a, b)) = xml::element_span(s, &t) else { break };
            out.push(s[a..b].to_string());
            pos = b;
        }
        out
    };
    let types = xml::inner(part, "metadataTypes")?;
    let mut pos = 0;
    let mut index = 0;
    let mut type_index = None;
    while let Some(t) = xml::find_open(types, "metadataType", pos) {
        index += 1;
        if xml::attr(t.text, "name").as_deref() == Some("XLDAPR") {
            type_index = Some(index);
        }
        pos = t.end;
    }
    let type_index = type_index?;
    let mut pos = 0;
    let mut value_index = None;
    while let Some(t) = xml::find_open(part, "futureMetadata", pos) {
        let (a, b) = xml::element_span(part, &t)?;
        if xml::attr(t.text, "name").as_deref() == Some("XLDAPR") {
            // `fDynamic` is an xsd:boolean: "1" or "true", read as XML reads the attribute
            let dynamic = |bk: &str| {
                let mut at = 0;
                while let Some(t) = xml::next_tag(bk, at) {
                    at = t.end;
                    if !t.closing && t.name == "xda:dynamicArrayProperties" {
                        return matches!(xml::attr(t.text, "fDynamic").as_deref().map(str::trim), Some("1" | "true"));
                    }
                }
                false
            };
            value_index = bks(&part[a..b]).iter().position(|bk| dynamic(bk));
        }
        pos = b;
    }
    let value_index = value_index?;
    let cells = xml::inner(part, "cellMetadata")?;
    bks(cells)
        .iter()
        .position(|bk| {
            xml::find_open(bk, "rc", 0).is_some_and(|rc| {
                xml::attr(rc.text, "t").as_deref() == Some(type_index.to_string().as_str())
                    && xml::attr(rc.text, "v").as_deref() == Some(value_index.to_string().as_str())
            })
        })
        .map(|i| i as u32 + 1)
}

/// A metadata part with the dynamic-array entry appended (its type, its future metadata, a cell
/// entry pointing at them, each list's `count` kept true), and that cell entry's `cm`.
fn add_dynamic_entry(part: &str) -> Option<(String, u32)> {
    let count_of = |s: &str, list: &str, item: &str| -> usize {
        xml::inner(s, list).map(|inner| {
            let mut n = 0;
            let mut pos = 0;
            while let Some(t) = xml::find_open(inner, item, pos) {
                n += 1;
                pos = xml::element_span(inner, &t).map(|(_, b)| b).unwrap_or(t.end);
            }
            n
        }).unwrap_or(0)
    };
    // append `item` inside `list` (made before `before` when it does not exist), its count set
    let append = |s: &str, list: &str, attrs: &str, item: &str, before: &[&str]| -> Option<String> {
        // a new future-metadata list (one per name) holds its one entry
        let n = if list == "futureMetadata" { 1 } else { count_of(s, list, if list == "metadataTypes" { "metadataType" } else { "bk" }) + 1 };
        match xml::find_open(s, list, 0).filter(|_| list != "futureMetadata") {
            Some(open) => {
                let (_, b) = xml::element_span(s, &open)?;
                let close = b - format!("</{list}>").len();
                let tag = xml::set_attr(open.text, "count", &n.to_string());
                Some(format!("{}{tag}{}{item}{}", &s[..open.start], &s[open.end..close], &s[close..]))
            }
            None => {
                // before the first of the root's own children that must follow it (an `extLst`
                // nested in a rich value's entry is not one of them)
                let root = xml::find_open(s, "metadata", 0)?;
                let mut at = xml::find_close(s, &root)?.start;
                let mut pos = root.end;
                while let Some(t) = xml::next_tag(s, pos).filter(|t| !t.closing && t.start < at) {
                    if before.contains(&t.name) {
                        at = t.start;
                        break;
                    }
                    pos = xml::element_span(s, &t)?.1;
                }
                Some(format!("{}<{list}{attrs} count=\"{n}\">{item}</{list}>{}", &s[..at], &s[at..]))
            }
        }
    };
    let mut s = part.to_string();
    if !s.contains("xmlns:xda=") {
        let root = xml::find_open(&s, "metadata", 0)?;
        let tag = xml::set_attr(root.text, "xmlns:xda", "http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray");
        s = format!("{}{tag}{}", &s[..root.start], &s[root.end..]);
    }
    s = append(
        &s,
        "metadataTypes",
        "",
        r#"<metadataType name="XLDAPR" minSupportedVersion="120000" copy="1" pasteAll="1" pasteValues="1" merge="1" splitFirst="1" rowColShift="1" clearFormats="1" clearComments="1" assign="1" coerce="1" cellMeta="1"/>"#,
        &["metadataStrings", "mdxMetadata", "futureMetadata", "cellMetadata", "valueMetadata", "extLst"],
    )?;
    let type_index = count_of(&s, "metadataTypes", "metadataType");
    s = append(
        &s,
        "futureMetadata",
        r#" name="XLDAPR""#,
        r#"<bk><extLst><ext uri="{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}"><xda:dynamicArrayProperties fDynamic="1" fCollapsed="0"/></ext></extLst></bk>"#,
        &["cellMetadata", "valueMetadata", "extLst"],
    )?;
    s = append(&s, "cellMetadata", "", &format!(r#"<bk><rc t="{type_index}" v="0"/></bk>"#), &["valueMetadata", "extLst"])?;
    let cm = count_of(&s, "cellMetadata", "bk") as u32;
    Some((s, cm))
}

/// The charts a drawing part holds, their series formulas (`<c:f>`) moved with the grid; each
/// chart rewritten once, whichever drawings reach it.
fn shift_charts(
    pkg: &mut package::Package,
    drawing: &str,
    shifts: &[(String, Shift)],
    order: &[String],
    replaced: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), PatchError> {
    let drels = package::rels_of(drawing);
    if !pkg.has(&drels) {
        return Ok(());
    }
    let dx = pkg.read_text(&drels)?;
    for c in package::rels(drawing, &dx) {
        // a chart's series formulas are `c:f`, a chartEx's (Excel 2016's charts) `cx:f`
        let element = match c.kind.as_str() {
            "chart" => "c:f",
            "chartEx" => "cx:f",
            _ => continue,
        };
        if !c.external && pkg.has(&c.target) && !replaced.contains_key(&c.target) {
            let original = pkg.read_text(&c.target)?;
            // its series formulas, and Excel 2013's references beside them: a data label's range
            // (`c15:f`) and a series' cells (`c15:sqref`)
            let mut cx = original.clone();
            for name in [element, "c15:f", "c15:sqref"] {
                if let Some(n) = package::shift_formula_elements(&cx, name, shifts, None, order) {
                    cx = n;
                }
            }
            if cx != original {
                replaced.insert(c.target.clone(), cx.into_bytes());
            }
            // shapes drawn on the chart: their text links
            let crels = package::rels_of(&c.target);
            if pkg.has(&crels) {
                let cr = pkg.read_text(&crels)?;
                for u in package::rels(&c.target, &cr) {
                    if u.kind == "chartUserShapes" && !u.external && pkg.has(&u.target) && !replaced.contains_key(&u.target) {
                        let shapes = pkg.read_text(&u.target)?;
                        if let Some(n) = package::shift_textlinks(&shapes, shifts, None, order) {
                            replaced.insert(u.target.clone(), n.into_bytes());
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn value_or_formula(edit: &Edit) -> (String, bool) {
    match edit {
        Edit::Formula { formula, .. } => (formula.clone(), true),
        _ => (String::new(), false),
    }
}

/// `_xlfn.`, `_xlws.` and `_xlpm.` removed (outside string literals): the
/// engine's own names take user spelling.
fn strip_file_prefixes(f: &str) -> String {
    let mut out = String::with_capacity(f.len());
    let mut in_str = false;
    let mut i = 0;
    let b = f.as_bytes();
    while i < b.len() {
        if b[i] == b'"' {
            in_str = !in_str;
        }
        if !in_str {
            let rest = &f[i..];
            if let Some(p) = ["_xlfn.", "_xlws.", "_xlpm."]
                .iter()
                .find(|p| rest.starts_with(**p))
            {
                i += p.len();
                continue;
            }
        }
        let ch = f[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// The namespace prefix `workbook.xml` binds to relationships (`r` almost
/// always).
fn rid_prefix(workbook_xml: &str) -> String {
    if let Some(t) = xml::find_open(workbook_xml, "sheet", 0)
        && let Some(i) = t.text.find(":id=\"") {
            let start = t.text[..i].rfind(' ').map(|s| s + 1).unwrap_or(0);
            return t.text[start..i].to_string();
        }
    "r".to_string()
}

/// `<definedNames>`: every existing name's references moved; names the edits
/// set or deleted written from the model, in file spelling.
fn rewrite_defined_names(
    workbook_xml: &str,
    wb: &Workbook,
    shifts: &[(String, Shift)],
    order: &[String],
    entries: &[package::SheetEntry],
    set_names: &[(String, Option<String>)],
    deleted_names: &[(String, Option<String>)],
) -> Result<String, PatchError> {
    let model_names = wb.names();
    let in_file: Vec<String> = entries
        .iter()
        .map(|e| e.name.clone())
        .chain(order.iter().filter(|o| !entries.iter().any(|e| e.name.eq_ignore_ascii_case(o))).cloned())
        .collect();
    let scope_of = |local: Option<&str>| -> Option<String> {
        local
            .and_then(|i| i.parse::<usize>().ok())
            .and_then(|i| entries.get(i))
            .map(|e| e.name.clone())
    };
    let mut kept: Vec<String> = Vec::new();
    let mut seen: Vec<(String, Option<String>)> = Vec::new();
    let (open_end, close_start, had) = match xml::find_open(workbook_xml, "definedNames", 0) {
        // `<definedNames/>` (Google Sheets and openpyxl write it): no name inside
        Some(open) if open.self_closing => (open.end, open.end, Some((open.start, open.end))),
        Some(open) => {
            let close = xml::find_close(workbook_xml, &open)
                .ok_or_else(|| PatchError::Malformed("<definedNames> not closed".into()))?;
            (open.end, close.start, Some((open.start, close.end)))
        }
        None => (0, 0, None),
    };
    if had.is_some() {
        let inner = &workbook_xml[open_end..close_start];
        let mut pos = 0;
        while let Some(t) = xml::find_open(inner, "definedName", pos) {
            let (a, b) = xml::element_span(inner, &t)
                .ok_or_else(|| PatchError::Malformed("<definedName> not closed".into()))?;
            pos = b;
            let name = xml::raw_attr(t.text, "name")
                .map(xml::decode_attr)
                .unwrap_or_default();
            let scope = scope_of(xml::attr(t.text, "localSheetId").as_deref());
            // a name local to a chart sheet, which the engine cannot scope: the file's alone, its
            // references moved as text and nothing else
            let on_a_chart_sheet = xml::attr(t.text, "localSheetId")
                .and_then(|i| i.parse::<usize>().ok())
                .and_then(|i| entries.get(i))
                .is_some_and(|e| !e.is_worksheet());
            // still in the model? (a deleted name goes; a built-in one the engine may not hold,
            // `_xlnm._FilterDatabase`, stays unless an edit deleted it)
            let key = (name.to_lowercase(), scope.as_ref().map(|s| s.to_lowercase()));
            let in_model = (on_a_chart_sheet
                || model_names.iter().any(|n| {
                n.name.eq_ignore_ascii_case(&name)
                    && n.scope.as_deref().map(str::to_lowercase)
                        == scope.as_deref().map(str::to_lowercase)
            }) || name.starts_with("_xlnm."))
                && !deleted_names.contains(&key);
            if !in_model {
                continue;
            }
            seen.push((
                name.to_lowercase(),
                scope.as_ref().map(|s| s.to_lowercase()),
            ));
            let element = &inner[a..b];
            let body_raw = if t.self_closing {
                ""
            } else {
                &inner[t.end..b - "</definedName>".len()]
            };
            let mut text = xml::decode_text(body_raw);
            let mut changed = false;
            for (sheet, sh) in shifts {
                if let Some(n) = refs::shift_formula(&text, sheet, *sh, None, order) {
                    text = n;
                    changed = true;
                }
            }
            // a name the edits set: its new formula, from the model
            let key = (
                name.to_lowercase(),
                scope.as_ref().map(|s| s.to_lowercase()),
            );
            if set_names.contains(&key)
                && let Some(m) = model_names.iter().find(|n| {
                    n.name.eq_ignore_ascii_case(&name)
                        && n.scope.as_deref().map(str::to_lowercase) == key.1
                }) {
                    let user = strip_file_prefixes(&text);
                    // the engine reads `DAYS(…)` and `_xlfn.DAYS(…)` alike; Excel does not
                    let spelled = name_file_spelling(wb, &m.formula);
                    if !formulas_equal(wb, &user, &m.formula) || refs::function_calls(&text) != refs::function_calls(&spelled) {
                        text = name_file_spelling(wb, &m.formula);
                        changed = true;
                    }
                }
            kept.push(if changed {
                format!(
                    "{}{}</definedName>",
                    t.text
                        .trim_end_matches("/>")
                        .trim_end_matches('>')
                        .to_string()
                        + ">",
                    xml::escape_text(&text)
                )
            } else {
                element.to_string()
            });
        }
    }
    // names only the model has: added by the edits
    for n in &model_names {
        let key = (
            n.name.to_lowercase(),
            n.scope.as_ref().map(|s| s.to_lowercase()),
        );
        if seen.contains(&key) {
            continue;
        }
        let local = match &n.scope {
            Some(s) => {
                // the sheet's position among the file's `<sheet>` entries, chart sheets counted,
                // the new sheets after them (the engine's own order skips chart sheets)
                let i = in_file
                    .iter()
                    .position(|o| o.eq_ignore_ascii_case(s))
                    .ok_or_else(|| PatchError::Malformed(format!("scope {s:?}")))?;
                format!(" localSheetId=\"{i}\"")
            }
            None => String::new(),
        };
        kept.push(format!(
            "<definedName name=\"{}\"{local}>{}</definedName>",
            xml::escape_attr(&n.name),
            xml::escape_text(&name_file_spelling(wb, &n.formula))
        ));
    }
    let names = if kept.is_empty() {
        String::new()
    } else {
        format!("<definedNames>{}</definedNames>", kept.concat())
    };
    Ok(match had {
        Some((a, b)) => {
            let open = xml::find_open(workbook_xml, "definedNames", 0)
                .map(|t| if t.self_closing { format!("{}>", t.text.trim_end_matches("/>").trim_end()) } else { t.text.to_string() })
                .unwrap_or_default();
            let names = if kept.is_empty() {
                // an empty element stays as the file wrote it; one whose names all went goes
                if open_end == close_start { workbook_xml[a..b].to_string() } else { String::new() }
            } else {
                format!("{open}{}</definedNames>", kept.concat())
            };
            if workbook_xml[a..b] == names {
                workbook_xml.to_string()
            } else {
                format!("{}{}{}", &workbook_xml[..a], names, &workbook_xml[b..])
            }
        }
        None if names.is_empty() => workbook_xml.to_string(),
        None => {
            // CT_Workbook order: … sheets, functionGroups, externalReferences, definedNames, calcPr …
            let at = [
                "calcPr",
                "oleSize",
                "customWorkbookViews",
                "pivotCaches",
                "extLst",
                "fileRecoveryPr",
            ]
            .iter()
            .filter_map(|n| xml::find_open(workbook_xml, n, 0).map(|t| t.start))
            .min()
            .or_else(|| workbook_xml.rfind("</workbook>"))
            .ok_or_else(|| PatchError::Malformed("workbook.xml has no end".into()))?;
            format!("{}{}{}", &workbook_xml[..at], names, &workbook_xml[at..])
        }
    })
}

/// The engine does not move defined names when rows or columns move
/// (its displacement covers cell formulas and conditional formats): the
/// patcher moves the model's names with the same shifter it applies to the
/// file, so the two agree and a name keeps pointing at its cells, as in Excel.
fn move_model_names(wb: &mut Workbook, sheet: &str, shift: Shift) -> Result<(), String> {
    let order = wb.sheet_names();
    for n in wb.names() {
        let body = n.formula.trim_start_matches('=');
        if let Some(moved) = refs::shift_formula(body, sheet, shift, None, &order) {
            wb.delete_name(&n.name, n.scope.as_deref())
                .map_err(|e| e.to_string())?;
            wb.set_name(&n.name, &moved, n.scope.as_deref())
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// The engine does not move a table's range when rows or columns move (it reads a structured
/// reference as the table's cells when it parses it): the patcher moves the model's tables on
/// `sheet` as it moves the file's (`package::shift_table`), so a formula parsed after the move
/// reads a table where it now is. A move that cuts into a table, inserting or deleting rows or
/// columns inside it, is refused before anything is written: the table's columns are named in its
/// part and its structured references are cells to the engine, and the patcher resizes neither yet.
fn move_model_tables(wb: &mut Workbook, sheet: &str, shift: Shift) -> Result<(), PatchError> {
    let tables: Vec<(String, String)> = wb
        .model()
        .workbook
        .tables
        .values()
        .filter(|t| t.sheet_name.eq_ignore_ascii_case(sheet))
        .map(|t| (t.name.clone(), t.reference.clone()))
        .collect();
    for (name, reference) in tables {
        let Some(area) = refs::parse_area(&reference) else {
            continue;
        };
        let (r1, c1, r2, c2) = area.bounds();
        let cuts = match shift {
            Shift::InsertRows { at, .. } => at > r1 && at <= r2,
            Shift::DeleteRows { at, n } => at <= r2 && at + n - 1 >= r1,
            Shift::InsertCols { at, .. } => at > c1 && at <= c2,
            Shift::DeleteCols { at, n } => at <= c2 && at + n - 1 >= c1,
        };
        if cuts {
            return Err(PatchError::Unsupported(format!(
                "the move cuts into table {name} at {sheet}!{reference} (a table is moved whole, never resized, by the patcher yet)"
            )));
        }
        if let Some(moved) = shift.area(area) {
            let spelled = moved.spell();
            if spelled != reference {
                wb.model_mut()
                    .set_table_reference(&name, &spelled)
                    .map_err(PatchError::Malformed)?;
            }
        }
    }
    Ok(())
}

fn name_file_spelling(wb: &Workbook, formula: &str) -> String {
    file_spelling(wb, 0, Addr::new(1, 1), formula)
        .unwrap_or_else(|| formula.trim_start_matches('=').to_string())
}

fn formulas_equal(wb: &Workbook, a: &str, b: &str) -> bool {
    match (
        file_spelling(wb, 0, Addr::new(1, 1), a),
        file_spelling(wb, 0, Addr::new(1, 1), b),
    ) {
        (Some(x), Some(y)) => x == y,
        _ => a.trim_start_matches('=') == b.trim_start_matches('='),
    }
}

fn formula_text(c: &Content) -> Option<&str> {
    match c {
        Content::Formula(f) | Content::DynamicFormula(f) => Some(f),
        Content::ArrayFormula { formula, .. } => Some(formula),
        _ => None,
    }
}

/// Re-opens `dest` through the engine and compares every cell with the
/// model the edits produced; the number of cells compared.
fn verify(
    dest: &Path,
    wb: &mut Workbook,
    fixed: &Fixed,
    expected: &[(String, i32, i32, Value)],
) -> Result<usize, PatchError> {
    // the caches: every cell whose value the edits moved holds the engine's value in the file (the
    // engine recomputes the rest, so its reading of the file cannot show a stale cache)
    let caches: std::collections::HashMap<(String, i32, i32), Value> = cached::read(dest)?
        .into_iter()
        .map(|((s, r, c), cell)| ((s.to_lowercase(), r, c), cell.value))
        .collect();
    let mut stale: Vec<String> = Vec::new();
    for (sheet, row, col, want) in expected {
        let cached = caches.get(&(sheet.to_lowercase(), *row, *col)).cloned().unwrap_or(Value::Empty);
        if !cache_holds(&cached, want) {
            stale.push(format!("{sheet}!{}: cached {cached:?}, the engine {want:?}", Addr::new(*row, *col)));
        }
    }
    if !stale.is_empty() {
        let n = stale.len();
        stale.truncate(20);
        return Err(PatchError::Verify(format!("{n} cache(s) not the engine's value; first: {}", stale.join(" | "))));
    }
    fixed.reference();
    let opened = links::open(dest, LinkPolicy::AlwaysUseCached, |_| LinkChoice::UseCached)?;
    let mut back = Workbook::from_model(opened.model, crate::book::Origin::New);
    // the copy read back is named as the workbook was computed (the destination's own stem), so a
    // formula reading its name (`CELL("filename")`) reads what the destination will hold
    if back.model().workbook.name != wb.model().workbook.name {
        back.model_mut().workbook.name = wb.model().workbook.name.clone();
        back.recalc();
    }
    // a volatile cell (and what reads it) has a new value on every
    // evaluation: its CONTENT is compared, not its value
    let unstable = {
        let values = Before::snapshot(&back);
        Before::unstable(&mut back, &values, fixed)
    };
    let names = wb.sheet_names();
    if back.sheet_names() != names {
        return Err(PatchError::Verify(format!(
            "sheets {:?} read back as {:?}",
            names,
            back.sheet_names()
        )));
    }
    let mut compared = 0;
    let mut diffs: Vec<String> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let s = i as u32;
        let mut cells: Vec<Addr> = wb
            .cells(s)
            .map_err(|e| PatchError::Verify(e.to_string()))?
            .into_iter()
            .map(|(a, _)| a)
            .collect();
        cells.extend(
            back.cells(s)
                .map_err(|e| PatchError::Verify(e.to_string()))?
                .into_iter()
                .map(|(a, _)| a),
        );
        cells.sort();
        cells.dedup();
        for a in cells {
            compared += 1;
            let mine = wb
                .content(s, a)
                .map_err(|e| PatchError::Verify(e.to_string()))?;
            let theirs = back
                .content(s, a)
                .map_err(|e| PatchError::Verify(e.to_string()))?;
            // a formula compared as the file spells it (the `@` the engine marks where Excel
            // implies one is no difference), an array formula with an array formula; a dynamic
            // formula of one cell is written as a plain one (one that spills is refused), and
            // where Excel's reading of it differs, its value does
            let array = |c: &Content| matches!(c, Content::ArrayFormula { .. });
            let content_ok = match (formula_text(&mine), formula_text(&theirs)) {
                (Some(_), Some(_)) => {
                    array(&mine) == array(&theirs)
                        && current_file_formula(wb, s, a) == current_file_formula(&back, s, a)
                }
                (None, None) => true,
                _ => false,
            };
            let v1 = wb
                .value_now(s, a)
                .map_err(|e| PatchError::Verify(e.to_string()))?;
            let v2 = back
                .value_now(s, a)
                .map_err(|e| PatchError::Verify(e.to_string()))?;
            let shaky = unstable
                .get(i)
                .is_some_and(|u| u.contains(&(a.row, a.column)));
            if !content_ok || (!shaky && !same(&v1, &v2)) {
                if diffs.len() < 20 {
                    diffs.push(format!(
                        "{name}!{a}: model {mine:?} = {v1:?}, file {theirs:?} = {v2:?}"
                    ));
                } else {
                    diffs.push(String::new());
                }
            }
        }
    }
    if !diffs.is_empty() {
        let n = diffs.len();
        let shown: Vec<String> = diffs.into_iter().filter(|d| !d.is_empty()).collect();
        return Err(PatchError::Verify(format!(
            "{n} cell(s) differ; first: {}",
            shown.join(" | ")
        )));
    }
    Ok(compared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_one_value_when_it_is_one_cell_or_one_constant() {
        for one in ["S!$A$1", "=S!$A$1", "'My S'!$B$2", "0.21", "\"x\"", "{1}"] {
            assert!(names_one_value(one), "{one}");
        }
        for several in ["S!$A$1:$A$3", "{1,2,3}", "{\"Jan\";\"Feb\"}", "OFFSET(S!$A$1,0,0,3)", "S!A:A"] {
            assert!(!names_one_value(several), "{several}");
        }
    }

    #[test]
    fn a_cell_maps_back_through_the_moves() {
        let s = |sh: Shift| ("S".to_string(), sh);
        let shifts = [s(Shift::InsertRows { at: 2, n: 2 }), ("T".into(), Shift::DeleteRows { at: 1, n: 5 }), s(Shift::DeleteCols { at: 1, n: 1 })];
        assert_eq!(backward(&shifts, "S", 1, 1), Some((1, 2)));
        assert_eq!(backward(&shifts, "S", 4, 3), Some((2, 4)));
        assert_eq!(backward(&shifts, "S", 2, 1), None);
        assert_eq!(backward(&shifts, "s", 5, 1), Some((3, 2)));
        for (r, c) in [(1, 2), (2, 3), (9, 9)] {
            let there = forward(&shifts, 0, "S", r, c).unwrap();
            assert_eq!(backward(&shifts, "S", there.0, there.1), Some((r, c)));
        }
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        names.sort();
        names
    }

    #[test]
    fn a_failed_check_or_a_moved_source_leaves_the_destination_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("in.xlsx");
        let dest = dir.path().join("out.xlsx");
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(&dest, b"before").unwrap();
        // the check sees a copy of the new file at a short UTF-8 path, and refuses it
        let e = commit(&source, b"source", &dest, b"after", |written| {
            assert_eq!(std::fs::read(written).unwrap(), b"after");
            assert!(written.to_str().is_some() && !written.starts_with(dir.path()), "{}", written.display());
            Err(PatchError::Verify("refused".into()))
        })
        .unwrap_err();
        assert!(matches!(e, PatchError::Verify(_)), "{e}");
        assert_eq!(std::fs::read(&dest).unwrap(), b"before");
        // the source no longer holds what was read
        let e = commit(&source, b"what was read", &dest, b"after", |_| Ok(())).unwrap_err();
        assert!(matches!(e, PatchError::SourceMoved(_)), "{e}");
        assert_eq!(std::fs::read(&dest).unwrap(), b"before");
        // nothing left beside the destination, no `.bak` made
        assert_eq!(listing(dir.path()), ["in.xlsx", "out.xlsx"]);
        // all well: renamed over the destination, its previous version kept
        commit(&source, b"source", &dest, b"after", |_| Ok(())).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"after");
        assert_eq!(std::fs::read(dir.path().join("out.xlsx.bak")).unwrap(), b"before");
        assert_eq!(listing(dir.path()), ["in.xlsx", "out.xlsx", "out.xlsx.bak"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_destination_is_patched_again_and_again_and_stays_read_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("ro.xlsx");
        std::fs::write(&dest, b"first").unwrap();
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o444)).unwrap();
        for (n, bytes) in [b"second", b"third_"].iter().enumerate() {
            let before = std::fs::read(&dest).unwrap();
            commit(&dest, &before, &dest, &bytes[..], |_| Ok(())).unwrap_or_else(|e| panic!("patch {n}: {e}"));
            assert_eq!(std::fs::read(&dest).unwrap(), bytes.to_vec());
            assert_eq!(std::fs::read(dir.path().join("ro.xlsx.bak")).unwrap(), before);
            assert_eq!(std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777, 0o444);
        }
    }

    fn name(name: &str, scope: Option<&str>, formula: &str) -> Name {
        Name { name: name.into(), scope: scope.map(str::to_string), formula: formula.into() }
    }

    #[test]
    fn tokens_carry_the_sheet_that_qualifies_them() {
        let t = tokens("1+Data!R+'My ''data'''!S*T(2)+\"U!V\"");
        let got: Vec<(&str, Option<char>, Option<&str>)> =
            t.iter().map(|t| (t.text, t.next, t.sheet.as_deref())).collect();
        assert_eq!(
            got,
            [
                ("1", Some('+'), None),
                ("Data", Some('!'), None),
                ("R", Some('+'), Some("Data")),
                ("S", Some('*'), Some("My 'data'")),
                ("T", Some('('), None),
                ("2", Some(')'), None),
            ]
        );
    }

    #[test]
    fn a_name_resolves_as_excel_resolves_it_and_a_volatile_function_is_a_call() {
        let names = vec![
            name("R", None, "Data!$A$1"),
            name("R", Some("Other"), "RAND()"),
            name("Coin", None, "_xlfn.LAMBDA(_xlfn.RANDBETWEEN(1,2))"),
            name("Twice", Some("Other"), "R*2"),
            name("Plain", None, "R*2"),
        ];
        let volatile = volatile_names(&names);
        // the local R is volatile, the workbook's is not; Twice reads Other's R, Plain the workbook's
        assert_eq!(volatile, [false, true, true, true, false]);
        let v = |f: &str, sheet: &str| formula_volatile(f, sheet, &names, &volatile);
        assert!(!v("R*2", "Data"), "Data reads the workbook's R");
        assert!(v("R*2", "Other"), "Other reads its own R");
        assert!(v("Other!R*2", "Data"), "qualified, Other's R from anywhere");
        assert!(!v("Data!R*2", "Other"), "Data has no local R: the workbook's");
        assert!(v("Coin()+1", "Data"), "a LAMBDA name called");
        assert!(v("LET(x,Twice,x)", "Other"));
        assert!(!v("R[Qty]+R!A1", "Other"), "a table and a sheet named R");
        assert!(!v("\"R\"&'R'!A1", "Other"));
        assert!(!v("R1*2", "Other"));
        assert!(!v("My.R", "Other"));
        assert!(calls_volatile("1+RAND()"));
        assert!(calls_volatile("_xlfn.RANDARRAY(3)"));
        assert!(calls_volatile("today ()+1"));
        assert!(calls_volatile("LET(x,RAND(),x)"));
        assert!(!calls_volatile("GRANDTOTAL(1)"));
        assert!(!calls_volatile("\"RAND()\""));
        assert!(!calls_volatile("NOW"));
        assert!(!calls_volatile("'RAND('!A1"));
    }

    #[test]
    fn edits_read_from_json_lines() {
        let e: Edit = serde_json::from_str(r#"{"op":"formula","sheet":"S","cell":"Q5","formula":"=LET(t,1,t)","expect":{"formula":"A1&B1"}}"#).unwrap();
        assert_eq!(
            e,
            Edit::Formula {
                sheet: "S".into(),
                cell: "Q5".into(),
                formula: "=LET(t,1,t)".into(),
                expect: Some(Expect::Formula {
                    formula: "A1&B1".into()
                })
            }
        );
        let e: Edit = serde_json::from_str(
            r#"{"op":"value","sheet":"S","cell":"T5","value":"x","expect":{"value":null}}"#,
        )
        .unwrap();
        assert!(matches!(
            e,
            Edit::Value {
                expect: Some(Expect::Value { .. }),
                ..
            }
        ));
        let e: Edit = serde_json::from_str(
            r#"{"op":"border","sheet":"S","area":"A1:B2","rgb":"0070C0","style":"thin"}"#,
        )
        .unwrap();
        assert!(matches!(
            e,
            Edit::Border {
                style: Some(LineStyle::Thin),
                ..
            }
        ));
    }

    #[test]
    fn file_prefixes_strip_outside_strings_only() {
        assert_eq!(
            strip_file_prefixes(r#"_xlfn.LET(_xlpm.t,1,"_xlpm.t")"#),
            r#"LET(t,1,"_xlpm.t")"#
        );
    }
}
