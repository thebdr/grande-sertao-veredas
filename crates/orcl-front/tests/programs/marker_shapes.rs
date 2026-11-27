//! A generic function at types the program holds no value of, each beside its twin at a type of the same name in
//! another module: a reference, a tuple, a slice, a raw pointer and core's `PhantomData` of a marker, and a program's
//! own `PhantomData` beside core's: the names the file gives the instances.
#![forbid(unsafe_code)]
use core::marker::PhantomData;
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub x: u8,
    pub y: u8,
}

pub trait Rate {
    const RATE: u8;
}

pub mod a {
    pub struct Marker;
}

pub mod b {
    pub struct Marker;
}

pub mod mine {
    #[derive(Debug, Default)]
    pub struct PhantomData;
}

impl Rate for &a::Marker {
    const RATE: u8 = 1;
}
impl Rate for &b::Marker {
    const RATE: u8 = 2;
}
impl Rate for (a::Marker, u8) {
    const RATE: u8 = 3;
}
impl Rate for (b::Marker, u8) {
    const RATE: u8 = 4;
}
impl Rate for [a::Marker] {
    const RATE: u8 = 5;
}
impl Rate for [b::Marker] {
    const RATE: u8 = 6;
}
impl Rate for *const a::Marker {
    const RATE: u8 = 7;
}
impl Rate for *const b::Marker {
    const RATE: u8 = 8;
}
impl Rate for PhantomData<a::Marker> {
    const RATE: u8 = 9;
}
impl Rate for PhantomData<b::Marker> {
    const RATE: u8 = 10;
}

pub fn rate<M: Rate + ?Sized>(x: u8) -> u8 {
    x.wrapping_mul(M::RATE)
}

#[derive(Debug, Default)]
pub struct QShapes {
    own: mine::PhantomData,
}

impl Program for QShapes {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        o.x = rate::<&a::Marker>(i.a)
            .wrapping_add(rate::<(a::Marker, u8)>(i.a))
            .wrapping_add(rate::<[a::Marker]>(i.a))
            .wrapping_add(rate::<*const a::Marker>(i.a))
            .wrapping_add(rate::<PhantomData<a::Marker>>(i.a));
        o.y = rate::<&b::Marker>(i.a)
            .wrapping_add(rate::<(b::Marker, u8)>(i.a))
            .wrapping_add(rate::<[b::Marker]>(i.a))
            .wrapping_add(rate::<*const b::Marker>(i.a))
            .wrapping_add(rate::<PhantomData<b::Marker>>(i.a));
    }
}
