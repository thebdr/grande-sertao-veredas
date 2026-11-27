//! A program whose scans fault: each of MIR's run-time checks a scan can fail, one a selector, for what the fault
//! names, its kind and its place.
#![forbid(unsafe_code)]

use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub sel: u8,
    pub a: i32,
    pub b: i32,
    pub idx: u8,
    pub c: u8,
    pub sh: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub v: i32,
    pub w: u8,
}

#[derive(Debug)]
pub struct Faults {
    table: [i32; 4],
}

impl Default for Faults {
    fn default() -> Faults {
        Faults { table: [1, 2, 3, 4] }
    }
}

impl Program for Faults {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        if i.sel == 0 {
            o.v = i.a % i.b;
        }
        if i.sel == 1 {
            o.v = i.a + i.b;
        }
        if i.sel == 2 {
            o.v = -i.a;
        }
        if i.sel == 3 {
            o.v = self.table[i.idx as usize];
        }
        if i.sel == 4 {
            o.v = i.a * i.b;
        }
        if i.sel == 5 {
            o.v = i.a - i.b;
        }
        if i.sel == 6 {
            o.w = i.c << i.sh;
        }
        if i.sel == 7 {
            o.v = i.a / i.b;
        }
    }
}
