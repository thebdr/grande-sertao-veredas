//! A conformance program: what the others do not reach. `>` and `<=` on integers at their
//! boundary; a match on negative values; an enum with a negative discriminant, cast and matched; a constant enum
//! whose niche holds two variants; `[first, .., last]`; a generic function whose unused argument MIR drops; a
//! promoted reference into a constant at an offset; a constant enum of one variant.
#![forbid(unsafe_code)]

use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: i16,
    pub b: i16,
    pub n: u8,
    pub flag: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub gt: bool,
    pub le: bool,
    pub ge: bool,
    pub lt: bool,
    pub ne: bool,
    pub eq: bool,
    pub signed_match: u8,
    pub dir_code: i8,
    pub dir_match: u8,
    pub light: u8,
    pub first_last: u16,
    pub picked: u8,
    pub off_ref: u16,
    pub only: u8,
    pub sat16: i16,
    pub off_ref2: u16,
    pub off_ref3: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Back = -1,
    Stop = 0,
    Fwd = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Light {
    Off,
    Blink,
    On(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Only {
    One(u8),
}

const DIRS: [Dir; 3] = [Dir::Back, Dir::Stop, Dir::Fwd];
const LIGHTS: [Light; 4] = [Light::Off, Light::Blink, Light::On(true), Light::On(false)];
const ONLY: Only = Only::One(42);
const LIMITS: (u8, u16, u16) = (1, 500, 600);
const SECOND: &u16 = &LIMITS.2;
const TABLE: [u16; 4] = [11, 222, 3_333, 44_444];

fn code(l: &Light) -> u8 {
    match l {
        Light::Off => 1,
        Light::Blink => 2,
        Light::On(true) => 3,
        Light::On(false) => 4,
    }
}

fn pick<T>(first: bool, a: T, b: T) -> T {
    if first { a } else { b }
}

fn read16(v: &u16) -> u16 {
    *v
}

#[derive(Debug)]
pub struct Edges {
    history: [u16; 4],
}

// Written out: an array's `Default` is off the closed list.
#[allow(clippy::derivable_impls)]
impl Default for Edges {
    fn default() -> Edges {
        Edges { history: [0; 4] }
    }
}

impl Program for Edges {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        o.gt = i.a > i.b;
        o.le = i.a <= i.b;
        o.ge = i.a >= i.b;
        o.lt = i.a < i.b;
        o.ne = i.a != i.b;
        o.eq = i.a == i.b;
        o.signed_match = match i.a {
            -3 => 1,
            -1 => 2,
            0 => 3,
            2 => 4,
            _ => 5,
        };
        let d = DIRS[(i.n % 3) as usize];
        o.dir_code = d as i8;
        o.dir_match = match d {
            Dir::Back => 10,
            Dir::Stop => 20,
            Dir::Fwd => 30,
        };
        o.light = code(&LIGHTS[(i.n % 4) as usize]);
        self.history[(i.n % 4) as usize] = i.b as u16;
        let [first, .., last] = self.history;
        o.first_last = first.wrapping_add(last);
        o.picked = pick(i.flag, i.n, 7);
        o.off_ref = read16(&LIMITS.2);
        let Only::One(v) = ONLY;
        o.only = v;
        o.sat16 = i.a.saturating_mul(i.b);
        o.off_ref2 = read16(SECOND);
        o.off_ref3 = read16(&TABLE[2]);
    }
}
