//! A program the front end refuses: a call into core outside the closed list, `!=` on the program's own enum (its
//! derived `PartialEq`'s `ne`, core's default method), named in full with the trait's own arguments.
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Io {
    pub a: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    Run,
}

#[derive(Debug, Default)]
pub struct P {
    state: State,
}

impl Program for P {
    type Inputs = Io;
    type Outputs = Io;

    fn scan(&mut self, _scan: &Scan, i: &Io, o: &mut Io) {
        let next = if i.a > 9 { State::Run } else { State::Idle };
        o.a = (self.state != next) as u8;
        self.state = next;
    }
}
