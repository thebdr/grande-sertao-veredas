//! Accented identifiers before a division on its line: the fault's column counted in characters, as an editor shows it.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: i32,
    pub b: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub v: i32,
}

#[derive(Debug, Default)]
pub struct RAccents {
    leitura: i32,
}

impl Program for RAccents {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        let razão = i.a; let pressão = i.b; o.v = razão / pressão;
        self.leitura = o.v;
    }
}
