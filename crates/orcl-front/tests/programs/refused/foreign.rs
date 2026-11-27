//! A program the front end refuses: a call the interpreter cannot make, into a foreign function the program's own
//! crate declares, which has no MIR and is no crate's built without the front end's flags.
use orcl_std::{Program, Scan};

unsafe extern "C" {
    safe fn abs(v: i32) -> i32;
}

#[derive(Default)]
pub struct P {
    x: i32,
}

impl Program for P {
    type Inputs = i32;
    type Outputs = i32;

    fn scan(&mut self, _scan: &Scan, i: &i32, o: &mut i32) {
        self.x = abs(*i);
        *o = self.x;
    }
}
