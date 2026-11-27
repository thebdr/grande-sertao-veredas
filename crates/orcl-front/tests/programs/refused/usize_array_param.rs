//! A program the front end refuses: `usize` as the program declares it, a parameter's array element, the array
//! first met in the caller's temporary.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
}

fn first(at: [usize; 2]) -> u8 {
    (at[0] % 251) as u8
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        *o = first([*i as usize; 2]).wrapping_add(self.x);
    }
}
