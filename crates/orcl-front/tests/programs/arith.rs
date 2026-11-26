//! A conformance program: MIR's arithmetic, bits, comparisons and casts on every width of ORCL's integers and on
//! both floats, with their run-time checks, and the closed list's integer operations. Its inputs keep every checked
//! operation in range, so the native program never panics; the program's faults are the interpreter's own tests'.
#![forbid(unsafe_code)]
// `unwrap_or` is core's and off the closed list: a program says it with a `match`
#![allow(clippy::manual_unwrap_or)]

use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Inputs {
    /// Within ±10 000.
    pub a: i32,
    /// Within ±100, never zero.
    pub b: i32,
    pub c: u8,
    pub d: u16,
    /// Within ±10^9.
    pub e: i64,
    pub f: u64,
    pub k: i8,
    /// Any shift amount, past every width too.
    pub sh: u32,
    pub flag: bool,
    /// Finite, within ±10^6.
    pub x: f32,
    /// Finite, within ±10^12.
    pub y: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outputs {
    pub sum: i32,
    pub diff: i32,
    pub product: i32,
    pub quotient: i32,
    pub remainder: i32,
    pub negated: i32,
    pub wide: i64,
    pub wide_rem: u64,
    pub bits: u16,
    pub not_c: u8,
    pub shifted_left: u8,
    pub shifted_right: i64,
    pub shl: u16,
    pub shl_ok: bool,
    pub shr: u64,
    pub shr_ok: bool,
    pub sat_add: u8,
    pub sat_sub: u16,
    pub sat_mul: i8,
    pub wrap_add: u8,
    pub wrap_sub: i64,
    pub wrap_mul: i8,
    pub min: i32,
    pub max: u64,
    pub less: bool,
    pub at_least: bool,
    pub equal: bool,
    pub not_flag: bool,
    pub to_u8: u8,
    pub to_i64: i64,
    pub float_to_int: i32,
    pub double_to_u8: u8,
    pub int_to_float: f32,
    pub uint_to_double: f64,
    pub widened: f64,
    pub narrowed: f32,
    pub flag_as_int: u8,
    pub fsum: f32,
    pub fproduct: f32,
    pub dquotient: f64,
    pub ddiff: f64,
    pub frem: f32,
    pub fneg: f32,
    pub fless: bool,
    pub dge: bool,
    pub total: u64,
}

/// What every scan adds to: a running total, wrapped.
#[derive(Debug, Default)]
pub struct Arith {
    total: u64,
}

fn halve(v: i64) -> i64 {
    v / 2
}

impl Program for Arith {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        o.sum = i.a + i.b;
        o.diff = i.a - i.b;
        o.product = i.a * i.b;
        o.quotient = i.a / i.b;
        o.remainder = i.a % i.b;
        o.negated = -i.a;
        o.wide = halve(i.e) * 3 - i.e;
        o.wide_rem = i.f % 7;
        o.bits = (i.d & 0x0ff0) | ((i.flag as u16) << 15) ^ 0x5555;
        o.not_c = !i.c;
        o.shifted_left = i.c << (i.sh % 8);
        o.shifted_right = i.e >> (i.sh % 64);
        o.shl = match i.d.checked_shl(i.sh) {
            Some(v) => v,
            None => 1,
        };
        o.shl_ok = i.d.checked_shl(i.sh).is_some();
        o.shr = match i.f.checked_shr(i.sh) {
            Some(v) => v,
            None => 2,
        };
        o.shr_ok = i.f.checked_shr(i.sh).is_none();
        o.sat_add = i.c.saturating_add(200);
        o.sat_sub = i.d.saturating_sub(40_000);
        o.sat_mul = i.k.saturating_mul(5);
        o.wrap_add = i.c.wrapping_add(i.c);
        o.wrap_sub = i.e.wrapping_sub(i64::MAX);
        o.wrap_mul = i.k.wrapping_mul(i.k);
        o.min = i.a.min(i.b);
        o.max = i.f.max(1_000);
        o.less = i.a < i.b;
        o.at_least = i.e >= 0;
        o.equal = i.c == 7 || i.d != 3;
        o.not_flag = !i.flag;
        o.to_u8 = i.a as u8;
        o.to_i64 = i.c as i64 + i.k as i64;
        o.float_to_int = i.y as i32;
        o.double_to_u8 = i.y as u8;
        o.int_to_float = i.a as f32;
        o.uint_to_double = i.f as f64;
        o.widened = i.x as f64;
        o.narrowed = i.y as f32;
        o.flag_as_int = i.flag as u8;
        o.fsum = i.x + 1.5;
        o.fproduct = i.x * 0.25;
        o.dquotient = i.y / 3.0;
        o.ddiff = i.y - i.x as f64;
        o.frem = i.x % 2.5;
        o.fneg = -i.x;
        o.fless = i.x < 0.5;
        o.dge = i.y >= 2.0;
        self.total = self.total.wrapping_add(i.f).wrapping_mul(3);
        o.total = self.total;
    }
}
