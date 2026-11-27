//! A conformance program: traits with two impls behind generics, default methods and associated constants, a
//! generic structure at two types, struct update, `while let`, `let else`, `continue`, casts across every width,
//! shifts at their edges, bools compared and xor-ed, a state of no slot's fields beside others.
#![forbid(unsafe_code)]
// written on purpose: `<` on two bools, and a cast of a `u8` to its own type
#![allow(clippy::bool_comparison, clippy::unnecessary_cast)]

use core::time::Duration;
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub n: u8,
    pub p: bool,
    pub q: bool,
    pub a8: i8,
    pub u64v: u64,
    pub i64v: i64,
    pub u32v: u32,
    pub i16v: i16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outputs {
    pub sa: u32,
    pub sb: u32,
    pub la: u16,
    pub lb: u16,
    pub pair_u8: u8,
    pub pair_dur: Duration,
    pub cfg: u32,
    pub count: u8,
    pub skipped: u8,
    pub got: u8,
    pub blt: bool,
    pub bge: bool,
    pub bxor: bool,
    pub band: bool,
    pub c1: u64,
    pub c2: i8,
    pub c3: u32,
    pub c4: i16,
    pub c5: i64,
    pub c6: u16,
    pub c7: u8,
    pub s1: u64,
    pub s2: i16,
    pub s3: u8,
    pub s4: i64,
    pub s5: u32,
    pub f1: f32,
    pub f2: i32,
    pub f3: u64,
    pub f4: f64,
}

pub trait Sensor {
    const SCALE: u16;
    fn raw(&self) -> u16;
    fn scaled(&self) -> u32 {
        self.raw() as u32 * Self::SCALE as u32
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct A {
    v: u16,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct B {
    v: u16,
    off: u16,
}

impl Sensor for A {
    const SCALE: u16 = 3;
    fn raw(&self) -> u16 {
        self.v
    }
}

impl Sensor for B {
    const SCALE: u16 = 7;
    fn raw(&self) -> u16 {
        self.v.wrapping_add(self.off)
    }
    fn scaled(&self) -> u32 {
        self.raw() as u32 + Self::SCALE as u32
    }
}

fn read<S: Sensor>(s: &S) -> u32 {
    s.scaled()
}

fn limit<S: Sensor>() -> u16 {
    S::SCALE
}

#[derive(Clone, Copy, Debug)]
pub struct Pair<T: Copy> {
    a: T,
    b: T,
}

impl<T: Copy> Pair<T> {
    fn swap(self) -> Pair<T> {
        Pair { a: self.b, b: self.a }
    }
    fn first(&self) -> T {
        self.a
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Cfg {
    a: u8,
    b: u8,
    c: u16,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Unit;

#[derive(Debug, Default)]
pub struct PTraits {
    a: A,
    b: B,
    unit: Unit,
    ticks: u32,
}

fn maybe(n: u8) -> Option<u8> {
    if n % 3 == 0 { None } else { Some(n / 3) }
}

fn pick(n: u8) -> u8 {
    let Some(v) = maybe(n) else {
        return 99;
    };
    v + 1
}

fn touch(_u: &Unit, x: u8) -> u8 {
    x
}

impl Program for PTraits {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.ticks = self.ticks.wrapping_add(1);
        self.a.v = self.a.v.wrapping_add(i.n as u16);
        self.b.v = i.n as u16 * 2;
        self.b.off = (self.ticks % 1000) as u16;
        o.sa = read(&self.a);
        o.sb = read(&self.b);
        o.la = limit::<A>();
        o.lb = limit::<B>();
        let p8 = Pair { a: i.n, b: i.n.wrapping_mul(3) };
        o.pair_u8 = p8.swap().first();
        let pd = Pair { a: Duration::from_millis(i.n as u64), b: Duration::from_secs(2) };
        o.pair_dur = if i.p { pd.swap().first() } else { pd.first() };
        let base = Cfg { a: 1, b: 2, c: 300 };
        let cfg = Cfg { b: i.n, ..base };
        o.cfg = cfg.a as u32 + cfg.b as u32 * 1000 + cfg.c as u32;
        let mut opt = Some(i.n % 6);
        let mut count = 0u8;
        let mut skipped = 0u8;
        while let Some(k) = opt {
            opt = if k == 0 { None } else { Some(k - 1) };
            if k % 2 == 1 {
                skipped += 1;
                continue;
            }
            count += 1;
        }
        o.count = count;
        o.skipped = skipped;
        o.got = touch(&self.unit, pick(i.n));
        o.blt = i.p < i.q;
        o.bge = i.p >= i.q;
        o.bxor = i.p ^ i.q;
        o.band = i.p & !i.q;
        o.c1 = i.a8 as u64;
        o.c2 = i.u64v as i8;
        o.c3 = i.i64v as u32;
        o.c4 = i.u32v as i16;
        o.c5 = i.u32v as i64 - i.a8 as i64;
        o.c6 = i.i16v as u16;
        o.c7 = i.p as u8 + i.q as u8;
        o.s1 = i.u64v << 63;
        o.s2 = (i.a8 >> 7) as i16;
        o.s3 = (i.n as u8) >> (i.n % 8);
        o.s4 = i.i64v >> (i.n % 64);
        o.s5 = i.u32v >> 31;
        o.f1 = -(i.i16v as f32) / 4.0;
        o.f2 = (i.i64v as f64 * 1.0e10) as i32;
        o.f3 = (-(i.a8 as f32)) as u64;
        o.f4 = i.u64v as f64 - 1.0e19;
    }
}
