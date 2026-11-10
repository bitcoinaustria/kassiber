//! The shared finite-price boundary of Python's custody_pricing.finite_decimal.
use crate::{Context, Decimal, DecimalError, Rounding};

/// The tax engine quantizes comparisons to 13 decimal places within 32
/// significant digits, so at most 19 integer digits fit.
const TAX_DECIMAL_LIMIT: f64 = 1e19;

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
    bounded_price(value)
}

/// [`finite_price`]'s bounds of an already parsed finite decimal.
pub fn bounded_price(value: Decimal) -> Result<Decimal, PriceError> {
    let mirror = value
        .to_string()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or(PriceError::NonFinite)?;
    if value.is_zero() {
        return Ok(Decimal::zero());
    }
    if value.exponent().unsigned_abs() > 308
        || value.copy_abs() < "1e-13".parse::<Decimal>().unwrap()
        || value.copy_abs() >= "1e19".parse::<Decimal>().unwrap()
        // The legacy numeric mirror must not round up across the limit.
        || mirror.abs() >= TAX_DECIMAL_LIMIT
    {
        return Err(PriceError::OutOfRange);
    }
    Ok(value)
}

/// Why [`validate_transaction_price`] refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransactionPriceError {
    /// The `validation` message, naming the transaction and field.
    Validation(String),
    /// Context arithmetic failed (CPython raises the signal).
    Arithmetic(DecimalError),
}

/// `validate_transaction_price`: both stored prices and the valuations they
/// imply for `amount_msat`, without persisting the inferred one. A refusal
/// names its field as `transaction {id}: {field}`.
pub fn validate_transaction_price(
    id: &str,
    amount_msat: &Decimal,
    rate: Option<&str>,
    value: Option<&str>,
    ctx: &Context,
) -> Result<(), TransactionPriceError> {
    use TransactionPriceError::{Arithmetic, Validation};
    let refuse = |field: &'static str| {
        move |e: PriceError| Validation(e.message(&format!("transaction {id}: {field}")))
    };
    for (field, text) in [("fiat_value_exact", value), ("fiat_rate_exact", rate)] {
        if let Some(text) = text.filter(|t| !t.is_empty()) {
            finite_price(text).map_err(refuse(field))?;
        }
    }
    let amount = amount_msat
        .div(&Decimal::from(100_000_000_000_i64), ctx)
        .and_then(|btc| btc.abs(ctx))
        .map_err(Arithmetic)?;
    if amount.is_zero() {
        return Ok(());
    }
    if let Some(rate) = rate.filter(|t| !t.is_empty()) {
        let rate = finite_price(rate).map_err(|e| Validation(e.message("fiat_rate")))?;
        let total = rate.mul(&amount, ctx).map_err(Arithmetic)?;
        finite_price(&total.to_string()).map_err(refuse("derived fiat_value"))?;
    }
    if let Some(value) = value.filter(|t| !t.is_empty()) {
        let value = finite_price(value).map_err(|e| Validation(e.message("fiat_value")))?;
        let unit = value.div(&amount, ctx).map_err(Arithmetic)?;
        finite_price(&unit.to_string()).map_err(refuse("derived fiat_rate"))?;
    }
    Ok(())
}

/// `btc_to_msat`: BTC to msat, rounding half up after the context product.
pub fn btc_to_msat(btc: &Decimal, ctx: &Context) -> Result<Decimal, DecimalError> {
    btc.mul(&Decimal::from(100_000_000_000_i64), ctx)?
        .quantize(0, &Context::new(crate::MAX_PRECISION, Rounding::HalfUp))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_price_matches_python_precision_and_legacy_float_limits() {
        for text in ["Infinity", "NaN", "-Infinity", "1e309"] {
            assert_eq!(finite_price(text), Err(PriceError::NonFinite));
        }
        for text in [
            "9e-14",
            "-9e-14",
            "1e-309",
            "1e19",
            "-1e19",
            "1e40",
            "1e300",
            // Below 1e19 exactly, but its float mirror rounds up to it.
            "9999999999999999999.5",
        ] {
            assert_eq!(finite_price(text), Err(PriceError::OutOfRange));
        }
        assert_eq!(
            finite_price(&format!("1.{}", "0".repeat(309))),
            Err(PriceError::OutOfRange)
        );
        for text in ["1e18", "9999999999999997952", "-9999999999999997952"] {
            assert_eq!(
                finite_price(text).unwrap().to_string(),
                text.replace("e", "E+")
            );
        }
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
    #[test]
    fn transaction_price_bounds_both_implied_valuations() {
        let ctx = Context::RP2;
        let msat = |btc: &str| btc_to_msat(&btc.parse().unwrap(), &ctx).unwrap();
        let check = |amount: &str, rate: Option<&str>, value: Option<&str>| {
            validate_transaction_price("t", &msat(amount), rate, value, &ctx)
        };
        assert_eq!(msat("0.000000000005").to_string(), "1");
        assert_eq!(
            check("0.002", Some("1e-12"), None),
            Err(TransactionPriceError::Validation(
                "transaction t: derived fiat_value is outside the supported decimal range".into()
            ))
        );
        assert_eq!(
            check("0.00000000001", None, Some("1e18")),
            Err(TransactionPriceError::Validation(
                "transaction t: derived fiat_rate is outside the supported decimal range".into()
            ))
        );
        assert_eq!(
            check("1", Some("1e19"), None),
            Err(TransactionPriceError::Validation(
                "transaction t: fiat_rate_exact is outside the supported decimal range".into()
            ))
        );
        // The stored value is checked before the rate, as Python's field order.
        assert_eq!(
            check("1", Some("1e19"), Some("NaN")),
            Err(TransactionPriceError::Validation(
                "transaction t: fiat_value_exact must be a finite decimal number".into()
            ))
        );
        assert_eq!(check("0", Some("1e-12"), Some("1e18")), Ok(()));
        assert_eq!(check("-0.002", Some("50000"), Some("100")), Ok(()));
        assert_eq!(check("0.002", Some(""), Some("")), Ok(()));
    }
}
