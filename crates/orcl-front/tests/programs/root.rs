//! A program's own `Option` at the crate's root beside core's, a root `Timer` beside `fast::Timer`, a module `timers`
//! with its own `Ton` beside orcl-std's, two traits of one name implemented for one type, and a generic over both
//! `Timer`s: names at the crate's root and by the crate's name.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan, Ton, ms};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: u8,
    pub on: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub x: u8,
    pub y: u8,
    pub q: bool,
    pub k: u8,
    pub f: u8,
    pub h: u8,
    pub c: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Option {
    #[default]
    Nothing,
    Some(u8),
}

#[derive(Debug, Default)]
pub struct Timer {
    pub n: u8,
}

pub mod fast {
    #[derive(Debug, Default)]
    pub struct Timer {
        pub n: u8,
    }
}

pub mod timers {
    #[derive(Debug, Default)]
    pub struct Ton {
        pub k: u8,
    }
    impl Ton {
        pub fn scan(&mut self, a: u8) -> u8 {
            self.k = self.k.wrapping_add(a);
            self.k
        }
    }
}

pub mod a {
    pub trait Tr {
        fn f(&self) -> u8;
    }
}
pub mod b {
    pub trait Tr {
        fn f(&self) -> u8;
    }
}
impl a::Tr for Timer {
    fn f(&self) -> u8 {
        self.n
    }
}
impl b::Tr for Timer {
    fn f(&self) -> u8 {
        self.n.wrapping_add(7)
    }
}

#[derive(Debug, Default)]
pub struct Hold<T> {
    pub v: T,
    pub c: u8,
}
impl<T> Hold<T> {
    pub fn bump(&mut self, by: u8) -> u8 {
        self.c = self.c.wrapping_add(by);
        self.c
    }
}

#[derive(Debug, Default)]
pub struct QRoot {
    root: Timer,
    fast: fast::Timer,
    mine: timers::Ton,
    ton: Ton,
    own: Option,
    core: core::option::Option<u8>,
    hr: Hold<Timer>,
    hf: Hold<fast::Timer>,
}

impl Program for QRoot {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.root.n = self.root.n.wrapping_add(i.a);
        self.fast.n = self.fast.n.wrapping_add(1);
        o.x = self.root.n;
        o.y = self.fast.n;
        o.q = self.ton.scan(scan, i.on, ms(30));
        o.k = self.mine.scan(i.a);
        o.f = a::Tr::f(&self.root).wrapping_add(b::Tr::f(&self.root) / 2);
        self.own = if i.a % 3 == 0 { Option::Nothing } else { Option::Some(i.a) };
        self.core = if i.a % 2 == 0 { None } else { Some(i.a / 2) };
        o.c = match (self.own, self.core) {
            (Option::Some(v), Some(w)) => v.wrapping_add(w),
            (Option::Some(v), None) => v,
            (Option::Nothing, Some(w)) => w,
            (Option::Nothing, None) => 0,
        };
        o.h = self.hr.bump(1).wrapping_add(self.hf.bump(2));
    }
}
