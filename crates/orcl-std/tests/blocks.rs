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
    let mut off = Tof::default();
    assert!(off.scan(&at(0), true, ms(0)));
    assert!(!off.scan(&at(10), false, ms(0)), "off on the scan of the drop");
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
    assert!(!Tp::default().scan(&at(0), true, ms(0)), "a pulse of zero length is off at once");
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
fn the_manifest_names_no_other_root_and_no_file_brings_in_another_crate() {
    let manifest = include_str!("../Cargo.toml");
    assert!(!manifest.contains("[lib]"), "the crate's root is src/lib.rs, the file the root's test reads");
    for (file, text) in [
        ("lib.rs", include_str!("../src/lib.rs")),
        ("bits.rs", include_str!("../src/bits.rs")),
        ("counters.rs", include_str!("../src/counters.rs")),
        ("edges.rs", include_str!("../src/edges.rs")),
        ("o14.rs", include_str!("../src/o14.rs")),
        ("scan.rs", include_str!("../src/scan.rs")),
        ("timers.rs", include_str!("../src/timers.rs")),
    ] {
        assert!(!text.contains("extern crate"), "{file} brings in a crate of its own");
    }
    let listed = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).unwrap().count();
    assert_eq!(listed, 7, "every source file is read above");
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

/// A source with every whitespace character taken out, so a spacing Rust allows hides nothing from a reading.
fn squeezed(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

#[test]
fn read_without_whitespace_no_source_brings_in_a_crate_or_a_file_or_builds_apart_for_tests() {
    for (file, text) in SOURCES {
        let tight = squeezed(text);
        for refused in ["externcrate", "#[path", "include!(", "include_str!(", "include_bytes!(", "cfg!("] {
            assert!(!tight.contains(refused), "{file} holds {refused}");
        }
        // `not(test)` only in the crate root's no_std switch
        let switches = if file == "lib.rs" { 1 } else { 0 };
        assert_eq!(tight.matches("not(test)").count(), switches, "{file}: code built apart from the tests");
        assert_eq!(tight.matches("#![cfg_attr(not(test),no_std)]").count(), switches);
        // `cfg(test)` only on the unit tests' own module
        let mut rest = tight.as_str();
        while let Some(at) = rest.find("cfg(test)") {
            let after = &rest[at + "cfg(test)".len()..];
            let tests_module = after.strip_prefix(']').is_some_and(|mut next| {
                // any further attributes, then the unit tests' module itself
                while let Some(attribute) = next.strip_prefix("#[") {
                    match attribute.find(']') {
                        Some(end) => next = &attribute[end + 1..],
                        None => return false,
                    }
                }
                next.starts_with("modtests{")
            });
            assert!(tests_module, "{file}: cfg(test) outside the unit tests' module");
            rest = after;
        }
    }
}

#[test]
fn no_source_file_pulls_one_in_from_elsewhere() {
    for (file, text) in [
        ("lib.rs", include_str!("../src/lib.rs")),
        ("bits.rs", include_str!("../src/bits.rs")),
        ("counters.rs", include_str!("../src/counters.rs")),
        ("edges.rs", include_str!("../src/edges.rs")),
        ("o14.rs", include_str!("../src/o14.rs")),
        ("scan.rs", include_str!("../src/scan.rs")),
        ("timers.rs", include_str!("../src/timers.rs")),
    ] {
        for refused in ["#[path", "include!(", "include_str!(", "include_bytes!("] {
            assert!(!text.contains(refused), "{file} pulls a file in with {refused}");
        }
    }
}

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

// The eighth refute round: an edge's q() after a scan with no edge, and the words that let a source build apart,
// bring in a crate or pull in a file, refused everywhere but at the five places the crate needs them.

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

#[test]
fn the_sources_hold_no_word_that_builds_apart_or_brings_anything_in_but_at_their_five_places() {
    for (file, text) in SOURCES {
        let tight = squeezed(text);
        for word in ["path", "include", "extern", "env"] {
            assert!(!tight.contains(word), "{file} holds `{word}`");
        }
        // `cfg` at exactly the five places: the root's no_std switch, and each unit tests' module
        let allowed: &[&str] = match file {
            "lib.rs" => &["#![cfg_attr(not(test),no_std)]"],
            "bits.rs" | "counters.rs" | "edges.rs" | "timers.rs" => &["#[cfg(test)]"],
            _ => &[],
        };
        assert_eq!(tight.matches("cfg").count(), allowed.len(), "{file} holds `cfg` elsewhere");
        for site in allowed {
            assert_eq!(tight.matches(site).count(), 1, "{file}: {site}");
        }
    }
}
