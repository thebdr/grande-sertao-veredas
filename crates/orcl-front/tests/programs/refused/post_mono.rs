//! A program rustc refuses after its analysis (a constant failed when a function the program does not reach
//! is instantiated), after the front end has lowered it: no file.
use orcl_std::{Program, Scan};

pub struct Check<const N: usize>;

impl<const N: usize> Check<N> {
    pub const OK: () = assert!(N < 4, "N past 3");
}

fn tagged<const N: usize>() -> u8 {
    let () = Check::<N>::OK;
    N as u8
}

pub fn unused() -> u8 {
    tagged::<5>()
}

#[derive(Default)]
pub struct P {
    x: u8,
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        *o = *i;
    }
}
