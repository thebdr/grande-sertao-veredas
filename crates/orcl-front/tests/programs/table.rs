//! A recipe table: a constant array of structures holding durations, enums and options, indexed by an input; a
//! `Duration` matched against constant patterns and compared with constants; a step chain through an `impl Trait`.
#![forbid(unsafe_code)]
use core::time::Duration;
use orcl_std::{Program, Scan, Ton, ms};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub recipe: u8,
    pub run: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub temp: i16,
    pub done: bool,
    pub phase: u8,
    pub kind: u8,
    pub limit: u16,
    pub has_limit: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mix {
    Off,
    Slow(u8),
    Fast { rpm: u16, reverse: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recipe {
    pub temp: i16,
    pub hold: Duration,
    pub mix: Mix,
    pub limit: Option<u16>,
    pub cool: bool,
}

const RECIPES: [Recipe; 5] = [
    Recipe { temp: -40, hold: Duration::ZERO, mix: Mix::Off, limit: None, cool: true },
    Recipe { temp: 85, hold: ms(30), mix: Mix::Slow(3), limit: Some(900), cool: false },
    Recipe { temp: 120, hold: ms(500), mix: Mix::Fast { rpm: 1_450, reverse: true }, limit: Some(0), cool: false },
    Recipe { temp: i16::MIN, hold: Duration::from_secs(1), mix: Mix::Fast { rpm: 60_000, reverse: false }, limit: None, cool: true },
    Recipe { temp: 7, hold: ms(70), mix: Mix::Slow(255), limit: Some(u16::MAX), cool: false },
];

const HALF: Duration = ms(500);

fn phase_of(d: Duration) -> u8 {
    if d == Duration::ZERO {
        0
    } else if d == HALF {
        2
    } else if d < HALF {
        1
    } else {
        3
    }
}

trait Stage {
    fn kind(&self) -> u8;
}

#[derive(Clone, Copy)]
struct Heat(i16);

impl Stage for Heat {
    fn kind(&self) -> u8 {
        if self.0 > 100 { 2 } else if self.0 < 0 { 0 } else { 1 }
    }
}

fn stage_for(r: &Recipe) -> impl Stage {
    Heat(r.temp)
}

#[derive(Debug, Default)]
pub struct RTable {
    hold: Ton,
}

impl Program for RTable {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, scan: &Scan, i: &Inputs, o: &mut Outputs) {
        let r = RECIPES[(i.recipe % 5) as usize];
        o.temp = r.temp;
        o.done = self.hold.scan(scan, i.run, r.hold);
        o.phase = phase_of(r.hold);
        o.kind = stage_for(&r).kind();
        o.limit = match r.mix {
            Mix::Off => 0,
            Mix::Slow(n) => n as u16,
            Mix::Fast { rpm, reverse: true } => rpm / 2,
            Mix::Fast { rpm, .. } => rpm,
        };
        o.has_limit = match r.limit {
            Some(l) => l > 0 && r.hold != Duration::ZERO,
            None => r.cool,
        };
    }
}
