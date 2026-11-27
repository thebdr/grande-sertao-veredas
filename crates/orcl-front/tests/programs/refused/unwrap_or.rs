//! Refused: an inherent Option operation outside the list.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    last: Option<u8>,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        *o = self.last.unwrap_or(7);
        self.last = Some(*i);
    }
}
