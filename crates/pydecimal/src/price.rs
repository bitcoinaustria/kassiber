//! The shared finite-price boundary of Python's custody_pricing.finite_decimal.
use crate::{Decimal, DecimalError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriceError {
    Invalid,
    NonFinite,
    OutOfRange,
}
impl PriceError {
    pub fn message(self, field: &str) -> String {
        match self {
            Self::Invalid => format!("{field} must be a decimal number"),
            Self::NonFinite => format!("{field} must be a finite decimal number"),
            Self::OutOfRange => format!("{field} is outside the supported decimal range"),
        }
    }
}

/// Parse before formatting, retain authored scale, and canonicalize every zero.
/// Surrounding whitespace follows CPython's str.strip, including U+001C..001F.
pub fn finite_price(text: &str) -> Result<Decimal, PriceError> {
    let value: Decimal = text
        .trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
        .parse()
        .map_err(|e| match e {
            DecimalError::NonFinite => PriceError::NonFinite,
            _ => PriceError::Invalid,
        })?;
    if !value.to_string().parse::<f64>().is_ok_and(f64::is_finite) {
        return Err(PriceError::NonFinite);
    }
    if value.is_zero() {
        return Ok(Decimal::zero());
    }
    if value.exponent().unsigned_abs() > 308
        || value.copy_abs() < "1e-13".parse::<Decimal>().unwrap()
    {
        return Err(PriceError::OutOfRange);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_price_matches_python_precision_and_legacy_float_limits() {
        for text in ["Infinity", "NaN", "-Infinity", "1e309"] {
            assert_eq!(finite_price(text), Err(PriceError::NonFinite));
        }
        for text in ["9e-14", "-9e-14", "1e-309"] {
            assert_eq!(finite_price(text), Err(PriceError::OutOfRange));
        }
        assert_eq!(
            finite_price(&format!("1.{}", "0".repeat(309))),
            Err(PriceError::OutOfRange)
        );
        assert_eq!(finite_price("bad"), Err(PriceError::Invalid));
        assert_eq!(finite_price("-0e-999").unwrap().to_string(), "0");
        assert_eq!(
            finite_price("\u{1c}1e-13\u{1f}").unwrap().to_string(),
            "1E-13"
        );
        assert_eq!(
            finite_price("12.500").unwrap().to_fixed_string().unwrap(),
            "12.500"
        );
    }
}
