//! Const arguments of every integer width and sign, a one-part tuple, a tuple holding a `&mut` and options of
//! options as a type argument, in a state type with a const argument: the names the file gives them.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

#[derive(Debug, Default)]
pub struct A8<const K: i8> { v: i8 }
#[derive(Debug, Default)]
pub struct A16<const K: i16> { v: i16 }
#[derive(Debug, Default)]
pub struct A64<const K: i64> { v: i64 }
#[derive(Debug, Default)]
pub struct B64<const K: u64> { v: u64 }
#[derive(Debug, Default)]
pub struct B16<const K: u16> { v: u16 }
#[derive(Debug, Default)]
pub struct Tup<T> { v: T }

#[derive(Debug, Default)]
pub struct M<const N: usize> {
    a: A8<-1>,
    b: A16<-300>,
    c: A64<-5_000_000_000>,
    d: B64<{ u64::MAX }>,
    e: B16<65535>,
    h: Tup<Option<Option<u8>>>,
}

impl<const N: usize> M<N> {
    fn n(&self) -> u8 {
        N as u8
    }
}

impl Program for M<3> {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        self.a.v = -1;
        self.b.v = -300;
        self.c.v = -5;
        self.d.v = 7;
        self.e.v = 9;
        let pair: (&mut u16, u16) = (&mut self.e.v, 1);
        *pair.0 = pair.0.wrapping_add(pair.1);
        let t: (u8,) = (*i,);
        self.h.v = Some(Some(*i));
        *o = t.0.wrapping_add(self.n());
    }
}
