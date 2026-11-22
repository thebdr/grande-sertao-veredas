//! User-defined functions: a host-supplied hook for names the engine does
//! not know.
//!
//! A call to an unknown name (`=MINHA(A1)`) already parses, as a named
//! function call; at evaluation the engine looks it up as a defined-name
//! LAMBDA and answers `#NAME?` when there is none. With a [`UserFunctions`]
//! hook set on the model, that last step first asks the hook: when it
//! [`has`](UserFunctions::has) the name, the arguments are evaluated (a range
//! becomes a 2-D array of its cells' values), the hook is called, and its
//! answer is the cell's value. A name the hook does not have stays `#NAME?`,
//! exactly as without a hook. The lookup happens at evaluation time, so a
//! function registered after the workbook was loaded resolves on the next
//! `evaluate()`.
//!
//! The hook may be called more than once per evaluation (the engine can
//! abandon and restart a pass); it is never called with the arguments of an
//! abandoned pass.

use crate::calc_result::CalcResult;
use crate::expressions::parser::{ArrayNode, Node};
use crate::expressions::token::Error;
use crate::expressions::types::CellReferenceIndex;
use crate::model::Model;

/// A value passed to, or returned by, a user function.
#[derive(Debug, Clone, PartialEq)]
pub enum UserValue {
    /// An empty cell or an omitted argument.
    Empty,
    Number(f64),
    Text(String),
    Boolean(bool),
    Error(Error),
    /// A range or an array, row by row.
    Array(Vec<Vec<UserValue>>),
}

/// The host's functions, keyed by name (compared case-insensitively by the
/// host: the engine passes the name as the formula spells it).
pub trait UserFunctions {
    /// Whether `name` is one of the host's functions.
    fn has(&self, name: &str) -> bool;
    /// Calls `name`; an `Err` becomes `#VALUE!` with the message attached.
    fn call(&mut self, name: &str, args: Vec<UserValue>) -> Result<UserValue, String>;
}

/// Ranges larger than this many cells are refused as user-function
/// arguments (`#VALUE!`) rather than materialised (`A:A` is a million rows).
pub const MAX_ARGUMENT_CELLS: usize = 1_000_000;

impl Model<'_> {
    /// Sets (or, with `None`, removes) the user-function hook. Values already
    /// evaluated do not change until the next `evaluate`.
    pub fn set_user_functions(&mut self, functions: Option<Box<dyn UserFunctions>>) {
        self.user_functions = functions;
    }

    /// `None` when the hook does not know `name` (the caller answers
    /// `#NAME?`), else the call's result.
    pub(crate) fn call_user_function(
        &mut self,
        name: &str,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> Option<CalcResult> {
        if !self.user_functions.as_ref()?.has(name) {
            return None;
        }
        let mut values = Vec::with_capacity(args.len());
        for arg in args {
            let result = self.evaluate_node_in_context(arg, cell);
            values.push(self.user_value(result));
        }
        if self.evaluation.restart.is_some() || self.evaluation.unwinding {
            // The pass is being abandoned: its arguments are not to be seen.
            return Some(CalcResult::EmptyCell);
        }
        let mut hook = self.user_functions.take()?;
        let answer = hook.call(name, values);
        self.user_functions = Some(hook);
        Some(match answer {
            Ok(value) => to_calc(value, cell),
            Err(message) => CalcResult::new_error(Error::VALUE, cell, message),
        })
    }

    fn user_value(&mut self, result: CalcResult) -> UserValue {
        match result {
            CalcResult::String(s) => UserValue::Text(s),
            CalcResult::Number(n) => UserValue::Number(n),
            CalcResult::Boolean(b) => UserValue::Boolean(b),
            CalcResult::Error { error, .. } => UserValue::Error(error),
            CalcResult::EmptyCell | CalcResult::EmptyArg => UserValue::Empty,
            CalcResult::Lambda(_) => UserValue::Error(Error::VALUE),
            CalcResult::Array(rows) => UserValue::Array(
                rows.into_iter()
                    .map(|row| row.into_iter().map(from_array_node).collect())
                    .collect(),
            ),
            CalcResult::Range { left, right } => {
                if left.sheet != right.sheet {
                    return UserValue::Error(Error::VALUE);
                }
                // Clip whole-column/row references to the cells that exist.
                let (max_row, max_column) = match self.workbook.worksheet(left.sheet) {
                    Ok(ws) => {
                        let d = ws.dimension();
                        (d.max_row, d.max_column)
                    }
                    Err(_) => return UserValue::Error(Error::REF),
                };
                let last_row = right.row.min(max_row.max(left.row));
                let last_column = right.column.min(max_column.max(left.column));
                let cells = (last_row - left.row + 1).max(0) as usize
                    * (last_column - left.column + 1).max(0) as usize;
                if cells > MAX_ARGUMENT_CELLS {
                    return UserValue::Error(Error::VALUE);
                }
                let mut rows = Vec::new();
                for row in left.row..=last_row {
                    let mut line = Vec::new();
                    for column in left.column..=last_column {
                        let r = self.evaluate_cell(CellReferenceIndex {
                            sheet: left.sheet,
                            row,
                            column,
                        });
                        line.push(match r {
                            CalcResult::Range { .. } | CalcResult::Array(_) => {
                                UserValue::Error(Error::VALUE)
                            }
                            other => self.user_value(other),
                        });
                    }
                    rows.push(line);
                }
                UserValue::Array(rows)
            }
        }
    }
}

fn from_array_node(node: ArrayNode) -> UserValue {
    match node {
        ArrayNode::Boolean(b) => UserValue::Boolean(b),
        ArrayNode::Number(n) => UserValue::Number(n),
        ArrayNode::String(s) => UserValue::Text(s),
        ArrayNode::Error(e) => UserValue::Error(e),
        ArrayNode::Empty => UserValue::Empty,
    }
}

fn to_array_node(value: UserValue) -> ArrayNode {
    match value {
        UserValue::Empty => ArrayNode::Empty,
        UserValue::Number(n) => ArrayNode::Number(n),
        UserValue::Text(s) => ArrayNode::String(s),
        UserValue::Boolean(b) => ArrayNode::Boolean(b),
        UserValue::Error(e) => ArrayNode::Error(e),
        UserValue::Array(_) => ArrayNode::Error(Error::VALUE),
    }
}

fn to_calc(value: UserValue, cell: CellReferenceIndex) -> CalcResult {
    match value {
        UserValue::Empty => CalcResult::EmptyCell,
        UserValue::Number(n) if n.is_finite() => CalcResult::Number(n),
        UserValue::Number(_) => {
            CalcResult::new_error(Error::NUM, cell, "not a finite number".to_string())
        }
        UserValue::Text(s) => CalcResult::String(s),
        UserValue::Boolean(b) => CalcResult::Boolean(b),
        UserValue::Error(e) => CalcResult::new_error(e, cell, "user function".to_string()),
        UserValue::Array(rows) => CalcResult::Array(
            rows.into_iter()
                .map(|row| row.into_iter().map(to_array_node).collect())
                .collect(),
        ),
    }
}
