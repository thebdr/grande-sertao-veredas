//! A conformance program: references held in a structure of the program's with a lifetime argument, references chosen
//! and returned at run time, an `Option` of a reference built and matched, a `&mut` handed down two calls, an opaque
//! return type (`impl Shape`) and a trait's associated type, each as a program might write them.
#![forbid(unsafe_code)]

use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: u8,
    pub b: u8,
    pub sel: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub x: u8,
    pub y: u16,
    pub found: bool,
    pub z: u8,
    pub area: u16,
    pub got: i16,
}

struct Ctx<'a> {
    inputs: &'a Inputs,
    table: &'a mut [u8; 4],
    hits: u8,
}

impl<'a> Ctx<'a> {
    fn slot(&mut self, k: u8) -> &mut u8 {
        &mut self.table[(k % 4) as usize]
    }

    fn pick(&self) -> &'a u8 {
        if self.inputs.sel % 2 == 0 { &self.inputs.a } else { &self.inputs.b }
    }
}

fn larger<'x>(p: &'x u8, q: &'x u8) -> &'x u8 {
    if *p >= *q { p } else { q }
}

fn find(t: &[u8; 4], v: u8) -> Option<&u8> {
    if t[0] == v {
        Some(&t[0])
    } else if t[1] == v {
        Some(&t[1])
    } else if t[2] == v {
        Some(&t[2])
    } else if t[3] == v {
        Some(&t[3])
    } else {
        None
    }
}

fn add_twice(v: &mut u16, by: u8) {
    add_once(v, by);
    add_once(v, by);
}

fn add_once(v: &mut u16, by: u8) {
    *v = v.wrapping_add(by as u16);
}

trait Shape {
    type Out;
    fn area(&self) -> Self::Out;
}

#[derive(Clone, Copy)]
struct Square(u8);

impl Shape for Square {
    type Out = u16;
    fn area(&self) -> u16 {
        (self.0 as u16) * (self.0 as u16)
    }
}

struct Signed(i8);

impl Shape for Signed {
    type Out = i16;
    fn area(&self) -> i16 {
        (self.0 as i16).saturating_mul(3)
    }
}

fn make(v: u8) -> impl Shape<Out = u16> {
    Square(v % 200)
}

fn measure<S: Shape>(s: &S) -> S::Out {
    s.area()
}

#[derive(Debug)]
pub struct PRefs {
    table: [u8; 4],
    count: u16,
}

// Written out: an array's `Default` is off the closed list.
#[allow(clippy::derivable_impls)]
impl Default for PRefs {
    fn default() -> PRefs {
        PRefs { table: [0; 4], count: 0 }
    }
}

impl Program for PRefs {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        let mut ctx = Ctx { inputs: i, table: &mut self.table, hits: 0 };
        let s = ctx.slot(i.a);
        *s = s.wrapping_add(i.b);
        ctx.hits = ctx.hits.wrapping_add(1 + i.sel % 3);
        let p = ctx.pick();
        o.x = *larger(p, &i.b);
        let hits = ctx.hits;
        o.found = match find(&self.table, i.b) {
            Some(r) => {
                o.z = *r;
                true
            }
            None => false,
        };
        add_twice(&mut self.count, hits);
        o.y = self.count;
        o.area = make(i.a).area();
        o.got = measure(&Signed(i.sel as i8));
    }
}
