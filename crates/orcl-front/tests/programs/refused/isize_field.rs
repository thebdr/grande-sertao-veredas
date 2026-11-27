//! A program the front end refuses: `isize` as the program declares it, a field.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
    offset: isize,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        self.offset = self.offset.wrapping_add(1);
        *o = *i;
    }
}
