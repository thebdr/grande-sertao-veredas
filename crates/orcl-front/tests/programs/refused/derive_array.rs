//! A program the front end refuses: a derived `Default` reaching an array's, off the closed list, refused where the
//! field is.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
    table: [u8; 4],
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        *o = i.wrapping_add(self.table[0]);
    }
}
