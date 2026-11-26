//! A program the front end refuses: an array of one value repeated past the copies the front end builds it from.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        let wide = [self.x; 70_000];
        *o = wide[1] + *i;
    }
}
