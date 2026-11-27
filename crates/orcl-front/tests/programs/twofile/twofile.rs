//! A program in two files: its scan here, the division it calls in `helper.rs`, where a divisor of zero faults.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

mod helper;

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
pub struct Twofile {
    n: u8,
}

impl Program for Twofile {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.n = self.n.wrapping_add(1);
        o.v = helper::ratio(i.a, i.b);
    }
}
