//! A program the front end refuses: `usize` as the program declares it, a named variable's, behind an array, an
//! `Option`, a tuple and a reference.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        let held: [Option<(u8, &usize)>; 2] = [None, None];
        *o = match held[0] {
            Some((v, _)) => v,
            None => i.wrapping_add(self.x),
        };
    }
}
