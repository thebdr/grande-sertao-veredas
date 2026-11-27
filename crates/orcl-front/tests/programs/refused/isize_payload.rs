//! A program the front end refuses: `isize` as the program declares it, deep in an enum's payload.
use orcl_std::{Program, Scan};

#[derive(Default)]
pub enum Step {
    #[default]
    Idle,
    Fault(Option<(u8, isize)>),
}

#[derive(Default)]
pub struct P {
    x: u8,
    step: Step,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        if let Step::Fault(Some((code, _))) = self.step {
            self.x = code;
        }
        *o = i.wrapping_add(self.x);
    }
}
