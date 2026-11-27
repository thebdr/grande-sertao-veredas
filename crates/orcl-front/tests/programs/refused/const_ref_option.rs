//! A program the front end refuses: a `const R: &T`, a reference to a constant, in a function whose result holds a
//! reference without being one, an `Option<&'static Recipe>`.
use orcl_std::{Program, Scan};

#[derive(Clone, Copy)]
pub struct Recipe {
    temp: u16,
}

const SLOW: Recipe = Recipe { temp: 180 };
const DEFAULT: &Recipe = &SLOW;

fn find(k: u8) -> Option<&'static Recipe> {
    if k == 0 { Some(DEFAULT) } else { None }
}

#[derive(Default)]
pub struct P {
    x: u8,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u16;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u16) {
        self.x = *i;
        *o = match find(*i) {
            Some(r) => r.temp,
            None => 0,
        };
    }
}
