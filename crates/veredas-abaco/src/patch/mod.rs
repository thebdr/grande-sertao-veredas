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
//! - the write is atomic, the destination's previous version kept as `.bak`,
//!   and the source is re-read before the rename (it must not have moved
//!   since it was read);
//! - [`PatchOptions::verify`] re-opens the written file through the engine
//!   and compares every cell (content and value) with the model the edits
//!   produced.

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
use crate::book::{Content, Value, Workbook};

pub use refs::Shift;
pub use styles::{LineStyle, StyleOp};

use sheet::{CellWrite, ColOp, NewContent, RowOp, SheetJob};

const NS_WORKSHEET: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
const CT_WORKSHEET: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
const NS_STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
const CT_STYLES: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml";

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

#[derive(Debug, Clone, Default)]
pub struct PatchOptions {
    /// Re-open the written file and compare every cell with the model.
    pub verify: bool,
    /// Set `fullCalcOnLoad` (Excel recalculates everything on open). Off by
    /// default: the patcher writes the engine's values.
    pub full_calc_on_load: bool,
}

/// Whether a formula's text calls a function whose value changes from one evaluation to the
/// next (a text test: a false positive costs a value comparison, never a byte).
fn calls_volatile(formula: &str) -> bool {
    let f = formula.to_ascii_uppercase();
    f.contains("RAND") || f.contains("NOW(") || f.contains("TODAY(")
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
/// would read as "moved" on every evaluation: when the workbook holds one,
/// the unedited model is evaluated a second time and every cell whose value
/// differs between those two passes (the volatile cells and whatever reads
/// them) is `unstable` — its cache is kept unless an edit wrote the cell
/// itself. Excel recalculates volatile cells on every recalculation anyway.
struct Before {
    values: Vec<std::collections::HashMap<(i32, i32), Value>>,
    unstable: Vec<std::collections::HashSet<(i32, i32)>>,
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

    /// Whether any formula of `wb` calls a volatile function (a text test:
    /// a false positive costs one more evaluation, nothing else).
    fn volatile(wb: &Workbook) -> bool {
        wb.model().workbook.worksheets.iter().any(|ws| ws.shared_formulas.iter().any(|f| calls_volatile(f)))
    }

    /// The cells whose own formula calls a volatile function, and the cells such a formula spills
    /// into: unstable whatever two evaluations happen to give (a random draw can repeat).
    fn volatile_cells(wb: &Workbook) -> Vec<std::collections::HashSet<(i32, i32)>> {
        wb.model()
            .workbook
            .worksheets
            .iter()
            .map(|ws| {
                let mut cells: std::collections::HashSet<(i32, i32)> = ws
                    .sheet_data
                    .cells()
                    .filter(|(_, _, c)| {
                        c.get_formula()
                            .and_then(|i| ws.shared_formulas.get(i as usize))
                            .is_some_and(|f| calls_volatile(f))
                    })
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
    ) -> Vec<std::collections::HashSet<(i32, i32)>> {
        if !Before::volatile(wb) {
            return vec![std::collections::HashSet::new(); values.len()];
        }
        // by text: the volatile formulas and what they spill into; measured: what moves with them,
        // over three more evaluations (a value that repeats once by chance seldom repeats thrice)
        let mut out = Before::volatile_cells(wb);
        out.resize(values.len(), std::collections::HashSet::new());
        for _ in 0..3 {
            wb.recalc();
            let again = Before::snapshot(wb);
            for ((a, b), u) in values.iter().zip(again.iter()).zip(out.iter_mut()) {
                u.extend(a.iter().filter(|(k, v)| !b.get(*k).is_some_and(|w| same(v, w))).map(|(k, _)| *k));
            }
        }
        out
    }

    /// `wb` as `links::open` left it: evaluated, unedited.
    fn take(wb: &mut Workbook) -> Before {
        let values = Before::snapshot(wb);
        let unstable = Before::unstable(wb, &values);
        Before { values, unstable }
    }

    /// Whether the edits moved the engine's value of the formula that was at
    /// `orig` on sheet `sheet` and evaluates to `live` now.
    fn moved(&self, sheet: u32, orig: (i32, i32), live: &Value) -> bool {
        let s = sheet as usize;
        if self.unstable.get(s).is_some_and(|u| u.contains(&orig)) {
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
            Some(e) => Value::Error(e.to_string()),
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

fn same(a: &Value, b: &Value) -> bool {
    sheet::same_value(a, b) && sheet::same_value(b, a)
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

/// Whether the cell holds what `expect` says.
fn holds(wb: &Workbook, sheet: u32, a: Addr, expect: &Expect) -> Result<bool, String> {
    let current = current_file_formula(wb, sheet, a);
    match expect {
        Expect::Formula { formula } => {
            let want = file_spelling(wb, sheet, a, formula)
                .ok_or_else(|| format!("the expected formula does not parse: {formula}"))?;
            Ok(current.as_deref() == Some(want.as_str()))
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

    // ---- the engine's model, links from the cache (Excel's closed-book view)
    let opened = links::open(source, LinkPolicy::AlwaysUseCached, |_| {
        LinkChoice::UseCached
    })?;
    let mut wb = Workbook::from_model(opened.model, crate::book::Origin::New);
    if crate::files::contents(source).map_err(|e| io_err(source, e))? != fingerprint {
        return Err(PatchError::SourceMoved(source.to_path_buf()));
    }
    // the engine's value of every formula BEFORE the edits: a cache is
    // re-written only where the edits moved it (see `Before`)
    let before = Before::take(&mut wb);

    let mut report = PatchReport::default();
    let mut shifts: Vec<(String, Shift)> = Vec::new();
    let mut writes: Vec<PendingWrite> = Vec::new();
    let mut rows_ops: Vec<PendingRows> = Vec::new();
    let mut cols_ops: Vec<PendingCols> = Vec::new();
    let mut new_sheets: Vec<String> = Vec::new();
    let mut names_touched = false;
    let mut set_names: Vec<(String, Option<String>)> = Vec::new();

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
                    // a CSE/dynamic array is one object; a cell of it is not editable alone
                    if formula_node(&wb, s, a).is_none()
                        || matches!(wb.content(s, a), Ok(Content::ArrayFormula { .. }))
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
                    current_file_formula(&wb, s, a).as_deref() == Some(want.as_str())
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
                    && !holds(&wb, s, a, expect).map_err(fail)? {
                        outcome.status = Status::Conflict;
                        outcome.note = format!("{canonical}!{a} now holds {}", describe(&wb, s, a));
                        report.outcomes.push(outcome);
                        continue;
                    }
                let content = if is_formula {
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
                let existing: Vec<Addr> = wb
                    .cells(s)
                    .map_err(|e| fail(e.to_string()))?
                    .into_iter()
                    .map(|(a, _)| a)
                    .filter(|a| {
                        a.row >= ar.first.row
                            && a.row <= ar.last.row
                            && a.column >= ar.first.column
                            && a.column <= ar.last.column
                    })
                    .collect();
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
                shifts.push((canonical, sh));
            }
            Edit::AddSheet { name } => {
                if wb.sheet_index(name).is_ok() {
                    return Err(fail(format!("a sheet named {name:?} exists")));
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
                // the engine stores user spelling; give it that
                let user = strip_file_prefixes(body);
                let exists = wb.names().iter().any(|n| {
                    n.name.eq_ignore_ascii_case(name)
                        && n.scope.as_deref().map(str::to_lowercase)
                            == scope.as_deref().map(str::to_lowercase)
                });
                if exists {
                    wb.delete_name(name, scope.as_deref())
                        .map_err(|e| fail(e.to_string()))?;
                }
                wb.set_name(name, &user, scope.as_deref())
                    .map_err(|e| fail(e.to_string()))?;
                set_names.push((
                    name.to_lowercase(),
                    scope.as_ref().map(|s| s.to_lowercase()),
                ));
                names_touched = true;
            }
            Edit::DeleteName { name, scope } => {
                wb.delete_name(name, scope.as_deref())
                    .map_err(|e| fail(e.to_string()))?;
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
                if *level > 7 || first > last || *first < 1 {
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

    wb.recalc();

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
    for w in &writes {
        let Some((row, col)) = map_pos(&w.sheet, w.after, w.row, w.col) else {
            continue;
        };
        let entry = final_writes
            .entry(w.sheet.clone())
            .or_default()
            .entry((row, col))
            .or_default();
        entry.style.extend(w.style.iter().copied());
        if let Some(c) = &w.content {
            let s = wb
                .sheet_index(&w.sheet)
                .map_err(|e| PatchError::Malformed(e.to_string()))?;
            let a = Addr::new(row, col);
            entry.content = Some(match c {
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
                    let text = current_file_formula(&wb, s, a).ok_or_else(|| {
                        PatchError::Malformed(format!(
                            "{}!{a}: the formula vanished from the model",
                            w.sheet
                        ))
                    })?;
                    let cached = wb
                        .value_now(s, a)
                        .map_err(|e| PatchError::Malformed(e.to_string()))?;
                    NewContent::Formula { text, cached }
                }
            });
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
        let (Some((_, a)), Some((_, b))) = (
            map_pos(&c.sheet, c.after, 1, c.first),
            map_pos(&c.sheet, c.after, 1, c.last),
        ) else {
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
        while pkg.has(&format!("xl/worksheets/sheet{n}.xml"))
            || sheet_jobs
                .iter()
                .any(|(_, p, _)| *p == format!("xl/worksheets/sheet{n}.xml"))
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
        let moved = |orig: (i32, i32), live: &Value| -> bool {
            fresh.is_none() && before.moved(sheet_index, orig, live)
        };
        let key = order
            .iter()
            .find(|n| n.eq_ignore_ascii_case(name))
            .cloned()
            .unwrap_or_else(|| name.clone());
        let job = SheetJob {
            name: &key,
            order: &order,
            shifts: &shifts,
            writes: final_writes.get(&key).unwrap_or(&empty_writes),
            row_ops: final_rows.get(&key).unwrap_or(&empty_rows),
            col_ops: final_cols.get(&key).map(|v| v.as_slice()).unwrap_or(&[]),
            value_at: &value_at,
            moved: &moved,
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
                        "vmlDrawing" if !own.is_empty() => package::shift_vml(&content, &own),
                        "drawing" => {
                            // charts hang off the drawing; their series formulas
                            // may point at any moved sheet
                            let drels = package::rels_of(&r.target);
                            if pkg.has(&drels) {
                                let dx = pkg.read_text(&drels)?;
                                for c in package::rels(&r.target, &dx) {
                                    if c.kind == "chart"
                                        && pkg.has(&c.target)
                                        && !replaced.contains_key(&c.target)
                                    {
                                        let cx = pkg.read_text(&c.target)?;
                                        if let Some(n) = package::shift_formula_elements(
                                            &cx, "c:f", &shifts, None, &order,
                                        ) {
                                            replaced.insert(c.target.clone(), n.into_bytes());
                                        }
                                    }
                                }
                            }
                            if own.is_empty() {
                                None
                            } else {
                                package::shift_drawing(&content, &own)
                            }
                        }
                        "table" => {
                            let t = if own.is_empty() {
                                None
                            } else {
                                package::shift_table(&content, &own)?
                            };
                            let base = t.clone().unwrap_or(content.clone());
                            package::shift_formula_elements(
                                &base,
                                "calculatedColumnFormula",
                                &shifts,
                                Some(name),
                                &order,
                            )
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

    // ---- workbook.xml: defined names, calcPr ------------------------------
    if !shifts.is_empty() || names_touched {
        workbook_xml =
            rewrite_defined_names(&workbook_xml, &wb, &shifts, &order, &entries, &set_names)?;
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

    // ---- atomic write, re-read before the rename --------------------------
    let dir = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let file_name = dest
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "patched.xlsx".into());
    let tmp = dir.join(format!(".{file_name}.abaco-tmp"));
    std::fs::write(&tmp, &out).map_err(|e| io_err(&tmp, e))?;
    if crate::files::contents(source).map_err(|e| io_err(source, e))? != fingerprint {
        let _ = std::fs::remove_file(&tmp);
        return Err(PatchError::SourceMoved(source.to_path_buf()));
    }
    if dest.exists() {
        let bak = dir.join(format!("{file_name}.bak"));
        std::fs::copy(dest, &bak).map_err(|e| io_err(&bak, e))?;
    }
    std::fs::rename(&tmp, dest).map_err(|e| io_err(dest, e))?;

    if options.verify {
        report.verified_cells = verify(dest, &mut wb)?;
    }
    Ok(report)
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
            let name = xml::attr(t.text, "name")
                .map(xml::decode_text)
                .unwrap_or_default();
            let scope = scope_of(xml::attr(t.text, "localSheetId"));
            // still in the model? (a deleted name goes)
            let in_model = model_names.iter().any(|n| {
                n.name.eq_ignore_ascii_case(&name)
                    && n.scope.as_deref().map(str::to_lowercase)
                        == scope.as_deref().map(str::to_lowercase)
            }) || name.starts_with("_xlnm.");
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
                    if !formulas_equal(wb, &user, &m.formula) {
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
                .map(|t| t.text.to_string())
                .unwrap_or_default();
            let names = if kept.is_empty() {
                String::new()
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
fn verify(dest: &Path, wb: &mut Workbook) -> Result<usize, PatchError> {
    let opened = links::open(dest, LinkPolicy::AlwaysUseCached, |_| LinkChoice::UseCached)?;
    let mut back = Workbook::from_model(opened.model, crate::book::Origin::New);
    // a volatile cell (and what reads it) has a new value on every
    // evaluation: its CONTENT is compared, not its value
    let unstable = {
        let values = Before::snapshot(&back);
        Before::unstable(&mut back, &values)
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
            let content_ok = match (formula_text(&mine), formula_text(&theirs)) {
                (Some(x), Some(y)) => x == y,
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
