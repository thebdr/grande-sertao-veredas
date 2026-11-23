//! Ábaco, the engineering spreadsheet of Veredas 2 (`docs/abaco-plan.md`).
//!
//! The engine is Ábaco's fork of IronCalc, at the repository's `forks/ironcalc` (its record in
//! `forks/ironcalc/VEREDAS-FORK.md`); this crate adds policy, never engine behaviour.
//!
//! - [`links`]: a workbook that links to other workbooks opened under a policy that decides whether
//!   the person is asked how its links resolve, from the cache the file carries or from the linked
//!   workbooks opened beside it; [`linkdiff`]: under a linked resolution, one log line per external
//!   cell whose live value differs from the cache.
//! - [`book`]: a workbook's cells, sheets and names in A1 terms ([`a1`]), over the engine's model.
//! - [`patch`]: byte-faithful edits of a workbook Ábaco did not write, every part no edit touched
//!   copied as it was; [`files`]: the atomic write it and the export share.
//! - [`abacofile`]: the `.abaco` file, a zip of stamped TOML parts, a workbook born in Ábaco and
//!   the session around it ([`workspace`]: the foreign files it works on, its tools' records, the
//!   log).

pub mod a1;
pub mod abacofile;
pub mod book;
pub mod files;
pub mod linkdiff;
pub mod links;
pub mod patch;
pub mod workspace;

pub use a1::{Addr, Area};
pub use book::{Content, ModelError, Name, Origin, Value, Workbook};
