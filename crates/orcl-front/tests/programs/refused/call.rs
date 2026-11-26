//! A program the front end refuses: a call the interpreter cannot make, into a crate neither the program's nor core.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        std::thread::yield_now();
        *o = *i;
    }
}
