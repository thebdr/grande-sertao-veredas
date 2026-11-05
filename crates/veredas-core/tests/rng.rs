//! The deterministic PCG32 (contract: the rng clause), from Veredas 1's `veredas-telegram` tests.

use veredas_core::rng::*;

#[test]
fn the_same_seed_replays_and_streams_diverge() {
    let a: Vec<u32> = (0..8).map(|_| Pcg32::new(7, 1).next_u32()).collect();
    let mut once = Pcg32::new(7, 1);
    let b: Vec<u32> = (0..8).map(|_| once.next_u32()).collect();
    assert_eq!(a[0], b[0], "a fresh rng with the same seed starts equal");
    let mut s1 = Pcg32::new(7, 1);
    let mut s2 = Pcg32::new(7, 2);
    let d1: Vec<u32> = (0..8).map(|_| s1.next_u32()).collect();
    let d2: Vec<u32> = (0..8).map(|_| s2.next_u32()).collect();
    assert_ne!(d1, d2, "different streams must not run in lockstep");
}

#[test]
fn uniform_never_leaves_the_range_even_when_rounding_fights_it() {
    // The trap this guards: a draw of 0.058 in [0, 0.06] rounds to 0.1
    // at one decimal — outside the range it came from. Clamping has to
    // happen after the rounding, so tight ranges cannot leak.
    let mut rng = Pcg32::new(42, 1);
    for _ in 0..10_000 {
        let v = rng.uniform(0.0, 0.06, 1);
        assert!((0.0..=0.06).contains(&v), "{v} escaped [0, 0.06]");
    }
    for _ in 0..10_000 {
        let v = rng.uniform(-5.0, 12.5, 2);
        assert!((-5.0..=12.5).contains(&v), "{v} escaped [-5, 12.5]");
        assert_eq!(v, (v * 100.0).round() / 100.0, "{v} is not 2-decimal");
    }
}

#[test]
fn uniform_survives_the_values_a_gui_lets_you_type() {
    let mut rng = Pcg32::new(1, 1);
    // Inverted range: swapped, never an empty span or a panic.
    for _ in 0..100 {
        let v = rng.uniform(10.0, 2.0, 1);
        assert!((2.0..=10.0).contains(&v), "{v}");
    }
    // A constant range is a constant.
    assert_eq!(rng.uniform(3.5, 3.5, 2), 3.5);
    // Half-typed limits must not poison a log line with NaN.
    assert_eq!(rng.uniform(f64::NAN, 1.0, 1), 0.0);
    assert_eq!(rng.uniform(0.0, f64::INFINITY, 1), 0.0);
}

#[test]
fn int_in_covers_both_ends_and_flip_produces_both_faces() {
    let mut rng = Pcg32::new(9, 3);
    let mut seen_low = false;
    let mut seen_high = false;
    for _ in 0..2_000 {
        let v = rng.int_in(0, 3);
        assert!((0..=3).contains(&v), "{v}");
        seen_low |= v == 0;
        seen_high |= v == 3;
    }
    assert!(seen_low && seen_high, "both ends must be reachable");
    assert_eq!(rng.int_in(5, 5), 5, "a single-value range is constant");
    for _ in 0..200 {
        let v = rng.int_in(7, 2);
        assert!((2..=7).contains(&v), "an inverted range is swapped: {v}");
    }

    let mut heads = false;
    let mut tails = false;
    for _ in 0..200 {
        if rng.flip() {
            heads = true
        } else {
            tails = true
        }
    }
    assert!(heads && tails);
}
