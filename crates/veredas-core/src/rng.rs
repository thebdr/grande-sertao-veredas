//! A small deterministic RNG — PCG32.
//!
//! Hand-rolled rather than pulling `rand`: the simulator needs uniform
//! draws in a range and a coin flip, nothing more, and the workspace's
//! crate table is an approved list (a new dependency for twenty lines
//! would be the wrong trade). Seedable so tests can assert exact draws;
//! streams so two emitters seeded from the same clock don't move in
//! lockstep.

/// PCG-XSH-RR 64/32 — the reference construction.
pub struct Pcg32 {
    state: u64,
    inc: u64,
}

const MULT: u64 = 6_364_136_223_846_793_005;

impl Pcg32 {
    /// Deterministic: the same (seed, stream) always yields the same draws.
    pub fn new(seed: u64, stream: u64) -> Pcg32 {
        let mut rng = Pcg32 {
            state: 0,
            inc: (stream << 1) | 1,
        };
        rng.next_u32();
        rng.state = rng.state.wrapping_add(seed);
        rng.next_u32();
        rng
    }

    /// Seeded from the clock — for live emitters, where repeating the same
    /// sequence every launch would make the fake plant obviously fake.
    pub fn from_entropy(stream: u64) -> Pcg32 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x5DEE_CE66);
        Pcg32::new(nanos ^ (stream.wrapping_mul(0x9E37_79B9_7F4A_7C15)), stream)
    }

    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(MULT).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// A double in [0, 1) with 53 bits of entropy.
    fn unit(&mut self) -> f64 {
        let hi = self.next_u32() as u64;
        let lo = self.next_u32() as u64;
        let bits = ((hi << 21) ^ (lo >> 11)) & ((1u64 << 53) - 1);
        bits as f64 / (1u64 << 53) as f64
    }

    /// A value in `[low, high]`, rounded to `decimals`.
    ///
    /// Three guards, each of which was a real bug waiting: a non-finite
    /// limit (the GUI lets one be typed) yields 0.0 rather than NaN
    /// poisoning a log line; an inverted range is swapped rather than
    /// producing an empty span; and the result is **clamped AFTER
    /// rounding** — rounding can push a draw outside the range it came
    /// from (0.058 in [0, 0.06] rounds to 0.1 at one decimal), so
    /// clamping first would still emit an out-of-bounds value.
    pub fn uniform(&mut self, low: f64, high: f64, decimals: u8) -> f64 {
        if !low.is_finite() || !high.is_finite() {
            return 0.0;
        }
        let (lo, hi) = if low <= high {
            (low, high)
        } else {
            (high, low)
        };
        let raw = lo + self.unit() * (hi - lo);
        let factor = 10f64.powi(decimals.min(15) as i32);
        let rounded = (raw * factor).round() / factor;
        rounded.clamp(lo, hi)
    }

    /// An integer in `[low, high]`, both ends inclusive.
    pub fn int_in(&mut self, low: i64, high: i64) -> i64 {
        let (lo, hi) = if low <= high {
            (low, high)
        } else {
            (high, low)
        };
        let span = (hi - lo) as u128 + 1;
        // Modulo bias is irrelevant at simulator scale — one comment
        // instead of a rejection loop.
        let draw = ((self.next_u32() as u128) << 32 | self.next_u32() as u128) % span;
        lo + draw as i64
    }

    pub fn flip(&mut self) -> bool {
        self.next_u32() & 1 == 1
    }
}
