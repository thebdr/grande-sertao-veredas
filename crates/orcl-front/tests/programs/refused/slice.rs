//! A program the front end refuses: a slice, a type outside ORCL's.
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
        let view: &[u8] = &self.table;
        let _ = view;
        *o = *i;
    }
}
