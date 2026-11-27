//! Refused: an inherent integer operation outside the list.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P;

impl Program for P {
    type Inputs = i8;
    type Outputs = i8;

    fn scan(&mut self, _scan: &Scan, i: &i8, o: &mut i8) {
        *o = (*i).wrapping_neg();
    }
}
