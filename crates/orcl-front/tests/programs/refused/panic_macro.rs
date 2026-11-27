//! A program the front end refuses: what one of core's macros holds, refused where the program calls the macro.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        if *i == 200 {
            unreachable!("never two hundred");
        }
        *o = *i;
    }
}
