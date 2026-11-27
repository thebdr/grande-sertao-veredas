//! A program the front end refuses (C-156's fourteenth refute round): references to constants returned from a
//! function, a promoted `&1` first and a table's `&FAST` after, and read after another call, which the interpreter
//! would hold in the returning function's frame.
#![forbid(unsafe_code)]

use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub sel: u8,
    pub k: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub a: u8,
    pub b: u8,
    pub temp: u16,
    pub mins: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recipe {
    pub temp: u16,
    pub mins: u8,
}

const SLOW: Recipe = Recipe { temp: 180, mins: 40 };
const FAST: Recipe = Recipe { temp: 220, mins: 15 };

fn recipe(fast: bool) -> &'static Recipe {
    if fast { &FAST } else { &SLOW }
}

fn one() -> &'static u8 {
    &1
}

fn two() -> &'static u8 {
    &2
}

#[derive(Debug, Default)]
pub struct Dangle {
    n: u8,
}

impl Program for Dangle {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        let a = one();
        let b = two();
        o.a = *a;
        o.b = *b;
        let r = recipe(i.sel % 2 == 0);
        let other = recipe(i.k % 3 == 0);
        o.temp = r.temp;
        o.mins = other.mins;
        self.n = self.n.wrapping_add(1);
    }
}
