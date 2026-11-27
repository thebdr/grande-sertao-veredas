//! A program whose state is an enum with data: the machine's state is the program.
#![forbid(unsafe_code)]
use core::time::Duration;
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub go: bool,
    pub stop: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub code: u8,
    pub ticks: u16,
    pub since: Duration,
}

#[derive(Debug, Default)]
pub enum Machine {
    #[default]
    Idle,
    Running { ticks: u16, since: Duration },
    Stopped(u8),
}

impl Program for Machine {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, scan: &Scan, i: &Inputs, o: &mut Outputs) {
        let next = match self {
            Machine::Idle if i.go => Some(Machine::Running { ticks: 0, since: scan.now }),
            Machine::Running { ticks, .. } if i.stop => Some(Machine::Stopped((*ticks % 200) as u8)),
            Machine::Running { ticks, .. } => {
                *ticks = ticks.wrapping_add(1);
                None
            }
            Machine::Stopped(n) if i.go && *n > 3 => Some(Machine::Idle),
            _ => None,
        };
        if let Some(n) = next {
            *self = n;
        }
        match self {
            Machine::Idle => *o = Outputs { code: 0, ticks: 0, since: Duration::ZERO },
            Machine::Running { ticks, since } => *o = Outputs { code: 1, ticks: *ticks, since: scan.now.saturating_sub(*since) },
            Machine::Stopped(n) => *o = Outputs { code: 2, ticks: *n as u16, since: Duration::ZERO },
        }
    }
}
