//! A conformance program: the compiler's own `usize` and `isize` temporaries through inherent operations of the list, a
//! ring buffer indexed by a `u8` cursor widened to `usize`.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: u8,
    pub b: i8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub oldest: u8,
    pub back: i8,
    pub low: u8,
    pub widest: u8,
}

#[derive(Debug)]
pub struct UsizeTemps {
    ring: [u8; 4],
    at: u8,
}

// written out: a derived `Default` over an array field calls `<[T; N] as Default>::default`, not on the list yet
#[allow(clippy::derivable_impls)]
impl Default for UsizeTemps {
    fn default() -> UsizeTemps {
        UsizeTemps { ring: [0; 4], at: 0 }
    }
}

impl Program for UsizeTemps {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.ring[self.at as usize % 4] = i.a;
        self.at = ((self.at as usize).wrapping_add(1) % 4) as u8;
        o.oldest = self.ring[(self.at as usize).wrapping_add(4) % 4];
        o.back = (i.b as isize).wrapping_sub(3) as i8;
        o.low = (i.a as usize).saturating_sub(200) as u8;
        o.widest = (i.a as usize).max(self.ring[0] as usize) as u8;
    }
}
