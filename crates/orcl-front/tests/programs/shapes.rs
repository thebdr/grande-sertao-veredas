//! A conformance program: the shapes of ORCL's data and what MIR does with them. Enums with data and with chosen
//! discriminants, matched and cast; `Option`s built, read and tested; arrays indexed, destructured and repeated;
//! tuples; references handed down to functions that write through them; a function that calls itself; `Duration`
//! built, compared and subtracted; and constants of every shape, decoded from their layout, a promoted reference to
//! one among them.
#![forbid(unsafe_code)]
// `unwrap_or` is core's and off the closed list: a program says it with a `match`
#![allow(clippy::manual_unwrap_or, clippy::manual_unwrap_or_default)]

use core::time::Duration;
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Code(pub u16);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Off,
    Hold(u8),
    Run {
        speed: i16,
        forward: bool,
    },
    Fault(Code),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Low = 3,
    Mid = 7,
    High = 200,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub sel: u8,
    pub val: u8,
    pub idx: u8,
    pub flag: bool,
    pub t_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub mode: Mode,
    pub mode_code: u16,
    pub forward: bool,
    pub first: u8,
    pub ends: u16,
    pub table: u16,
    pub sum: u32,
    pub last: Option<u8>,
    pub opt_some: bool,
    pub opt_none: bool,
    pub opt_val: u8,
    pub held: Duration,
    pub quotient: u8,
    pub remainder: u8,
    pub factorial: u32,
    pub count: u32,
    pub dur_eq: bool,
    pub dur_ne: bool,
    pub dur_lt: bool,
    pub dur_le: bool,
    pub dur_gt: bool,
    pub dur_ge: bool,
    pub dur_min: Duration,
    pub dur_max: Duration,
    pub level: u8,
    pub niche: u8,
    pub const_mode: u16,
    pub timeout: Duration,
    pub pair: u32,
    pub repeated: u8,
    pub levels: bool,
}

const TABLE: [u16; 4] = [10, 200, 3_000, 40_000];
const LEVELS: [Level; 3] = [Level::Low, Level::Mid, Level::High];
const NICHE: [Option<bool>; 3] = [Some(true), None, Some(false)];
const DEFAULT_MODE: Mode = Mode::Run { speed: -3, forward: true };
const TIMEOUT: Duration = Duration::from_millis(1_500);
const PAIR: (u8, u16, bool) = (7, 300, true);
const FAULTS: [Mode; 2] = [Mode::Fault(Code(9)), Mode::Hold(4)];

#[derive(Debug)]
pub struct Shapes {
    mode: Mode,
    history: [u8; 4],
    last: Option<u8>,
    since: Option<Duration>,
    count: u32,
}

// Written out, since an array's `Default` is core's and off the closed list; `[0; 4]` is MIR's repeat.
impl Default for Shapes {
    fn default() -> Shapes {
        Shapes { mode: Mode::default(), history: [0; 4], last: None, since: Some(Duration::ZERO), count: 7 }
    }
}

fn code_of(mode: &Mode) -> u16 {
    match mode {
        Mode::Off => 0,
        Mode::Hold(v) => *v as u16,
        Mode::Run { speed, forward: true } => 1_000 + (*speed + 100) as u16,
        Mode::Run { speed, forward: false } => 2_000 + (*speed + 100) as u16,
        Mode::Fault(code) => 3_000 + code.0,
    }
}

fn sum(values: &[u8; 4]) -> u32 {
    let mut total = 0u32;
    let mut k: u8 = 0;
    while k < 4 {
        total += values[k as usize] as u32;
        k += 1;
    }
    total
}

fn divmod(a: u8, b: u8) -> (u8, u8) {
    (a / b, a % b)
}

fn factorial(n: u8) -> u32 {
    if n <= 1 { 1 } else { n as u32 * factorial(n - 1) }
}

fn bump(counter: &mut u32, by: u8) {
    *counter = counter.wrapping_add(by as u32);
}

impl Program for Shapes {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.mode = match i.sel % 5 {
            0 => Mode::Off,
            1 => Mode::Hold(i.val),
            2 => Mode::Run { speed: i.val as i16 - 50, forward: i.flag },
            3 => Mode::Fault(Code(i.val as u16 * 3)),
            _ => FAULTS[(i.idx % 2) as usize],
        };
        o.mode = self.mode;
        o.mode_code = code_of(&self.mode);
        o.forward = matches!(self.mode, Mode::Run { forward: true, .. });

        self.history[(i.idx % 4) as usize] = i.val;
        o.first = self.history[0];
        let [a, _, _, d] = self.history;
        o.ends = a as u16 + d as u16;
        o.table = TABLE[(i.idx % 4) as usize];
        o.sum = sum(&self.history);

        if i.flag {
            self.last = Some(i.val);
        } else if i.sel == 7 {
            self.last = None;
        }
        o.last = self.last;
        o.opt_some = self.last.is_some();
        o.opt_none = self.last.is_none();
        o.opt_val = match self.last {
            Some(v) => v,
            None => 0,
        };
        let since = *self.since.get_or_insert(scan.now);
        o.held = scan.now.saturating_sub(since);
        if i.sel == 9 {
            self.since = None;
        }

        let (q, r) = divmod(i.val, 7);
        o.quotient = q;
        o.remainder = r;
        o.factorial = factorial(i.val % 10);
        bump(&mut self.count, i.val);
        o.count = self.count;

        let t = Duration::from_millis(i.t_ms);
        let limit = Duration::from_secs(1);
        o.dur_eq = t == limit;
        o.dur_ne = t != limit;
        o.dur_lt = t < limit;
        o.dur_le = t <= limit;
        o.dur_gt = t > limit;
        o.dur_ge = t >= limit;
        o.dur_min = t.min(limit);
        o.dur_max = t.max(limit);

        o.level = LEVELS[(i.idx % 3) as usize] as u8;
        o.niche = match NICHE[(i.sel % 3) as usize] {
            Some(true) => 1,
            Some(false) => 2,
            None => 3,
        };
        o.const_mode = code_of(&DEFAULT_MODE);
        o.timeout = TIMEOUT;
        o.pair = PAIR.0 as u32 + PAIR.1 as u32 + PAIR.2 as u32;
        let repeated = [i.val; 3];
        o.repeated = repeated[(i.idx % 3) as usize];
        let levels = [i.flag, !i.flag];
        o.levels = levels[1];
    }
}
