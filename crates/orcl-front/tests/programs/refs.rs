//! Places behind references and in enums: a mutable binding into an `Option`'s payload, a destructuring swap, a
//! nested field set through `&mut`, a structure returned and matched, `get_or_insert` on an array element, a match on
//! a reference, a block's method through an array element's field, and a constant structure holding options.
#![forbid(unsafe_code)]
// `unwrap_or` is core's and off the closed list: a program says it with a `match`
#![allow(clippy::manual_unwrap_or)]

use core::time::Duration;
use orcl_std::{Program, Scan, Ton, ms};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: u8,
    pub b: u8,
    pub on: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub counted: u8,
    pub swapped: u16,
    pub inner: u8,
    pub ret: u16,
    pub since: Duration,
    pub by_ref: u8,
    pub timers: u8,
    pub cfg: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inner {
    pub v: u8,
    pub w: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outer {
    pub inner: Inner,
    pub k: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Off,
    Run { speed: u8, dir: bool },
    Hold(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cfg {
    pub t: Option<Duration>,
    pub n: Option<u8>,
    pub m: Mode,
}

const CFGS: [Cfg; 2] = [
    Cfg { t: Some(Duration::from_secs(1)), n: None, m: Mode::Hold(300) },
    Cfg { t: None, n: Some(4), m: Mode::Run { speed: 9, dir: true } },
];

fn set(o: &mut Outer, v: u8) {
    o.inner.v = v;
    o.inner.w = v.wrapping_add(1);
    o.k = o.k.wrapping_add(v as u16);
}

fn made(a: u8, b: u8) -> Outer {
    Outer { inner: Inner { v: a, w: b }, k: a as u16 * 2 }
}

#[derive(Debug, Default)]
pub struct Station {
    pub ton: Ton,
    pub hits: u8,
}

#[derive(Debug)]
pub struct Refs {
    count: Option<u8>,
    outer: Outer,
    starts: [Option<Duration>; 2],
    mode: Mode,
    stations: [Station; 2],
}

impl Default for Refs {
    fn default() -> Refs {
        Refs {
            count: None,
            outer: Outer::default(),
            starts: [None, None],
            mode: Mode::Off,
            stations: [Station { ton: Ton::default(), hits: 0 }, Station { ton: Ton::default(), hits: 0 }],
        }
    }
}

impl Program for Refs {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, scan: &Scan, i: &Inputs, o: &mut Outputs) {
        if i.a % 5 == 0 {
            self.count = Some(0);
        }
        if let Some(ref mut c) = self.count {
            *c = c.wrapping_add(i.b);
        }
        o.counted = match self.count {
            Some(c) => c,
            None => 255,
        };
        let (mut x, mut y) = (i.a as u16, i.b as u16 * 3);
        if i.on {
            (x, y) = (y, x);
        }
        o.swapped = x.wrapping_mul(7).wrapping_add(y);
        set(&mut self.outer, i.a);
        o.inner = self.outer.inner.w ^ (self.outer.k as u8);
        o.ret = match made(i.a, i.b) {
            Outer { inner: Inner { v: 0, .. }, k } => k + 1,
            Outer { inner: Inner { v, w }, .. } if v > w => v as u16 * 10,
            Outer { k, .. } => k,
        };
        if i.a % 7 == 0 {
            self.starts[(i.b % 2) as usize] = None;
        }
        let start = *self.starts[(i.b % 2) as usize].get_or_insert(scan.now);
        o.since = scan.now.saturating_sub(start);
        self.mode = match i.a % 3 {
            0 => Mode::Off,
            1 => Mode::Run { speed: i.b, dir: i.on },
            _ => Mode::Hold(i.a as u16 * 4),
        };
        o.by_ref = match &self.mode {
            Mode::Off => 1,
            Mode::Run { speed, dir: true } => *speed,
            Mode::Run { speed, .. } => speed.wrapping_add(100),
            Mode::Hold(h) => (*h % 200) as u8,
        };
        if self.stations[(i.a % 2) as usize].ton.scan(scan, i.on, ms(30)) {
            self.stations[(i.a % 2) as usize].hits = self.stations[(i.a % 2) as usize].hits.wrapping_add(1);
        }
        o.timers = self.stations[0].hits.wrapping_mul(16).wrapping_add(self.stations[1].hits);
        let c = &CFGS[(i.b % 2) as usize];
        o.cfg = match (c.t, c.n, c.m) {
            (Some(t), None, Mode::Hold(h)) if t > ms(500) => h,
            (None, Some(n), Mode::Run { speed, dir }) => n as u16 * 100 + speed as u16 + dir as u16,
            _ => 7,
        };
    }
}
