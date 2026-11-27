//! A program the front end refuses: a reference to a constant written out through a parameter, a `&mut` to a
//! reference (`fn set(out: &mut &'static Recipe, fast: bool)`), then read by the caller after a second call to it.
#![forbid(unsafe_code)]

use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub fast: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub temp: u16,
    pub other: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recipe {
    pub temp: u16,
    pub mins: u8,
}

const SLOW: Recipe = Recipe { temp: 180, mins: 40 };
const FAST: Recipe = Recipe { temp: 220, mins: 15 };

fn set(out: &mut &'static Recipe, fast: bool) {
    *out = if fast { &FAST } else { &SLOW };
}

#[derive(Debug, Default)]
pub struct Out {
    n: u8,
}

impl Program for Out {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        let mut r: &'static Recipe = &SLOW;
        set(&mut r, i.fast);
        let mut q: &'static Recipe = &SLOW;
        set(&mut q, !i.fast);
        o.other = q.mins;
        o.temp = r.temp;
        self.n = self.n.wrapping_add(1);
    }
}
