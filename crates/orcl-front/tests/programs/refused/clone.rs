//! Refused: a scalar cloned through Clone::clone, a trait outside the list.
use orcl_std::{Program, Scan};

#[derive(Clone, Default)]
pub struct Held {
    v: u8,
}

#[derive(Default)]
pub struct P {
    held: Held,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        let copy = self.held.clone();
        *o = copy.v;
        self.held.v = *i;
    }
}
