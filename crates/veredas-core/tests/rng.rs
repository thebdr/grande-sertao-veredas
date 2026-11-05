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

// ---------------------------------------------------------------------------
// Refute round 1 (C-016): the sequence pinned, the extremes survived.
// ---------------------------------------------------------------------------

#[test]
fn it_is_the_reference_pcg32_and_replays_its_whole_sequence() {
    // The published output of the reference PCG32 demo for seed 42, stream 54.
    let mut r = Pcg32::new(42, 54);
    let first: Vec<u32> = (0..6).map(|_| r.next_u32()).collect();
    assert_eq!(first, [0xa15c02b7, 0x7b47f409, 0xba1d3330, 0x83d2f293, 0xbfa4784b, 0xcbed606e]);

    let run = || {
        let mut r = Pcg32::new(7, 1);
        (0..1_000)
            .map(|i| match i % 4 {
                0 => r.next_u32() as i64,
                1 => r.int_in(-5, 5),
                2 => r.uniform(0.0, 10.0, 3).to_bits() as i64,
                _ => r.flip() as i64,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run(), "every draw of every kind replays");
}

#[test]
fn streams_are_taken_modulo_two_to_the_63() {
    let seq = |stream: u64| {
        let mut r = Pcg32::new(7, stream);
        (0..8).map(|_| r.next_u32()).collect::<Vec<_>>()
    };
    assert_ne!(seq(0), seq(1));
    assert_ne!(seq(2), seq(3));
    assert_eq!(seq(0), seq(1 << 63), "the top bit of a stream id is shifted out");
    assert_eq!(seq(2), seq(2 | 1 << 63));
}

#[test]
fn int_in_survives_ranges_wider_than_i64() {
    let mut r = Pcg32::new(11, 3);
    for (low, high) in [(i64::MIN, i64::MAX), (-1, i64::MAX), (i64::MIN, 0), (i64::MAX, i64::MIN)] {
        let (lo, hi) = (low.min(high), low.max(high));
        for _ in 0..1_000 {
            let v = r.int_in(low, high);
            assert!(lo <= v && v <= hi, "{v} outside [{lo}, {hi}]");
        }
    }
    // Both ends reachable at the extremes.
    for (lo, hi) in [(i64::MAX - 1, i64::MAX), (i64::MIN, i64::MIN + 1)] {
        let seen: std::collections::BTreeSet<i64> = (0..64).map(|_| r.int_in(lo, hi)).collect();
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), [lo, hi]);
    }
}

#[test]
fn uniform_never_returns_nan_when_the_width_overflows() {
    // This seed leaves the state at 0 after `new`, so the first unit draw is 0: with
    // `lo + 0 * inf` that was NaN.
    for decimals in [0, 2, 15] {
        let v = Pcg32::new(0x3f68_1078_cd61_d75a, 0).uniform(-f64::MAX, f64::MAX, decimals);
        assert!(!v.is_nan(), "decimals {decimals}");
        assert_eq!(v, -f64::MAX, "a unit draw of 0 is the low bound");
    }
    let mut r = Pcg32::new(5, 5);
    let draws: Vec<f64> = (0..1_000).map(|_| r.uniform(-1e308, 1e308, 2)).collect();
    assert!(draws.iter().all(|v| (-1e308..=1e308).contains(v)), "in range, never NaN");
    let distinct: std::collections::BTreeSet<u64> = draws.iter().map(|v| v.to_bits()).collect();
    assert!(distinct.len() > 900, "not stuck on one value: {} distinct", distinct.len());
}
