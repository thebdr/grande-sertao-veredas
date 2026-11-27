//! A program the front end refuses: an `i128` inside core's `Option`, a variable the program declares.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        let wide: Option<i128> = None;
        if wide.is_none() {
            self.x = 1;
        }
        *o = *i;
    }
}
