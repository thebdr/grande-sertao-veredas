//! Constants whose fields rustc lays out in another order than they are declared, each field read on its own: a
//! tuple `(u8, u16, bool)` and a structure `{ a: u8, b: u32, c: u16 }`.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub sel: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub p0: u8,
    pub p1: u16,
    pub p2: bool,
    pub a: u8,
    pub b: u32,
    pub c: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cfg {
    pub a: u8,
    pub b: u32,
    pub c: u16,
}

const PAIR: (u8, u16, bool) = (7, 300, true);
const CFG: Cfg = Cfg { a: 9, b: 70_000, c: 513 };

#[derive(Debug, Default)]
pub struct Order {
    n: u8,
}

impl Program for Order {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.n = self.n.wrapping_add(i.sel);
        let p = PAIR;
        o.p0 = p.0.wrapping_add(self.n);
        o.p1 = p.1;
        o.p2 = p.2 && i.sel > 3;
        let c = CFG;
        o.a = c.a;
        o.b = c.b;
        o.c = c.c;
    }
}
