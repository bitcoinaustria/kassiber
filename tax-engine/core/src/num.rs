//! RP2's arithmetic and comparison rules on top of [`Decimal`].
//!
//! Every operation is one 32-digit, half-even step ([`Context::RP2`]), and
//! every comparison rounds the difference to 13 places first. Failures are
//! the `decimal` exceptions RP2 would raise.

use std::cmp::Ordering;

use crate::decimal::{Context, Decimal, RP2_COMPARE_EXPONENT};
use crate::error::EngineResult;

/// RP2's arithmetic context.
pub(crate) const CTX: Context = Context::RP2;

/// Exponent of RP2's balance tolerance mask (`1.0000000000`).
pub(crate) const BALANCE_EXPONENT: i64 = -10;

/// Exponent of RP2's fiat mask (`1.00`), used by its constructor warnings.
pub(crate) const FIAT_EXPONENT: i64 = -2;

/// RP2's `ZERO` (`Decimal("0")`, exponent 0).
pub(crate) fn zero() -> Decimal {
    Decimal::zero()
}

pub(crate) fn add(a: &Decimal, b: &Decimal) -> EngineResult<Decimal> {
    Ok(a.add(b, &CTX)?)
}

pub(crate) fn sub(a: &Decimal, b: &Decimal) -> EngineResult<Decimal> {
    Ok(a.sub(b, &CTX)?)
}

pub(crate) fn mul(a: &Decimal, b: &Decimal) -> EngineResult<Decimal> {
    Ok(a.mul(b, &CTX)?)
}

pub(crate) fn div(a: &Decimal, b: &Decimal) -> EngineResult<Decimal> {
    Ok(a.div(b, &CTX)?)
}

/// Unary minus, which rounds to the context like every RP2 operation.
pub(crate) fn neg(a: &Decimal) -> EngineResult<Decimal> {
    Ok(a.neg(&CTX)?)
}

/// RP2's three-way comparison: the sign of `(a - b)` rounded half-even to
/// 13 places. Not transitive: values within `5E-14` compare equal.
pub(crate) fn cmp13(a: &Decimal, b: &Decimal) -> EngineResult<Ordering> {
    Ok(a.sub(b, &CTX)?
        .quantize(RP2_COMPARE_EXPONENT, &CTX)?
        .signum())
}

/// RP2's `a == b`.
pub(crate) fn eq13(a: &Decimal, b: &Decimal) -> EngineResult<bool> {
    Ok(cmp13(a, b)? == Ordering::Equal)
}

/// RP2's `a > b`.
pub(crate) fn gt13(a: &Decimal, b: &Decimal) -> EngineResult<bool> {
    Ok(cmp13(a, b)? == Ordering::Greater)
}

/// RP2's `a < b` (`not a >= b`).
pub(crate) fn lt13(a: &Decimal, b: &Decimal) -> EngineResult<bool> {
    Ok(cmp13(a, b)? == Ordering::Less)
}

/// RP2's `is_equal_within_precision(a, b, mask)`: `(a - b)` rounded to the
/// mask's exponent, then compared with zero by RP2's rule.
pub(crate) fn equal_within(a: &Decimal, b: &Decimal, exponent: i64) -> EngineResult<bool> {
    let rounded = a.sub(b, &CTX)?.quantize(exponent, &CTX)?;
    eq13(&rounded, &zero())
}

/// Python truthiness of a decimal: nonzero, compared exactly.
pub(crate) fn truthy(a: &Decimal) -> bool {
    !a.is_zero()
}
