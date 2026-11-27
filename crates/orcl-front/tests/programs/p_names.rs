//! A program's own `Duration` and `Option`, a structure and an enum of those names, beside core's (one through
//! `orcl_std`'s re-export): the front end knows core's by their definitions, never by a printed name.
#![forbid(unsafe_code)]
// `unwrap_or` is core's and off the closed list: a program says it with a `match`
#![allow(clippy::manual_unwrap_or, clippy::manual_unwrap_or_default)]

use orcl_std::{Duration as Time, Program, Scan};

pub mod mine {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Duration {
        pub ms: u32,
        pub tag: u8,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum Option {
        #[default]
        Nothing,
        Some(u16),
        Pair(u8, u8),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: u16,
    pub sel: u8,
    pub t_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub d_ms: u32,
    pub d_tag: u8,
    pub opt: u16,
    pub core_some: bool,
    pub core_val: u16,
    pub held: Time,
}

#[derive(Debug, Default)]
pub struct PNames {
    mine: mine::Duration,
    choice: mine::Option,
    last: Option<u16>,
    since: Option<Time>,
}

const MINE: mine::Duration = mine::Duration { ms: 70_000, tag: 3 };
const CHOICES: [mine::Option; 3] = [mine::Option::Nothing, mine::Option::Some(500), mine::Option::Pair(2, 9)];

impl Program for PNames {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.mine.ms = self.mine.ms.wrapping_add(i.a as u32);
        self.mine.tag = MINE.tag + i.sel % 4;
        o.d_ms = self.mine.ms % MINE.ms;
        o.d_tag = self.mine.tag;
        self.choice = match i.sel % 4 {
            0 => mine::Option::Nothing,
            1 => mine::Option::Some(i.a),
            _ => CHOICES[(i.sel % 3) as usize],
        };
        o.opt = match self.choice {
            mine::Option::Nothing => 1,
            mine::Option::Some(v) => v,
            mine::Option::Pair(a, b) => a as u16 * 100 + b as u16,
        };
        if i.sel % 5 == 0 {
            self.last = None;
        } else if i.sel % 5 == 1 {
            self.last = Some(i.a);
        }
        o.core_some = self.last.is_some();
        o.core_val = match self.last {
            Some(v) => v,
            None => 0,
        };
        let since = *self.since.get_or_insert(scan.now);
        o.held = scan.now.saturating_sub(since).min(Time::from_millis(i.t_ms));
    }
}
