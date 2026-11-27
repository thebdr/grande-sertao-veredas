//! A program the front end refuses: `usize` as the program declares it, the payload of the program's own enum named
//! `Option`, refused at the payload as any enum of the program's is, not taken for core's `Option` by its name.
use orcl_std::{Program, Scan};

pub mod mine {
    #[derive(Clone, Copy, Default)]
    pub enum Option {
        #[default]
        Nothing,
        Some(usize),
    }
}

#[derive(Default)]
pub struct P {
    held: mine::Option,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        if let mine::Option::Some(_) = self.held {
            *o = 1;
        }
        self.held = mine::Option::Nothing;
        *o = *i;
    }
}
