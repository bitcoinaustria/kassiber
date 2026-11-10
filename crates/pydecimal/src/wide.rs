//! Fixed-width helpers for the fast paths: coefficients that fit `u128`,
//! and the 256-bit intermediates of their products and scaled dividends.
//! Every helper is exact. A caller falls back to `BigUint` arithmetic when
//! a value does not fit, so these never decide a result the `BigUint` path
//! would decide differently.

use core::cmp::Ordering;

use num_bigint::BigUint;

use crate::Discarded;

/// `10^k` for every `k` whose power fits `u128` (`0..=38`).
pub(crate) const POW10: [u128; 39] = {
    let mut table = [1u128; 39];
    let mut k = 1;
    while k < 39 {
        table[k] = table[k - 1] * 10;
        k += 1;
    }
    table
};

/// `10^19`, the largest power of ten below `2^64`.
const CHUNK: u64 = 10_000_000_000_000_000_000;
const CHUNK_DIGITS: usize = 19;

/// Number of decimal digits in `v`; zero has one digit.
pub(crate) fn ndigits(v: u128) -> usize {
    // A value of `bits` bits has `guess` or `guess + 1` digits, where
    // `guess = floor(bits * 1233 / 4096)` and 1233 / 4096 is just below
    // log10(2); the test checks this for every bit length.
    let bits = (u128::BITS - v.leading_zeros()) as usize;
    let guess = (bits * 1233) >> 12;
    (guess + usize::from(v >= POW10[guess])).max(1)
}

/// `c * 10^k`, when it fits.
pub(crate) fn scale(c: u128, k: i128) -> Option<u128> {
    let k = usize::try_from(k).ok()?;
    c.checked_mul(*POW10.get(k)?)
}

/// What dropping a remainder `r` below a divisor `p` discards, with `sticky`
/// marking a nonzero amount below `r`'s last digit (`shift_right`'s rule).
fn discarded(r: u128, p: u128, sticky: bool) -> Discarded {
    if r == 0 {
        return if sticky {
            Discarded::BelowHalf
        } else {
            Discarded::Zero
        };
    }
    // r < p <= 10^38, so 2r < 2^128.
    match (r * 2).cmp(&p) {
        Ordering::Less => Discarded::BelowHalf,
        Ordering::Equal if sticky => Discarded::AboveHalf,
        Ordering::Equal => Discarded::Half,
        Ordering::Greater => Discarded::AboveHalf,
    }
}

/// `shift_right` for a coefficient that fits `u128`: drops the last `k`
/// digits of `c` (which has `nd` digits).
pub(crate) fn shift_right(c: u128, nd: usize, k: i128, sticky: bool) -> (u128, Discarded) {
    if k <= 0 {
        let d = if sticky {
            Discarded::BelowHalf
        } else {
            Discarded::Zero
        };
        return (c, d);
    }
    let k = match usize::try_from(k) {
        Ok(k) if k <= nd => k,
        _ => {
            let d = if c == 0 && !sticky {
                Discarded::Zero
            } else {
                Discarded::BelowHalf
            };
            return (0, d);
        }
    };
    if k >= POW10.len() {
        // k = nd = 39: every digit goes, and c < 2^128 < 10^39 / 2.
        return (0, Discarded::BelowHalf);
    }
    let p = POW10[k];
    (c / p, discarded(c % p, p, sticky))
}

/// An unsigned 256-bit integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct U256 {
    hi: u128,
    lo: u128,
}

const LOW: u128 = u64::MAX as u128;

/// `10^k` for every `k` whose power fits 256 bits (`0..=77`).
const POW10_WIDE: [U256; 78] = {
    let mut table = [U256 { hi: 0, lo: 1 }; 78];
    let mut k = 1;
    while k < 78 {
        let prev = table[k - 1];
        let lo = mul(prev.lo, 10);
        table[k] = U256 {
            hi: prev.hi * 10 + lo.hi,
            lo: lo.lo,
        };
        k += 1;
    }
    table
};

/// The full product of two `u128`s.
pub(crate) const fn mul(a: u128, b: u128) -> U256 {
    let (a1, a0) = (a >> 64, a & LOW);
    let (b1, b0) = (b >> 64, b & LOW);
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    // Each term is below 2^64, so the sum is below 2^66.
    let mid = (p00 >> 64) + (p01 & LOW) + (p10 & LOW);
    U256 {
        hi: p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64),
        lo: (p00 & LOW) | (mid << 64),
    }
}

impl U256 {
    pub(crate) fn from_u128(v: u128) -> Self {
        U256 { hi: 0, lo: v }
    }

    pub(crate) fn to_u128(self) -> Option<u128> {
        (self.hi == 0).then_some(self.lo)
    }

    pub(crate) fn to_big(self) -> BigUint {
        (BigUint::from(self.hi) << 128u32) + BigUint::from(self.lo)
    }

    /// `self + other`, when it fits.
    pub(crate) fn checked_add(self, other: Self) -> Option<Self> {
        let (lo, carry) = self.lo.overflowing_add(other.lo);
        let hi = self
            .hi
            .checked_add(other.hi)?
            .checked_add(u128::from(carry))?;
        Some(U256 { hi, lo })
    }

    /// `self - other`, for `self >= other`.
    pub(crate) fn sub(self, other: Self) -> Self {
        let (lo, borrow) = self.lo.overflowing_sub(other.lo);
        U256 {
            hi: self.hi - other.hi - u128::from(borrow),
            lo,
        }
    }

    /// `self * m`, when it fits.
    pub(crate) fn checked_mul(self, m: u128) -> Option<Self> {
        let low = mul(self.lo, m);
        let high = mul(self.hi, m);
        if high.hi != 0 {
            return None;
        }
        Some(U256 {
            hi: low.hi.checked_add(high.lo)?,
            lo: low.lo,
        })
    }

    /// `self * 10^k`, when it fits.
    pub(crate) fn scale(self, k: usize) -> Option<Self> {
        let mut value = self;
        let mut rest = k;
        while rest > 0 {
            let step = rest.min(POW10.len() - 1);
            value = value.checked_mul(POW10[step])?;
            rest -= step;
        }
        Some(value)
    }

    /// Quotient and remainder by a nonzero `d`.
    pub(crate) fn div_rem_u64(self, d: u64) -> (Self, u64) {
        let d = u128::from(d);
        let mut rem: u128 = 0;
        let mut out = [0u128; 4];
        let limbs = [self.hi >> 64, self.hi & LOW, self.lo >> 64, self.lo & LOW];
        for (slot, limb) in out.iter_mut().zip(limbs) {
            // rem < d < 2^64, so this fits.
            let current = (rem << 64) | limb;
            *slot = current / d;
            rem = current % d;
        }
        (
            U256 {
                hi: (out[0] << 64) | out[1],
                lo: (out[2] << 64) | out[3],
            },
            rem as u64,
        )
    }

    /// Number of decimal digits; zero has one digit.
    pub(crate) fn ndigits(self) -> usize {
        if self.hi == 0 {
            return ndigits(self.lo);
        }
        POW10_WIDE.partition_point(|p| *p <= self).max(1)
    }

    /// `shift_right`: drops the last `k` digits (of `nd`), lowest chunks
    /// first. A chunk's remainder only marks the digits below the last chunk
    /// as nonzero, which is what `sticky` records.
    pub(crate) fn shift_right(self, nd: usize, k: usize, sticky: bool) -> (Self, Discarded) {
        if k == 0 {
            let d = if sticky {
                Discarded::BelowHalf
            } else {
                Discarded::Zero
            };
            return (self, d);
        }
        if k > nd {
            let d = if self == U256::from_u128(0) && !sticky {
                Discarded::Zero
            } else {
                Discarded::BelowHalf
            };
            return (U256::from_u128(0), d);
        }
        let (mut value, mut rest, mut sticky) = (self, k, sticky);
        while rest > CHUNK_DIGITS {
            let (q, r) = value.div_rem_u64(CHUNK);
            sticky |= r != 0;
            value = q;
            rest -= CHUNK_DIGITS;
        }
        let p = POW10[rest];
        let (q, r) = value.div_rem_u64(p as u64);
        (q, discarded(u128::from(r), p, sticky))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big(v: U256) -> BigUint {
        v.to_big()
    }

    #[test]
    fn powers_and_digit_counts_match_bignum() {
        for k in 0..78usize {
            let expected = num_traits::pow(BigUint::from(10u32), k);
            assert_eq!(big(POW10_WIDE[k]), expected, "10^{k}");
            if k < 39 {
                assert_eq!(BigUint::from(POW10[k]), expected);
                assert_eq!(ndigits(POW10[k]), k + 1);
                if k > 0 {
                    assert_eq!(ndigits(POW10[k] - 1), k);
                }
            }
            assert_eq!(POW10_WIDE[k].ndigits(), k + 1);
            if k > 0 {
                let below = U256 {
                    hi: POW10_WIDE[k].hi - u128::from(POW10_WIDE[k].lo == 0),
                    lo: POW10_WIDE[k].lo.wrapping_sub(1),
                };
                assert_eq!(below.ndigits(), k, "10^{k} - 1");
            }
        }
        assert_eq!(ndigits(0), 1);
        assert_eq!(ndigits(u128::MAX), 39);
        // Every bit length: its smallest and largest values, and the
        // powers of ten and their neighbours inside it.
        for bits in 1..=128u32 {
            let low = 1u128 << (bits - 1);
            let high = if bits == 128 {
                u128::MAX
            } else {
                (1u128 << bits) - 1
            };
            for v in [low, high] {
                assert_eq!(ndigits(v), v.to_string().len(), "{v}");
            }
        }
        for &power in &POW10[1..] {
            for v in [power - 1, power, power + 1] {
                assert_eq!(ndigits(v), v.to_string().len(), "{v}");
            }
        }
        let max = U256 {
            hi: u128::MAX,
            lo: u128::MAX,
        };
        assert_eq!(max.ndigits(), 78);
    }

    #[test]
    fn products_and_divisions_match_bignum() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..20_000 {
            let mut operand = || {
                let shift = (next() % 128) as u32;
                (u128::from(next()) << 64 | u128::from(next())) >> shift
            };
            let (a, b) = (operand(), operand());
            let product = mul(a, b);
            assert_eq!(big(product), BigUint::from(a) * BigUint::from(b));
            let d = next() >> (next() % 64);
            if d != 0 {
                let (q, r) = product.div_rem_u64(d);
                let (bq, br) = num_integer::Integer::div_rem(&big(product), &BigUint::from(d));
                assert_eq!((big(q), BigUint::from(r)), (bq, br));
            }
            let m = u128::from(next());
            let scaled = product.checked_mul(m);
            let expected = big(product) * BigUint::from(m);
            match scaled {
                Some(value) => assert_eq!(big(value), expected),
                None => assert!(expected.bits() > 256),
            }
        }
    }
}
