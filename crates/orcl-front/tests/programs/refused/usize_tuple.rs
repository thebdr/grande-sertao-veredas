//! A program the front end refuses: `usize` as the program declares it, inside a tuple field.
use orcl_std::{Program, Scan};

pub struct P {
    x: u8,
    pair: (usize, u8),
}

impl Default for P {
    fn default() -> P {
        P { x: 0, pair: (0, 0) }
    }
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        self.pair.0 = self.pair.0.wrapping_add(1);
        *o = *i;
    }
}
