//! A sound program of a large state: a log of 200 000 samples, built from repeats the front end lowers (50 000 each).
// a state of 200 000 slots, built on the stack by its `Default` on purpose: the size the pre-write check must take
#![allow(clippy::large_unopt_stack_frames)]
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub v: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub oldest: u16,
    pub at: u32,
}

#[derive(Debug)]
pub struct RLarge {
    log: [[u16; 50_000]; 4],
    at: u32,
}

impl Default for RLarge {
    fn default() -> RLarge {
        RLarge { log: [[0; 50_000]; 4], at: 0 }
    }
}

impl Program for RLarge {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        o.oldest = self.log[(self.at / 50_000) as usize][(self.at % 50_000) as usize];
        self.log[(self.at / 50_000) as usize][(self.at % 50_000) as usize] = i.v;
        self.at = (self.at + 7_919) % 200_000;
        o.at = self.at;
    }
}
