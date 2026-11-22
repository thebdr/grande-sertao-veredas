//! Bit access on unsigned values, as methods: `status.get_bit(3)`, `status.set_bit(3, true)`. ORCL unifies bit
//! strings and unsigned integers, so a `u16` serves as both a word and a number. An index past the width is not
//! an error: it reads `false` and writes nothing.

pub trait Bits: Copy {
    /// The value's width in bits.
    const WIDTH: u32;

    /// Bit `n` (0 = least significant). `false` when `n` is past the width.
    fn get_bit(self, n: u32) -> bool;

    /// The value with bit `n` set to `value`; unchanged when `n` is past the width.
    #[must_use]
    fn with_bit(self, n: u32, value: bool) -> Self;

    /// Sets bit `n` in place; does nothing when `n` is past the width.
    fn set_bit(&mut self, n: u32, value: bool) {
        *self = self.with_bit(n, value);
    }
}

macro_rules! impl_bits {
    ($($t:ty),*) => {$(
        impl Bits for $t {
            const WIDTH: u32 = <$t>::BITS;

            fn get_bit(self, n: u32) -> bool {
                self.checked_shr(n).is_some_and(|v| (v & 1) == 1)
            }

            fn with_bit(self, n: u32, value: bool) -> Self {
                match (1 as $t).checked_shl(n) {
                    Some(mask) if value => self | mask,
                    Some(mask) => self & !mask,
                    None => self,
                }
            }
        }
    )*};
}

impl_bits!(u8, u16, u32, u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_single_bits() {
        let mut status: u16 = 0b1000_0000_0000_1001;
        assert!(status.get_bit(0) && status.get_bit(3) && status.get_bit(15));
        assert!(!status.get_bit(1));
        status.set_bit(1, true);
        status.set_bit(15, false);
        assert_eq!(status, 0b0000_0000_0000_1011);
    }

    #[test]
    fn an_index_past_the_width_reads_false_and_writes_nothing() {
        let value: u8 = 0xFF;
        assert!(!value.get_bit(8));
        assert_eq!(value.with_bit(8, false), 0xFF);
        assert_eq!(u8::WIDTH, 8);
        assert_eq!(u64::WIDTH, 64);
    }
}
