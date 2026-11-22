//! # orcl-std - the ORCL standard control library
//!
//! ORCL (Open Rust Control Language) is control logic written in a translatable subset of Rust. This crate is
//! the library every ORCL program builds on:
//! - the scan model: [`Scan`], [`Program`];
//! - IEC-style timers: [`Ton`], [`Tof`], [`Tp`];
//! - edge triggers: [`RTrig`], [`FTrig`];
//! - a counter: [`Ctu`];
//! - bit access on unsigned values: [`Bits`].
//!
//! Rules this crate keeps, and every ORCL program keeps:
//! - **no heap** - `no_std`, nothing allocated at run time;
//! - **no panics** - every operation has a defined result: time arithmetic saturates, and a bit index past the
//!   width reads `false` and writes nothing. The clippy lints below deny the usual panic sources;
//! - **time comes from the scan** - one clock reading per scan, carried in [`Scan`]. The same inputs then give
//!   the same outputs, whether on a softPLC, in a unit test with simulated time, or in a translated PLC program;
//! - **no interior mutability** - no `Cell` and no atomics, so a `&T` is read-only and a `static` is a constant.
//!   a program crate's `clippy.toml` lists the types, and `clippy::disallowed_types` below denies them. This
//!   crate carries no `clippy.toml` in Veredas 2 yet: law L7 refuses a TOML file without the suite's stamp, and
//!   clippy refuses any key it does not know (`docs/questions.md` Q225). Until that is ruled the rule is kept by
//!   convention here (DECISION PENDING);
//! - **function blocks are not copied** - a timer, an edge or a counter is state with an identity. It is neither
//!   `Copy` nor `Clone`, so scanning a copy by mistake does not compile:
//!
//! ```compile_fail,E0507
//! use orcl_std::{Scan, Ton, ms};
//!
//! struct Guard {
//!     delay: Ton,
//! }
//!
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan, open: bool) -> bool {
//!         let mut delay = self.delay; // error: cannot move out of `self.delay`
//!         delay.scan(scan, open, ms(500))
//!     }
//! }
//! ```
//!
//! Scanned in place, the same guard compiles (and so the example above fails for the copy alone):
//!
//! ```
//! use orcl_std::{Scan, Ton, ms};
//!
//! struct Guard {
//!     delay: Ton,
//! }
//!
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan, open: bool) -> bool {
//!         self.delay.scan(scan, open, ms(500))
//!     }
//! }
//! ```
//!
//! The last two are the ownership rules O13 and O14 in `docs/orcl/language/ownership.md`.
#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![deny(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_types
)]

mod bits;
mod counters;
mod edges;
mod o14;
mod scan;
mod timers;

pub use bits::Bits;
pub use core::time::Duration;
pub use counters::Ctu;
pub use edges::{FTrig, RTrig};
pub use scan::{Program, Scan, ms, s};
pub use timers::{Tof, Ton, Tp};
