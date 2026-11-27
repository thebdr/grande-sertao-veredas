//! A program the front end refuses: `usize` as the program declares it, an array's element.
use orcl_std::{Program, Scan};

pub struct P {
    x: u8,
    counts: [usize; 2],
}

impl Default for P {
    fn default() -> P {
        P { x: 0, counts: [0; 2] }
    }
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        self.counts[0] = self.counts[0].wrapping_add(1);
        *o = *i;
    }
}
