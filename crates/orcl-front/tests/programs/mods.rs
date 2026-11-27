//! Two modules, each with its own `Timer` and its own `step`: types and functions of one name.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub x: u8,
    pub y: u16,
}

mod fast {
    #[derive(Debug, Default)]
    pub struct Timer {
        pub n: u8,
    }
    pub fn step(t: &mut Timer, a: u8) -> u8 {
        t.n = t.n.wrapping_add(a);
        t.n
    }
}

mod slow {
    #[derive(Debug, Default)]
    pub struct Timer {
        pub n: u16,
        pub k: u8,
    }
    pub fn step(t: &mut Timer, a: u8) -> u16 {
        t.k = t.k.wrapping_add(1);
        if t.k % 3 == 0 {
            t.n = t.n.wrapping_add(a as u16);
        }
        t.n
    }
}

#[derive(Debug, Default)]
pub struct RMods {
    f: fast::Timer,
    s: slow::Timer,
}

impl Program for RMods {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        o.x = fast::step(&mut self.f, i.a);
        o.y = slow::step(&mut self.s, i.a);
    }
}
