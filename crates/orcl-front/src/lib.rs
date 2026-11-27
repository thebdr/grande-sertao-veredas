//! # orcl-front - ORCL's front end
//!
//! Makes the program `orcl-interp` runs from an ORCL program's crates, while rustc compiles them
//! (`docs/orcl/README.md`, "The front end"; `docs/orcl/front-end.md`).
//!
//! On stable this crate is a shell: it names the nightly the front end is pinned to, [`NIGHTLY`], and nothing else,
//! so the workspace builds and tests on stable (`docs/questions.md` Q221). The crate's `rust-toolchain` names the
//! same nightly, and a test holds the two to one another.
//!
//! Behind the feature `driver`, which only that nightly builds, the binary `orcl-front-driver` is run as the compiler
//! of a program's crates: it compiles every crate it is given with `-Zalways-encode-mir -Zmir-opt-level=0`, and for
//! the program's own crate, named by `--orcl-program` (or `ORCL_FRONT_PROGRAM`) and built as a library, it lowers
//! the crate's `orcl_std::Program` implementation through `rustc_public` and writes the program file `--orcl-out`
//! names (or `ORCL_FRONT_OUT`), under a header naming `orcl_std::LANGUAGE`, [`NIGHTLY`] and the revision
//! `--orcl-revision` (or `ORCL_FRONT_REVISION`) gives. What the interpreter cannot run is refused by name and source
//! location, and no file is written. Every crate it compiles depends on the three variables, so cargo rebuilds it when
//! one changes; a wrapped build runs in a target directory of its own, never one a plain build shares.
#![cfg_attr(feature = "driver", feature(rustc_private))]
#![forbid(unsafe_code)]
#![deny(clippy::panic, clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

/// The nightly toolchain the front end is pinned to (`docs/questions.md` Q222): the one whose `rustc_public` its
/// driver is built against, and the one every program file's header names.
pub const NIGHTLY: &str = "nightly-2026-11-21";

#[cfg(feature = "driver")]
extern crate rustc_driver;
#[cfg(feature = "driver")]
extern crate rustc_interface;
#[cfg(feature = "driver")]
extern crate rustc_middle;
#[cfg(feature = "driver")]
extern crate rustc_public;
#[cfg(feature = "driver")]
extern crate rustc_session;
#[cfg(feature = "driver")]
extern crate rustc_span;
#[cfg(feature = "driver")]
extern crate rustc_structures;

#[cfg(feature = "driver")]
mod body;
#[cfg(feature = "driver")]
mod consts;
#[cfg(feature = "driver")]
mod defs;
#[cfg(feature = "driver")]
pub mod driver;
#[cfg(feature = "driver")]
mod error;
#[cfg(feature = "driver")]
mod lower;
#[cfg(feature = "driver")]
mod names;
#[cfg(feature = "driver")]
mod types;

#[cfg(feature = "driver")]
pub use error::{Error, Location, Refusal, Refused};
