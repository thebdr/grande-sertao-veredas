//! ORCL's O14, pinned: a function block is neither `Copy` nor `Clone`, so scanning a copy of one does not compile
//! (`docs/orcl/language/ownership.md`). Each block is tried three ways: scanned in place through `&mut`, which
//! compiles, then through a copy (E0507) and through a clone (E0599), which must not. The twin that compiles is
//! what keeps the other two honest: stable rustdoc does not check the error code a `compile_fail` names, so
//! without it an unrelated error (an import that no longer resolves) would keep the pin green. This module holds
//! no code; `cargo test -p orcl-std` runs its examples.
//!
//! `Ton`, scanned in place:
//!
//! ```
//! use orcl_std::{Ton, Scan, ms};
//! struct Guard { block: Ton }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let block = &mut self.block; block.scan(scan, true, ms(500)) }
//! }
//! ```
//!
//! `Ton`, scanned through a copy:
//!
//! ```compile_fail,E0507
//! use orcl_std::{Ton, Scan, ms};
//! struct Guard { block: Ton }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let mut copy = self.block; copy.scan(scan, true, ms(500)) }
//! }
//! ```
//!
//! `Ton`, scanned through a clone:
//!
//! ```compile_fail,E0599
//! use orcl_std::{Ton, Scan, ms};
//! struct Guard { block: Ton }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let mut copy = self.block.clone(); copy.scan(scan, true, ms(500)) }
//! }
//! ```
//!
//! `Tof`, scanned in place:
//!
//! ```
//! use orcl_std::{Tof, Scan, ms};
//! struct Guard { block: Tof }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let block = &mut self.block; block.scan(scan, true, ms(500)) }
//! }
//! ```
//!
//! `Tof`, scanned through a copy:
//!
//! ```compile_fail,E0507
//! use orcl_std::{Tof, Scan, ms};
//! struct Guard { block: Tof }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let mut copy = self.block; copy.scan(scan, true, ms(500)) }
//! }
//! ```
//!
//! `Tof`, scanned through a clone:
//!
//! ```compile_fail,E0599
//! use orcl_std::{Tof, Scan, ms};
//! struct Guard { block: Tof }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let mut copy = self.block.clone(); copy.scan(scan, true, ms(500)) }
//! }
//! ```
//!
//! `Tp`, scanned in place:
//!
//! ```
//! use orcl_std::{Tp, Scan, ms};
//! struct Guard { block: Tp }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let block = &mut self.block; block.scan(scan, true, ms(500)) }
//! }
//! ```
//!
//! `Tp`, scanned through a copy:
//!
//! ```compile_fail,E0507
//! use orcl_std::{Tp, Scan, ms};
//! struct Guard { block: Tp }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let mut copy = self.block; copy.scan(scan, true, ms(500)) }
//! }
//! ```
//!
//! `Tp`, scanned through a clone:
//!
//! ```compile_fail,E0599
//! use orcl_std::{Tp, Scan, ms};
//! struct Guard { block: Tp }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let mut copy = self.block.clone(); copy.scan(scan, true, ms(500)) }
//! }
//! ```
//!
//! `RTrig`, scanned in place:
//!
//! ```
//! use orcl_std::{RTrig, Scan, ms};
//! struct Guard { block: RTrig }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let block = &mut self.block; block.scan(true) }
//! }
//! ```
//!
//! `RTrig`, scanned through a copy:
//!
//! ```compile_fail,E0507
//! use orcl_std::{RTrig, Scan, ms};
//! struct Guard { block: RTrig }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let mut copy = self.block; copy.scan(true) }
//! }
//! ```
//!
//! `RTrig`, scanned through a clone:
//!
//! ```compile_fail,E0599
//! use orcl_std::{RTrig, Scan, ms};
//! struct Guard { block: RTrig }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let mut copy = self.block.clone(); copy.scan(true) }
//! }
//! ```
//!
//! `FTrig`, scanned in place:
//!
//! ```
//! use orcl_std::{FTrig, Scan, ms};
//! struct Guard { block: FTrig }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let block = &mut self.block; block.scan(true) }
//! }
//! ```
//!
//! `FTrig`, scanned through a copy:
//!
//! ```compile_fail,E0507
//! use orcl_std::{FTrig, Scan, ms};
//! struct Guard { block: FTrig }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let mut copy = self.block; copy.scan(true) }
//! }
//! ```
//!
//! `FTrig`, scanned through a clone:
//!
//! ```compile_fail,E0599
//! use orcl_std::{FTrig, Scan, ms};
//! struct Guard { block: FTrig }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let mut copy = self.block.clone(); copy.scan(true) }
//! }
//! ```
//!
//! `Ctu`, scanned in place:
//!
//! ```
//! use orcl_std::{Ctu, Scan, ms};
//! struct Guard { block: Ctu }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let block = &mut self.block; block.scan(true, false, 3) }
//! }
//! ```
//!
//! `Ctu`, scanned through a copy:
//!
//! ```compile_fail,E0507
//! use orcl_std::{Ctu, Scan, ms};
//! struct Guard { block: Ctu }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let mut copy = self.block; copy.scan(true, false, 3) }
//! }
//! ```
//!
//! `Ctu`, scanned through a clone:
//!
//! ```compile_fail,E0599
//! use orcl_std::{Ctu, Scan, ms};
//! struct Guard { block: Ctu }
//! impl Guard {
//!     fn scan(&mut self, scan: &Scan) -> bool { let _ = (scan, ms(0)); let mut copy = self.block.clone(); copy.scan(true, false, 3) }
//! }
//! ```
