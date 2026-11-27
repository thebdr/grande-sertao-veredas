//! Const generics of other types than usize: a `bool`, an `i32`, a `u8`, and a function generic over a const.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub v: i16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub inv: i16,
    pub plain: i16,
    pub off_a: i32,
    pub off_b: i32,
    pub scaled: u8,
}

#[derive(Debug, Default)]
pub struct Sign<const INVERT: bool> {
    last: i16,
}

impl<const INVERT: bool> Sign<INVERT> {
    fn scan(&mut self, v: i16) -> i16 {
        self.last = if INVERT { v.wrapping_neg_free() } else { v };
        self.last
    }
}

trait NegFree {
    fn wrapping_neg_free(self) -> Self;
}

impl NegFree for i16 {
    fn wrapping_neg_free(self) -> i16 {
        0i16.wrapping_sub(self)
    }
}

#[derive(Debug, Default)]
pub struct Offset<const K: i32> {
    acc: i32,
}

impl<const K: i32> Offset<K> {
    fn scan(&mut self, v: i16) -> i32 {
        self.acc = (v as i32).wrapping_add(K);
        self.acc
    }
}

fn scale<const K: u8>(v: i16) -> u8 {
    (v as u8).wrapping_mul(K)
}

#[derive(Debug, Default)]
pub struct RCgen2 {
    inv: Sign<true>,
    plain: Sign<false>,
    a: Offset<-1000>,
    b: Offset<70000>,
}

impl Program for RCgen2 {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        o.inv = self.inv.scan(i.v);
        o.plain = self.plain.scan(i.v);
        o.off_a = self.a.scan(i.v);
        o.off_b = self.b.scan(i.v);
        o.scaled = scale::<3>(i.v);
    }
}
