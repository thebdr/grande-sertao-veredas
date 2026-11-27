//! Refused: a union the program declares.
use orcl_std::{Program, Scan};

#[derive(Clone, Copy)]
pub union Word {
    raw: u32,
    level: f32,
}

impl Default for Word {
    fn default() -> Word {
        Word { raw: 0 }
    }
}

#[derive(Default)]
pub struct P {
    word: Word,
}

impl Program for P {
    type Inputs = u32;
    type Outputs = u32;

    fn scan(&mut self, _scan: &Scan, i: &u32, o: &mut u32) {
        self.word = Word { raw: *i };
        *o = 1;
    }
}
