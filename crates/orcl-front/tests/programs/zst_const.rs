//! Constants of no byte, passed by value and matched: a one-variant fieldless enum held in a `const`, and an array, a
//! structure and a tuple made only of such enums (and of `()`). rustc gives each no byte; the interpreter holds each
//! such enum in one slot, its discriminant.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Both {
    pub first: Only,
    pub second: Only,
}

const ONLY: Only = Only::One;
const ONLIES: [Only; 3] = [Only::One, Only::One, Only::One];
const BOTH: Both = Both { first: Only::One, second: Only::One };
const PAIR: (Only, (), Only) = (Only::One, (), Only::One);

#[derive(Debug, Default)]
pub struct ZstConst {
    x: u8,
}

fn code(o: Only) -> u8 {
    match o {
        Only::One => 5,
    }
}

fn first(b: Both) -> Only {
    b.first
}

impl Program for ZstConst {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.x = self.x.wrapping_add(1);
        let pick = ONLIES[(i.a % 3) as usize];
        let both = code(first(BOTH)).wrapping_add(code(BOTH.second));
        let (head, (), tail) = PAIR;
        let pair = code(head).wrapping_add(code(tail));
        let sum = code(ONLY).wrapping_add(code(pick)).wrapping_add(both).wrapping_add(pair);
        o.v = i.a.wrapping_add(sum).wrapping_add(self.x);
    }
}
