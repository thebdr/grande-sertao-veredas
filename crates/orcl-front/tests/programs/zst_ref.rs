//! A promoted reference to a one-variant fieldless enum, `&Only::One`: a reference to no byte.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub v: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Only {
    One,
}

#[derive(Debug, Default)]
pub struct ZstRef {
    x: u8,
}

fn code(o: &Only) -> u8 {
    match o {
        Only::One => 5,
    }
}

impl Program for ZstRef {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.x = self.x.wrapping_add(1);
        o.v = i.a.wrapping_add(code(&Only::One)).wrapping_add(self.x);
    }
}
