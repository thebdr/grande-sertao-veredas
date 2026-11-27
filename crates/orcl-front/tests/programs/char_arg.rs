//! A structure generic over a `char` const parameter, at two values: the names the file gives them.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub x: u8,
    pub y: u8,
}

#[derive(Debug, Default)]
pub struct Tag<const C: char> {
    pub n: u8,
}

impl<const C: char> Tag<C> {
    pub fn bump(&mut self, a: u8) -> u8 {
        self.n = self.n.wrapping_add(a);
        self.n
    }
}

#[derive(Debug, Default)]
pub struct QChar {
    a: Tag<'a'>,
    b: Tag<'b'>,
}

impl Program for QChar {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        o.x = self.a.bump(i.a);
        o.y = self.b.bump(i.a / 3);
    }
}
