//! A program the front end refuses: `u128`, a type outside ORCL's, a field.
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Io {
    pub a: u8,
}

#[derive(Debug, Default)]
pub struct P {
    total: u128,
}

impl Program for P {
    type Inputs = Io;
    type Outputs = Io;

    fn scan(&mut self, _scan: &Scan, i: &Io, o: &mut Io) {
        self.total = self.total.wrapping_add(i.a as u128);
        o.a = self.total as u8;
    }
}
