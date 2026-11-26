//! A program the front end refuses: `String`, a type outside ORCL's.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        let name = String::new();
        *o = name.len() as u8 + *i;
    }
}
