//! A program the front end refuses: `usize` as the program declares it, a parameter of no name.
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Io {
    pub a: u8,
}

fn pick(_: usize, v: u8) -> u8 {
    v
}

#[derive(Debug, Default)]
pub struct P;

impl Program for P {
    type Inputs = Io;
    type Outputs = Io;

    fn scan(&mut self, _scan: &Scan, i: &Io, o: &mut Io) {
        o.a = pick(i.a as usize, i.a);
    }
}
