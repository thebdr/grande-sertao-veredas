//! A program the front end lowers and the interpreter's loader refuses: its state past the driver's check's room.
use orcl_std::{Program, Scan};

pub struct P {
    table: [[u8; 60_000]; 600],
}

impl Default for P {
    fn default() -> P {
        P { table: [[0; 60_000]; 600] }
    }
}

impl Program for P {
    type Inputs = u8;
    type Outputs = u8;

    fn scan(&mut self, _scan: &Scan, i: &u8, o: &mut u8) {
        *o = i.wrapping_add(self.table[1][2]);
    }
}
