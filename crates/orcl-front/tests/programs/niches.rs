//! Constants in a niche the others do not reach: `Option<Duration>` (its nanoseconds' niche), an `Option` of an
//! enum of negative discriminants, a `Duration` array, read on their own.
#![forbid(unsafe_code)]
use core::time::Duration;
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub sel: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub held: Duration,
    pub has: bool,
    pub dir: i8,
    pub step: Duration,
}

#[repr(i8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Back = -1,
    Stop = 0,
    Fwd = 1,
}

const HELD: [Option<Duration>; 2] = [None, Some(Duration::from_millis(1_500))];
const DIRS: [Option<Dir>; 3] = [None, Some(Dir::Back), Some(Dir::Fwd)];
const STEPS: [Duration; 2] = [Duration::from_secs(3), Duration::from_millis(250)];

#[derive(Debug, Default)]
pub struct Niches {
    n: u8,
}

impl Program for Niches {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.n = self.n.wrapping_add(1);
        match HELD[(i.sel % 2) as usize] {
            Some(d) => {
                o.held = d;
                o.has = true;
            }
            None => {
                o.held = Duration::ZERO;
                o.has = false;
            }
        }
        o.dir = match DIRS[(i.sel % 3) as usize] {
            None => 9,
            Some(d) => d as i8,
        };
        o.step = STEPS[(i.sel % 2) as usize];
    }
}
