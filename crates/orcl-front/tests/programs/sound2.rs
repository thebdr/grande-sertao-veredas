//! A conformance program: `repr(C)` enums with data and without, a `repr(i8)` enum with data at a negative
//! discriminant, several niche variants beside a `bool`, constant arrays of each indexed at run time, a newtype, a
//! match on a reference binding by reference, `let else`, `while let` over a program's method, signed range patterns,
//! shifts by narrow and signed amounts, and wide constants inside enums.
#![forbid(unsafe_code)]
use core::time::Duration;
use orcl_std::{Program, Scan, ms};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub sel: u8,
    pub v: i16,
    pub sh: i8,
    pub on: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub mode: u16,
    pub lamp: u8,
    pub step: i16,
    pub flag: u8,
    pub level: i16,
    pub band: u8,
    pub shl: u16,
    pub sar: i32,
    pub popped: u8,
    pub wide: i64,
    pub early: bool,
    pub held: Duration,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Slow(u8),
    Fast { speed: u16, dir: bool },
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lamp {
    Red = 1,
    Amber = 5,
    Green = 9,
}

#[repr(i8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Back(i16) = -3,
    Stop = 0,
    Fwd { by: i16 } = 7,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    Set(bool),
    A,
    B,
    C,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wide {
    Small(u8),
    Big(i64),
    Top(u64),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Level(pub i16);

impl Level {
    pub fn raise(&mut self, by: i16) -> i16 {
        self.0 = self.0.saturating_add(by);
        self.0
    }
}

const MODES: [Mode; 4] = [Mode::Off, Mode::Slow(7), Mode::Fast { speed: 0xBEEF, dir: true }, Mode::Fast { speed: 3, dir: false }];
const LAMPS: [Lamp; 3] = [Lamp::Green, Lamp::Red, Lamp::Amber];
const STEPS: [Step; 3] = [Step::Back(-300), Step::Stop, Step::Fwd { by: 12_000 }];
const FLAGS: [Flag; 5] = [Flag::Set(true), Flag::C, Flag::Set(false), Flag::A, Flag::B];
const WIDES: [Wide; 4] = [Wide::Big(i64::MIN), Wide::Top(u64::MAX), Wide::Small(200), Wide::Big(-1)];
const WAITS: [Option<Option<Duration>>; 3] = [Some(None), None, Some(Some(Duration::from_millis(1_500)))];

#[derive(Debug)]
pub struct Sound {
    mode: Mode,
    level: Level,
    queue: [u8; 4],
    len: u8,
    held: Option<Duration>,
}

impl Default for Sound {
    fn default() -> Self {
        Sound { mode: Mode::Off, level: Level(0), queue: [0; 4], len: 0, held: None }
    }
}

impl Sound {
    fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        Some(self.queue[self.len as usize])
    }

    fn push(&mut self, v: u8) {
        if self.len < 4 {
            self.queue[self.len as usize] = v;
            self.len += 1;
        }
    }
}

fn band(v: i16) -> u8 {
    match v {
        i16::MIN..=-1000 => 0,
        -999..=-1 => 1,
        0 => 2,
        1..=99 => 3,
        100..=i16::MAX => 4,
    }
}

impl Program for Sound {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, scan: &Scan, i: &Inputs, o: &mut Outputs) {
        let k = i.sel % 4;
        self.mode = MODES[k as usize];
        o.mode = match &self.mode {
            Mode::Off => 0,
            Mode::Slow(s) => (*s as u16).wrapping_add(1),
            Mode::Fast { speed, dir: true } => *speed,
            Mode::Fast { speed, .. } => speed.wrapping_mul(2),
        };
        o.lamp = LAMPS[(i.sel % 3) as usize] as u8;
        o.step = match STEPS[(i.sel / 4 % 3) as usize] {
            Step::Back(by) => by.saturating_sub(i.v),
            Step::Stop => 0,
            Step::Fwd { by } => by.wrapping_add(i.v),
        };
        o.flag = match FLAGS[(i.sel % 5) as usize] {
            Flag::Set(true) => 1,
            Flag::Set(false) => 2,
            Flag::A => 3,
            Flag::B => 4,
            Flag::C => 5,
        };
        o.level = self.level.raise(i.v / 64);
        o.band = band(i.v);
        o.shl = (i.v as u16) << (i.sel % 16);
        o.sar = (i.v as i32) >> ((i.sh & 31) as u8);
        if i.on {
            self.push(i.sel);
        }
        let mut popped = 0u8;
        if i.sel % 7 == 0 {
            while let Some(v) = self.pop() {
                popped = popped.wrapping_add(v);
            }
        }
        o.popped = popped;
        o.wide = match WIDES[(i.sel % 4) as usize] {
            Wide::Small(s) => s as i64,
            Wide::Big(b) => b,
            Wide::Top(t) => (t >> 1) as i64,
        };
        let w = WAITS[(i.sel % 3) as usize];
        let Some(inner) = w else {
            o.early = true;
            return;
        };
        o.early = false;
        let got = *self.held.get_or_insert(scan.now);
        o.held = match inner {
            Some(d) => d.min(ms(2_000)),
            None => got,
        };
        if i.sel % 11 == 0 {
            self.held = None;
        }
    }
}
