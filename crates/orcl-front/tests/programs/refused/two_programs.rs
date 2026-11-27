//! A program the front end refuses: a crate with two `orcl_std::Program` implementations.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        *o = *i;
    }
}

#[derive(Default)]
pub struct Q {
    y: u8,
}

impl Program for Q {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        *o = i.wrapping_add(self.y);
    }
}
