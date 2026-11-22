//! Edge triggers (IEC R_TRIG / F_TRIG): `q` is on for exactly one scan when the signal changes.

/// Rising edge (IEC R_TRIG). As in IEC, a signal that is already on at the first scan counts as a rising edge.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RTrig {
    q: bool,
    last: bool,
}

impl RTrig {
    pub fn scan(&mut self, clk: bool) -> bool {
        self.q = clk && !self.last;
        self.last = clk;
        self.q
    }

    pub fn q(&self) -> bool {
        self.q
    }
}

/// Falling edge (IEC F_TRIG). A signal that is off at the first scan is not an edge.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FTrig {
    q: bool,
    last: bool,
}

impl FTrig {
    pub fn scan(&mut self, clk: bool) -> bool {
        self.q = !clk && self.last;
        self.last = clk;
        self.q
    }

    pub fn q(&self) -> bool {
        self.q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rising_edge_is_one_scan_long() {
        let mut r = RTrig::default();
        let seen: [bool; 5] = [false, true, true, false, true].map(|clk| r.scan(clk));
        assert_eq!(seen, [false, true, false, false, true]);
    }

    #[test]
    fn falling_edge_is_one_scan_long_and_not_at_start() {
        let mut f = FTrig::default();
        let seen: [bool; 5] = [false, true, false, false, true].map(|clk| f.scan(clk));
        assert_eq!(seen, [false, false, true, false, false]);
    }
}
