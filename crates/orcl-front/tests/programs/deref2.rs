//! A place behind two references, `**r` read and `**m = v` written, which MIR reaches through `CopyForDeref`.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub v: u8,
}

#[derive(Debug, Default)]
pub struct Deref2 {
    x: u8,
}

fn get(r: &&u8) -> u8 {
    **r
}

fn set(m: &mut &mut u8, v: u8) {
    **m = v;
}

impl Program for Deref2 {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        let mut r = &mut self.x;
        set(&mut r, i.a.wrapping_mul(3));
        let v = &i.a;
        o.v = get(&v).wrapping_add(self.x);
    }
}
