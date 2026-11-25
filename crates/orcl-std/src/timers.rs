//! IEC 61131-3 style timers. Each is a function block: a struct holding its state, with a `scan` method called
//! once per scan. Time is read from the [`Scan`], so the timers behave identically on a real clock and on
//! simulated time.
use crate::Scan;
use core::time::Duration;

/// On-delay timer (IEC TON). `q` turns on once `input` has been on for `pt`, and turns off as soon as `input`
/// does. `et` is the time elapsed so far, capped at `pt`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Ton {
    q: bool,
    et: Duration,
    since: Option<Duration>,
}

impl Ton {
    pub fn scan(&mut self, scan: &Scan, input: bool, pt: Duration) -> bool {
        if input {
            let since = *self.since.get_or_insert(scan.now);
            let elapsed = scan.now.saturating_sub(since);
            self.et = elapsed.min(pt);
            self.q = elapsed >= pt;
        } else {
            self.since = None;
            self.et = Duration::ZERO;
            self.q = false;
        }
        self.q
    }

    pub fn q(&self) -> bool {
        self.q
    }

    pub fn et(&self) -> Duration {
        self.et
    }
}

/// Off-delay timer (IEC TOF). `q` is on while `input` is on, and stays on for `pt` after `input` turns off.
/// After that, `et` holds at `pt` until `input` turns on again.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tof {
    q: bool,
    et: Duration,
    since: Option<Duration>,
}

impl Tof {
    pub fn scan(&mut self, scan: &Scan, input: bool, pt: Duration) -> bool {
        if input {
            self.q = true;
            self.et = Duration::ZERO;
            self.since = None;
        } else if self.q {
            let since = *self.since.get_or_insert(scan.now);
            let elapsed = scan.now.saturating_sub(since);
            self.et = elapsed.min(pt);
            if elapsed >= pt {
                self.q = false;
            }
        }
        self.q
    }

    pub fn q(&self) -> bool {
        self.q
    }

    pub fn et(&self) -> Duration {
        self.et
    }
}

/// Pulse timer (IEC TP). A rising edge of `input` starts a pulse: `q` is on for exactly `pt`, whatever `input`
/// does meanwhile, and re-triggering during the pulse is ignored. A pulse is judged on each scan against that
/// scan's `pt`, as every timer here, and once it has ended it stays off until the next rising edge, whatever `pt`
/// or the time then does (`docs/questions.md` Q229): one pulse per rising edge. After the pulse, `et` holds at
/// that scan's `pt` while `input` stays on, and resets once it is off.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tp {
    q: bool,
    et: Duration,
    since: Option<Duration>,
    last_input: bool,
}

impl Tp {
    pub fn scan(&mut self, scan: &Scan, input: bool, pt: Duration) -> bool {
        let rising = input && !self.last_input;
        self.last_input = input;
        if rising && self.since.is_none() {
            self.since = Some(scan.now);
        }
        if let Some(since) = self.since {
            let elapsed = scan.now.saturating_sub(since);
            if elapsed < pt {
                self.q = true;
                self.et = elapsed;
                return self.q;
            }
            // The pulse is over: only a new rising edge starts another.
            self.since = None;
        }
        self.q = false;
        self.et = if input { pt } else { Duration::ZERO };
        self.q
    }

    pub fn q(&self) -> bool {
        self.q
    }

    pub fn et(&self) -> Duration {
        self.et
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests {
    use super::*;
    use crate::ms;

    fn at(t: u64) -> Scan {
        Scan {
            now: ms(t),
            period: ms(10),
            cycle: t / 10,
        }
    }

    #[test]
    fn ton_turns_on_after_pt_and_off_with_the_input() {
        let mut t = Ton::default();
        assert!(!t.scan(&at(0), true, ms(100)));
        assert!(!t.scan(&at(90), true, ms(100)));
        assert_eq!(t.et(), ms(90));
        assert!(t.scan(&at(100), true, ms(100)), "on exactly at pt");
        assert!(t.scan(&at(500), true, ms(100)));
        assert_eq!(t.et(), ms(100), "et is capped at pt");
        assert!(!t.scan(&at(510), false, ms(100)), "off with the input");
        assert_eq!(t.et(), Duration::ZERO);
    }

    #[test]
    fn ton_restarts_from_zero_after_an_interruption() {
        let mut t = Ton::default();
        t.scan(&at(0), true, ms(100));
        t.scan(&at(80), false, ms(100));
        t.scan(&at(90), true, ms(100));
        assert!(
            !t.scan(&at(170), true, ms(100)),
            "80 ms into the second try"
        );
        assert!(t.scan(&at(190), true, ms(100)));
    }

    #[test]
    fn tof_holds_q_for_pt_after_the_input_drops() {
        let mut t = Tof::default();
        assert!(!t.scan(&at(0), false, ms(100)), "never on: off");
        assert!(t.scan(&at(10), true, ms(100)));
        assert!(t.scan(&at(20), false, ms(100)), "the off-delay starts");
        assert!(t.scan(&at(110), false, ms(100)));
        assert!(!t.scan(&at(120), false, ms(100)), "off once pt has passed");
        assert_eq!(t.et(), ms(100), "et holds at pt");
        assert!(t.scan(&at(130), true, ms(100)));
        assert_eq!(t.et(), Duration::ZERO);
    }

    #[test]
    fn tp_gives_one_fixed_pulse_and_ignores_retriggers() {
        let mut t = Tp::default();
        assert!(
            t.scan(&at(0), true, ms(100)),
            "the rising edge starts the pulse"
        );
        assert!(
            t.scan(&at(50), false, ms(100)),
            "still on: the pulse length is fixed"
        );
        assert!(
            t.scan(&at(60), true, ms(100)),
            "a retrigger during the pulse is ignored"
        );
        assert!(!t.scan(&at(100), true, ms(100)), "the pulse ends at pt");
        assert_eq!(t.et(), ms(100), "et holds at pt while the input is on");
        assert!(!t.scan(&at(150), false, ms(100)));
        assert_eq!(t.et(), Duration::ZERO, "et resets once the input is off");
        assert!(t.scan(&at(160), true, ms(100)), "a new edge, a new pulse");
    }
}
