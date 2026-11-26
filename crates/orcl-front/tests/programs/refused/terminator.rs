//! A program the front end refuses: a terminator the interpreter does not have, inline assembly.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        unsafe { core::arch::asm!("nop") };
        *o = *i;
    }
}
