//! What C-150 states of `orcl-std` beyond Veredas 1's own unit tests (`src/*.rs`), pinned after its first
//! refute round (`.zen/evidence/C-150.md`): every block's behaviour where a mutant of it passed those tests, bit
//! access on every width, and a program's default safe outputs. O14 (function blocks are not copied) is pinned by
//! the `compile_fail` doctests of `src/o14.rs`, each beside a twin that compiles. The second refute round added
//! boundaries off the 10 ms grid, scans whose time disagrees with their cycle, every block's `q()`, the counter at
//! a preset of zero and at its top, and the crate's own attributes. The third round added timers started off
//! the grid, a preset time of zero, a one-scan trigger, a negative preset, `set_bit` past the width, and the crate
//! root compared line by line.
use orcl_std::{Bits, Ctu, Duration, FTrig, Program, RTrig, Scan, Tof, Ton, Tp, ms};

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

/// A scan whose time is `now` ms whatever its cycle number says.
fn scan_at(now: u64, cycle: u64) -> Scan {
    Scan {
        now: ms(now),
        period: ms(10),
        cycle,
    }
}

#[test]
fn every_timer_switches_at_pt_and_not_a_moment_before() {
    let pt = ms(100);
    let mut on = Ton::default();
    on.scan(&at(0), true, pt);
    assert!(!on.scan(&scan_at(99, 9), true, pt), "the on-delay still off 1 ms before pt");
    assert_eq!(on.et(), ms(99));
    assert!(on.scan(&scan_at(100, 10), true, pt));

    let mut off = Tof::default();
    off.scan(&at(0), true, pt);
    off.scan(&at(10), false, pt);
    assert!(off.scan(&scan_at(109, 10), false, pt), "the off-delay still on 1 ms before pt");
    assert!(!off.scan(&scan_at(110, 11), false, pt));

    let mut pulse = Tp::default();
    pulse.scan(&at(0), true, pt);
    assert!(pulse.scan(&scan_at(99, 9), true, pt), "the pulse still on 1 ms before pt");
    assert!(!pulse.scan(&scan_at(100, 10), true, pt));
}

#[test]
fn the_timers_take_their_time_from_the_scan_s_time_not_its_cycle_number() {
    // cycle 1 arrives 150 ms after cycle 0, as after an overrun: the timers read 150 ms, not 10
    let pt = ms(100);
    let mut on = Ton::default();
    on.scan(&scan_at(0, 0), true, pt);
    assert!(on.scan(&scan_at(150, 1), true, pt));
    assert_eq!(on.et(), pt);

    let mut off = Tof::default();
    off.scan(&scan_at(0, 0), true, pt);
    off.scan(&scan_at(10, 1), false, pt);
    assert!(!off.scan(&scan_at(150, 2), false, pt));

    let mut pulse = Tp::default();
    pulse.scan(&scan_at(0, 0), true, pt);
    assert!(!pulse.scan(&scan_at(150, 1), true, pt), "the pulse is over");
    assert_eq!(pulse.et(), pt);
}

#[test]
fn an_off_delay_scanned_late_switches_off_with_its_elapsed_time_capped_at_pt() {
    let pt = ms(100);
    let mut t = Tof::default();
    t.scan(&at(0), true, pt);
    t.scan(&at(20), false, pt);
    assert!(!t.scan(&scan_at(150, 3), false, pt), "130 ms after the drop");
    assert_eq!(t.et(), pt, "capped at pt, not 130 ms");
    assert!(!t.scan(&scan_at(400, 4), false, pt));
    assert_eq!(t.et(), pt);
}

#[test]
fn every_block_s_q_says_what_its_last_scan_returned() {
    let pt = ms(100);
    let mut on = Ton::default();
    for (t, input) in [(0, true), (100, true), (110, false)] {
        let q = on.scan(&at(t), input, pt);
        assert_eq!(on.q(), q);
    }
    let mut off = Tof::default();
    for (t, input) in [(0, true), (10, false), (110, false)] {
        let q = off.scan(&at(t), input, pt);
        assert_eq!(off.q(), q);
    }
    let mut pulse = Tp::default();
    for (t, input) in [(0, true), (100, true)] {
        let q = pulse.scan(&at(t), input, pt);
        assert_eq!(pulse.q(), q);
    }
    assert!(!pulse.q());
    let mut rise = RTrig::default();
    for clk in [true, true] {
        let q = rise.scan(clk);
        assert_eq!(rise.q(), q);
    }
    assert!(!rise.q());
    let mut fall = FTrig::default();
    for clk in [true, false, false] {
        let q = fall.scan(clk);
        assert_eq!(fall.q(), q);
    }
    let mut seen = Vec::new();
    let mut fall = FTrig::default();
    for clk in [true, false, false] {
        fall.scan(clk);
        seen.push(fall.q());
    }
    assert_eq!(seen, vec![false, true, false]);
    let mut on = Ton::default();
    on.scan(&at(0), true, pt);
    assert!(!on.q());
    on.scan(&at(100), true, pt);
    assert!(on.q());
}

#[test]
fn the_counter_at_a_preset_of_zero_at_a_held_reset_and_at_its_top() {
    let mut c = Ctu::default();
    assert!(c.scan(false, false, 0), "a count of zero reaches a preset of zero");
    assert!(c.q());

    let mut c = Ctu::default();
    c.scan(false, false, 3);
    c.scan(true, true, 3);
    assert!(!c.scan(true, false, 3), "released with the input held high: no new edge, nothing counted");
    assert_eq!(c.cv(), 0);

    let mut c = Ctu::default();
    for _ in 0..3 {
        c.scan(true, false, 2);
        c.scan(false, false, 2);
    }
    assert_eq!(c.cv(), 3);
    assert!(c.q(), "past the preset, still on");
}

#[test]
fn the_crate_root_is_exactly_its_attributes_its_modules_and_its_exports() {
    // Every line of src/lib.rs but its `//!` documentation, as it must stand: `no_std`, `unsafe` forbidden, the six
    // denied lints, nothing allowed back, no `std` brought in, nothing commented out. A change to the root is a
    // change to this list.
    let root: Vec<&str> = include_str!("../src/lib.rs")
        .lines()
        .filter(|line| !line.starts_with("//!") && !line.trim().is_empty())
        .collect();
    assert_eq!(
        root,
        vec![
            "#![cfg_attr(not(test), no_std)]",
            "#![forbid(unsafe_code)]",
            "#![deny(",
            "    clippy::arithmetic_side_effects,",
            "    clippy::indexing_slicing,",
            "    clippy::panic,",
            "    clippy::unwrap_used,",
            "    clippy::expect_used,",
            "    clippy::disallowed_types",
            ")]",
            "mod bits;",
            "mod counters;",
            "mod edges;",
            "mod o14;",
            "mod scan;",
            "mod timers;",
            "pub use bits::Bits;",
            "pub use core::time::Duration;",
            "pub use counters::Ctu;",
            "pub use edges::{FTrig, RTrig};",
            "pub use scan::{Program, Scan, ms, s};",
            "pub use timers::{Tof, Ton, Tp};",
        ]
    );
    let _: Duration = ms(1);
}

#[test]
fn a_timer_started_off_the_cycle_grid_times_from_the_scan_s_time() {
    // every timer started by a scan whose time (500 ms) is not its cycle number times the period (30 ms)
    let pt = ms(100);
    let mut on = Ton::default();
    assert!(!on.scan(&scan_at(500, 3), true, pt));
    assert!(!on.scan(&scan_at(510, 4), true, pt));
    assert_eq!(on.et(), ms(10));
    assert!(on.scan(&scan_at(600, 5), true, pt));

    let mut off = Tof::default();
    off.scan(&scan_at(490, 2), true, pt);
    assert!(off.scan(&scan_at(500, 3), false, pt), "the drop starts the delay at 500 ms");
    assert!(off.scan(&scan_at(510, 4), false, pt));
    assert_eq!(off.et(), ms(10));
    assert!(!off.scan(&scan_at(600, 5), false, pt));

    let mut pulse = Tp::default();
    assert!(pulse.scan(&scan_at(500, 3), true, pt), "the edge starts the pulse at 500 ms");
    assert!(pulse.scan(&scan_at(510, 4), true, pt));
    assert_eq!(pulse.et(), ms(10));
    assert!(!pulse.scan(&scan_at(600, 5), true, pt));
}

#[test]
fn a_preset_time_of_zero_switches_on_the_same_scan() {
    let mut on = Ton::default();
    assert!(on.scan(&at(0), true, ms(0)), "on the first scan its input is on");
    assert_eq!((on.q(), on.et()), (true, ms(0)));
    let mut off = Tof::default();
    assert!(off.scan(&at(0), true, ms(0)));
    assert!(!off.scan(&at(10), false, ms(0)), "off on the scan of the drop");
    assert_eq!((off.q(), off.et()), (false, ms(0)));
    assert!(!off.scan(&at(20), false, ms(100)), "and off still when its pt is raised, its input off");
}

#[test]
fn a_pulse_triggered_for_one_scan_lasts_exactly_pt() {
    let pt = ms(100);
    let mut pulse = Tp::default();
    let seen: Vec<bool> = (0..12).map(|k| pulse.scan(&at(k * 10), k == 0, pt)).collect();
    assert_eq!(seen, [vec![true; 10], vec![false; 2]].concat(), "on from 0 to 90 ms with the input off, off at 100");
}

#[test]
fn a_negative_preset_is_reached_at_once() {
    let mut c = Ctu::default();
    assert!(c.scan(false, false, -1));
    assert_eq!(c.cv(), 0);
}

#[test]
fn set_bit_past_the_width_writes_nothing_on_every_width() {
    fn check<T: Bits + PartialEq + core::fmt::Debug + Copy>(value: T) {
        let mut v = value;
        v.set_bit(T::WIDTH, true);
        v.set_bit(T::WIDTH, false);
        v.set_bit(u32::MAX, true);
        assert_eq!(v, value);
    }
    check(0x5A_u8);
    check(0x5A5A_u16);
    check(0x5A5A_5A5A_u32);
    check(0x5A5A_5A5A_5A5A_5A5A_u64);
    check(0_u8);
    check(u64::MAX);
}

// The fourth refute round: a bit written with the value it holds, a reset released low, a pulse at a preset time of
// zero and an edge on the scan its pulse ends, a second falling edge, presets below the millisecond, the counter's
// q() below its preset, a read far past the width, and the version the clause states.

#[test]
fn a_bit_written_with_the_value_it_holds_stays_as_it_was_on_every_width() {
    fn check<T: Bits + PartialEq + core::fmt::Debug + Copy>(ones: T, zero: T) {
        let top = T::WIDTH - 1;
        for n in [0, top] {
            assert_eq!(ones.with_bit(n, true), ones, "setting a set bit {n}");
            assert_eq!(zero.with_bit(n, false), zero, "clearing a clear bit {n}");
            let mut v = ones;
            v.set_bit(n, true);
            assert_eq!(v, ones);
            let mut v = zero;
            v.set_bit(n, false);
            assert_eq!(v, zero);
        }
    }
    check(u8::MAX, 0_u8);
    check(u16::MAX, 0_u16);
    check(u32::MAX, 0_u32);
    check(u64::MAX, 0_u64);
    assert_eq!(0b1_u8.with_bit(0, true), 1);
    assert_eq!(0_u64.with_bit(63, false), 0);
}

#[test]
fn a_reset_released_with_the_input_low_counts_the_next_rising_edge() {
    let mut c = Ctu::default();
    c.scan(false, true, 3);
    assert!(!c.scan(true, false, 3));
    assert_eq!(c.cv(), 1, "the input rose after the reset: one edge");
}

#[test]
fn a_pulse_at_a_preset_time_of_zero_and_an_edge_on_the_scan_its_pulse_ends() {
    let mut zero = Tp::default();
    assert!(!zero.scan(&at(0), true, ms(0)), "a pulse of zero length is off at once");
    assert_eq!((zero.q(), zero.et()), (false, ms(0)));
    let pt = ms(100);
    let mut pulse = Tp::default();
    assert!(pulse.scan(&at(0), true, pt));
    assert!(pulse.scan(&at(50), false, pt));
    assert!(!pulse.scan(&at(100), true, pt), "a new edge on the scan the pulse ends starts nothing");
}

#[test]
fn a_falling_edge_is_seen_every_time() {
    let mut fall = FTrig::default();
    let clk = [true, false, true, false, true, false, true, false, true, false, true, false];
    let seen: Vec<bool> = clk.map(|c| fall.scan(c)).to_vec();
    let falls: Vec<bool> = clk.map(|c| !c).to_vec();
    assert_eq!(seen, falls, "on for each of the six falling edges, off between");
}

#[test]
fn every_timer_holds_its_state_until_a_preset_time_below_the_millisecond() {
    let pt = Duration::from_micros(100_500);
    let before = Scan { now: Duration::from_micros(100_200), period: ms(10), cycle: 10 };
    let at_pt = Scan { now: pt, period: ms(10), cycle: 11 };
    let mut on = Ton::default();
    on.scan(&at(0), true, pt);
    assert!(!on.scan(&before, true, pt));
    assert!(on.scan(&at_pt, true, pt));
    let mut off = Tof::default();
    off.scan(&at(0), true, pt);
    off.scan(&at(0), false, pt);
    assert!(off.scan(&before, false, pt));
    assert!(!off.scan(&at_pt, false, pt));
    let mut pulse = Tp::default();
    pulse.scan(&at(0), true, pt);
    assert!(pulse.scan(&before, true, pt));
    assert!(!pulse.scan(&at_pt, true, pt));
}

#[test]
fn the_counter_s_q_says_what_its_last_scan_returned_below_the_preset_too() {
    let mut c = Ctu::default();
    for (cu, reset) in [(true, false), (false, false), (true, false), (true, true), (false, false)] {
        let q = c.scan(cu, reset, 3);
        assert_eq!(c.q(), q);
    }
    assert!(!c.q());
}

#[test]
fn a_read_far_past_the_width_is_false() {
    assert!(!0xFF_u8.get_bit(9));
    assert!(!u16::MAX.get_bit(100));
    assert!(!u16::MAX.get_bit(2_000));
    assert!(!u32::MAX.get_bit(33));
    assert!(!u64::MAX.get_bit(u32::MAX));
}

#[test]
fn the_crate_is_the_version_its_clause_states() {
    assert_eq!(env!("CARGO_PKG_VERSION"), "2.0.0-alpha.1");
}

// The fifth refute round: a pulse ending with its input off, a preset raised above the count, many rising edges,
// the manifest naming no other root, and no file of the crate bringing in another crate.

#[test]
fn a_pulse_ending_with_its_input_off_resets_at_once_and_the_next_edge_pulses_again() {
    let pt = ms(100);
    let mut p = Tp::default();
    assert!(p.scan(&at(0), true, pt));
    assert!(p.scan(&at(50), false, pt));
    assert!(!p.scan(&at(100), false, pt), "the pulse ends, the input off");
    assert!(!p.q(), "and q() says so");
    assert_eq!(p.et(), ms(0), "and its elapsed time is back to zero on that scan");
    assert!(p.scan(&at(110), true, pt), "the next rising edge, a new pulse");
    assert!(p.scan(&at(200), true, pt));
    assert!(!p.scan(&at(210), true, pt), "exactly pt");
}

#[test]
fn the_counter_s_q_follows_its_preset_on_every_scan() {
    let mut c = Ctu::default();
    assert!(c.scan(true, false, 1), "a count of 1 reaches a preset of 1");
    assert!(!c.scan(false, false, 5), "the preset raised to 5: off");
    assert!(!c.q());
    assert!(c.scan(false, false, 1), "back to 1: on");
}

#[test]
fn a_rising_edge_is_seen_every_time() {
    let mut rise = RTrig::default();
    let clk = [true, false, true, false, true, false, true, false, true, false, true, false];
    let seen: Vec<bool> = clk.map(|c| rise.scan(c)).to_vec();
    assert_eq!(seen, clk.to_vec(), "on for each of the six rising edges, off between");
}

#[test]
fn every_file_of_src_is_read() {
    // the library's root is checked by cargo itself (below, and in the recorded command), and what a source brings
    // in by the single reading of the sources; this pins that the reading reads every file there is
    let listed = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).unwrap().count();
    assert_eq!(listed, SOURCES.len(), "every source file is read");
}

// The sixth refute round: what cargo itself says of the crate, a reset at a preset of zero or below, and no file
// pulled in from elsewhere.

#[test]
fn cargo_says_the_crate_depends_on_nothing_and_its_library_s_root_is_src_lib_rs() {
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let out = std::process::Command::new(env!("CARGO"))
        .args(["metadata", "--no-deps", "--format-version", "1", "--offline", "--manifest-path", manifest])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let package = meta["packages"].as_array().unwrap().iter().find(|p| p["name"] == "orcl-std").unwrap();
    let deps: Vec<(String, String)> = package["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["name"].as_str().unwrap().to_string(), d["kind"].as_str().unwrap_or("normal").to_string()))
        .collect();
    assert_eq!(deps, vec![("serde_json".to_string(), "dev".to_string())], "no normal, build or target dependency");
    let lib: Vec<String> = package["targets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["kind"].as_array().unwrap().iter().any(|k| k == "lib"))
        .map(|t| t["src_path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(lib, vec![concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs").to_string()]);
}

/// Every source file of the crate, by name, as the tests that read the sources need them.
const SOURCES: [(&str, &str); 7] = [
    ("lib.rs", include_str!("../src/lib.rs")),
    ("bits.rs", include_str!("../src/bits.rs")),
    ("counters.rs", include_str!("../src/counters.rs")),
    ("edges.rs", include_str!("../src/edges.rs")),
    ("o14.rs", include_str!("../src/o14.rs")),
    ("scan.rs", include_str!("../src/scan.rs")),
    ("timers.rs", include_str!("../src/timers.rs")),
];

#[test]
fn a_reset_at_a_preset_of_zero_or_below_leaves_q_on() {
    let mut c = Ctu::default();
    assert!(c.scan(false, true, 0), "a count of 0 reaches a preset of 0, reset or not");
    assert!(c.scan(true, true, -1));
    assert_eq!(c.cv(), 0);
}

// The seventh refute round: a set bit cleared below the top, the extreme presets, a preset time at the nanosecond,
// q() mid-delay, restarts after a timer has switched, and the unit tests' cases on the build programs link.

#[test]
fn clearing_a_set_bit_below_the_top_leaves_every_other_bit() {
    fn check<T: Bits + PartialEq + core::fmt::Debug + Copy>(all: T, all_but_bit_3: T) {
        assert_eq!(all.with_bit(3, false), all_but_bit_3);
        let mut v = all;
        v.set_bit(3, false);
        assert_eq!(v, all_but_bit_3);
    }
    check(u8::MAX, u8::MAX ^ 8);
    check(u16::MAX, u16::MAX ^ 8);
    check(u32::MAX, u32::MAX ^ 8);
    check(u64::MAX, u64::MAX ^ 8);
}

#[test]
fn the_counter_at_the_extreme_presets() {
    let mut c = Ctu::default();
    assert!(c.scan(false, false, i32::MIN), "a count of 0 reaches the lowest preset");
    assert!(c.scan(true, false, i32::MIN));
    assert_eq!(c.cv(), 1);
}

#[test]
fn every_timer_switches_at_a_preset_time_of_nanoseconds_and_not_a_nanosecond_before() {
    let pt = Duration::from_nanos(100_000_500);
    let before = Scan { now: Duration::from_nanos(100_000_499), period: ms(10), cycle: 10 };
    let at_pt = Scan { now: pt, period: ms(10), cycle: 11 };
    let mut on = Ton::default();
    on.scan(&at(0), true, pt);
    assert!(!on.scan(&before, true, pt));
    assert!(on.scan(&at_pt, true, pt));
    let mut off = Tof::default();
    off.scan(&at(0), true, pt);
    off.scan(&at(0), false, pt);
    assert!(off.scan(&before, false, pt));
    assert!(!off.scan(&at_pt, false, pt));
    let mut pulse = Tp::default();
    pulse.scan(&at(0), true, pt);
    assert!(pulse.scan(&before, true, pt));
    assert!(!pulse.scan(&at_pt, true, pt));
}

#[test]
fn every_timer_s_q_mid_delay_and_a_fresh_off_delay_s() {
    let pt = ms(100);
    assert!(!Tof::default().q(), "a fresh off-delay is off");
    let mut on = Ton::default();
    on.scan(&at(0), true, pt);
    assert_eq!((on.scan(&at(50), true, pt), on.q()), (false, false), "mid-delay: off");
    let mut off = Tof::default();
    off.scan(&at(0), true, pt);
    off.scan(&at(10), false, pt);
    assert_eq!((off.scan(&at(50), false, pt), off.q()), (true, true), "mid-delay: on");
    let mut pulse = Tp::default();
    pulse.scan(&at(0), true, pt);
    assert_eq!((pulse.scan(&at(50), true, pt), pulse.q()), (true, true), "mid-pulse: on");
}

#[test]
fn a_timer_restarts_from_its_new_start_after_it_has_switched() {
    let pt = ms(100);
    // an on-delay that switched on, dropped and came back times pt from the new start
    let mut on = Ton::default();
    on.scan(&at(0), true, pt);
    assert!(on.scan(&at(100), true, pt));
    on.scan(&at(110), false, pt);
    assert!(!on.scan(&at(120), true, pt));
    assert!(!on.scan(&at(210), true, pt), "90 ms into the new start");
    assert!(on.scan(&at(220), true, pt));
    // an off-delay that ran out, came back and dropped again holds pt from the new drop
    let mut off = Tof::default();
    off.scan(&at(0), true, pt);
    off.scan(&at(10), false, pt);
    assert!(!off.scan(&at(110), false, pt));
    assert!(off.scan(&at(200), true, pt));
    assert!(off.scan(&at(210), false, pt));
    assert!(off.scan(&at(300), false, pt), "90 ms after the new drop");
    assert!(!off.scan(&at(310), false, pt));
}

#[test]
fn the_unit_tests_cases_on_the_build_programs_link() {
    // the interrupted on-delay restarts from zero
    let pt = ms(100);
    let mut on = Ton::default();
    on.scan(&at(0), true, pt);
    on.scan(&at(80), false, pt);
    assert_eq!(on.et(), ms(0));
    on.scan(&at(90), true, pt);
    assert!(!on.scan(&at(170), true, pt));
    assert!(on.scan(&at(190), true, pt));
    // a retrigger during a pulse is ignored: the pulse ends at pt from the first edge
    let mut pulse = Tp::default();
    pulse.scan(&at(0), true, pt);
    pulse.scan(&at(50), false, pt);
    assert!(pulse.scan(&at(60), true, pt));
    assert!(!pulse.scan(&at(100), true, pt));
    // an off-delay's elapsed time is zero again when its input returns
    let mut off = Tof::default();
    off.scan(&at(0), true, pt);
    off.scan(&at(10), false, pt);
    off.scan(&at(50), false, pt);
    assert_eq!(off.et(), ms(40));
    off.scan(&at(60), true, pt);
    assert_eq!(off.et(), ms(0));
    // a held input counts once
    let mut c = Ctu::default();
    for _ in 0..5 {
        c.scan(true, false, 3);
    }
    assert_eq!(c.cv(), 1);
}

// The eighth refute round: an edge's q() after a scan with no edge. (Its reading of the sources, and the sixth,
// seventh and ninth rounds', is the twelfth round's single reading, below.)

#[test]
fn an_edge_s_q_is_off_again_on_the_scan_after_its_edge() {
    let mut rise = RTrig::default();
    rise.scan(true);
    rise.scan(false);
    assert!(!rise.q(), "a rising edge's q() is off on the next low scan");
    let mut fall = FTrig::default();
    fall.scan(true);
    fall.scan(false);
    assert!(fall.q());
    fall.scan(true);
    assert!(!fall.q(), "a falling edge's q() is off on the next high scan");
}

// The sources read as the compiler reads them (refute rounds 6 to 12): one token after another, comments taken out
// and every literal emptied, so a check finds code where rustc does and words where they are words ("an open valve"
// holds no `env`). One reading for every textual promise of the clause.

/// A source as the compiler reads it, its tokens joined by spaces: comments taken out (line and block comments,
/// nested); every string and character literal emptied to `""` or `''`, with its escapes, its `b`, `c`, `r`, `br`
/// or `cr` prefix and a raw string's hashes; a lifetime or a label kept whole; every other character a token of its
/// own. Doc examples are comments, so no code of the library.
fn code_of(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut tokens: Vec<String> = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        let next = chars.get(i + 1).copied();
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && next == Some('/') {
            while chars.get(i).is_some_and(|&c| c != '\n') {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0usize;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if is_word(c) {
            let start = i;
            while chars.get(i).is_some_and(|&c| is_word(c)) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let hashes = chars[i..].iter().take_while(|h| **h == '#').count();
            if matches!(word.as_str(), "r" | "br" | "cr") && chars.get(i + hashes) == Some(&'"') {
                i += hashes + 1;
                while i < chars.len() && !(chars[i] == '"' && chars[i + 1..].iter().take_while(|h| **h == '#').count() >= hashes) {
                    i += 1;
                }
                i += 1 + hashes;
                tokens.push("\"\"".into());
            } else if matches!(word.as_str(), "b" | "c") && next_is(&chars, i, '"') {
                i = string_end(&chars, i);
                tokens.push("\"\"".into());
            } else if word == "b" && next_is(&chars, i, '\'') {
                i = char_end(&chars, i);
                tokens.push("''".into());
            } else {
                tokens.push(word);
            }
        } else if c == '"' {
            i = string_end(&chars, i);
            tokens.push("\"\"".into());
        } else if c == '\'' && (next == Some('\\') || chars.get(i + 2) == Some(&'\'')) {
            i = char_end(&chars, i);
            tokens.push("''".into());
        } else if c == '\'' {
            let start = i;
            i += 1;
            while chars.get(i).is_some_and(|&c| is_word(c)) {
                i += 1;
            }
            tokens.push(chars[start..i].iter().collect());
        } else {
            tokens.push(c.to_string());
            i += 1;
        }
    }
    tokens.join(" ")
}

fn next_is(chars: &[char], at: usize, c: char) -> bool {
    chars.get(at) == Some(&c)
}

/// Past a string literal whose opening quote is at `open`, its escapes skipped.
fn string_end(chars: &[char], open: usize) -> usize {
    let mut i = open + 1;
    while let Some(&c) = chars.get(i) {
        if c == '"' {
            return i + 1;
        }
        i += if c == '\\' { 2 } else { 1 };
    }
    i
}

/// Past a character literal whose opening quote is at `open`, an escape included.
fn char_end(chars: &[char], open: usize) -> usize {
    let mut i = open + if chars.get(open + 1) == Some(&'\\') { 3 } else { 2 };
    while chars.get(i).is_some_and(|&c| c != '\'') {
        i += 1;
    }
    i + 1
}

/// Where a source's unit tests' module starts: its `cfg(test)` attribute, however spaced.
const TESTS_SITE: &str = "# [ cfg ( test ) ]";
/// The crate root's `no_std` switch, however spaced.
const NO_STD_SWITCH: &str = "# ! [ cfg_attr ( not ( test ) , no_std ) ]";

/// A source's code split at its unit tests' module, bounded by its braces: the library's code (before the module
/// and after it), then the module (empty when it has none).
fn library_and_tests(text: &str) -> (String, String) {
    let code = code_of(text);
    let Some(start) = code.find(TESTS_SITE) else { return (code, String::new()) };
    let mut end = code.len();
    if let Some(open) = code[start..].find('{').map(|at| start + at) {
        let mut depth = 0;
        for (at, c) in code[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + at + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    (format!("{} {}", &code[..start], &code[end..]), code[start..end].to_string())
}

/// Whether a `cfg(test)` stands on the unit tests' module itself, after any other attribute.
fn is_tests_module(module: &str) -> bool {
    let mut rest = module.strip_prefix(TESTS_SITE).unwrap_or_default().trim_start();
    while let Some(attribute) = rest.strip_prefix("# [") {
        let mut depth = 1;
        let close = attribute.char_indices().find_map(|(at, c)| {
            match c {
                '[' => depth += 1,
                ']' => depth -= 1,
                _ => {}
            }
            (depth == 0).then_some(at)
        });
        let Some(close) = close else { return false };
        rest = attribute[close + 1..].trim_start();
    }
    rest.starts_with("mod tests {")
}

/// The words of a piece of code: what an identifier or a keyword is made of.
fn words(code: &str) -> Vec<&str> {
    code.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect()
}

#[test]
fn the_sources_read_as_compiled_hold_only_the_words_the_clause_allows() {
    for (file, text) in SOURCES {
        let code = code_of(text);
        let (library, tests_module) = library_and_tests(text);
        let (all, library_words, module_words) = (words(&code), words(&library), words(&tests_module));
        let count = |words: &[&str], word: &str| words.iter().filter(|w| **w == word).count();
        // nothing brought in from another crate or file or read from the build's environment, and no panic source
        // or debug-only code, anywhere
        for refused in [
            "extern",
            "path",
            "include",
            "include_str",
            "include_bytes",
            "env",
            "option_env",
            "unwrap",
            "unwrap_err",
            "unwrap_unchecked",
            "expect_err",
            "debug_assertions",
            "todo",
            "unimplemented",
            "unreachable",
            "debug_assert",
            "debug_assert_eq",
            "debug_assert_ne",
            "debug_assert_matches",
        ] {
            assert_eq!(count(&all, refused), 0, "{file} holds {refused}");
        }
        // `expect` only as a lint attribute, never as a call
        let expect_attributes = code.matches("# [ expect (").count() + code.matches("# ! [ expect (").count();
        assert_eq!(count(&all, "expect"), expect_attributes, "{file} holds `expect` as no lint attribute");
        // `panic` once, among the root's denied lints, which are compared line by line
        assert_eq!(count(&all, "panic"), usize::from(file == "lib.rs"), "{file} names `panic`");
        for refused in ["assert", "assert_eq", "assert_ne", "assert_matches"] {
            assert_eq!(count(&library_words, refused), 0, "{file}: the library's code holds {refused}");
        }
        // built apart for the tests only at the root's no_std switch and on each unit tests' module: `cfg` and
        // `test` nowhere else, so no test attribute by any path, name or macro
        let unit_tested = matches!(file, "bits.rs" | "counters.rs" | "edges.rs" | "timers.rs");
        assert_eq!(count(&all, "cfg_attr"), usize::from(file == "lib.rs"), "{file}: cfg_attr");
        let cfg_words = all.iter().filter(|w| w.starts_with("cfg")).count();
        assert_eq!(cfg_words, usize::from(file == "lib.rs") + usize::from(unit_tested), "{file}: a `cfg` word elsewhere");
        assert_eq!(code.matches(NO_STD_SWITCH).count(), usize::from(file == "lib.rs"), "{file}: the no_std switch");
        assert_eq!(count(&all, "cfg"), usize::from(unit_tested), "{file}: cfg");
        assert_eq!(code.matches(TESTS_SITE).count(), usize::from(unit_tested), "{file}: cfg(test)");
        assert_eq!(count(&library_words, "test"), usize::from(file == "lib.rs"), "{file}: `test` in the library's code");
        assert_eq!(is_tests_module(&tests_module), unit_tested, "{file}: cfg(test) on the unit tests' module");
        // the library's one macro is `impl_bits!`; the unit tests' modules hold no item that could make the test
        // build differ from the one programs link
        assert_eq!(count(&all, "macro_rules"), usize::from(file == "bits.rs"), "{file}: macro_rules");
        for refused in ["impl", "trait", "macro_rules", "macro_use"] {
            assert_eq!(count(&module_words, refused), 0, "{file}: the unit tests' module holds {refused}");
        }
    }
}

#[test]
fn the_reading_finds_code_where_the_compiler_does_and_words_alone() {
    let source = r####"//! an example: assert!(x), and an impl; an open valve stops at half and includes nothing
pub fn f(asserted: bool) -> bool { asserted } // de-asserted at pt; the trait's default
const S: &str = "
// */ assert!(false) is in a string, \" and an escaped quote
";
const R: &str = r#"a raw "string" with a # and impl"#;
const CR: &core::ffi::CStr = cr"\"; const BR: &[u8] = br##"x"# raw"##; const BC: u8 = b'"';
const C: [char; 3] = ['}', '\'', '"'];
const L: &str = core::stringify!('r"\" "); fn shown_after_a_lifetime() {}
fn life<'a>(x: &'a u8) -> &'a u8 { 'outer: loop { break 'outer x; } }
/* a block comment /* nested */
// */ fn hidden() {}
#[cfg( test )]
#[allow(dead_code)]
mod tests {
    // a simple test, the trait's default
    #[test]
    fn t() { let _ = "impl {"; let _ = '}'; }
}
pub fn after() {}
#[expect(dead_code)]
fn called() { let _ = Some(1).expect("x"); let _ = Err::<(), u8>(1).unwrap_err(); }
core::cfg_select! { debug_assertions => { fn d() {} } _ => { fn d() {} } }
"####;
    let code = code_of(source);
    let (library, tests_module) = library_and_tests(source);
    let library_words = words(&library);
    for kept in ["asserted", "CR", "BR", "BC", "life", "a", "outer", "shown_after_a_lifetime", "hidden", "after", "unwrap_err", "cfg_select", "debug_assertions"] {
        assert!(library_words.contains(&kept), "{kept} is the library's code");
    }
    for gone in ["assert", "example", "impl", "trait", "string", "raw", "nested", "open", "valve", "half", "includes"] {
        assert!(!library_words.contains(&gone), "{gone} is a comment's or a literal's");
    }
    assert_eq!(code.matches(TESTS_SITE).count(), 1, "cfg(test) found however spaced");
    assert_eq!((code.matches("# [ expect (").count(), code.matches(". expect (").count()), (1, 1), "an attribute, a call");
    assert!(is_tests_module(&tests_module), "on the module, after another attribute");
    let module_words = words(&tests_module);
    assert!(module_words.contains(&"t") && !module_words.contains(&"after"), "the module ends at its brace");
    for gone in ["simple", "trait", "impl"] {
        assert!(!module_words.contains(&gone), "{gone} is a comment's or a literal's");
    }
}

// The tenth and eleventh refute rounds: timers on scans whose time goes back, whose period changes or is zero, and
// at the longest preset time; the twelfth: a preset time that changes, and the longest one from a start past zero.

#[test]
fn a_timer_s_elapsed_time_is_measured_from_its_start_on_a_scan_whose_time_goes_back_and_nothing_panics() {
    let pt = ms(100);
    // an on-delay started at 100 ms
    let mut on = Ton::default();
    let mut ton = |t: u64, input: bool| (on.scan(&at(t), input, pt), on.et());
    assert_eq!(ton(100, true), (false, ms(0)));
    assert_eq!(ton(150, true), (false, ms(50)));
    assert_eq!(ton(199, true), (false, ms(99)));
    assert_eq!(ton(200, true), (true, ms(100)));
    assert_eq!(ton(150, true), (false, ms(50)), "back after switching on: 50 ms from its start");
    assert_eq!(ton(50, true), (false, ms(0)), "back before its start: nothing elapsed");
    assert_eq!(ton(120, true), (false, ms(20)), "and its start where it was");
    assert_eq!(ton(200, true), (true, ms(100)));
    assert_eq!(ton(150, false), (false, ms(0)), "its input off on a scan back: off and restarted");
    assert_eq!(ton(120, true), (false, ms(0)));
    assert_eq!(ton(219, true), (false, ms(99)));
    assert_eq!(ton(220, true), (true, ms(100)), "pt from the new start");
    // an off-delay dropped at 100 ms
    let mut off = Tof::default();
    let mut tof = |t: u64, input: bool| (off.scan(&at(t), input, pt), off.et());
    assert_eq!(tof(0, true), (true, ms(0)));
    assert_eq!(tof(100, false), (true, ms(0)));
    assert_eq!(tof(180, false), (true, ms(80)));
    assert_eq!(tof(150, false), (true, ms(50)), "back mid-delay: 50 ms from the drop");
    assert_eq!(tof(50, false), (true, ms(0)), "back before the drop: nothing elapsed");
    assert_eq!(tof(120, false), (true, ms(20)), "and the drop where it was");
    assert_eq!(tof(199, false), (true, ms(99)));
    assert_eq!(tof(200, false), (false, ms(100)));
    assert_eq!(tof(150, false), (false, ms(100)), "back after switching off: off, its elapsed time held");
    assert_eq!(tof(120, true), (true, ms(0)), "its input on on a scan back: on");
    assert_eq!(tof(130, false), (true, ms(0)));
    assert_eq!(tof(229, false), (true, ms(99)));
    assert_eq!(tof(230, false), (false, ms(100)), "pt from the new drop");
    // a pulse from an edge at 100 ms, its input held on
    let mut pulse = Tp::default();
    let mut tp = |t: u64, input: bool| (pulse.scan(&at(t), input, pt), pulse.et());
    assert_eq!(tp(100, true), (true, ms(0)));
    assert_eq!(tp(150, true), (true, ms(50)));
    assert_eq!(tp(199, true), (true, ms(99)));
    assert_eq!(tp(200, true), (false, ms(100)));
    assert_eq!(tp(210, true), (false, ms(100)));
    assert_eq!(tp(150, true), (true, ms(50)), "back after the pulse, its input on: 50 ms from its edge");
    assert_eq!(tp(50, true), (true, ms(0)), "back before its edge: nothing elapsed");
    assert_eq!(tp(120, true), (true, ms(20)), "and its edge where it was");
    assert_eq!(tp(210, true), (false, ms(100)));
    assert_eq!(tp(150, false), (true, ms(50)), "its input off on a scan back: 50 ms from its edge");
    assert_eq!(tp(220, false), (false, ms(0)), "over, its input off: reset");
    assert_eq!(tp(230, true), (true, ms(0)), "the next edge pulses again");
}

#[test]
fn a_period_that_changes_or_is_zero_moves_no_timer_s_switching_time() {
    let pt = ms(100);
    let scan = |t: u64, period: u64| Scan { now: ms(t), period: ms(period), cycle: t / 10 };
    let mut on = Ton::default();
    on.scan(&scan(0, 10), true, pt);
    assert!(!on.scan(&scan(50, 0), true, pt), "a period of zero: still off");
    assert!(!on.scan(&scan(95, 20), true, pt), "95 ms on, the period now 20 ms: still off");
    assert!(on.scan(&scan(100, 0), true, pt), "on at pt, the period zero");
    assert!(on.scan(&scan(110, 20), true, pt));
    let mut off = Tof::default();
    off.scan(&scan(0, 10), true, pt);
    off.scan(&scan(10, 0), false, pt);
    assert!(off.scan(&scan(105, 20), false, pt), "95 ms after a drop on a period of zero: still on");
    assert!(!off.scan(&scan(110, 0), false, pt), "off at pt, the period zero");
    let mut pulse = Tp::default();
    pulse.scan(&scan(0, 0), true, pt);
    assert!(pulse.scan(&scan(95, 20), true, pt), "95 ms into a pulse begun on a period of zero: still on");
    assert!(!pulse.scan(&scan(100, 0), true, pt), "over at pt, the period zero");
}

#[test]
fn every_timer_at_the_longest_preset_time_waits_to_the_end_of_time_and_panics_none() {
    let longest = Duration::MAX;
    let late = Scan { now: ms(u64::MAX), period: ms(10), cycle: u64::MAX };
    let end = Scan { now: Duration::MAX, period: ms(10), cycle: u64::MAX };
    let mut on = Ton::default();
    assert!(!on.scan(&at(0), true, longest));
    assert_eq!((on.scan(&late, true, longest), on.et()), (false, ms(u64::MAX)));
    assert_eq!((on.scan(&end, true, longest), on.et()), (true, Duration::MAX), "on at the very end");
    let mut off = Tof::default();
    off.scan(&at(0), true, longest);
    off.scan(&at(0), false, longest);
    assert_eq!((off.scan(&late, false, longest), off.et()), (true, ms(u64::MAX)));
    assert_eq!((off.scan(&end, false, longest), off.et()), (false, Duration::MAX), "off at the very end");
    let mut pulse = Tp::default();
    pulse.scan(&at(0), true, longest);
    assert_eq!((pulse.scan(&late, true, longest), pulse.et()), (true, ms(u64::MAX)));
    assert_eq!((pulse.scan(&end, true, longest), pulse.et()), (false, Duration::MAX), "over at the very end");
}

#[test]
fn every_timer_is_judged_on_each_scan_against_that_scan_s_preset_time() {
    // an on-delay started at 0
    let mut on = Ton::default();
    let mut ton = |t: u64, pt: u64| {
        let q = on.scan(&at(t), true, ms(pt));
        assert_eq!(on.q(), q, "q() at {t} ms");
        (q, on.et())
    };
    assert_eq!(ton(0, 100), (false, ms(0)));
    assert_eq!(ton(50, 40), (true, ms(40)), "pt lowered below the elapsed time: on at once, et at the new pt");
    assert_eq!(ton(60, 100), (false, ms(60)), "pt raised past it: off again");
    assert_eq!(ton(100, 100), (true, ms(100)));
    assert_eq!(ton(120, 50), (true, ms(50)), "lowered after switching on: on, et at the new pt");
    assert_eq!(ton(130, 110), (true, ms(110)), "raised after switching on, below the elapsed time: on, et at it");
    assert_eq!(ton(150, 200), (false, ms(150)), "raised after switching on past the elapsed time: off");
    assert_eq!(ton(200, 200), (true, ms(200)));
    // an off-delay dropped at 10 ms
    let mut off = Tof::default();
    let mut tof = |t: u64, input: bool, pt: u64| {
        let q = off.scan(&at(t), input, ms(pt));
        assert_eq!(off.q(), q, "q() at {t} ms");
        (q, off.et())
    };
    assert_eq!(tof(0, true, 100), (true, ms(0)));
    assert_eq!(tof(10, false, 100), (true, ms(0)));
    assert_eq!(tof(60, false, 200), (true, ms(50)), "pt raised: still on");
    assert_eq!(tof(120, false, 200), (true, ms(110)), "past the old pt: still on");
    assert_eq!(tof(130, false, 100), (false, ms(100)), "pt lowered below the elapsed time: off at once, et at the new pt");
    assert_eq!(tof(135, false, 50), (false, ms(100)), "once off, lowered: its elapsed time held at the pt it switched off at");
    assert_eq!(tof(137, false, 110), (false, ms(100)), "once off, raised below the time since its drop: held");
    assert_eq!(tof(140, false, 300), (false, ms(100)), "once off, raised past it: held at it all the same");
    // a pulse from an edge at 0, its input held on
    let mut pulse = Tp::default();
    let mut tp = |t: u64, input: bool, pt: u64| {
        let q = pulse.scan(&at(t), input, ms(pt));
        assert_eq!(pulse.q(), q, "q() at {t} ms");
        (q, pulse.et())
    };
    assert_eq!(tp(0, true, 100), (true, ms(0)));
    assert_eq!(tp(50, true, 40), (false, ms(40)), "pt lowered below the elapsed time: over at once");
    assert_eq!(tp(60, true, 100), (true, ms(60)), "raised past it, the input still on: on again (Q229)");
    assert_eq!(tp(100, true, 100), (false, ms(100)));
    assert_eq!(tp(120, true, 50), (false, ms(50)), "held after the pulse, lowered: over, et at the new pt");
    assert_eq!(tp(130, true, 110), (false, ms(110)), "raised below the elapsed time: over, et at it");
    assert_eq!(tp(150, true, 200), (true, ms(150)), "raised after the pulse, the input on: on again, no new edge (Q229)");
    assert_eq!(tp(160, false, 200), (true, ms(160)), "its input off: on to the raised pt");
    assert_eq!(tp(200, false, 200), (false, ms(0)), "over, its input off: reset");
    // a pt lowered to zero: an on-delay mid-delay on at once, a pulse mid-pulse over, each elapsed time at zero; an
    // off-delay dropped at a zero pt off at once, and off still when its pt is raised again, its input off
    let mut on = Ton::default();
    on.scan(&at(0), true, ms(100));
    assert_eq!((on.scan(&at(50), true, ms(0)), on.q(), on.et()), (true, true, ms(0)));
    let mut pulse = Tp::default();
    pulse.scan(&at(0), true, ms(100));
    assert_eq!((pulse.scan(&at(50), true, ms(0)), pulse.q(), pulse.et()), (false, false, ms(0)));
    let mut off = Tof::default();
    off.scan(&at(0), true, ms(100));
    assert_eq!((off.scan(&at(10), false, ms(0)), off.q(), off.et()), (false, false, ms(0)));
    assert_eq!((off.scan(&at(20), false, ms(100)), off.q(), off.et()), (false, false, ms(0)), "off it stays");
    // an off-delay lowered to zero mid-delay: off at once, and off still when its pt is raised again
    let mut off = Tof::default();
    off.scan(&at(0), true, ms(100));
    off.scan(&at(10), false, ms(100));
    assert_eq!((off.scan(&at(50), false, ms(0)), off.q(), off.et()), (false, false, ms(0)));
    assert_eq!((off.scan(&at(60), false, ms(100)), off.q(), off.et()), (false, false, ms(0)), "off it stays");
    // a pt raised while the timer still measures: an on-delay waits for the raised pt, a pulse lasts to it
    let mut on = Ton::default();
    on.scan(&at(0), true, ms(100));
    assert_eq!((on.scan(&at(50), true, ms(200)), on.q(), on.et()), (false, false, ms(50)));
    assert_eq!((on.scan(&at(100), true, ms(200)), on.q(), on.et()), (false, false, ms(100)), "not at the old pt");
    assert_eq!((on.scan(&at(150), true, ms(200)), on.q(), on.et()), (false, false, ms(150)));
    assert_eq!((on.scan(&at(200), true, ms(200)), on.q(), on.et()), (true, true, ms(200)), "on at the raised pt");
    let mut pulse = Tp::default();
    pulse.scan(&at(0), true, ms(100));
    assert_eq!((pulse.scan(&at(50), true, ms(200)), pulse.q(), pulse.et()), (true, true, ms(50)));
    assert_eq!((pulse.scan(&at(150), true, ms(200)), pulse.q(), pulse.et()), (true, true, ms(150)), "past the old pt");
    assert_eq!((pulse.scan(&at(160), false, ms(200)), pulse.q(), pulse.et()), (true, true, ms(160)), "its input off");
    assert_eq!((pulse.scan(&at(200), false, ms(200)), pulse.q(), pulse.et()), (false, false, ms(0)), "over at it");
}

#[test]
fn every_timer_started_after_zero_at_the_longest_preset_time_panics_none_at_the_end_of_time() {
    let longest = Duration::MAX;
    let end = Scan { now: Duration::MAX, period: ms(10), cycle: u64::MAX };
    let almost = Duration::MAX.saturating_sub(ms(1));
    let mut on = Ton::default();
    on.scan(&at(1), true, longest);
    assert_eq!((on.scan(&end, true, longest), on.et()), (false, almost));
    let mut off = Tof::default();
    off.scan(&at(0), true, longest);
    off.scan(&at(1), false, longest);
    assert_eq!((off.scan(&end, false, longest), off.et()), (true, almost));
    let mut pulse = Tp::default();
    pulse.scan(&at(1), true, longest);
    assert_eq!((pulse.scan(&end, true, longest), pulse.et()), (true, almost));
}

// The fourteenth refute round: the counter at its top on the build programs link, where `no_std` leaves out the
// standard prelude the unit tests' build has, so a name could resolve differently there. Counting to the top takes
// 2^31 rising edges: 23 s in a debug build, under a second in release, which C-150's command runs.

#[test]
#[ignore = "2^31 rising edges: run in release by C-150's command"]
fn the_counter_counts_to_its_top_and_saturates_there_on_the_build_programs_link() {
    let mut c = Ctu::default();
    for edge in 1..=3 {
        c.scan(false, false, i32::MAX);
        assert_eq!((c.scan(true, false, i32::MAX), c.cv()), (false, edge), "one edge, one count, at a preset of the top");
    }
    let mut c = Ctu::default();
    let mut q = false;
    for _ in 0..i32::MAX {
        c.scan(false, false, i32::MAX);
        q = c.scan(true, false, i32::MAX);
    }
    assert_eq!((c.cv(), q), (i32::MAX, true), "i32::MAX edges count to the top, on at a preset of it");
    c.scan(false, false, i32::MAX);
    assert!(c.scan(true, false, i32::MAX), "one edge more at the top");
    assert_eq!(c.cv(), i32::MAX, "saturating, never overflowing");
}

// The sixteenth refute round: a fresh timer scanned with its input off, on the build programs link (only Veredas 1's
// unit tests did, on theirs).

#[test]
fn a_fresh_timer_scanned_with_its_input_off_stays_off() {
    let mut on = Ton::default();
    assert_eq!((on.scan(&at(0), false, ms(100)), on.q(), on.et()), (false, false, ms(0)));
    let mut off = Tof::default();
    assert_eq!((off.scan(&at(0), false, ms(100)), off.q(), off.et()), (false, false, ms(0)), "an off-delay with no drop");
    assert_eq!((off.scan(&at(200), false, ms(100)), off.q(), off.et()), (false, false, ms(0)));
    let mut pulse = Tp::default();
    assert_eq!((pulse.scan(&at(0), false, ms(100)), pulse.q(), pulse.et()), (false, false, ms(0)));
}

// The seventeenth refute round: an edge's input held for several scans is one edge, so a block that remembers its
// output instead of its input, re-firing while its input is held, is caught.

#[test]
fn an_input_held_for_several_scans_gives_one_edge() {
    let (t, f) = (true, false);
    let mut rise = RTrig::default();
    let clk = [t, t, t, t, f, f, f, t, t, t, f, f, f];
    let seen: Vec<(bool, bool)> = clk.map(|c| (rise.scan(c), rise.q())).to_vec();
    let want = [t, f, f, f, f, f, f, t, f, f, f, f, f];
    assert_eq!(seen, want.map(|q| (q, q)).to_vec(), "a rising edge held for four scans and for three: one pulse each");
    let mut fall = FTrig::default();
    let clk = [f, f, f, t, t, t, f, f, f, t, f];
    let seen: Vec<(bool, bool)> = clk.map(|c| (fall.scan(c), fall.q())).to_vec();
    let want = [f, f, f, f, f, f, t, f, f, f, t];
    assert_eq!(seen, want.map(|q| (q, q)).to_vec(), "low from the start: nothing; then one pulse per falling edge");
}

// The eighteenth refute round: a pulse reset with its input off stays off whatever its pt or its time then does (so
// under every option Q229 offers); a pulse's elapsed time read mid-pulse with its input off; Veredas 1's held end
// and new press on the build programs link; bits read and set beside other set bits.

#[test]
fn a_pulse_reset_with_its_input_off_stays_off_until_a_new_edge() {
    let tp = |p: &mut Tp, t: u64, input: bool, pt: u64| (p.scan(&at(t), input, ms(pt)), p.q(), p.et());
    // dropped mid-pulse, over at pt: reset, then pt raised past the time since the edge
    let mut p = Tp::default();
    tp(&mut p, 0, true, 100);
    assert_eq!(tp(&mut p, 30, false, 100), (true, true, ms(30)), "mid-pulse, its input off");
    assert_eq!(tp(&mut p, 60, false, 100), (true, true, ms(60)), "and again: its elapsed time from its edge");
    assert_eq!(tp(&mut p, 100, false, 100), (false, false, ms(0)), "over, its input off: reset");
    assert_eq!(tp(&mut p, 110, false, 200), (false, false, ms(0)), "pt raised past the time since the edge");
    assert_eq!(tp(&mut p, 60, false, 100), (false, false, ms(0)), "a scan whose time goes back");
    // held past its end, then dropped: reset, then pt raised
    let mut p = Tp::default();
    tp(&mut p, 0, true, 100);
    assert_eq!(tp(&mut p, 100, true, 100), (false, false, ms(100)), "over, its input held");
    assert_eq!(tp(&mut p, 110, false, 100), (false, false, ms(0)), "dropped: reset");
    assert_eq!(tp(&mut p, 120, false, 200), (false, false, ms(0)), "pt raised, its input off");
    // Veredas 1's unit test on the build programs link: a retrigger ignored, a held end, a drop, a new press
    let mut p = Tp::default();
    assert_eq!(tp(&mut p, 0, true, 100), (true, true, ms(0)));
    assert_eq!(tp(&mut p, 50, false, 100), (true, true, ms(50)));
    assert_eq!(tp(&mut p, 60, true, 100), (true, true, ms(60)), "a retrigger during the pulse is ignored");
    assert_eq!(tp(&mut p, 100, true, 100), (false, false, ms(100)));
    assert_eq!(tp(&mut p, 150, false, 100), (false, false, ms(0)));
    assert_eq!(tp(&mut p, 160, true, 100), (true, true, ms(0)), "a new edge, a new pulse");
}

#[test]
fn a_bit_is_read_and_written_beside_other_set_bits_on_every_width() {
    fn check<T: Bits + PartialEq + core::fmt::Debug + Copy>(low_bits: T, top: T, top_with_bit_3: T) {
        // 0b1010: bits 1 and 3 set, 0 and 2 clear
        assert_eq!([0, 1, 2, 3].map(|n| low_bits.get_bit(n)), [false, true, false, true]);
        // the top bit and bit 0 set: bit 3 set beside them, the others kept
        assert_eq!(top.with_bit(3, true), top_with_bit_3);
        let mut v = top;
        v.set_bit(3, true);
        assert_eq!(v, top_with_bit_3);
        assert_eq!(top_with_bit_3.with_bit(3, false), top, "and cleared again, the others kept");
    }
    check(0b1010_u8, 0x81_u8, 0x89_u8);
    check(0b1010_u16, 0x8001_u16, 0x8009_u16);
    check(0b1010_u32, 0x8000_0001_u32, 0x8000_0009_u32);
    check(0b1010_u64, 0x8000_0000_0000_0001_u64, 0x8000_0000_0000_0009_u64);
}
