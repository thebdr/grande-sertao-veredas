//! veredas-comissionador: the testing platform of Veredas 2; screens say Comissionador.
//!
//! A plant's acceptance tests (`docs/comissionador-plan.md`): a test is one plain-text file
//! (`docs/test-schema.md`) that drives the machine through portal's gated writes, waits for and
//! measures what the plant does through portal's data, live or replayed over a stored window,
//! branches on what it measured, and ends every run in an acceptance report. Functions are
//! written once in function files and placed in a test as blocks.
//!
//! - [`lex`]: the tokens of a test or a function file, each with its place.
//! - [`parse`]: the stamp read by core's one header reader, then the file into the [`model`].
//! - [`model`]: a test and its functions as the engine runs them, every part with its place.
//! - [`check`]: a test and the function files it uses checked whole, every problem named.
//!
//! Nothing here names a device, a vendor or a product: a plant's tests are files like the
//! reference's, kept outside the suite's code.

use std::fmt;

pub mod check;
pub mod lex;
pub mod model;
pub mod parse;

pub use check::{Checked, Counts, UsedFile, check_functions, check_test, load_functions, load_test};
pub use model::{FunctionFile, Place, Span, TestFile};
pub use parse::{FUNCTIONS_FORMAT, TEST_FORMAT, VERSION, parse_functions, parse_test};

/// A problem a file has, named by its file, line and column (from 1, the column in characters).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl Problem {
    /// A problem at byte `at` of `text`.
    pub fn at(file: &str, text: &str, at: usize, message: impl Into<String>) -> Problem {
        let p = Place::of(text, at);
        Problem { file: file.to_string(), line: p.line, column: p.column, message: message.into() }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}:{}: {}", self.file, self.line, self.column, self.message)
    }
}

impl std::error::Error for Problem {}
