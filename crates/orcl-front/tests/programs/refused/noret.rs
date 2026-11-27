//! A program the front end refuses: a terminator the interpreter does not have, a call to a function of the
//! program's that never returns.
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Io {
    pub a: u8,
}

fn halt() -> ! {
    loop {}
}

#[derive(Debug, Default)]
pub struct P;

impl Program for P {
    type Inputs = Io;
    type Outputs = Io;

    fn scan(&mut self, _scan: &Scan, i: &Io, o: &mut Io) {
        if i.a == 255 {
            halt();
        }
        o.a = i.a;
    }
}
