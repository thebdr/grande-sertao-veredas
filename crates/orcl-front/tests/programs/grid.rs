//! A conformance program: a block generic over two const arguments, `Grid<2, 3>` and `Grid<2, 4>` side by side.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub v: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub small: u16,
    pub large: u16,
}

#[derive(Debug)]
pub struct Grid<const W: usize, const H: usize> {
    cells: [[u16; W]; H],
    at: u8,
}

impl<const W: usize, const H: usize> Grid<W, H> {
    fn new() -> Self {
        Grid { cells: [[0; W]; H], at: 0 }
    }

    fn push(&mut self, v: u16) -> u16 {
        let n = (W * H) as u8;
        let k = self.at % n;
        self.cells[(k as usize) / W][(k as usize) % W] = v;
        self.at = self.at.wrapping_add(1);
        let mut sum = 0u16;
        let mut r = 0u8;
        while (r as usize) < H {
            sum = sum.wrapping_add(self.cells[r as usize][W - 1]);
            r += 1;
        }
        sum
    }
}

#[derive(Debug)]
pub struct Grids {
    small: Grid<2, 3>,
    large: Grid<2, 4>,
}

impl Default for Grids {
    fn default() -> Grids {
        Grids { small: Grid::new(), large: Grid::new() }
    }
}

impl Program for Grids {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        o.small = self.small.push(i.v);
        o.large = self.large.push(i.v);
    }
}
