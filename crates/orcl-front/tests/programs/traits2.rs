//! A conformance program: calls resolved through traits as a program might write them: nested generic impls,
//! a default method reading an associated constant one impl overrides, an associated function through a bound, an
//! inherent method and a trait's method of one name, two traits' methods of one name by their paths, a supertrait's
//! method through its subtrait's bound, a program's trait on an array, on `u8` and on `Option<u8>`, and a blanket impl.
#![forbid(unsafe_code)]

use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub v: i16,
    pub k: u8,
    pub on: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub low: i16,
    pub dead: i16,
    pub chained: i16,
    pub chained2: i16,
    pub gains: i16,
    pub made: i16,
    pub same_name: u16,
    pub by_path: u16,
    pub sup: u16,
    pub arr: u16,
    pub byte: u16,
    pub opt: u16,
    pub blanket: i16,
}

trait Filter {
    const GAIN: i16 = 1;
    fn step(&mut self, v: i16) -> i16;
    fn scaled(&mut self, v: i16) -> i16 {
        self.step(v).wrapping_mul(Self::GAIN)
    }
    fn fresh() -> Self;
}

#[derive(Clone, Copy, Default)]
struct Lowpass {
    y: i16,
}

impl Filter for Lowpass {
    fn step(&mut self, v: i16) -> i16 {
        self.y = self.y.wrapping_add((v.wrapping_sub(self.y)) / 4);
        self.y
    }
    fn fresh() -> Self {
        Lowpass { y: 10 }
    }
}

#[derive(Clone, Copy, Default)]
struct Deadband {
    band: i16,
}

impl Filter for Deadband {
    const GAIN: i16 = 3;
    fn step(&mut self, v: i16) -> i16 {
        if v > self.band || v < -self.band { v } else { 0 }
    }
    fn fresh() -> Self {
        Deadband { band: 50 }
    }
}

#[derive(Clone, Copy, Default)]
struct Chain<T> {
    inner: T,
    last: i16,
}

impl<T: Filter> Filter for Chain<T> {
    const GAIN: i16 = 2;
    fn step(&mut self, v: i16) -> i16 {
        let out = self.inner.step(v);
        let diff = out.wrapping_sub(self.last);
        self.last = out;
        diff
    }
    fn fresh() -> Self {
        Chain { inner: T::fresh(), last: 0 }
    }
}

fn run<F: Filter>(f: &mut F, v: i16) -> i16 {
    f.scaled(v)
}

fn made<F: Filter>(v: i16) -> i16 {
    let mut f = F::fresh();
    f.step(v)
}

struct Meter {
    n: u16,
}

impl Meter {
    fn read(&self) -> u16 {
        self.n
    }
}

trait Read {
    fn read(&self) -> u16;
}

impl Read for Meter {
    fn read(&self) -> u16 {
        self.n.wrapping_mul(10)
    }
}

trait Left {
    fn value(&self) -> u16;
}

trait Right {
    fn value(&self) -> u16;
}

impl Left for Meter {
    fn value(&self) -> u16 {
        self.n.wrapping_add(1)
    }
}

impl Right for Meter {
    fn value(&self) -> u16 {
        self.n.wrapping_add(1000)
    }
}

trait Base {
    fn base(&self) -> u16;
}

trait Derived: Base {
    fn derived(&self) -> u16 {
        self.base().wrapping_mul(2)
    }
}

impl Base for Meter {
    fn base(&self) -> u16 {
        self.n ^ 0x55
    }
}

impl Derived for Meter {}

fn through<D: Derived>(d: &D) -> u16 {
    d.base().wrapping_add(d.derived())
}

trait Sum {
    fn sum(&self) -> u16;
}

impl Sum for [u8; 3] {
    fn sum(&self) -> u16 {
        self[0] as u16 + self[1] as u16 + self[2] as u16
    }
}

impl Sum for u8 {
    fn sum(&self) -> u16 {
        (*self as u16) * 3
    }
}

impl Sum for Option<u8> {
    fn sum(&self) -> u16 {
        match self {
            Some(v) => *v as u16 + 1,
            None => 0,
        }
    }
}

trait Report {
    fn report(&mut self, v: i16) -> i16;
}

impl<T: Filter> Report for T {
    fn report(&mut self, v: i16) -> i16 {
        self.step(v).wrapping_add(T::GAIN)
    }
}

#[derive(Clone, Copy)]
pub struct Traits2 {
    low: Lowpass,
    dead: Deadband,
    chain: Chain<Lowpass>,
    chain2: Chain<Chain<Deadband>>,
    held: [u8; 3],
    last: Option<u8>,
}

// written out: a derived `Default` calls `<[u8; 3] as Default>::default`, outside the closed list
#[allow(clippy::derivable_impls)]
impl Default for Traits2 {
    fn default() -> Self {
        Traits2 {
            low: Lowpass::default(),
            dead: Deadband::default(),
            chain: Chain::default(),
            chain2: Chain::default(),
            held: [0; 3],
            last: None,
        }
    }
}

impl Program for Traits2 {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        if self.dead.band == 0 {
            self.dead = Deadband::fresh();
            self.chain2 = Chain::fresh();
        }
        o.low = run(&mut self.low, i.v);
        o.dead = run(&mut self.dead, i.v);
        o.chained = run(&mut self.chain, i.v);
        o.chained2 = run(&mut self.chain2, i.v);
        o.gains = Lowpass::GAIN * 100 + Deadband::GAIN * 10 + <Chain<Deadband> as Filter>::GAIN;
        o.made = made::<Chain<Deadband>>(i.v).wrapping_add(made::<Lowpass>(i.v));
        let m = Meter { n: i.k as u16 };
        o.same_name = m.read().wrapping_add(Read::read(&m));
        o.by_path = Left::value(&m) ^ <Meter as Right>::value(&m);
        o.sup = through(&m);
        self.held[(i.k % 3) as usize] = i.k;
        o.arr = self.held.sum();
        o.byte = i.k.sum();
        self.last = if i.on { Some(i.k) } else { None };
        o.opt = self.last.sum();
        o.blanket = self.dead.report(i.v).wrapping_add(self.low.report(i.v));
    }
}
