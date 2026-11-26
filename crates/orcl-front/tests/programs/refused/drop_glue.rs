//! A program the front end refuses: a type with drop glue.
use orcl_std::{Program, Scan};

pub struct Guard(u8);

impl Drop for Guard {
    fn drop(&mut self) {}
}

#[derive(Default)]
pub struct P {
    x: u8,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        let guard = Guard(*i);
        *o = guard.0;
    }
}
