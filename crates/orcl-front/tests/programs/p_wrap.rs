//! Constants of an enum whose niche wraps past its tag's greatest value (`Outer`'s `Y` at 255 and `Z` at 0, around
//! `E`'s range 1 to 254).
#![forbid(unsafe_code)]

use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub sel: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub v: u8,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum E {
    A = 1,
    B = 254,
}

#[derive(Clone, Copy, Debug)]
pub enum Outer {
    X(E),
    Y,
    Z,
}

const ALL: [Outer; 4] = [Outer::X(E::A), Outer::Y, Outer::Z, Outer::X(E::B)];

#[derive(Debug, Default)]
pub struct PWrap;

impl Program for PWrap {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        o.v = match ALL[(i.sel % 4) as usize] {
            Outer::X(E::A) => 1,
            Outer::X(E::B) => 2,
            Outer::Y => 3,
            Outer::Z => 4,
        };
    }
}
