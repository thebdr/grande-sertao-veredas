//! A program the front end refuses (C-156's fourteenth refute round): a recipe looked up by reference from a table of
//! constants, as a program writes one (`fn recipe(fast: bool) -> &'static Recipe`), its dwell timed by orcl-std's
//! on-delay, then its temperature read.
#![forbid(unsafe_code)]

use orcl_std::{Duration, Program, Scan, Ton, ms};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub fast: bool,
    pub start: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub heat: bool,
    pub setpoint: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recipe {
    pub temp: u16,
    pub dwell: Duration,
}

const SLOW: Recipe = Recipe { temp: 180, dwell: ms(400) };
const FAST: Recipe = Recipe { temp: 220, dwell: ms(150) };

fn recipe(fast: bool) -> &'static Recipe {
    if fast { &FAST } else { &SLOW }
}

#[derive(Debug, Default)]
pub struct Oven {
    dwell: Ton,
}

impl Program for Oven {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, scan: &Scan, i: &Inputs, o: &mut Outputs) {
        let r = recipe(i.fast);
        let done = self.dwell.scan(scan, i.start, r.dwell);
        o.heat = i.start && !done;
        o.setpoint = r.temp;
    }
}
