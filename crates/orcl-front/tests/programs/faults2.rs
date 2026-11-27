//! A program whose scans fault on the run-time checks the `faults` program leaves out, a shift right past the width
//! and a remainder's overflow, each on its own selector, with a division's overflow beside them.
#![forbid(unsafe_code)]

use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub sel: u8,
    pub a: i32,
    pub b: i32,
    pub sh: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub v: i32,
    pub w: u16,
}

#[derive(Debug, Default)]
pub struct PFaults {
    x: u16,
}

impl Program for PFaults {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        if i.sel == 0 {
            o.w = self.x >> i.sh;
        }
        if i.sel == 1 {
            o.v = i.a % i.b;
        }
        if i.sel == 2 {
            o.v = i.a / i.b;
        }
        self.x = self.x.wrapping_add(1);
    }
}
