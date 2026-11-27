//! Types of one name in two modules reached only deep in other types: a generic function at an array of each (an
//! associated constant, the array in no local), tuples of each, and a generic of a generic of each.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub x: u8,
    pub y: u8,
    pub t: u8,
    pub h: u8,
}

pub mod fast {
    #[derive(Clone, Copy, Debug, Default)]
    pub struct Timer {
        pub n: u8,
    }
}

pub mod slow {
    #[derive(Clone, Copy, Debug, Default)]
    pub struct Timer {
        pub n: u8,
        pub k: u8,
    }
}

pub trait Tagged {
    const TAG: u8;
}
impl Tagged for [fast::Timer; 2] {
    const TAG: u8 = 1;
}
impl Tagged for [slow::Timer; 2] {
    const TAG: u8 = 2;
}

pub fn tag<T: Tagged>(a: u8) -> u8 {
    a.wrapping_add(T::TAG)
}

pub fn pick<T: Copy>(a: T, b: T, first: bool) -> T {
    if first { a } else { b }
}

#[derive(Debug, Default)]
pub struct Hold<T> {
    pub v: T,
    pub c: u8,
}

#[derive(Debug)]
pub struct QDeep {
    f: fast::Timer,
    s: slow::Timer,
    tf: (fast::Timer, u8),
    ts: (slow::Timer, u8),
    hf: Hold<Hold<fast::Timer>>,
    hs: Hold<Hold<slow::Timer>>,
    f32s: Hold<f32>,
    f64s: Hold<f64>,
}

// written out: a derived `Default` would call `<(fast::Timer, u8) as Default>::default`, core's and outside the list
#[allow(clippy::derivable_impls)]
impl Default for QDeep {
    fn default() -> QDeep {
        QDeep {
            f: fast::Timer::default(),
            s: slow::Timer::default(),
            tf: (fast::Timer::default(), 0),
            ts: (slow::Timer::default(), 0),
            hf: Hold::default(),
            hs: Hold::default(),
            f32s: Hold::default(),
            f64s: Hold::default(),
        }
    }
}

impl Program for QDeep {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.f.n = self.f.n.wrapping_add(i.a);
        self.s.k = self.s.k.wrapping_add(1);
        o.x = tag::<[fast::Timer; 2]>(self.f.n);
        o.y = tag::<[slow::Timer; 2]>(self.s.k);
        self.tf.1 = self.tf.1.wrapping_add(1);
        self.ts.1 = self.ts.1.wrapping_add(2);
        let tf = pick(self.tf, self.tf, i.a % 2 == 0);
        let ts = pick(self.ts, self.ts, i.a % 3 == 0);
        o.t = tf.1.wrapping_add(ts.1);
        self.f32s.c = self.f32s.c.wrapping_add(1);
        self.f64s.c = self.f64s.c.wrapping_add(self.f32s.c);
        self.hf.v.c = self.hf.v.c.wrapping_add(i.a);
        self.hs.c = self.hs.c.wrapping_add(3);
        o.h = self.hf.v.c.wrapping_add(self.hs.c);
    }
}
