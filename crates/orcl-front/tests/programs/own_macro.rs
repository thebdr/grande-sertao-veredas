//! A conformance program: a program's own macro whose expansion divides; its fault named inside the macro, as orcl-std's
//! own macros keep their lines.
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

macro_rules! ratio {
    ($a:expr, $b:expr) => {{
        let top: i32 = $a;
        let bottom: i32 = $b;
        top / bottom
    }};
}

#[derive(Debug, Default)]
pub struct OwnMacro;

impl Program for OwnMacro {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        o.v = ratio!(i.a, i.b);
    }
}
