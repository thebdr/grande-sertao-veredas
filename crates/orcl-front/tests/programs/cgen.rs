//! A function block generic over its length, `Avg<4>` and `Avg<8>` side by side: a const generic argument in a
//! structure's type, and the const generic value in its code.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub v: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub short: u16,
    pub long: u16,
}

#[derive(Debug)]
pub struct Avg<const N: usize> {
    buf: [u16; N],
    at: u8,
}

impl<const N: usize> Avg<N> {
    fn new() -> Avg<N> {
        Avg { buf: [0; N], at: 0 }
    }

    fn push(&mut self, v: u16) -> u16 {
        self.buf[self.at as usize % N] = v;
        self.at = self.at.wrapping_add(1);
        let mut sum = 0u32;
        let mut k = 0u8;
        while (k as usize) < N {
            sum += self.buf[k as usize] as u32;
            k += 1;
        }
        (sum / N as u32) as u16
    }
}

#[derive(Debug)]
pub struct Cgen {
    short: Avg<4>,
    long: Avg<8>,
}

impl Default for Cgen {
    fn default() -> Cgen {
        Cgen { short: Avg::new(), long: Avg::new() }
    }
}

impl Program for Cgen {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        o.short = self.short.push(i.v);
        o.long = self.long.push(i.v);
    }
}
