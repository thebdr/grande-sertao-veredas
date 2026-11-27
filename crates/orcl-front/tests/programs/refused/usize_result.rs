//! A program the front end refuses: `usize` as the program declares it, a function's result.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
}

fn width() -> usize {
    4
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        *o = i.wrapping_add(width() as u8);
    }
}
