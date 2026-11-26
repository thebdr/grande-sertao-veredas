//! A program the front end refuses: `usize` as the program declares it, a type outside ORCL's.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
    count: usize,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        self.count = self.count.wrapping_add(1);
        *o = *i;
    }
}
