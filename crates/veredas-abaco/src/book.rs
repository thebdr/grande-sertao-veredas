//! A workbook as Ábaco's code sees it (Veredas 1's `workbook`, its model half; the workspace it
//! also was there, AB10, arrives with the `.abaco` file, `docs/abaco-plan.md` phase 3).
//!
//! A [`Workbook`] is the fork's `Model` with cell, sheet and name operations in A1 terms: an
//! `.xlsx`/`.xlsm` is read through the fork, its links resolved under the link policy
//! ([`crate::links`]), and is never the save path of a foreign file: a foreign file's edits go to
//! the patcher ([`crate::patch`]); [`Workbook::export`] writes the model through the fork's writer,
//! for work born in Ábaco.
//!
//! Recalculation: every edit marks the model stale and the next read recalculates
//! ([`Workbook::value`]); [`Workbook::recalc`] forces it.

use std::path::{Path, PathBuf};

use ironcalc_base::expressions::token::get_error_by_english_name;
use ironcalc_base::expressions::types::Area as EngineArea;
use ironcalc_base::types::{Cell, FormulaValue, SpillValue, Style};
use ironcalc_base::{DefinedNameEntry, Model};

use crate::a1::{Addr, Area, BadAddress};
use ironcalc_base::external::ExternalBook;

use crate::links::{self, LinkChoice, LinkOutcome, LinkPolicy};

/// The engine's settings for every model Ábaco makes (formulas are stored in
/// English in the files; plant files carry no locale of their own).
pub const LOCALE: &str = "en";
pub const TZ: &str = "UTC";
pub const LANGUAGE: &str = "en";
/// Excel's default column width (character units) for a column the file sets none for.
pub const EXCEL_DEFAULT_COLUMN_WIDTH: f64 = 8.43;
// The engine's private COLUMN_WIDTH_FACTOR: its set_column_width takes pixels = width × 9.
const ENGINE_PX_PER_CHAR: f64 = 9.0;
/// The default font's maximum digit width in pixels (Calibri 11 / Aptos
/// Narrow 11: 7) — what ties a file's `<col width>` to Excel's ColumnWidth.
const MAX_DIGIT_PX: f64 = 7.0;

/// A file's `<col width>` (character count plus the 5-pixel padding, in
/// 1/256ths) → Excel's `ColumnWidth` (ECMA-376 §18.3.1.13): 12.5703125 → 11.86.
pub fn column_width_of_stored(stored: f64) -> f64 {
    let px = (((256.0 * stored + (128.0 / MAX_DIGIT_PX).trunc()) / 256.0) * MAX_DIGIT_PX).trunc();
    (((px - 5.0) / MAX_DIGIT_PX * 100.0 + 0.5).trunc() / 100.0).max(0.0)
}

/// Excel's `ColumnWidth` → the `<col width>` a file stores: 11.86 → 12.5703125.
pub fn stored_of_column_width(chars: f64) -> f64 {
    if chars <= 0.0 {
        return 0.0;
    }
    // a ColumnWidth is a whole number of pixels (Excel snaps it): the pixels first
    (((chars * MAX_DIGIT_PX).round() + 5.0) / MAX_DIGIT_PX * 256.0).trunc() / 256.0
}

/// A cell's value, as a formula sees it.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
    /// The error as Excel spells it: `#N/A`, `#VALUE!`, …
    Error(String),
}

/// What a cell holds — the `.abaco` writer's and the round-trip test's view.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    /// No content (a cell that only carries a style).
    Empty,
    Value(Value),
    /// `=…` in user spelling (`=LET(x,1,x)`, never `_xlfn.`).
    Formula(String),
    /// A CSE array formula anchored here, spanning `columns × rows`.
    ArrayFormula { formula: String, columns: i32, rows: i32 },
    /// A dynamic-array formula anchored here (it spills when evaluated).
    DynamicFormula(String),
    /// A cell an array/dynamic formula spilled into: computed, not content.
    Spill,
}

/// Where the workbook came from.
#[derive(Debug, Clone, PartialEq)]
pub enum Origin {
    New,
    Xlsx {
        path: PathBuf,
        /// `None` when the file links to no other workbook.
        choice: Option<LinkChoice>,
        links: LinkOutcome,
    },
}

/// A defined name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
    pub name: String,
    /// The sheet a local name belongs to; `None` for a workbook name.
    pub scope: Option<String>,
    /// As the engine spells it, e.g. `Sheet1!$A$1`.
    pub formula: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("no sheet named {0:?}")]
    UnknownSheet(String),
    #[error(transparent)]
    Address(#[from] BadAddress),
    #[error("{0}")]
    Engine(String),
    #[error(transparent)]
    Open(#[from] links::OpenError),
    #[error("{path}: {message}")]
    File { path: PathBuf, message: String },
}

impl ModelError {
    pub(crate) fn file(path: &Path, message: impl std::fmt::Display) -> ModelError {
        ModelError::File {
            path: path.to_path_buf(),
            message: message.to_string(),
        }
    }
}

fn engine(e: String) -> ModelError {
    ModelError::Engine(e)
}

/// A workbook: the engine's model and where it came from.
pub struct Workbook {
    model: Model<'static>,
    pub origin: Origin,
    /// Under a linked resolution, one line per external cell whose live value differs from the
    /// file's cache, as the workbook was opened ([`crate::linkdiff`]).
    pub link_log: Vec<String>,
    stale: bool,
}

impl Workbook {
    /// An empty workbook with one sheet, `Sheet1`.
    pub fn new(name: &str) -> Result<Workbook, ModelError> {
        // The engine ties the model's lifetime to the name it is given; a
        // static name, then the real one, keeps the model 'static.
        let mut model = Model::new_empty("workbook", LOCALE, TZ, LANGUAGE).map_err(engine)?;
        model.workbook.name = name.to_string();
        Ok(Workbook::from_model(model, Origin::New))
    }

    pub fn from_model(model: Model<'static>, origin: Origin) -> Workbook {
        Workbook { model, origin, link_log: Vec::new(), stale: true }
    }

    /// Opens an `.xlsx` or `.xlsm`. A workbook with links to other workbooks is resolved under
    /// `policy`; `prompt` is asked only under [`LinkPolicy::AlwaysAsk`] and only for a file with
    /// links. The model is evaluated.
    pub fn open(
        path: &Path,
        policy: LinkPolicy,
        prompt: impl FnOnce(&[ExternalBook]) -> LinkChoice,
    ) -> Result<Workbook, ModelError> {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "xlsx" | "xlsm" => {
                let mut opened = links::open(path, policy, prompt)?;
                // Q4: under a linked resolution, one log line per external
                // cell whose live value differs from the file's cache
                let diffs: Vec<String> = opened.link_diffs().iter().map(|d| d.to_string()).collect();
                let mut wb = Workbook::from_model(
                    opened.model,
                    Origin::Xlsx {
                        path: opened.path,
                        choice: opened.choice,
                        links: opened.links,
                    },
                );
                wb.stale = false; // links::open evaluated it
                wb.link_log = diffs;
                Ok(wb)
            }
            _ => Err(ModelError::file(path, "not a workbook Ábaco opens (.xlsx, .xlsm)")),
        }
    }

    /// The engine model, for what this API does not cover yet. Edits made
    /// through it are not seen as edits: call [`Workbook::mark_stale`].
    pub fn model(&self) -> &Model<'static> {
        &self.model
    }

    pub fn model_mut(&mut self) -> &mut Model<'static> {
        &mut self.model
    }

    pub fn mark_stale(&mut self) {
        self.stale = true;
    }

    pub fn is_stale(&self) -> bool {
        self.stale
    }

    /// The workbook's name (the engine's; the file name for an xlsx).
    pub fn name(&self) -> &str {
        &self.model.workbook.name
    }

    // ---- sheets ---------------------------------------------------------

    pub fn sheet_names(&self) -> Vec<String> {
        self.model.workbook.get_worksheet_names()
    }

    /// The index of the sheet called `name` (exact, then case-insensitive,
    /// as Excel resolves sheet names).
    pub fn sheet_index(&self, name: &str) -> Result<u32, ModelError> {
        let names = self.sheet_names();
        names
            .iter()
            .position(|n| n == name)
            .or_else(|| names.iter().position(|n| n.eq_ignore_ascii_case(name)))
            .map(|i| i as u32)
            .ok_or_else(|| ModelError::UnknownSheet(name.to_string()))
    }

    pub fn sheet_name(&self, sheet: u32) -> Result<String, ModelError> {
        self.model
            .workbook
            .worksheet(sheet)
            .map(|w| w.get_name())
            .map_err(engine)
    }

    /// Appends a sheet; its index.
    pub fn add_sheet(&mut self, name: &str) -> Result<u32, ModelError> {
        self.model.add_sheet(name).map_err(engine)?;
        self.stale = true;
        self.sheet_index(name)
    }

    pub fn rename_sheet(&mut self, old: &str, new: &str) -> Result<(), ModelError> {
        self.sheet_index(old)?;
        self.model.rename_sheet(old, new).map_err(engine)?;
        self.stale = true;
        Ok(())
    }

    pub fn delete_sheet(&mut self, name: &str) -> Result<(), ModelError> {
        let index = self.sheet_index(name)?;
        self.model.delete_sheet(index).map_err(engine)?;
        self.stale = true;
        Ok(())
    }

    // ---- cells ----------------------------------------------------------

    /// Recalculates when an edit happened since the last calculation.
    pub fn ensure_calculated(&mut self) {
        if self.stale {
            self.recalc();
        }
    }

    /// Recalculates the whole workbook now.
    pub fn recalc(&mut self) {
        self.model.evaluate();
        self.stale = false;
    }

    /// The value at `addr`, recalculating first if an edit is pending.
    pub fn value(&mut self, sheet: u32, addr: Addr) -> Result<Value, ModelError> {
        self.ensure_calculated();
        self.value_now(sheet, addr)
    }

    /// The value as stored now, without recalculating.
    pub fn value_now(&self, sheet: u32, addr: Addr) -> Result<Value, ModelError> {
        let ws = self.model.workbook.worksheet(sheet).map_err(engine)?;
        let Some(cell) = ws.cell(addr.row, addr.column) else {
            return Ok(Value::Empty);
        };
        Ok(match cell.as_ref() {
            Cell::EmptyCell { .. } => Value::Empty,
            Cell::BooleanCell { v, .. } => Value::Bool(*v),
            Cell::NumberCell { v, .. } => Value::Number(*v),
            Cell::ErrorCell { ei, .. } => Value::Error(ei.to_string()),
            Cell::SharedString { si, .. } => Value::Text(
                self.model
                    .workbook
                    .shared_strings
                    .get(*si as usize)
                    .cloned()
                    .unwrap_or_default(),
            ),
            Cell::CellFormula { v, .. } | Cell::ArrayFormula { v, .. } => formula_value(v),
            Cell::SpillCell { v, .. } => match v {
                SpillValue::Boolean(b) => Value::Bool(*b),
                SpillValue::Number(n) => Value::Number(*n),
                SpillValue::Text(s) => Value::Text(s.clone()),
                SpillValue::Error(e) => Value::Error(e.to_string()),
            },
        })
    }

    /// What the cell shows, formatted by its number format (`0025`, `12%`).
    pub fn text(&mut self, sheet: u32, addr: Addr) -> Result<String, ModelError> {
        self.ensure_calculated();
        self.model
            .get_formatted_cell_value(sheet, addr.row, addr.column)
            .map_err(engine)
    }

    /// The formula at `addr` in user spelling (`=LET(x,1,x)`), `None` for a
    /// value cell.
    pub fn formula(&self, sheet: u32, addr: Addr) -> Result<Option<String>, ModelError> {
        self.model
            .get_cell_formula(sheet, addr.row, addr.column)
            .map_err(engine)
    }

    /// The cell's content kind and text — see [`Content`].
    pub fn content(&self, sheet: u32, addr: Addr) -> Result<Content, ModelError> {
        let ws = self.model.workbook.worksheet(sheet).map_err(engine)?;
        let Some(cell) = ws.cell(addr.row, addr.column) else {
            return Ok(Content::Empty);
        };
        let formula = || -> Result<String, ModelError> {
            Ok(self.formula(sheet, addr)?.unwrap_or_default())
        };
        Ok(match cell.as_ref() {
            Cell::EmptyCell { .. } => Content::Empty,
            Cell::CellFormula { .. } => Content::Formula(formula()?),
            Cell::ArrayFormula { r, kind, .. } => match kind {
                ironcalc_base::types::ArrayKind::Cse => Content::ArrayFormula {
                    formula: formula()?,
                    columns: r.0,
                    rows: r.1,
                },
                ironcalc_base::types::ArrayKind::Dynamic => Content::DynamicFormula(formula()?),
            },
            Cell::SpillCell { .. } => Content::Spill,
            _ => Content::Value(self.value_now(sheet, addr)?),
        })
    }

    /// The cell's style index (an opaque handle into the workbook's styles).
    pub fn style_index(&self, sheet: u32, addr: Addr) -> Result<i32, ModelError> {
        let ws = self.model.workbook.worksheet(sheet).map_err(engine)?;
        Ok(ws.get_style(addr.row, addr.column))
    }

    /// The cell's style, as a value (opaque pass-through: copy it to another
    /// cell, of this or another workbook, with [`Workbook::set_style`]).
    pub fn style(&self, sheet: u32, addr: Addr) -> Result<Style, ModelError> {
        self.model
            .get_style_for_cell(sheet, addr.row, addr.column)
            .map_err(engine)
    }

    /// A column's width in Excel's character units (what `ColumnWidth`
    /// answers): the file's `<col>` width when it set one, else Excel's
    /// default 8.43 (the engine does not keep a sheet's `defaultColWidth`).
    pub fn column_width(&self, sheet: u32, column: i32) -> Result<f64, ModelError> {
        let ws = self.model.workbook.worksheet(sheet).map_err(engine)?;
        for c in &ws.cols {
            if c.min <= column && column <= c.max {
                return Ok(if c.custom_width { column_width_of_stored(c.width) } else { EXCEL_DEFAULT_COLUMN_WIDTH });
            }
        }
        Ok(EXCEL_DEFAULT_COLUMN_WIDTH)
    }

    /// Sets a column's width in Excel's character units (hidden and the
    /// outline level kept).
    pub fn set_column_width(&mut self, sheet: u32, column: i32, width: f64) -> Result<(), ModelError> {
        let ws = self.model.workbook.worksheet_mut(sheet).map_err(engine)?;
        ws.set_column_width(column, stored_of_column_width(width) * ENGINE_PX_PER_CHAR).map_err(engine)
    }

    pub fn set_column_hidden(&mut self, sheet: u32, column: i32, hidden: bool) -> Result<(), ModelError> {
        let ws = self.model.workbook.worksheet_mut(sheet).map_err(engine)?;
        ws.set_column_hidden(column, hidden).map_err(engine)
    }

    pub fn is_column_hidden(&self, sheet: u32, column: i32) -> Result<bool, ModelError> {
        let ws = self.model.workbook.worksheet(sheet).map_err(engine)?;
        ws.is_column_hidden(column).map_err(engine)
    }

    /// Groups columns `first..=last` one outline level deeper (Excel's
    /// `Columns.Group`, levels capped at 7); `collapsed` also hides them, as a
    /// group collapsed with its − button. Fork change 11.
    pub fn group_columns(&mut self, sheet: u32, first: i32, last: i32, collapsed: bool) -> Result<(), ModelError> {
        let ws = self.model.workbook.worksheet_mut(sheet).map_err(engine)?;
        for c in first.min(last)..=first.max(last) {
            let (level, _) = ws.column_outline(c);
            ws.set_column_outline(c, (level + 1).min(7), false).map_err(engine)?;
            if collapsed {
                ws.set_column_hidden(c, true).map_err(engine)?;
            }
        }
        if collapsed {
            // Excel marks the collapse on the column after the group (summary right)
            let after = first.max(last) + 1;
            let (level, _) = ws.column_outline(after);
            ws.set_column_outline(after, level, true).map_err(engine)?;
        }
        Ok(())
    }

    /// Every column's outline level and collapsed flag back to none (Excel's
    /// `Cells.ClearOutline`; hidden columns stay hidden).
    pub fn clear_outline(&mut self, sheet: u32) -> Result<(), ModelError> {
        let ws = self.model.workbook.worksheet_mut(sheet).map_err(engine)?;
        for c in ws.cols.iter_mut() {
            c.outline_level = 0;
            c.collapsed = false;
        }
        Ok(())
    }

    /// The column outline groups of a sheet: `(first, last, level, collapsed)`
    /// runs of equal level, collapsed when the run's columns are hidden.
    pub fn column_groups(&self, sheet: u32) -> Result<Vec<(i32, i32, u8, bool)>, ModelError> {
        let ws = self.model.workbook.worksheet(sheet).map_err(engine)?;
        let mut cols: Vec<(i32, u8, bool)> = Vec::new();
        for c in &ws.cols {
            if c.outline_level > 0 {
                for k in c.min..=c.max.min(c.min + 16_384) {
                    cols.push((k, c.outline_level, c.hidden));
                }
            }
        }
        cols.sort_unstable();
        let mut out: Vec<(i32, i32, u8, bool)> = Vec::new();
        for (k, level, hidden) in cols {
            match out.last_mut() {
                Some(g) if g.1 + 1 == k && g.2 == level => {
                    g.1 = k;
                    g.3 = g.3 && hidden;
                }
                _ => out.push((k, k, level, hidden)),
            }
        }
        Ok(out)
    }

    pub fn set_style(&mut self, sheet: u32, addr: Addr, style: &Style) -> Result<(), ModelError> {
        self.model
            .set_cell_style(sheet, addr.row, addr.column, style)
            .map_err(engine)
    }

    /// Writes a value. Text is stored as text, never parsed (`"0025"` stays
    /// `0025`, `"=x"` stays the text `=x`); the engine gives number-like text
    /// a quote prefix, as Excel does when one types `'0025`.
    pub fn set_value(&mut self, sheet: u32, addr: Addr, value: &Value) -> Result<(), ModelError> {
        let (row, column) = (addr.row, addr.column);
        match value {
            Value::Empty => return self.clear(sheet, Area { first: addr, last: addr }),
            Value::Number(n) => {
                if !n.is_finite() {
                    return Err(ModelError::Engine(format!("{n} is not a finite number")));
                }
                self.model.update_cell_with_number(sheet, row, column, *n)
            }
            Value::Text(s) => self.model.update_cell_with_text(sheet, row, column, s),
            Value::Bool(b) => self.model.update_cell_with_bool(sheet, row, column, *b),
            Value::Error(code) => {
                if get_error_by_english_name(code).is_none() {
                    return Err(ModelError::Engine(format!("{code:?} is not an error code")));
                }
                self.model
                    .set_user_input(sheet, row, column, code.to_string())
            }
        }
        .map_err(engine)?;
        self.stale = true;
        Ok(())
    }

    /// Writes a formula (`=A1*2`; the leading `=` is added when missing).
    /// `_xlfn.`/`_xlpm.` spellings are accepted; the formula reads back in
    /// user spelling.
    pub fn set_formula(&mut self, sheet: u32, addr: Addr, formula: &str) -> Result<(), ModelError> {
        let f = formula.trim();
        let f = if f.starts_with('=') {
            f.to_string()
        } else {
            format!("={f}")
        };
        self.model
            .set_user_input(sheet, addr.row, addr.column, f)
            .map_err(engine)?;
        self.stale = true;
        Ok(())
    }

    /// Clears the contents (not the styles) of `area`.
    pub fn clear(&mut self, sheet: u32, area: Area) -> Result<(), ModelError> {
        self.model
            .range_clear_contents(&EngineArea {
                sheet,
                row: area.first.row,
                column: area.first.column,
                width: area.columns(),
                height: area.rows(),
            })
            .map_err(engine)?;
        self.stale = true;
        Ok(())
    }

    /// Every stored cell of a sheet in row-major order, with its style index
    /// (style-only and spill cells included).
    pub fn cells(&self, sheet: u32) -> Result<Vec<(Addr, i32)>, ModelError> {
        let ws = self.model.workbook.worksheet(sheet).map_err(engine)?;
        let mut out: Vec<(Addr, i32)> = ws
            .sheet_data
            .cells()
            .map(|(row, column, cell)| (Addr::new(row, column), cell.get_style()))
            .collect();
        out.sort_by_key(|(a, _)| *a);
        Ok(out)
    }

    /// The smallest area holding every cell with content (values, formulas,
    /// spills — not style-only cells), `None` for an empty sheet. What
    /// Excel's "last cell with something in it" means.
    pub fn used_area(&self, sheet: u32) -> Result<Option<Area>, ModelError> {
        let ws = self.model.workbook.worksheet(sheet).map_err(engine)?;
        let mut area: Option<Area> = None;
        for (row, column, cell) in ws.sheet_data.cells() {
            if matches!(cell.as_ref(), Cell::EmptyCell { .. }) {
                continue;
            }
            let a = Addr::new(row, column);
            area = Some(match area {
                None => Area { first: a, last: a },
                Some(x) => Area::spanning(
                    Addr::new(x.first.row.min(row), x.first.column.min(column)),
                    Addr::new(x.last.row.max(row), x.last.column.max(column)),
                ),
            });
        }
        Ok(area)
    }

    /// The values of `area`, row by row (recalculating first if needed).
    pub fn values(&mut self, sheet: u32, area: Area) -> Result<Vec<Vec<Value>>, ModelError> {
        self.ensure_calculated();
        let mut rows = Vec::with_capacity(area.rows() as usize);
        for row in area.first.row..=area.last.row {
            let mut line = Vec::with_capacity(area.columns() as usize);
            for column in area.first.column..=area.last.column {
                line.push(self.value_now(sheet, Addr::new(row, column))?);
            }
            rows.push(line);
        }
        Ok(rows)
    }

    // ---- defined names --------------------------------------------------

    pub fn names(&self) -> Vec<Name> {
        let sheets = self.sheet_names();
        self.model
            .get_defined_name_list()
            .into_iter()
            .map(|e| Name {
                name: e.name,
                scope: e.scope.and_then(|s| sheets.get(s as usize).cloned()),
                formula: e.formula,
            })
            .collect()
    }

    /// Creates or replaces a name. `formula` as Excel writes it
    /// (`Sheet1!$A$1`, `=0.21`; a leading `=` is accepted).
    pub fn set_name(&mut self, name: &str, formula: &str, scope: Option<&str>) -> Result<(), ModelError> {
        let scope = scope.map(|s| self.sheet_index(s)).transpose()?;
        let mut entry = DefinedNameEntry::new(name, formula.trim_start_matches('='));
        if let Some(s) = scope {
            entry = entry.scope(s);
        }
        let exists = self
            .model
            .get_defined_name_list()
            .iter()
            .any(|e| e.name.eq_ignore_ascii_case(name) && e.scope == scope);
        if exists {
            self.model.update_defined_name(name, scope, entry).map_err(engine)?;
        } else {
            self.model.new_defined_name(entry).map_err(engine)?;
        }
        self.stale = true;
        Ok(())
    }

    pub fn delete_name(&mut self, name: &str, scope: Option<&str>) -> Result<(), ModelError> {
        let scope = scope.map(|s| self.sheet_index(s)).transpose()?;
        self.model.delete_defined_name(name, scope).map_err(engine)?;
        self.stale = true;
        Ok(())
    }

    // ---- files ----------------------------------------------------------

    /// Writes the model as `.xlsx` through the fork's writer: the exporter of work born in Ábaco,
    /// never the save path of a foreign file. Atomic, with a `.bak` of what it replaces.
    pub fn export(&mut self, path: &Path) -> Result<(), ModelError> {
        self.ensure_calculated();
        crate::files::atomic_write(path, |file| {
            ironcalc::export::save_xlsx_to_writer(&self.model, std::io::BufWriter::new(file))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .map_err(|e| ModelError::file(path, e))
    }
}

fn formula_value(v: &FormulaValue) -> Value {
    match v {
        FormulaValue::Unevaluated => Value::Empty,
        FormulaValue::Boolean(b) => Value::Bool(*b),
        FormulaValue::Number(n) => Value::Number(*n),
        FormulaValue::Text(s) => Value::Text(s.clone()),
        FormulaValue::Error { ei, .. } => Value::Error(ei.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Addr {
        Addr::parse(s).unwrap()
    }

    #[test]
    fn values_formulas_and_recalc_after_edits() {
        let mut wb = Workbook::new("t").unwrap();
        wb.set_value(0, a("A1"), &Value::Number(21.0)).unwrap();
        wb.set_formula(0, a("B1"), "A1*2").unwrap();
        assert_eq!(wb.value(0, a("B1")).unwrap(), Value::Number(42.0));
        // an edit is seen by the next read
        wb.set_value(0, a("A1"), &Value::Number(1.0)).unwrap();
        assert!(wb.is_stale());
        assert_eq!(wb.value(0, a("B1")).unwrap(), Value::Number(2.0));
        assert_eq!(wb.formula(0, a("B1")).unwrap().as_deref(), Some("=A1*2"));
        assert_eq!(wb.formula(0, a("A1")).unwrap(), None);
        assert_eq!(wb.value(0, a("Z99")).unwrap(), Value::Empty);
    }

    #[test]
    fn text_stays_text_and_errors_are_errors() {
        let mut wb = Workbook::new("t").unwrap();
        wb.set_value(0, a("A1"), &Value::Text("0025".into())).unwrap();
        wb.set_value(0, a("A2"), &Value::Text("=x".into())).unwrap();
        wb.set_value(0, a("A3"), &Value::Error("#N/A".into())).unwrap();
        wb.set_value(0, a("A4"), &Value::Bool(true)).unwrap();
        wb.set_formula(0, a("B1"), "=A1+0").unwrap();
        wb.set_formula(0, a("B2"), "=1/0").unwrap();
        assert_eq!(wb.value(0, a("A1")).unwrap(), Value::Text("0025".into()));
        assert_eq!(wb.value(0, a("A2")).unwrap(), Value::Text("=x".into()));
        assert_eq!(wb.value(0, a("A3")).unwrap(), Value::Error("#N/A".into()));
        assert_eq!(wb.value(0, a("A4")).unwrap(), Value::Bool(true));
        assert_eq!(wb.value(0, a("B1")).unwrap(), Value::Number(25.0));
        assert_eq!(wb.value(0, a("B2")).unwrap(), Value::Error("#DIV/0!".into()));
        assert!(wb.set_value(0, a("A5"), &Value::Error("#NOPE".into())).is_err());
        assert!(wb.set_value(0, a("A5"), &Value::Number(f64::NAN)).is_err());
    }

    #[test]
    fn xlfn_spelling_is_accepted_and_reads_back_in_user_spelling() {
        let mut wb = Workbook::new("t").unwrap();
        wb.set_formula(0, a("A1"), "=_xlfn.LET(_xlpm.x,21,_xlpm.x*2)").unwrap();
        assert_eq!(wb.value(0, a("A1")).unwrap(), Value::Number(42.0));
        assert_eq!(wb.formula(0, a("A1")).unwrap().as_deref(), Some("=LET(x,21,x*2)"));
    }

    #[test]
    fn sheets_names_and_unknown_sheets() {
        let mut wb = Workbook::new("t").unwrap();
        assert_eq!(wb.sheet_names(), vec!["Sheet1"]);
        let i = wb.add_sheet("R1.S1.C1").unwrap();
        assert_eq!(i, 1);
        assert_eq!(wb.sheet_index("r1.s1.c1").unwrap(), 1);
        assert!(matches!(wb.sheet_index("nope"), Err(ModelError::UnknownSheet(_))));
        wb.rename_sheet("Sheet1", "Cover").unwrap();
        assert_eq!(wb.sheet_names(), vec!["Cover", "R1.S1.C1"]);
        wb.set_value(1, a("B2"), &Value::Number(7.0)).unwrap();
        wb.set_name("Seven", "'R1.S1.C1'!$B$2", None).unwrap();
        wb.set_formula(0, a("A1"), "=Seven*2").unwrap();
        assert_eq!(wb.value(0, a("A1")).unwrap(), Value::Number(14.0));
        let names = wb.names();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].name, "Seven");
        assert_eq!(names[0].scope, None);
        wb.set_name("Seven", "'R1.S1.C1'!$B$2*10", None).unwrap();
        assert_eq!(wb.value(0, a("A1")).unwrap(), Value::Number(140.0));
        wb.delete_name("Seven", None).unwrap();
        assert!(wb.names().is_empty());
        wb.delete_sheet("Cover").unwrap();
        assert_eq!(wb.sheet_names(), vec!["R1.S1.C1"]);
    }

    #[test]
    fn used_area_ignores_style_only_cells_and_values_are_row_major() {
        let mut wb = Workbook::new("t").unwrap();
        assert_eq!(wb.used_area(0).unwrap(), None);
        let bold = {
            let mut s = wb.style(0, a("A1")).unwrap();
            s.font.b = true;
            s
        };
        wb.set_style(0, a("H40"), &bold).unwrap();
        wb.set_value(0, a("B2"), &Value::Number(1.0)).unwrap();
        wb.set_value(0, a("C3"), &Value::Text("x".into())).unwrap();
        assert_eq!(wb.used_area(0).unwrap(), Some(Area::parse("B2:C3").unwrap()));
        assert!(wb.style(0, a("H40")).unwrap().font.b);
        let v = wb.values(0, Area::parse("B2:C3").unwrap()).unwrap();
        assert_eq!(
            v,
            vec![
                vec![Value::Number(1.0), Value::Empty],
                vec![Value::Empty, Value::Text("x".into())]
            ]
        );
        wb.clear(0, Area::parse("B2:C3").unwrap()).unwrap();
        assert_eq!(wb.used_area(0).unwrap(), None);
    }
}

#[cfg(test)]
mod column_width_tests {
    use super::{column_width_of_stored, stored_of_column_width};

    #[test]
    fn stored_widths_and_column_widths_convert_as_excel_does() {
        // what a real workbook stores, and what Excel's ColumnWidth answers
        assert_eq!(column_width_of_stored(12.5703125), 11.86);
        assert_eq!(stored_of_column_width(11.86), 12.5703125);
        // Excel's default column: 8.43 characters = 64 px
        assert_eq!(stored_of_column_width(8.43), 9.140625);
        assert_eq!(column_width_of_stored(9.140625), 8.43);
        assert_eq!(stored_of_column_width(0.0), 0.0);
        for chars in [1.0, 8.43, 11.86, 12.0, 20.0, 45.71] {
            assert_eq!(column_width_of_stored(stored_of_column_width(chars)), chars, "{chars}");
        }
    }
}
