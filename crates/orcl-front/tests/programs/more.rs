//! Sound code the other programs leave out: wildcard lets, unnamed and pattern parameters, recursion, a generic
//! function dropping its parameter, a reference to an array element, a constant enum padded past a structure of
//! several slots, `continue`, a `repr(u8)` enum with data and explicit discriminants, floats at their edges (NaN, the
//! infinities, -0.0) cast and compared, an arithmetic shift right, `Duration::MAX` and a constant past 2^32 seconds,
//! a niche of two niche variants in a bool, a two-dimensional array, and structure update syntax.
#![forbid(unsafe_code)]
// a float compared with itself is the point: a NaN neither below nor equal to itself, as the interpreter compares it
#![allow(clippy::eq_op)]

use core::time::Duration;
use orcl_std::{Program, Scan, ms};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Inputs {
    pub a: i16,
    pub b: u8,
    pub x: f32,
    pub y: f64,
    pub sel: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outputs {
    pub mention: u8,
    pub unnamed: u8,
    pub fact: u32,
    pub bumped: u16,
    pub setpoint: Duration,
    pub setv: u16,
    pub skipped: u8,
    pub tagged: u8,
    pub const_tag: u16,
    pub to_u8: u8,
    pub to_i32: i32,
    pub big_f32: f32,
    pub narrowed: f32,
    pub below_zero: bool,
    pub is_zero: bool,
    pub sar: i16,
    pub forever: Duration,
    pub far: Duration,
    pub self_lt: bool,
    pub self_ne: bool,
    pub oob: u8,
    pub grid: u16,
    pub updated: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setpoint {
    pub t: Duration,
    pub v: u16,
    pub ramp: (u8, u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Idle,
    Run(Setpoint),
    Hold(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Tag {
    A(u8) = 3,
    B = 7,
    C { v: u16 } = 200,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cfg {
    pub a: u8,
    pub b: u8,
    pub c: u16,
}

const STEPS: [Step; 3] = [
    Step::Idle,
    Step::Run(Setpoint { t: Duration::from_millis(750), v: 40, ramp: (2, 9) }),
    Step::Hold(5),
];
const TAGS: [Tag; 3] = [Tag::A(9), Tag::B, Tag::C { v: 1_000 }];
const FOREVER: Duration = Duration::MAX;
const FAR: Duration = Duration::from_secs(5_000_000_000);
const OOB: [Option<Option<bool>>; 4] = [None, Some(None), Some(Some(true)), Some(Some(false))];
const CFG: Cfg = Cfg { a: 1, b: 20, c: 300 };

fn id(v: u8) -> u8 {
    v
}

fn first(_: u8, (a, _): (u8, u16)) -> u8 {
    a
}

fn fact(n: u32) -> u32 {
    if n == 0 { 1 } else { n * fact(n - 1) }
}

/// A generic function that drops its parameter on every path: the block after the drop is reached only through it.
fn ignore<T>(_v: T) -> u8 {
    1
}

fn bump(c: &mut u16) {
    *c = c.wrapping_add(3);
}

#[derive(Debug)]
pub struct More {
    arr: [u8; 4],
    counts: [u16; 3],
    grid: [[u16; 3]; 2],
}

// Written out: an array's `Default` is off the closed list.
#[allow(clippy::derivable_impls)]
impl Default for More {
    fn default() -> More {
        More { arr: [0; 4], counts: [0; 3], grid: [[0; 3]; 2] }
    }
}

impl Program for More {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.arr[(i.sel % 4) as usize] = i.b;
        let _ = self.arr[(i.b % 4) as usize];
        _ = id(i.b);
        o.mention = self.arr[(i.b % 4) as usize];
        o.unnamed = first(i.b, (i.b.wrapping_add(1), 7)).wrapping_add(ignore(i.sel));
        o.fact = fact((i.b % 13) as u32);
        bump(&mut self.counts[(i.b % 3) as usize]);
        o.bumped = self.counts[0].wrapping_add(self.counts[1]).wrapping_add(self.counts[2]);
        let (t, v) = match STEPS[(i.sel % 3) as usize] {
            Step::Idle => (Duration::ZERO, 0),
            Step::Run(s) => (s.t, s.v + s.ramp.0 as u16 + s.ramp.1 as u16),
            Step::Hold(h) => (ms(h as u64), h as u16),
        };
        o.setpoint = t;
        o.setv = v;
        let mut k = 0u8;
        let mut s = 0u8;
        while k < 10 {
            k += 1;
            if k % 3 == 0 {
                continue;
            }
            s += k;
        }
        o.skipped = s + i.b % 2;
        let tag = match i.sel % 3 {
            0 => Tag::A(i.b),
            1 => Tag::B,
            _ => Tag::C { v: i.b as u16 * 3 },
        };
        o.tagged = match tag {
            Tag::A(v) => v,
            Tag::B => 77,
            Tag::C { v } => (v % 256) as u8,
        };
        o.const_tag = match TAGS[(i.b % 3) as usize] {
            Tag::A(v) => v as u16,
            Tag::B => 7,
            Tag::C { v } => v,
        };
        o.to_u8 = i.x as u8;
        o.to_i32 = i.x as i32;
        o.big_f32 = (i.b as u64).wrapping_mul(0x0101_0101_0101_0101) as f32;
        o.narrowed = (i.y * 1.0e300) as f32;
        o.below_zero = i.x < 0.0;
        o.is_zero = i.x == 0.0;
        o.sar = i.a >> (i.b % 16);
        o.forever = if i.sel % 5 == 0 { FOREVER } else { Duration::ZERO };
        o.far = if i.sel % 7 == 0 { FAR } else { Duration::ZERO };
        o.self_lt = i.x < i.x;
        o.self_ne = i.x != i.x;
        o.oob = match OOB[(i.sel % 4) as usize] {
            None => 1,
            Some(None) => 2,
            Some(Some(true)) => 3,
            Some(Some(false)) => 4,
        };
        self.grid[(i.b % 2) as usize][(i.sel % 3) as usize] = self.grid[(i.b % 2) as usize][(i.sel % 3) as usize].wrapping_add(1);
        o.grid = self.grid[0][0].wrapping_add(self.grid[1][2]).wrapping_mul(3).wrapping_add(self.grid[0][1]);
        let c = Cfg { a: i.b, ..CFG };
        o.updated = c.a.wrapping_add(c.b).wrapping_add(c.c as u8);
    }
}
