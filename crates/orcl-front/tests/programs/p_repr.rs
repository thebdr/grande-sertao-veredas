//! A conformance program: enums of a chosen representation, constants of wide and negative values, nested niches
//! and aggregates, switches at the edges of every width.
#![forbid(unsafe_code)]

use core::time::Duration;
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub sel: u8,
    pub a: i64,
    pub b: u64,
    pub c: i8,
    pub d: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub cmd: u8,
    pub cmd_match: u8,
    pub wide: u64,
    pub wide_match: u8,
    pub sign: i16,
    pub sign_match: u8,
    pub big_match: u8,
    pub neg_match: u8,
    pub sw8: u8,
    pub sw_i8: u8,
    pub nested: u16,
    pub nested_neg: i16,
    pub grid: u8,
    pub oo: u8,
    pub ob: u8,
    pub data: i16,
    pub data_dur: Duration,
    pub umax: u64,
    pub imin: i64,
    pub neg: i64,
    pub negs: i32,
    pub held: u8,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Idle = 10,
    Run = 20,
    Stop = 250,
}

#[repr(u64)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wide {
    Low = 1,
    High = 0xFFFF_FFFF_FFFF_FFF0,
}

#[repr(i16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sign {
    Neg = -300,
    Zero = 0,
    Pos = 300,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Data {
    Empty,
    Temp(i16),
    Pair(u8, Duration),
}

const CMDS: [Cmd; 3] = [Cmd::Idle, Cmd::Run, Cmd::Stop];
const WIDES: [Wide; 2] = [Wide::Low, Wide::High];
const SIGNS: [Sign; 3] = [Sign::Neg, Sign::Zero, Sign::Pos];
const NESTED: ((u8, bool), i16, [u16; 2]) = ((3, true), -7, [500, 60_000]);
const GRID: [[u8; 3]; 2] = [[1, 2, 3], [40, 50, 60]];
const OO: [Option<Option<u8>>; 3] = [None, Some(None), Some(Some(9))];
const OB: [Option<(u8, bool)>; 3] = [Some((1, false)), None, Some((200, true))];
const DATA: [Data; 3] = [Data::Empty, Data::Temp(-1234), Data::Pair(77, Duration::from_millis(2_500))];
const UMAX: u64 = u64::MAX;
const IMIN: i64 = i64::MIN;
const NEG: i64 = -5;
const NEGS: [i32; 3] = [-1, -2_000_000_000, 7];

#[derive(Debug)]
pub struct PRepr {
    last: Cmd,
    held: [Option<Option<u8>>; 2],
}

impl Default for PRepr {
    fn default() -> PRepr {
        PRepr { last: Cmd::Idle, held: [None, Some(None)] }
    }
}

fn cmd_code(c: Cmd) -> u8 {
    match c {
        Cmd::Idle => 1,
        Cmd::Run => 2,
        Cmd::Stop => 3,
    }
}

impl Program for PRepr {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        let k = i.sel % 3;
        let c = CMDS[k as usize];
        o.cmd = c as u8;
        o.cmd_match = cmd_code(c) + if c as u8 == self.last as u8 { 10 } else { 0 };
        self.last = c;
        let w = WIDES[(i.sel % 2) as usize];
        o.wide = w as u64;
        o.wide_match = match w {
            Wide::Low => 1,
            Wide::High => 2,
        };
        let s = SIGNS[k as usize];
        o.sign = s as i16;
        o.sign_match = match s {
            Sign::Neg => 1,
            Sign::Zero => 2,
            Sign::Pos => 3,
        };
        o.big_match = match i.b {
            0xFFFF_FFFF_FFFF_FFFF => 1,
            0x8000_0000_0000_0000 => 2,
            0 => 3,
            _ => 4,
        };
        o.neg_match = match i.a {
            i64::MIN => 1,
            -1 => 2,
            0 => 3,
            i64::MAX => 4,
            _ => 5,
        };
        o.sw8 = match i.d as u8 {
            255 => 1,
            128 => 2,
            0 => 3,
            _ => 4,
        };
        o.sw_i8 = match i.c {
            -128 => 1,
            -1 => 2,
            127 => 3,
            _ => 4,
        };
        o.nested = NESTED.0.0 as u16 + NESTED.0.1 as u16 + NESTED.2[(i.sel % 2) as usize];
        o.nested_neg = NESTED.1;
        o.grid = GRID[(i.sel % 2) as usize][(i.sel % 3) as usize];
        o.oo = match OO[k as usize] {
            None => 1,
            Some(None) => 2,
            Some(Some(v)) => v,
        };
        o.ob = match OB[k as usize] {
            Some((v, false)) => v,
            Some((v, true)) => v / 2,
            None => 3,
        };
        match DATA[k as usize] {
            Data::Empty => {
                o.data = 0;
                o.data_dur = Duration::ZERO;
            }
            Data::Temp(t) => {
                o.data = t;
                o.data_dur = Duration::from_secs(1);
            }
            Data::Pair(v, d) => {
                o.data = v as i16;
                o.data_dur = d;
            }
        }
        o.umax = UMAX - i.b % 3;
        o.imin = IMIN + (i.sel as i64);
        o.neg = NEG * (i.sel as i64);
        o.negs = NEGS[k as usize];
        self.held[(i.sel % 2) as usize] = OO[(i.sel / 2 % 3) as usize];
        o.held = match self.held[0] {
            None => 100,
            Some(None) => 101,
            Some(Some(v)) => v,
        };
    }
}
