//! A generic function at two marker types of one name in two modules, neither held by the program (each gives an
//! associated constant): two instances the file holds, at types it does not.
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

pub trait Rate {
    const RATE: u8;
}

pub mod a {
    pub struct Marker;
    impl super::Rate for Marker {
        const RATE: u8 = 3;
    }
}

pub mod b {
    pub struct Marker;
    impl super::Rate for Marker {
        const RATE: u8 = 5;
    }
}

pub fn rate<M: Rate>(x: u8) -> u8 {
    x.wrapping_mul(M::RATE)
}

#[derive(Debug, Default)]
pub struct QMarker;

impl Program for QMarker {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        o.x = rate::<a::Marker>(i.a);
        o.y = rate::<b::Marker>(i.a);
    }
}
