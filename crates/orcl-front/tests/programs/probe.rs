//! A conformance program: a probe that reaches every block of `orcl-std`. The three timers on presets its inputs
//! vary, both edges, the counter with its reset, and every bit operation on every width, past the width too; `ms`
//! and `s`; and the program's own `safe_outputs` in place of the library's.
#![forbid(unsafe_code)]

use core::time::Duration;
use orcl_std::{Bits, Ctu, FTrig, Program, RTrig, Scan, Tof, Ton, Tp, ms, s};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: bool,
    pub b: bool,
    pub reset: bool,
    pub preset: i32,
    pub pt_ms: u64,
    pub bit: u32,
    pub value: bool,
    pub w8: u8,
    pub w16: u16,
    pub w32: u32,
    pub w64: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub ton: bool,
    pub ton_q: bool,
    pub ton_et: Duration,
    pub tof: bool,
    pub tof_q: bool,
    pub tof_et: Duration,
    pub tp: bool,
    pub tp_q: bool,
    pub tp_et: Duration,
    pub rising: bool,
    pub rising_q: bool,
    pub falling: bool,
    pub falling_q: bool,
    pub counted: bool,
    pub cv: i32,
    pub counter_q: bool,
    pub get8: bool,
    pub get16: bool,
    pub get32: bool,
    pub get64: bool,
    pub with8: u8,
    pub with16: u16,
    pub with32: u32,
    pub with64: u64,
    pub set8: u8,
    pub set16: u16,
    pub set32: u32,
    pub set64: u64,
    pub widths: u32,
}

#[derive(Debug, Default)]
pub struct Probe {
    ton: Ton,
    tof: Tof,
    tp: Tp,
    rising: RTrig,
    falling: FTrig,
    counter: Ctu,
    w8: u8,
    w16: u16,
    w32: u32,
    w64: u64,
}

impl Program for Probe {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, scan: &Scan, i: &Inputs, o: &mut Outputs) {
        o.ton = self.ton.scan(scan, i.a, ms(i.pt_ms));
        o.ton_q = self.ton.q();
        o.ton_et = self.ton.et();
        o.tof = self.tof.scan(scan, i.b, ms(i.pt_ms / 2));
        o.tof_q = self.tof.q();
        o.tof_et = self.tof.et();
        o.tp = self.tp.scan(scan, i.a, if i.b { s(1) } else { ms(i.pt_ms) });
        o.tp_q = self.tp.q();
        o.tp_et = self.tp.et();

        o.rising = self.rising.scan(i.b);
        o.rising_q = self.rising.q();
        o.falling = self.falling.scan(i.b);
        o.falling_q = self.falling.q();

        o.counted = self.counter.scan(i.a, i.reset, i.preset);
        o.cv = self.counter.cv();
        o.counter_q = self.counter.q();

        o.get8 = i.w8.get_bit(i.bit);
        o.get16 = i.w16.get_bit(i.bit);
        o.get32 = i.w32.get_bit(i.bit);
        o.get64 = i.w64.get_bit(i.bit);
        o.with8 = i.w8.with_bit(i.bit, i.value);
        o.with16 = i.w16.with_bit(i.bit, i.value);
        o.with32 = i.w32.with_bit(i.bit, i.value);
        o.with64 = i.w64.with_bit(i.bit, i.value);
        // named by its trait: the nightly's core has an unstable `set_bit` of its own (`docs/orcl/front-end.md`)
        Bits::set_bit(&mut self.w8, i.bit, i.value);
        Bits::set_bit(&mut self.w16, i.bit, !i.value);
        Bits::set_bit(&mut self.w32, i.bit, i.value);
        Bits::set_bit(&mut self.w64, i.bit, !i.value);
        o.set8 = self.w8;
        o.set16 = self.w16;
        o.set32 = self.w32;
        o.set64 = self.w64;
        o.widths = u8::WIDTH + u16::WIDTH + u32::WIDTH + u64::WIDTH;
    }

    fn safe_outputs(&self) -> Outputs {
        Outputs { tof: true, cv: -1, with64: u64::MAX, ton_et: s(3), ..Outputs::default() }
    }
}
