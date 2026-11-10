//! CPython's `repr(float)`.

/// CPython's `repr(x)` (equally `str(x)`) for a float: the shortest digits
/// that read back as `x`, in fixed notation when the decimal point falls
/// within 16 places (`1000000000000000.0`, `0.0001`) and scientific notation
/// otherwise (`1e+16`, `1e-05`, `2.5e-308`), as `float.__repr__` writes it.
/// Specials are `inf`, `-inf`, and `nan` (whatever the NaN's sign); JSON
/// spells them differently, see [`crate::dumps`].
///
/// When two shortest candidates are equally close to `x`, CPython's dtoa
/// keeps the even last digit; Rust's own formatting rounds such ties up, so
/// they are corrected here.
pub fn float_repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".to_owned();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.to_owned();
    }
    let mut out = String::with_capacity(24);
    if x.is_sign_negative() {
        out.push('-');
    }
    if x == 0.0 {
        out.push_str("0.0");
        return out;
    }
    let (digits, exponent) = shortest_digits(x.abs());
    // CPython's `decpt`: x = 0.<digits> * 10^decpt.
    let decpt = exponent + 1;
    let n = digits.len() as i32;
    if decpt <= -4 || decpt > 16 {
        out.push_str(&digits[..1]);
        if n > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if exponent < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}", exponent.unsigned_abs()));
    } else if decpt <= 0 {
        out.push_str("0.");
        out.extend(core::iter::repeat_n('0', decpt.unsigned_abs() as usize));
        out.push_str(&digits);
    } else if decpt >= n {
        out.push_str(&digits);
        out.extend(core::iter::repeat_n('0', (decpt - n) as usize));
        out.push_str(".0");
    } else {
        out.push_str(&digits[..decpt as usize]);
        out.push('.');
        out.push_str(&digits[decpt as usize..]);
    }
    out
}

/// The shortest digits that read back as the finite, positive `x`, and the
/// decimal exponent of the first digit (`x ≈ d.ddd * 10^exponent`).
fn shortest_digits(x: f64) -> (String, i32) {
    // `{:e}` writes the shortest round-trip digits, e.g. `1.2345e-5`.
    let formatted = format!("{x:e}");
    let (mantissa, exponent) = formatted
        .split_once('e')
        .expect("`{:e}` always writes an exponent");
    let exponent: i32 = exponent.parse().expect("`{:e}` writes an integer exponent");
    let mut digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    break_tie_to_even(x, &mut digits, exponent);
    (digits, exponent)
}

/// Replaces an odd last digit by its even neighbour when `x` lies exactly
/// halfway between them and the neighbour also reads back as `x`.
fn break_tie_to_even(x: f64, digits: &mut String, exponent: i32) {
    // x is a candidate D * 10^k away by half a unit, so 2x = (2D ± 1) * 10^k.
    let k = exponent - (digits.len() as i32 - 1);
    // x = m * 2^e with m odd.
    let bits = x.to_bits();
    let biased = (bits >> 52) as i32;
    let fraction = bits & ((1 << 52) - 1);
    let (mut m, mut e) = if biased == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), biased - 1075)
    };
    let zeros = m.trailing_zeros();
    m >>= zeros;
    e += zeros as i32;
    // The power of two in (2D ± 1) * 10^k is 2^k, and in 2x it is 2^(e + 1).
    if e + 1 != k {
        return;
    }
    let Ok(d) = digits.parse::<u64>() else {
        return;
    };
    if d % 2 == 0 {
        return;
    }
    let m = u128::from(m);
    let d = u128::from(d);
    for (neighbour, odd) in [(d - 1, 2 * d - 1), (d + 1, 2 * d + 1)] {
        // Compare the odd parts: m = (2D ± 1) * 5^k, or m * 5^-k = 2D ± 1.
        let tie = if k >= 0 {
            5u128.checked_pow(k as u32).and_then(|p| odd.checked_mul(p)) == Some(m)
        } else {
            5u128
                .checked_pow(k.unsigned_abs())
                .and_then(|p| m.checked_mul(p))
                == Some(odd)
        };
        if tie {
            let candidate = neighbour.to_string();
            if candidate.len() == digits.len() && format!("{candidate}e{k}").parse::<f64>() == Ok(x)
            {
                *digits = candidate;
            }
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::float_repr;

    #[test]
    fn writes_cpython_layouts() {
        let cases = [
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.0, "1.0"),
            (0.1, "0.1"),
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (1234567890123456.7, "1234567890123456.8"),
            (12345678901234567.0, "1.2345678901234568e+16"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.5e-7, "1.5e-07"),
            (1e100, "1e+100"),
            (5e-324, "5e-324"),
            (f64::MAX, "1.7976931348623157e+308"),
            (f64::INFINITY, "inf"),
            (f64::NEG_INFINITY, "-inf"),
            (f64::NAN, "nan"),
            (-f64::NAN, "nan"),
        ];
        for (x, expected) in cases {
            assert_eq!(float_repr(x), expected, "{x:e}");
        }
    }

    #[test]
    fn breaks_exact_ties_to_the_even_digit() {
        // Exactly 2053637337633904.25: .2 and .3 are both shortest and
        // equally close, and Rust's own formatting would write .3.
        assert_eq!(
            float_repr(f64::from_bits(0x431d2f16d00751c1)),
            "2053637337633904.2"
        );
        // Exactly -233891771783429.625.
        assert_eq!(
            float_repr(f64::from_bits(0xc2ea97257f2320b4)),
            "-233891771783429.62"
        );
    }
}
