//! A program the front end refuses: `usize` as the program declares it, a parameter.
use orcl_std::{Program, Scan};

pub struct P {
    table: [u8; 4],
}

impl Default for P {
    fn default() -> P {
        P { table: [5, 6, 7, 8] }
    }
}

fn get(table: &[u8; 4], at: usize) -> u8 {
    table[at % 4]
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        *o = get(&self.table, *i as usize);
    }
}
