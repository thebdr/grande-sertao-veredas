//! Refused: a comparison through a generic bound, at u8.
use orcl_std::{Program, Scan};

fn above<T: PartialOrd>(a: T, b: T) -> bool {
    a > b
}

#[derive(Default)]
pub struct P;

impl Program for P {
    type Inputs = u8;
    type Outputs = bool;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut bool) {
        *o = above(*i, 100);
    }
}
