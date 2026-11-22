//! Ábaco, the engineering spreadsheet of Veredas 2 (`docs/abaco-plan.md`).
//!
//! The engine is Ábaco's fork of IronCalc, at the repository's `forks/ironcalc` (its record in
//! `forks/ironcalc/VEREDAS-FORK.md`); this crate adds policy, never engine behaviour. What is here so
//! far (phase 1, "the engine at parity"):
//!
//! - [`links`]: a workbook that links to other workbooks opened under a policy that decides whether
//!   the person is asked how its links resolve, from the cache the file carries or from the linked
//!   workbooks opened beside it;
//! - [`linkdiff`]: under a linked resolution, one log line per external cell whose live value
//!   differs from the cache.

pub mod linkdiff;
pub mod links;
