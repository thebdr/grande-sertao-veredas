//! Refused: a `Duration` matched against constant patterns (`Duration::ZERO =>`), which Rust lowers to a test of its
//! fields.
#![forbid(unsafe_code)]
use core::time::Duration;
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct RDurMatch {
    n: u8,
}

impl Program for RDurMatch {
    type Inputs = u64;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u64, o: &mut u8) {
        let d = Duration::from_millis(*i % 3);
        self.n = match d {
            Duration::ZERO => 0,
            _ => 1,
        };
        *o = self.n;
    }
}
