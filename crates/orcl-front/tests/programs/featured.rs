//! A program a cargo feature changes, round 13's probe: built with its package's `alt` feature, it adds 1000 to its
//! output; with the default features it copies its input.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub v: i32,
}

#[derive(Debug, Default)]
pub struct Featured {
    n: u8,
}

impl Program for Featured {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.n = self.n.wrapping_add(1);
        #[cfg(feature = "alt")]
        {
            o.v = i.a.wrapping_add(1000);
        }
        #[cfg(not(feature = "alt"))]
        {
            o.v = i.a;
        }
    }
}
