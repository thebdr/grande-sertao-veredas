//! A program the front end refuses: a statement the interpreter does not have, an assumption.
#![feature(core_intrinsics)]
#![allow(internal_features)]
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        unsafe { core::intrinsics::assume(self.x < 200) };
        *o = *i;
    }
}
