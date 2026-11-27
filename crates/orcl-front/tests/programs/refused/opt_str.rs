//! A program the front end refuses: a `&str` inside core's `Option`, a field the program declares.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub struct P {
    x: u8,
    tag: Option<&'static str>,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        if self.tag.is_none() {
            self.x = 1;
        }
        *o = *i;
    }
}
