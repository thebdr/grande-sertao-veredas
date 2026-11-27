//! A program the front end refuses: a reference to a constant written into the state, a field
//! `Option<&'static Recipe>` the scan's `&mut Self` reaches, and read on the next scan.
#![forbid(unsafe_code)]

use orcl_std::{Program, Scan};

#[derive(Clone, Copy)]
pub struct Recipe {
    temp: u16,
}

const SLOW: Recipe = Recipe { temp: 180 };
const FAST: Recipe = Recipe { temp: 220 };

pub struct P {
    recipe: Option<&'static Recipe>,
}

impl Default for P {
    fn default() -> P {
        P { recipe: None }
    }
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u16;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u16) {
        *o = match self.recipe {
            Some(r) => r.temp,
            None => 0,
        };
        self.recipe = Some(if *i > 127 { &FAST } else { &SLOW });
    }
}
