//! A program the front end refuses: a union inside core's `Option`, a field the program declares.
use orcl_std::{Program, Scan};

#[derive(Clone, Copy)]
pub union Word {
    a: u16,
    b: [u8; 2],
}

#[derive(Default)]
pub struct P {
    x: u8,
    word: Option<Word>,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        if self.word.is_none() {
            self.x = 1;
        }
        *o = *i;
    }
}
