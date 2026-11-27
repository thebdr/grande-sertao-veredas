//! The scalar names a type's arguments and a tuple's parts show, a const argument a renamed function shows, items of
//! one name whose paths from the crate's root differ above their last module, and a trait's arguments telling apart
//! two functions of one type: the names the file gives them.
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
    pub z: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct Hold<T> {
    pub v: T,
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
    }
}

/// A structure generic over a const and a type, whose method shows both.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pack<const N: u8, T> {
    pub v: T,
}
impl<const N: u8, T> Pack<N, T> {
    pub fn f(&mut self, a: u8) -> u8 {
        a.wrapping_mul(N)
    }
}

pub mod l1 {
    pub mod belt {
        #[derive(Clone, Copy, Debug, Default)]
        pub struct Motor {
            pub on: bool,
        }
        pub fn step(m: &mut Motor, a: u8) -> u8 {
            m.on = a % 2 == 0;
            a.wrapping_add(1)
        }
    }
}
pub mod l2 {
    pub mod belt {
        #[derive(Clone, Copy, Debug, Default)]
        pub struct Motor {
            pub on: bool,
        }
        pub fn step(m: &mut Motor, a: u8) -> u8 {
            m.on = a % 3 == 0;
            a.wrapping_add(2)
        }
    }
}

/// A trait generic over a marker, implemented for one type at two markers of one name the program holds no value of:
/// the functions differ only in the trait's argument.
pub trait Conv<T> {
    fn c(&self) -> u8;
}
pub mod ma {
    pub struct Marker;
}
pub mod mb {
    pub struct Marker;
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Pick {
    pub n: u8,
}
impl Conv<ma::Marker> for Pick {
    fn c(&self) -> u8 {
        self.n
    }
}
impl Conv<mb::Marker> for Pick {
    fn c(&self) -> u8 {
        self.n.wrapping_add(1)
    }
}

#[derive(Debug)]
pub struct QNames {
    pick: Pick,
    h_i32: Hold<i32>,
    h_i64: Hold<i64>,
    h_u32: Hold<u32>,
    h_u64: Hold<u64>,
    h_bool: Hold<bool>,
    h_u8: Hold<u8>,
    h_u16: Hold<u16>,
    h_i16: Hold<i16>,
    h_a2: Hold<[u8; 2]>,
    h_a4: Hold<[u8; 4]>,
    p2: Pack<2, fast::Timer>,
    p3: Pack<3, fast::Timer>,
    ps: Pack<2, slow::Timer>,
    m1: l1::belt::Motor,
    m2: l2::belt::Motor,
    t: [u8; 8],
}

impl Default for QNames {
    fn default() -> Self {
        QNames {
            pick: Pick { n: 4 },
            h_i32: Hold { v: 0 },
            h_i64: Hold { v: 0 },
            h_u32: Hold { v: 0 },
            h_u64: Hold { v: 0 },
            h_bool: Hold { v: false },
            h_u8: Hold { v: 0 },
            h_u16: Hold { v: 0 },
            h_i16: Hold { v: 0 },
            h_a2: Hold { v: [0; 2] },
            h_a4: Hold { v: [0; 4] },
            p2: Pack { v: fast::Timer { n: 0 } },
            p3: Pack { v: fast::Timer { n: 1 } },
            ps: Pack { v: slow::Timer { n: 2 } },
            m1: l1::belt::Motor { on: false },
            m2: l2::belt::Motor { on: true },
            t: [0; 8],
        }
    }
}

impl Program for QNames {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.h_i32.v = self.h_i32.v.wrapping_add(i.a as i32);
        self.h_i64.v = self.h_i64.v.wrapping_add(i.a as i64);
        self.h_u32.v = self.h_u32.v.wrapping_add(i.a as u32);
        self.h_u64.v = self.h_u64.v.wrapping_add(i.a as u64);
        self.h_bool.v = !self.h_bool.v;
        self.h_u8.v = i.a;
        self.h_u16.v = i.a as u16;
        self.h_i16.v = i.a as i16;
        self.h_a2.v[0] = i.a;
        self.h_a4.v[3] = i.a;
        // shared and mutable references in tuples
        let mut w = 3u8;
        let shared: (&u8, u8) = (&self.h_u8.v, 1);
        let x = *shared.0;
        let mutable: (&mut u8, u8) = (&mut w, 2);
        *mutable.0 = x;
        // a checked sum of `usize` temporaries and one of `u64`s
        let k = i.a % 4;
        self.t[(k as usize) + 1] = i.a;
        o.z = self.h_u64.v + 1;
        o.x = self.p2.f(i.a).wrapping_add(self.p3.f(i.a)).wrapping_add(self.ps.f(w));
        o.y = l1::belt::step(&mut self.m1, i.a).wrapping_add(l2::belt::step(&mut self.m2, i.a));
        self.pick.n = i.a;
        o.y = o.y.wrapping_add(<Pick as Conv<ma::Marker>>::c(&self.pick)).wrapping_add(<Pick as Conv<mb::Marker>>::c(&self.pick));
    }
}
