//! A program the front end refuses: `usize` as the program declares it, inside an `Option` field.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
    last: Option<usize>,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        if self.last.is_none() {
            self.x = 1;
        }
        *o = *i;
    }
}
