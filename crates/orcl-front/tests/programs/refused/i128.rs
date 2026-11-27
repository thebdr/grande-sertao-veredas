//! A program the front end refuses: `i128`, a type outside ORCL's, a field.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    total: i128,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        self.total += *i as i128;
        *o = self.total as u8;
    }
}
