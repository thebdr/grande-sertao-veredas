//! A program whose helper comes in by `include!`, as generated code does: the division in `part.rs`, where a divisor
//! of zero faults at its own file.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

include!("part.rs");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: i32,
    pub b: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub v: i32,
}

#[derive(Debug, Default)]
pub struct Include;

impl Program for Include {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        o.v = ratio(i.a, i.b);
    }
}
