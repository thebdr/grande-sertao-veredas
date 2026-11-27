//! A conformance program: references to constants read inside their own frames, as a program reads a table of
//! constants: a promoted `&FAST` and `&1` and a `const R: &T`, each read where it is taken, passed down to a function
//! that reads it and to one that returns a reference derived from its parameters, and read after other calls.
#![forbid(unsafe_code)]

use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub fast: bool,
    pub k: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub temp: u16,
    pub mins: u8,
    pub one: u8,
    pub limit: u16,
    pub sum: u16,
    pub n: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recipe {
    pub temp: u16,
    pub mins: u8,
}

const SLOW: Recipe = Recipe { temp: 180, mins: 40 };
const FAST: Recipe = Recipe { temp: 220, mins: 15 };
const DEFAULT: &Recipe = &SLOW;

/// A reference returned, derived from the parameters: the constants stay in the caller's frame.
fn pick<'a>(fast: bool, a: &'a Recipe, b: &'a Recipe) -> &'a Recipe {
    if fast { a } else { b }
}

fn temp_of(r: &Recipe) -> u16 {
    r.temp
}

/// References to constants taken and read in this function's own frame, its result a value.
fn limit(k: u8) -> u16 {
    let r = if k > 127 { &FAST } else { &SLOW };
    r.temp + temp_of(DEFAULT)
}

fn bump(n: &mut u8, by: &u8) {
    *n = n.wrapping_add(*by);
}

#[derive(Debug, Default)]
pub struct ConstRefs {
    n: u8,
}

impl Program for ConstRefs {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        let r = pick(i.fast, &FAST, &SLOW);
        let other = pick(i.k % 3 == 0, &FAST, DEFAULT);
        let one: &u8 = &1;
        bump(&mut self.n, &1);
        o.limit = limit(i.k);
        o.temp = r.temp;
        o.mins = other.mins;
        o.one = *one;
        o.sum = temp_of(&FAST) + temp_of(r) + temp_of(DEFAULT);
        o.n = self.n;
    }
}
