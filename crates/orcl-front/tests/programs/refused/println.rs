//! Refused: std's `println!`, where the program calls it.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    n: u8,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        println!("{}", *i);
        self.n = *i;
        *o = self.n;
    }
}
