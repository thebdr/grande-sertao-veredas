//! A program the front end refuses: a value the interpreter does not have, a transmutation.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        let signed: i8 = unsafe { core::mem::transmute::<u8, i8>(*i) };
        *o = signed as u8;
    }
}
