//! Counters (IEC CTU). The count input is edge-triggered, and the count saturates instead of overflowing.

/// Up counter (IEC CTU). `cv` counts the rising edges of `cu`; `reset` clears it and wins over counting.
/// `q` is on while `cv >= pv`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Ctu {
    cv: i32,
    q: bool,
    last_cu: bool,
}

impl Ctu {
    pub fn scan(&mut self, cu: bool, reset: bool, pv: i32) -> bool {
        let rising = cu && !self.last_cu;
        self.last_cu = cu;
        if reset {
            self.cv = 0;
        } else if rising {
            self.cv = self.cv.saturating_add(1);
        }
        self.q = self.cv >= pv;
        self.q
    }

    pub fn cv(&self) -> i32 {
        self.cv
    }

    pub fn q(&self) -> bool {
        self.q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_rising_edges_until_reset() {
        let mut c = Ctu::default();
        for cu in [true, true, false, true, false, true] {
            c.scan(cu, false, 3);
        }
        assert_eq!(c.cv(), 3, "three rising edges; a held input counts once");
        assert!(c.q());
        c.scan(true, true, 3);
        assert_eq!((c.cv(), c.q()), (0, false), "reset wins over counting");
    }

    #[test]
    fn saturates_instead_of_overflowing() {
        let mut c = Ctu {
            cv: i32::MAX,
            q: false,
            last_cu: false,
        };
        c.scan(true, false, 1);
        assert_eq!(c.cv(), i32::MAX);
    }
}
