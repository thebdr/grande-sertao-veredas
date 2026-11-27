//! Refused: a usize behind a mutable reference parameter.
use orcl_std::{Program, Scan};

fn bump(at: &mut usize) -> u8 {
    *at += 1;
    *at as u8
}

#[derive(Default)]
pub struct P;

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        *o = bump(&mut (*i as usize));
    }
}
