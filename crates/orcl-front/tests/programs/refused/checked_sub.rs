//! Refused: an inherent Duration operation outside the list.
use orcl_std::{Program, Scan};

use core::time::Duration;

#[derive(Default)]
pub struct P {
    left: Duration,
}

impl Program for P {
    type Inputs = bool;
    type Outputs = bool;

    fn scan(&mut self, scan: &Scan, i: &bool, o: &mut bool) {
        *o = self.left.checked_sub(scan.period).is_some();
        if *i {
            self.left = Duration::from_millis(100);
        }
    }
}
