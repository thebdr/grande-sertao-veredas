//! A conformance program: constants of nested niches and aggregates (options of options of durations, a `repr(i16)`
//! enum with data, structures holding options of floats at minus zero and infinity), decoded by their layouts and read
//! at run time.
#![forbid(unsafe_code)]
// `unwrap_or` is core's and off the closed list: a program says it with a `match`
#![allow(clippy::manual_unwrap_or, clippy::manual_unwrap_or_default)]

use core::time::Duration;
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub sel: u8,
    pub k: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outputs {
    pub code: u8,
    pub d: Duration,
    pub i: i64,
    pub f: f32,
    pub w: u64,
    pub b: u8,
}

#[derive(Clone, Copy, Debug)]
#[repr(i16)]
enum Cmd {
    Back(i64) = -300,
    Halt = -1,
    Go { speed: i8, far: bool } = 7,
}

#[derive(Clone, Copy, Debug)]
struct Item {
    on: bool,
    wait: Option<Duration>,
    gain: i8,
    level: Option<f32>,
}

const NESTED: [Option<Option<Duration>>; 3] = [None, Some(None), Some(Some(Duration::new(4_000_000_000, 999_999_999)))];
const MIXED: (Option<i8>, [Option<bool>; 3], Option<u64>) = (Some(-128), [Some(false), None, Some(true)], Some(u64::MAX));
const CMDS: [Cmd; 3] = [Cmd::Back(i64::MIN), Cmd::Halt, Cmd::Go { speed: -7, far: true }];
const ITEMS: [Item; 3] = [
    Item { on: true, wait: None, gain: -1, level: Some(-0.0) },
    Item { on: false, wait: Some(Duration::from_millis(1_500)), gain: 127, level: None },
    Item { on: true, wait: Some(Duration::MAX), gain: -128, level: Some(f32::INFINITY) },
];

fn item(table: &[Item; 3], k: u8) -> Item {
    table[(k % 3) as usize]
}

#[derive(Debug, Default)]
pub struct PConsts {
    n: u8,
}

impl Program for PConsts {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.n = self.n.wrapping_add(1);
        match i.sel % 4 {
            0 => {
                o.code = match NESTED[(i.k % 3) as usize] {
                    None => 1,
                    Some(None) => 2,
                    Some(Some(d)) => {
                        o.d = d;
                        3
                    }
                };
            }
            1 => {
                let (a, bs, w) = MIXED;
                o.i = match a {
                    Some(v) => v as i64,
                    None => 0,
                };
                o.b = match bs[(i.k % 3) as usize] {
                    Some(true) => 2,
                    Some(false) => 1,
                    None => 0,
                };
                o.w = match w {
                    Some(v) => v,
                    None => 0,
                };
            }
            2 => {
                let c = &CMDS[(i.k % 3) as usize];
                o.code = match *c {
                    Cmd::Back(v) => {
                        o.i = v;
                        10
                    }
                    Cmd::Halt => 11,
                    Cmd::Go { speed, far } => {
                        o.i = speed as i64;
                        if far { 12 } else { 13 }
                    }
                };
                o.b = o.b.wrapping_add(1);
            }
            _ => {
                let it = item(&ITEMS, i.k);
                o.code = if it.on { 20 } else { 21 };
                if let Some(w) = it.wait {
                    o.d = w;
                }
                o.i = it.gain as i64;
                o.f = match it.level {
                    Some(l) => l,
                    None => 1.5,
                };
            }
        }
    }
}
