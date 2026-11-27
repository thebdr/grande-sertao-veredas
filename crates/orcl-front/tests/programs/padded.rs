//! Constant enums whose largest variant holds an array or a tuple, met in a smaller variant too: the slots past a
//! variant's fields are unwritten, as many as the largest variant's array or tuple takes.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub sel: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub cmd: u16,
    pub pair: u16,
    pub recipe: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Idle,
    Set([u8; 3]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pair {
    Idle,
    Both((u16, [u8; 2])),
}

const CMDS: [Cmd; 2] = [Cmd::Idle, Cmd::Set([1, 2, 3])];
const PAIRS: [Pair; 2] = [Pair::Idle, Pair::Both((500, [7, 8]))];
const RECIPES: [Option<[u16; 4]>; 2] = [None, Some([10, 20, 30, 40])];

#[derive(Debug, Default)]
pub struct Padded {
    n: u8,
}

impl Program for Padded {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        self.n = self.n.wrapping_add(1);
        o.cmd = match CMDS[(i.sel % 2) as usize] {
            Cmd::Idle => 1,
            Cmd::Set([a, b, c]) => a as u16 * 100 + b as u16 * 10 + c as u16,
        };
        o.pair = match PAIRS[(i.sel % 2) as usize] {
            Pair::Idle => 2,
            Pair::Both((v, [x, y])) => v + x as u16 + y as u16,
        };
        o.recipe = match RECIPES[(i.sel % 2) as usize] {
            None => 3,
            Some([a, b, c, d]) => a + b + c + d,
        };
    }
}
