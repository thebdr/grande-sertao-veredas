//! A conformance program: corners the other programs leave out. An enum whose payload is a `Duration` and whose three
//! fieldless variants live in the nanoseconds' niche, and an `Option` of it one niche further; `Option<Option<
//! Duration>>`; a promoted reference into a constant array of such enums; `repr(u16)` and `repr(i32)` enums with
//! negative and wide discriminants; `Option<i16>` constants holding negatives; an array mutated through `&mut [u8; 4]`;
//! a `where` clause; an array returned by value and destructured; a loop's `break` value of a `Duration`; a constant
//! structure the compiler reorders around a `Duration`.
#![forbid(unsafe_code)]
use core::time::Duration;
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub sel: u8,
    pub v: u8,
    pub flag: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub step: u8,
    pub wait: Duration,
    pub opt_step: u8,
    pub nested: u8,
    pub nested_d: Duration,
    pub promoted: u8,
    pub promoted_d: Duration,
    pub r16: u16,
    pub r32: i32,
    pub r16_match: u8,
    pub r32_match: u8,
    pub oi: i16,
    pub oi_some: bool,
    pub arr: u32,
    pub picked: u16,
    pub abc: u16,
    pub broke: Duration,
    pub cfg_t: Duration,
    pub cfg_a: u8,
    pub cfg_b: bool,
    pub cfg_c: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Wait(Duration),
    Done,
    Idle,
    Skip,
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum R16 {
    Low = 1,
    Mid = 0x8000,
    Top = 0xFFFF,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum R32 {
    Neg = -2_000_000_000,
    Zero = 0,
    Pos = 2_000_000_000,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cfg {
    pub a: u8,
    pub t: Duration,
    pub b: bool,
    pub c: i16,
}

const STEPS: [Step; 4] = [Step::Wait(Duration::from_millis(1_250)), Step::Done, Step::Idle, Step::Skip];
const OPT_STEPS: [Option<Step>; 3] = [None, Some(Step::Skip), Some(Step::Wait(Duration::from_secs(7)))];
const NESTED: [Option<Option<Duration>>; 3] = [None, Some(None), Some(Some(Duration::from_millis(30)))];
const R16S: [R16; 3] = [R16::Low, R16::Mid, R16::Top];
const R32S: [R32; 3] = [R32::Neg, R32::Zero, R32::Pos];
const OI: [Option<i16>; 3] = [Some(-300), None, Some(i16::MIN)];
const CFGS: [Cfg; 2] = [
    Cfg { a: 200, t: Duration::from_millis(999), b: true, c: -5 },
    Cfg { a: 7, t: Duration::from_secs(3_000_000_000), b: false, c: i16::MAX },
];

fn code(s: Step) -> u8 {
    match s {
        Step::Wait(_) => 1,
        Step::Done => 2,
        Step::Idle => 3,
        Step::Skip => 4,
    }
}

fn wait_of(s: &Step) -> Duration {
    match s {
        Step::Wait(d) => *d,
        _ => Duration::ZERO,
    }
}

fn bump(a: &mut [u8; 4], at: u8, v: u8) {
    a[(at % 4) as usize] = a[(at % 4) as usize].wrapping_add(v);
}

fn pick<T>(a: T, b: T, first: bool) -> T
where
    T: Copy,
{
    if first { a } else { b }
}

fn three(v: u8) -> [u8; 3] {
    [v, v.wrapping_add(1), v.wrapping_mul(3)]
}

#[derive(Debug)]
pub struct Corners {
    acc: [u8; 4],
    n: u8,
}

// written out: a derived `Default` would call `<[u8; 4] as Default>::default`, outside the list
#[allow(clippy::derivable_impls)]
impl Default for Corners {
    fn default() -> Corners {
        Corners { acc: [0; 4], n: 0 }
    }
}

impl Program for Corners {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.n = self.n.wrapping_add(1);
        let s = STEPS[(i.sel % 4) as usize];
        o.step = code(s);
        o.wait = wait_of(&s);
        o.opt_step = match OPT_STEPS[(i.sel % 3) as usize] {
            None => 9,
            Some(s) => code(s) + if wait_of(&s) == Duration::from_secs(7) { 10 } else { 0 },
        };
        let (nested, nested_d) = match NESTED[(i.v % 3) as usize] {
            None => (1, Duration::ZERO),
            Some(None) => (2, Duration::ZERO),
            Some(Some(d)) => (3, d),
        };
        o.nested = nested;
        o.nested_d = nested_d;
        let r: &Step = &STEPS[0];
        let q: &Step = &STEPS[3];
        o.promoted = if i.flag { code(*r) } else { code(*q) };
        o.promoted_d = wait_of(r);
        let x = R16S[(i.v % 3) as usize];
        o.r16 = x as u16;
        o.r16_match = match x {
            R16::Low => 1,
            R16::Mid => 2,
            R16::Top => 3,
        };
        let y = R32S[(i.sel % 3) as usize];
        o.r32 = y as i32;
        o.r32_match = match y {
            R32::Neg => 1,
            R32::Zero => 2,
            R32::Pos => 3,
        };
        match OI[(i.v % 3) as usize] {
            Some(v) => {
                o.oi = v;
                o.oi_some = true;
            }
            None => {
                o.oi = 0;
                o.oi_some = false;
            }
        }
        bump(&mut self.acc, i.sel, i.v);
        o.arr = self.acc[0] as u32 + self.acc[1] as u32 * 256 + self.acc[2] as u32 * 65_536 + (self.acc[3] as u32) * 16_777_216;
        o.picked = pick(i.v as u16, 1_000, i.flag);
        let [a, b, c] = three(i.v);
        o.abc = a as u16 + b as u16 * 3 + c as u16 * 5;
        let mut k = 0u8;
        o.broke = loop {
            k += 1;
            if k > i.v % 5 {
                break Duration::from_millis(k as u64 * 100);
            }
        };
        let cfg = CFGS[(i.sel % 2) as usize];
        o.cfg_t = cfg.t;
        o.cfg_a = cfg.a;
        o.cfg_b = cfg.b;
        o.cfg_c = cfg.c;
    }
}
