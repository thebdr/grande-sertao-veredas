//! A division written over two lines: a divisor of zero faults at the operation's first line and column.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

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
pub struct Lines {
    n: u8,
}

impl Program for Lines {
    type Inputs = Inputs;
    type Outputs = Outputs;

    #[rustfmt::skip]
    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.n = self.n.wrapping_add(1);
        o.v = i.a
            / i.b;
    }
}
