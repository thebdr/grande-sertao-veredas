//! A conformance program: sound code as a program might write it, beyond the conformance programs:
//! destructuring assignment, nested repeats, a repeat of a constant `Duration`, a labeled block's value, `if let`
//! chains, `ref mut` and `&x` patterns, an associated constant of a const-generic impl, casts between widths and
//! signs, `&mut Self` chained, a `while` with `continue`, a constant table of tuples, `matches!` with alternatives,
//! an array of options written at run time.
#![forbid(unsafe_code)]
// the `&x` patterns are the point: a program may write them
#![allow(clippy::match_ref_pats)]

use orcl_std::{Duration, Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: u8,
    pub b: u8,
    pub s: i8,
    pub w: u64,
    pub sh: u8,
    pub t_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub swapped: u16,
    pub grid: u16,
    pub longest: Duration,
    pub labeled: i16,
    pub chain: u8,
    pub pattern: u16,
    pub twice: u16,
    pub casts: i32,
    pub wide: u64,
    pub shifted: i64,
    pub acc: u16,
    pub skipped: u8,
    pub table: i16,
    pub kind: u8,
    pub filled: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Idle,
    Low(u8),
    High { level: u8, hot: bool },
    Fault,
}

impl Kind {
    fn of(v: u8) -> Kind {
        match v {
            0 => Kind::Idle,
            1..=99 => Kind::Low(v),
            100..=249 => Kind::High { level: v - 100, hot: v > 200 },
            _ => Kind::Fault,
        }
    }

    fn code(self) -> u8 {
        match self {
            Kind::Idle => 1,
            Kind::Low(v) if v % 2 == 0 => 2,
            Kind::Low(_) => 3,
            Kind::High { hot: true, .. } => 4,
            Kind::High { level, .. } => 5 + level % 3,
            Kind::Fault => 9,
        }
    }
}

struct Gain<const N: u8>;

impl<const N: u8> Gain<N> {
    const TWICE: u16 = N as u16 * 2;

    fn apply(v: u8) -> u16 {
        (v as u16).wrapping_mul(Self::TWICE)
    }
}

#[derive(Clone, Copy, Default)]
struct Acc {
    total: u16,
    count: u8,
}

impl Acc {
    fn add(&mut self, v: u8) -> &mut Self {
        self.total = self.total.wrapping_add(v as u16);
        self.count = self.count.wrapping_add(1);
        self
    }
}

const TABLE: [(u8, i16); 4] = [(3, -300), (7, 1200), (0, -1), (255, i16::MIN)];

const ZERO: Duration = Duration::from_millis(0);

#[derive(Clone, Copy)]
pub struct Patterns {
    grid: [[u8; 3]; 2],
    times: [Duration; 3],
    held: Option<u8>,
    acc: Acc,
    slots: [Option<u8>; 4],
    k: u8,
}

impl Default for Patterns {
    fn default() -> Self {
        Patterns { grid: [[0; 3]; 2], times: [ZERO; 3], held: None, acc: Acc::default(), slots: [None; 4], k: 0 }
    }
}

fn longest(t: &[Duration; 3]) -> Duration {
    let mut best = t[0];
    if t[1] > best {
        best = t[1];
    }
    if t[2] > best {
        best = t[2];
    }
    best
}

impl Program for Patterns {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        // destructuring assignment
        let (mut x, mut y) = (i.a, i.b);
        if x > y {
            (x, y) = (y, x);
        }
        o.swapped = (x as u16) * 256 + y as u16;

        // nested repeat, written at run time
        let r = i.a % 2;
        let c = i.b % 3;
        self.grid[r as usize][c as usize] = self.grid[r as usize][c as usize].wrapping_add(i.a);
        o.grid = self.grid[0][0] as u16 + self.grid[0][1] as u16 + self.grid[0][2] as u16 + self.grid[1][0] as u16
            + self.grid[1][1] as u16
            + self.grid[1][2] as u16;

        // a repeat of a constant Duration
        self.k = (self.k + 1) % 3;
        self.times[self.k as usize] = Duration::from_millis(i.t_ms);
        o.longest = longest(&self.times);

        // a labeled block's value
        o.labeled = 'found: {
            if i.s < -100 {
                break 'found -1;
            }
            if i.s > 100 {
                break 'found 1;
            }
            (i.s as i16) * 3
        };

        // `if let` chains, `ref mut`
        match self.held {
            Some(ref mut v) if *v < 200 => *v += 1,
            Some(_) => self.held = None,
            None if i.a % 5 == 0 => self.held = Some(i.b % 100),
            None => {}
        }
        o.chain = if let Some(v) = self.held {
            v
        } else if let Kind::Low(v) = Kind::of(i.a) {
            v / 2
        } else {
            0
        };

        // `&x` patterns over a reference to an array
        let row = &self.grid[r as usize];
        o.pattern = match row {
            &[0, 0, 0] => 1,
            &[a, 0, _] => 100 + a as u16,
            &[_, b, c] => (b as u16) ^ (c as u16),
        };

        // an associated constant of a const-generic impl
        o.twice = Gain::<3>::apply(i.a).wrapping_add(Gain::<200>::apply(i.b));

        // casts between widths and signs
        let narrowed = (i.w as i16) as u8;
        let signed = narrowed as i8;
        let widened = signed as u16;
        o.casts = (widened as i32) - (i.s as i32) + (i.a as i8 as i32) + (true as i32) * (i.b > 128) as i32;

        // a u64 through an f64 and back
        o.wide = ((i.w >> 11) as f64 * 3.0) as u64;

        // an arithmetic shift of a negative value
        o.shifted = ((i.s as i64) * 1_000_003) >> (i.sh % 64);

        // `&mut Self` chained
        self.acc.add(i.a).add(i.b).add(7);
        o.acc = self.acc.total ^ self.acc.count as u16;

        // a `while` with `continue`
        let mut n = 0u8;
        let mut skipped = 0u8;
        while n < 8 {
            n += 1;
            if (i.a >> (n - 1)) & 1 == 0 {
                skipped += 1;
                continue;
            }
            skipped = skipped.wrapping_mul(2);
        }
        o.skipped = skipped;

        // a constant table of tuples
        let (key, value) = TABLE[(i.b % 4) as usize];
        o.table = if key > i.a { value } else { value.wrapping_neg_free() };

        // `matches!` with alternatives, and the kind's code
        let kind = Kind::of(i.b);
        o.kind = kind.code() + if matches!(kind, Kind::Idle | Kind::Fault) { 10 } else { 0 };

        // an array of options written at run time
        self.slots[(i.a % 4) as usize] = if i.b % 3 == 0 { None } else { Some(i.b) };
        let mut filled = 0u8;
        if self.slots[0].is_some() {
            filled += 1;
        }
        if self.slots[1].is_some() {
            filled += 2;
        }
        if self.slots[2].is_some() {
            filled += 4;
        }
        if self.slots[3].is_some() {
            filled += 8;
        }
        o.filled = filled;
    }
}

trait NegFree {
    fn wrapping_neg_free(self) -> Self;
}

impl NegFree for i16 {
    fn wrapping_neg_free(self) -> i16 {
        if self == i16::MIN { self } else { -self }
    }
}
