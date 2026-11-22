//! What C-150 states of `orcl-std` beyond Veredas 1's own unit tests (`src/*.rs`), pinned after its first
//! refute round (`.zen/evidence/C-150.md`): every block's behaviour where a mutant of it passed those tests, bit
//! access on every width, and a program's default safe outputs. O14 (function blocks are not copied) is pinned by
//! the `compile_fail` doctests of `src/o14.rs`.
use orcl_std::{Bits, Ctu, FTrig, Program, RTrig, Scan, Tof, Tp, ms};

fn at(t: u64) -> Scan {
    Scan {
        now: ms(t),
        period: ms(10),
        cycle: t / 10,
    }
}

#[test]
fn an_off_delay_holds_for_pt_after_every_drop_not_only_the_first() {
    let mut t = Tof::default();
    assert!(t.scan(&at(0), true, ms(100)));
    assert!(t.scan(&at(10), false, ms(100)), "the first drop starts the delay");
    assert!(t.scan(&at(50), true, ms(100)), "the input back during the delay");
    assert!(t.scan(&at(1_000), false, ms(100)), "the second drop starts a delay of its own");
    assert_eq!(t.et(), ms(0));
    assert!(t.scan(&at(1_090), false, ms(100)));
    assert_eq!(t.et(), ms(90));
    assert!(!t.scan(&at(1_100), false, ms(100)), "off pt after the second drop");
}

#[test]
fn an_off_delay_holds_its_elapsed_time_at_pt_once_off() {
    let mut t = Tof::default();
    t.scan(&at(0), true, ms(100));
    t.scan(&at(10), false, ms(100));
    assert!(!t.scan(&at(110), false, ms(100)));
    assert_eq!(t.et(), ms(100));
    assert!(!t.scan(&at(200), false, ms(100)));
    assert_eq!(t.et(), ms(100), "held at pt on the scans after, the input still off");
}

#[test]
fn a_pulse_holds_its_elapsed_time_at_pt_while_the_input_stays_on() {
    let mut t = Tp::default();
    assert!(t.scan(&at(0), true, ms(100)));
    assert!(!t.scan(&at(100), true, ms(100)));
    assert!(!t.scan(&at(130), true, ms(100)), "no new pulse without a new edge");
    assert_eq!(t.et(), ms(100), "held at pt while the input stays on");
    assert!(!t.scan(&at(140), false, ms(100)));
    assert_eq!(t.et(), ms(0), "and back to zero once it is off");
}

#[test]
fn a_rising_edge_counts_a_signal_already_on_at_the_first_scan() {
    let mut r = RTrig::default();
    assert!(r.scan(true), "on at the first scan: a rising edge");
    assert!(r.q());
    assert!(!r.scan(true), "for one scan only");
    let mut f = FTrig::default();
    assert!(!f.scan(false), "off at the first scan: no falling edge");
}

#[test]
fn the_counter_s_reset_wins_over_a_rising_edge_in_the_same_scan() {
    let mut c = Ctu::default();
    c.scan(true, false, 3);
    c.scan(false, false, 3);
    assert_eq!(c.cv(), 1);
    assert!(!c.scan(true, true, 3), "a rising edge and the reset together");
    assert_eq!(c.cv(), 0, "the reset wins");
}

#[test]
fn the_counter_s_q_is_off_below_the_preset_and_on_from_it() {
    let mut c = Ctu::default();
    let mut seen = Vec::new();
    for cu in [true, false, true, false, true, false, true] {
        seen.push((c.scan(cu, false, 3), c.cv()));
    }
    assert_eq!(
        seen,
        vec![(false, 1), (false, 1), (false, 2), (false, 2), (true, 3), (true, 3), (true, 4)],
        "q off below the preset, on once the count reaches it and past it"
    );
}

#[test]
fn bits_on_every_width_read_and_write_the_top_bit_and_nothing_past_it() {
    fn check<T: Bits + PartialEq + core::fmt::Debug + Default>(width: u32) {
        assert_eq!(T::WIDTH, width);
        let zero = T::default();
        let top = zero.with_bit(width - 1, true);
        assert!(top.get_bit(width - 1), "bit {} of a {width}-bit value", width - 1);
        assert!(!top.get_bit(width - 2));
        assert!(!top.get_bit(width), "past the width reads false");
        assert_eq!(top.with_bit(width, true), top, "past the width writes nothing");
        let mut v = top;
        v.set_bit(width - 1, false);
        assert_eq!(v, zero);
        v.set_bit(0, true);
        assert!(v.get_bit(0));
    }
    check::<u8>(8);
    check::<u16>(16);
    check::<u32>(32);
    check::<u64>(64);
    assert_eq!(0x8000_0001_u32.with_bit(31, false), 1);
    assert_eq!(u64::MAX.with_bit(63, false), u64::MAX >> 1);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Out {
    run: bool,
    speed: u16,
}

struct Plain;

impl Program for Plain {
    type Inputs = bool;
    type Outputs = Out;

    fn scan(&mut self, _scan: &Scan, inputs: &bool, outputs: &mut Out) {
        outputs.run = *inputs;
        outputs.speed = 900;
    }
}

struct Declares;

impl Program for Declares {
    type Inputs = bool;
    type Outputs = Out;

    fn scan(&mut self, _scan: &Scan, _inputs: &bool, _outputs: &mut Out) {}

    fn safe_outputs(&self) -> Out {
        Out { run: false, speed: 5 }
    }
}

#[test]
fn a_program_s_safe_outputs_are_all_default_unless_it_says_otherwise() {
    let mut o = Out::default();
    Plain.scan(&at(0), &true, &mut o);
    assert_eq!(o, Out { run: true, speed: 900 });
    assert_eq!(Plain.safe_outputs(), Out { run: false, speed: 0 });
    assert_eq!(Declares.safe_outputs(), Out { run: false, speed: 5 });
}
