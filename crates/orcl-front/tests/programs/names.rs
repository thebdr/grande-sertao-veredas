//! Generic structures at several type arguments: the names the file gives them.
#![forbid(unsafe_code)]
use core::time::Duration;
use orcl_std::{Program, Scan};

#[derive(Debug, Default, Clone, Copy)]
pub struct Hold<T> {
    v: T,
}

#[derive(Debug, Default)]
pub struct RNames {
    a: Hold<u8>,
    b: Hold<i8>,
    c: Hold<Option<u8>>,
    d: Hold<Option<u16>>,
    f: Hold<f32>,
    g: Hold<Duration>,
    k: Hold<Hold<i16>>,
}

impl Program for RNames {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        self.a.v = *i;
        self.b.v = *i as i8;
        self.c.v = Some(*i);
        self.d.v = None;
        self.f.v = *i as f32;
        self.g.v = Duration::ZERO;
        self.k.v.v = 3;
        *o = self.a.v;
    }
}
