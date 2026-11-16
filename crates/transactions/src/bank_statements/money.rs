//! Exact fiat amounts. A bank amount is read as decimal digits into an
//! integer with an explicit scale; nothing here passes through a float.
use std::cmp::Ordering;

/// Largest supported scale (fractional digits).
const MAX_SCALE: u32 = 18;
/// Largest supported integer-part length, far above any real bank amount.
const MAX_INTEGER_DIGITS: usize = 18;

/// `units / 10^scale`, exact.
#[derive(Clone, Copy, Debug)]
pub struct Money {
    units: i128,
    scale: u32,
}

impl Money {
    pub const ZERO: Money = Money { units: 0, scale: 0 };

    /// Plain decimal text: optional sign, digits, optional `.` and digits.
    /// No exponent, no grouping, no whitespace.
    pub fn parse(text: &str) -> Option<Money> {
        let (negative, digits) = match text.as_bytes().first()? {
            b'-' => (true, &text[1..]),
            b'+' => (false, &text[1..]),
            _ => (false, text),
        };
        let (integer, fraction) = match digits.split_once('.') {
            Some((integer, fraction)) => (integer, fraction),
            None => (digits, ""),
        };
        if integer.is_empty() && fraction.is_empty()
            || !integer.bytes().all(|b| b.is_ascii_digit())
            || !fraction.bytes().all(|b| b.is_ascii_digit())
            || integer.trim_start_matches('0').len() > MAX_INTEGER_DIGITS
            || fraction.len() > MAX_SCALE as usize
        {
            return None;
        }
        let mut units: i128 = 0;
        for b in integer.bytes().chain(fraction.bytes()) {
            units = units * 10 + i128::from(b - b'0');
        }
        Some(Money {
            units: if negative { -units } else { units },
            scale: fraction.len() as u32,
        })
    }

    pub fn is_negative(&self) -> bool {
        self.units < 0
    }

    pub fn is_zero(&self) -> bool {
        self.units == 0
    }

    pub fn abs(self) -> Money {
        Money {
            units: self.units.abs(),
            scale: self.scale,
        }
    }

    pub fn negated(self) -> Money {
        Money {
            units: -self.units,
            scale: self.scale,
        }
    }

    fn at(self, scale: u32) -> i128 {
        self.units * 10i128.pow(scale - self.scale)
    }

    pub fn plus(self, other: Money) -> Money {
        let scale = self.scale.max(other.scale);
        Money {
            units: self.at(scale) + other.at(scale),
            scale,
        }
    }

    /// Exact product with another exact decimal, kept within the supported
    /// scale by rounding half away from zero only past 18 fractional digits.
    pub fn times(self, other: Money) -> Option<Money> {
        let mut units = self.units.checked_mul(other.units)?;
        let mut scale = self.scale + other.scale;
        while scale > MAX_SCALE {
            let rem = units % 10;
            units /= 10;
            if rem >= 5 {
                units += 1;
            } else if rem <= -5 {
                units -= 1;
            }
            scale -= 1;
        }
        Some(Money { units, scale })
    }

    /// Canonical text: no trailing zeros beyond two fractional digits, at
    /// least two fractional digits, `-` for negatives.
    pub fn canonical(self) -> String {
        let mut units = self.units;
        let mut scale = self.scale;
        while scale > 2 && units % 10 == 0 {
            units /= 10;
            scale -= 1;
        }
        let units = Money { units, scale }.at(scale.max(2));
        let scale = scale.max(2) as usize;
        let digits = units.unsigned_abs().to_string();
        let digits = format!("{digits:0>width$}", width = scale + 1);
        let (integer, fraction) = digits.split_at(digits.len() - scale);
        format!("{}{integer}.{fraction}", if units < 0 { "-" } else { "" })
    }
}

impl PartialEq for Money {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Money {}
impl PartialOrd for Money {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Money {
    fn cmp(&self, other: &Self) -> Ordering {
        let scale = self.scale.max(other.scale);
        self.at(scale).cmp(&other.at(scale))
    }
}
