//! Finite decimal arithmetic that reproduces CPython's `decimal` module.
//!
//! RP2, the tax engine this crate replaces, computes with Python's `decimal`
//! module under context precision 32, `ROUND_HALF_EVEN`, the default
//! `Emin`/`Emax`, and `FloatOperation` trapped, and compares through
//! `RP2Decimal`'s fuzzy 13-place comparisons. [`Decimal`] reproduces those
//! results exactly: coefficient, exponent, and sign, including negative zero.
//! The implementation follows the General Decimal Arithmetic specification,
//! which CPython's `decimal` module implements.
//!
//! Every operation that can round takes an explicit [`Context`]; nothing reads
//! global state. The conditions CPython traps by default (`InvalidOperation`,
//! `DivisionByZero`, `Overflow`) are returned as [`DecimalError`]. The
//! conditions it does not trap (`Inexact`, `Rounded`, `Subnormal`,
//! `Underflow`, `Clamped`) pass silently, as they do in CPython. There are no
//! infinities, NaNs, or float conversions, and no input panics.
//!
//! Parsing ([`FromStr`]) accepts every string `decimal.Decimal(str)` accepts
//! for a finite number, including Unicode decimal digits from any script,
//! except that surrounding whitespace is rejected rather than stripped.
//! [`Decimal::to_canonical`] and
//! [`Decimal::from_canonical`] are the lossless form for the JSON boundary.
//! The cost of an operation grows with the context precision.

use core::cmp::Ordering;
use core::fmt;
use core::str::FromStr;

use num_bigint::BigUint;
use num_integer::Integer;
use num_traits::{ToPrimitive, Zero};

/// Largest adjusted exponent of an arithmetic result (CPython's default `Emax`).
pub const EMAX: i64 = 999_999;

/// Smallest adjusted exponent of a normal arithmetic result (CPython's default
/// `Emin`). Smaller results are subnormal and keep fewer digits.
pub const EMIN: i64 = -999_999;

/// Largest adjusted exponent a constructed value may have (CPython's
/// `decimal.MAX_EMAX`). Construction is exact, so it is not bound by [`EMAX`].
pub const MAX_CONSTRUCTED_ADJUSTED: i64 = 999_999_999_999_999_999;

/// Smallest exponent a constructed value may have (CPython's
/// `decimal.MIN_ETINY`).
pub const MIN_CONSTRUCTED_EXPONENT: i64 = -1_999_999_999_999_999_997;

/// Exponent of RP2's comparison mask `Decimal("1.0000000000000")`.
pub const RP2_COMPARE_EXPONENT: i64 = -13;

/// Most padding zeros [`Decimal::to_fixed_string`] writes. Every arithmetic
/// result fits; only constructed values with extreme exponents exceed it.
pub const MAX_FIXED_PADDING: u64 = 100_000_000;

/// Saturation bound for parsed exponent digits. It is far outside every valid
/// range, so a saturated exponent is always rejected.
const EXPONENT_SATURATION: i128 = 1_000_000_000_000_000_000_000_000_000_000;

/// How an operation discards digits; the variants mirror CPython's `ROUND_*`
/// constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rounding {
    /// `ROUND_CEILING`: toward positive infinity.
    Ceiling,
    /// `ROUND_DOWN`: toward zero.
    Down,
    /// `ROUND_FLOOR`: toward negative infinity.
    Floor,
    /// `ROUND_HALF_DOWN`: to nearest, ties toward zero.
    HalfDown,
    /// `ROUND_HALF_EVEN`: to nearest, ties to an even last digit (RP2's mode).
    HalfEven,
    /// `ROUND_HALF_UP`: to nearest, ties away from zero.
    HalfUp,
    /// `ROUND_UP`: away from zero.
    Up,
    /// `ROUND_05UP`: toward zero, unless that leaves a last digit of 0 or 5,
    /// in which case away from zero.
    ZeroFiveUp,
}

/// Precision and rounding for operations that can round.
///
/// `Emin`, `Emax`, `clamp`, and the trap set are CPython's defaults
/// ([`EMIN`], [`EMAX`], 0, and `InvalidOperation`/`DivisionByZero`/`Overflow`)
/// for every context.
/// Largest precision a [`Context`] accepts. RP2 uses 32; the cap bounds
/// arithmetic intermediates, whose size grows with the precision.
pub const MAX_PRECISION: u32 = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Context {
    /// Significant digits a rounded result keeps, from 1 to [`MAX_PRECISION`].
    pub prec: u32,
    /// How rounded results discard digits.
    pub rounding: Rounding,
}

impl Context {
    /// RP2's context: precision 32, `ROUND_HALF_EVEN`.
    pub const RP2: Context = Context {
        prec: 32,
        rounding: Rounding::HalfEven,
    };

    /// Builds a context; operations reject a precision of 0 or above
    /// [`MAX_PRECISION`].
    pub const fn new(prec: u32, rounding: Rounding) -> Self {
        Context { prec, rounding }
    }

    /// Smallest exponent a result may have: `Emin - prec + 1`.
    pub const fn etiny(&self) -> i64 {
        EMIN - self.prec as i64 + 1
    }

    fn check(&self) -> Result<(), DecimalError> {
        if self.prec == 0 || self.prec > MAX_PRECISION {
            Err(DecimalError::InvalidContext)
        } else {
            Ok(())
        }
    }
}

/// Why an operation produced no value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DecimalError {
    /// The string is not a decimal number (CPython's `ConversionSyntax`).
    ConversionSyntax,
    /// The string denotes an infinity or NaN, which this type cannot hold.
    NonFinite,
    /// CPython's `InvalidOperation`: a quantize result needs more than `prec`
    /// digits or its exponent is out of range, or a constructed value is
    /// outside CPython's representable range.
    InvalidOperation,
    /// A nonzero value divided by zero.
    DivisionByZero,
    /// Zero divided by zero (CPython's `DivisionUndefined`).
    DivisionUndefined,
    /// A result's adjusted exponent exceeds [`EMAX`].
    Overflow,
    /// The context's precision is 0.
    InvalidContext,
    /// A fixed-point rendering would need more than [`MAX_FIXED_PADDING`]
    /// padding zeros.
    FormatTooLong,
}

impl DecimalError {
    /// Stable name of the condition; CPython's signal name where one exists.
    pub const fn name(&self) -> &'static str {
        match self {
            DecimalError::ConversionSyntax => "ConversionSyntax",
            DecimalError::NonFinite => "NonFinite",
            DecimalError::InvalidOperation => "InvalidOperation",
            DecimalError::DivisionByZero => "DivisionByZero",
            DecimalError::DivisionUndefined => "DivisionUndefined",
            DecimalError::Overflow => "Overflow",
            DecimalError::InvalidContext => "InvalidContext",
            DecimalError::FormatTooLong => "FormatTooLong",
        }
    }
}

impl fmt::Display for DecimalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            DecimalError::ConversionSyntax => "not a decimal number",
            DecimalError::NonFinite => "infinities and NaNs are not supported",
            DecimalError::InvalidOperation => "invalid decimal operation",
            DecimalError::DivisionByZero => "division by zero",
            DecimalError::DivisionUndefined => "zero divided by zero",
            DecimalError::Overflow => "decimal overflow",
            DecimalError::InvalidContext => "decimal context precision must be between 1 and 1000",
            DecimalError::FormatTooLong => "fixed-point rendering is too long",
        };
        f.write_str(text)
    }
}

impl std::error::Error for DecimalError {}

/// A finite decimal number: `(-1)^sign * coefficient * 10^exponent`.
///
/// The representation is significant, as in CPython: `1.0` and `1.00` are
/// equal numbers with different exponents, and `-0` keeps its sign.
/// Equality and ordering ([`PartialEq`], [`Ord`]) compare numerical values
/// exactly, so `0 == -0` and `1.0 == 1.00`; use
/// [`Decimal::same_representation`] to compare representations.
#[derive(Clone)]
pub struct Decimal {
    negative: bool,
    coeff: BigUint,
    exp: i64,
}

/// What a rounding step discarded, relative to half a unit in the last kept
/// place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Discarded {
    Zero,
    BelowHalf,
    Half,
    AboveHalf,
}

fn pow10(k: usize) -> BigUint {
    num_traits::pow(BigUint::from(10u32), k)
}

fn pow10_wide(k: i128) -> Result<BigUint, DecimalError> {
    usize::try_from(k)
        .map(pow10)
        .map_err(|_| DecimalError::InvalidOperation)
}

fn ndigits_u64(mut v: u64) -> usize {
    let mut d = 1;
    while v >= 10 {
        v /= 10;
        d += 1;
    }
    d
}

/// Number of decimal digits in `c`; zero has one digit.
fn ndigits(c: &BigUint) -> usize {
    if let Some(v) = c.to_u64() {
        return ndigits_u64(v);
    }
    // c >= 2^(bits-1) and 0.301029995 < log10(2), so c >= 10^est.
    let bits = u128::from(c.bits());
    let est = usize::try_from((bits - 1) * 301_029_995 / 1_000_000_000).unwrap_or(0);
    let mut p = pow10(est);
    let mut d = est + 1;
    loop {
        let next = &p * 10u32;
        if next > *c {
            return d;
        }
        p = next;
        d += 1;
    }
}

/// Drops the last `k` digits of `c` (which has `nd` digits). `sticky` marks a
/// nonzero amount below the last digit of `c`.
fn shift_right(c: &BigUint, nd: usize, k: i128, sticky: bool) -> (BigUint, Discarded) {
    if k <= 0 {
        let d = if sticky {
            Discarded::BelowHalf
        } else {
            Discarded::Zero
        };
        return (c.clone(), d);
    }
    let k = match usize::try_from(k) {
        Ok(k) if k <= nd => k,
        _ => {
            // Every digit goes, and c < 10^nd <= 10^(k-1) is below half.
            let d = if c.is_zero() && !sticky {
                Discarded::Zero
            } else {
                Discarded::BelowHalf
            };
            return (BigUint::zero(), d);
        }
    };
    let p = pow10(k);
    let (q, r) = c.div_rem(&p);
    let d = if r.is_zero() {
        if sticky {
            Discarded::BelowHalf
        } else {
            Discarded::Zero
        }
    } else {
        match (&r * 2u32).cmp(&p) {
            Ordering::Less => Discarded::BelowHalf,
            Ordering::Equal if sticky => Discarded::AboveHalf,
            Ordering::Equal => Discarded::Half,
            Ordering::Greater => Discarded::AboveHalf,
        }
    };
    (q, d)
}

/// Whether rounding the truncated coefficient `q` moves it away from zero.
fn rounds_away(rounding: Rounding, d: Discarded, negative: bool, q: &BigUint) -> bool {
    if d == Discarded::Zero {
        return false;
    }
    match rounding {
        Rounding::Down => false,
        Rounding::Up => true,
        Rounding::Ceiling => !negative,
        Rounding::Floor => negative,
        Rounding::HalfUp => d >= Discarded::Half,
        Rounding::HalfDown => d == Discarded::AboveHalf,
        Rounding::HalfEven => d == Discarded::AboveHalf || (d == Discarded::Half && q.is_odd()),
        Rounding::ZeroFiveUp => {
            let last = (q % 10u32).to_u32().unwrap_or(1);
            last == 0 || last == 5
        }
    }
}

fn narrow_exponent(exp: i128) -> Result<i64, DecimalError> {
    i64::try_from(exp).map_err(|_| DecimalError::Overflow)
}

/// Rounds an exact intermediate `coeff * 10^exp` (plus a nonzero amount below
/// its last digit when `sticky`) to the context, as CPython finalizes every
/// arithmetic result: precision rounding, subnormal rounding at `Etiny`, zero
/// exponent clamping, and the overflow check.
fn finish(
    negative: bool,
    coeff: BigUint,
    exp: i128,
    sticky: bool,
    ctx: &Context,
) -> Result<Decimal, DecimalError> {
    let prec = i128::from(ctx.prec);
    let etiny = i128::from(ctx.etiny());
    let emax = i128::from(EMAX);
    if coeff.is_zero() && !sticky {
        let exp = narrow_exponent(exp.clamp(etiny, emax))?;
        return Ok(Decimal {
            negative,
            coeff,
            exp,
        });
    }
    let nd = ndigits(&coeff);
    // A subnormal result (adjusted exponent below Emin) is rounded at Etiny
    // instead, which always discards more digits than precision would.
    let discard = (nd as i128 - prec).max(etiny - exp).max(0);
    let (mut q, d) = shift_right(&coeff, nd, discard, sticky);
    let mut exp = exp + discard;
    if rounds_away(ctx.rounding, d, negative, &q) {
        q += 1u32;
        if ndigits(&q) as i128 > prec {
            // q was all nines and is now exactly 10^prec.
            q /= 10u32;
            exp += 1;
        }
    }
    if !q.is_zero() && exp + ndigits(&q) as i128 - 1 > emax {
        return Err(DecimalError::Overflow);
    }
    Ok(Decimal {
        negative,
        coeff: q,
        exp: narrow_exponent(exp)?,
    })
}

/// Reads a run of exponent digits, saturating far outside every valid range.
fn parse_exponent_digits(digits: &[u8]) -> i128 {
    let mut value: i128 = 0;
    for &b in digits {
        value = (value * 10 + i128::from(b - b'0')).min(EXPONENT_SATURATION);
    }
    value
}

/// First code point (digit zero) of every run of ten non-ASCII Unicode
/// decimal digits, sorted: the characters with a `decimal` value in the
/// Unicode Character Database 14.0.0, which CPython 3.11 uses. The vector
/// file checks every entry against `unicodedata`; a newer Unicode version
/// may add runs.
const UNICODE_DIGIT_ZEROS: [u32; 65] = [
    0x0660, 0x06F0, 0x07C0, 0x0966, 0x09E6, 0x0A66, 0x0AE6, 0x0B66, 0x0BE6, 0x0C66, 0x0CE6, 0x0D66,
    0x0DE6, 0x0E50, 0x0ED0, 0x0F20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80, 0x1A90,
    0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0, 0xFF10,
    0x104A0, 0x10D30, 0x11066, 0x110F0, 0x11136, 0x111D0, 0x112F0, 0x11450, 0x114D0, 0x11650,
    0x116C0, 0x11730, 0x118E0, 0x11950, 0x11C50, 0x11D50, 0x11DA0, 0x16A60, 0x16AC0, 0x16B50,
    0x1D7CE, 0x1D7D8, 0x1D7E2, 0x1D7EC, 0x1D7F6, 0x1E140, 0x1E2F0, 0x1E950, 0x1FBF0,
];

/// The value of a non-ASCII Unicode decimal digit (`unicodedata.decimal`),
/// or `None` for any other non-ASCII character.
fn unicode_digit_value(c: char) -> Option<u8> {
    let cp = u32::from(c);
    let run = UNICODE_DIGIT_ZEROS.partition_point(|&zero| zero <= cp);
    let zero = UNICODE_DIGIT_ZEROS[run.checked_sub(1)?];
    u8::try_from(cp - zero).ok().filter(|&d| d < 10)
}

/// Converts `s` to the ASCII text CPython's parser reads: underscores are
/// dropped and every Unicode decimal digit becomes its ASCII digit, so signs,
/// points, exponent markers, and NaN/Infinity words stay ASCII-only. Any
/// other non-ASCII character is a syntax error (CPython maps whitespace to a
/// space, which is one too, except surrounding whitespace, which it strips).
fn python_ascii(s: &str) -> Result<Vec<u8>, DecimalError> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        if c == '_' {
            continue;
        }
        if c.is_ascii() {
            out.push(c as u8);
        } else {
            let digit = unicode_digit_value(c).ok_or(DecimalError::ConversionSyntax)?;
            out.push(b'0' + digit);
        }
    }
    Ok(out)
}

fn is_non_finite_word(body: &[u8]) -> bool {
    let lower: Vec<u8> = body.iter().map(u8::to_ascii_lowercase).collect();
    if lower == b"inf" || lower == b"infinity" {
        return true;
    }
    let payload = if let Some(rest) = lower.strip_prefix(b"snan") {
        rest
    } else if let Some(rest) = lower.strip_prefix(b"nan") {
        rest
    } else {
        return false;
    };
    payload.iter().all(u8::is_ascii_digit)
}

impl Decimal {
    /// Positive zero with exponent 0.
    pub fn zero() -> Self {
        Decimal {
            negative: false,
            coeff: BigUint::zero(),
            exp: 0,
        }
    }

    /// Builds a value from its parts, exactly. Fails with `InvalidOperation`
    /// outside CPython's representable range (see
    /// [`MAX_CONSTRUCTED_ADJUSTED`] and [`MIN_CONSTRUCTED_EXPONENT`]).
    pub fn from_parts(
        negative: bool,
        coefficient: BigUint,
        exponent: i64,
    ) -> Result<Self, DecimalError> {
        Self::checked(negative, coefficient, i128::from(exponent))
    }

    fn checked(negative: bool, coeff: BigUint, exp: i128) -> Result<Self, DecimalError> {
        // Zero has one digit, so its adjusted exponent is its exponent.
        let adjusted = exp + ndigits(&coeff) as i128 - 1;
        if exp < i128::from(MIN_CONSTRUCTED_EXPONENT)
            || adjusted > i128::from(MAX_CONSTRUCTED_ADJUSTED)
        {
            return Err(DecimalError::InvalidOperation);
        }
        Ok(Decimal {
            negative,
            coeff,
            exp: narrow_exponent(exp)?,
        })
    }

    /// Parses the lossless canonical form `[-]<digits>E<exp>` written by
    /// [`Decimal::to_canonical`]. Only that exact form is accepted: no `+`,
    /// no leading zeros, and no `-0` exponent.
    pub fn from_canonical(s: &str) -> Result<Self, DecimalError> {
        let bytes = s.as_bytes();
        let (negative, rest) = match bytes.split_first() {
            Some((b'-', rest)) => (true, rest),
            _ => (false, bytes),
        };
        let e = rest
            .iter()
            .position(|&b| b == b'E')
            .ok_or(DecimalError::ConversionSyntax)?;
        let (digits, exp_part) = (&rest[..e], &rest[e + 1..]);
        let canonical_digits = |d: &[u8]| {
            !d.is_empty() && d.iter().all(u8::is_ascii_digit) && (d.len() == 1 || d[0] != b'0')
        };
        let (exp_negative, exp_digits) = match exp_part.split_first() {
            Some((b'-', rest)) => (true, rest),
            _ => (false, exp_part),
        };
        if !canonical_digits(digits)
            || !canonical_digits(exp_digits)
            || (exp_negative && exp_digits == b"0")
        {
            return Err(DecimalError::ConversionSyntax);
        }
        let values: Vec<u8> = digits.iter().map(|b| b - b'0').collect();
        let coeff = BigUint::from_radix_be(&values, 10).ok_or(DecimalError::ConversionSyntax)?;
        let mut exp = parse_exponent_digits(exp_digits);
        if exp_negative {
            exp = -exp;
        }
        Self::checked(negative, coeff, exp)
    }

    /// Parses any string `decimal.Decimal(str)` accepts for a finite number,
    /// exactly (no rounding), with CPython's range limits. As in CPython,
    /// underscores are ignored wherever they appear and any Unicode decimal
    /// digit counts as its ASCII digit. Unlike CPython, surrounding
    /// whitespace is rejected rather than stripped.
    fn parse_python(s: &str) -> Result<Self, DecimalError> {
        let bytes = python_ascii(s)?;
        let (negative, body) = match bytes.split_first() {
            Some((b'-', rest)) => (true, rest),
            Some((b'+', rest)) => (false, rest),
            _ => (false, &bytes[..]),
        };
        if is_non_finite_word(body) {
            return Err(DecimalError::NonFinite);
        }
        let mut values: Vec<u8> = Vec::new();
        let mut frac_len: usize = 0;
        let mut seen_digit = false;
        let mut seen_point = false;
        let mut pos = 0;
        while let Some(&b) = body.get(pos) {
            match b {
                b'0'..=b'9' => {
                    seen_digit = true;
                    if seen_point {
                        frac_len += 1;
                    }
                    // Leading zeros do not enter the coefficient.
                    if b != b'0' || !values.is_empty() {
                        values.push(b - b'0');
                    }
                }
                b'.' if !seen_point => seen_point = true,
                _ => break,
            }
            pos += 1;
        }
        if !seen_digit {
            return Err(DecimalError::ConversionSyntax);
        }
        let mut exponent: i128 = 0;
        if let Some(&marker) = body.get(pos) {
            if marker != b'e' && marker != b'E' {
                return Err(DecimalError::ConversionSyntax);
            }
            let mut rest = &body[pos + 1..];
            let mut exp_negative = false;
            if let Some((&sign, tail)) = rest.split_first() {
                if sign == b'+' || sign == b'-' {
                    exp_negative = sign == b'-';
                    rest = tail;
                }
            }
            if rest.is_empty() || !rest.iter().all(u8::is_ascii_digit) {
                return Err(DecimalError::ConversionSyntax);
            }
            exponent = parse_exponent_digits(rest);
            if exp_negative {
                exponent = -exponent;
            }
        }
        let coeff = if values.is_empty() {
            BigUint::zero()
        } else {
            BigUint::from_radix_be(&values, 10).ok_or(DecimalError::ConversionSyntax)?
        };
        Self::checked(negative, coeff, exponent - frac_len as i128)
    }

    /// Whether the sign bit is set; true for negative values and `-0`.
    pub fn is_sign_negative(&self) -> bool {
        self.negative
    }

    /// The coefficient, without sign.
    pub fn coefficient(&self) -> &BigUint {
        &self.coeff
    }

    /// The exponent: the value is `coefficient * 10^exponent`.
    pub fn exponent(&self) -> i64 {
        self.exp
    }

    /// The adjusted exponent: the exponent of the most significant digit.
    pub fn adjusted(&self) -> i64 {
        let adj = i128::from(self.exp) + ndigits(&self.coeff) as i128 - 1;
        i64::try_from(adj).unwrap_or(i64::MAX)
    }

    /// Whether the value is zero (of either sign).
    pub fn is_zero(&self) -> bool {
        self.coeff.is_zero()
    }

    /// Whether the value is less than zero; false for `-0`.
    pub fn is_negative(&self) -> bool {
        self.negative && !self.is_zero()
    }

    /// Whether the value is greater than zero.
    pub fn is_positive(&self) -> bool {
        !self.negative && !self.is_zero()
    }

    /// The sign of the value: `Less`, `Equal` (either zero), or `Greater`.
    pub fn signum(&self) -> Ordering {
        if self.is_zero() {
            Ordering::Equal
        } else if self.negative {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }

    /// Whether sign, coefficient, and exponent are all identical.
    pub fn same_representation(&self, other: &Decimal) -> bool {
        self.negative == other.negative && self.exp == other.exp && self.coeff == other.coeff
    }

    /// The value with its sign flipped, exactly (CPython's `copy_negate`).
    pub fn copy_negate(&self) -> Decimal {
        Decimal {
            negative: !self.negative,
            ..self.clone()
        }
    }

    /// The value with its sign cleared, exactly (CPython's `copy_abs`).
    pub fn copy_abs(&self) -> Decimal {
        Decimal {
            negative: false,
            ..self.clone()
        }
    }

    /// `+x` under `ctx`: the value rounded to the context. A zero becomes
    /// `+0` unless rounding is `Floor`.
    pub fn plus(&self, ctx: &Context) -> Result<Decimal, DecimalError> {
        ctx.check()?;
        let negative = self.negative && !(self.is_zero() && ctx.rounding != Rounding::Floor);
        finish(
            negative,
            self.coeff.clone(),
            i128::from(self.exp),
            false,
            ctx,
        )
    }

    /// `-x` under `ctx` (Python's unary minus, which RP2 uses): the negated
    /// value rounded to the context. A zero becomes `+0` unless rounding is
    /// `Floor`, which flips its sign.
    pub fn neg(&self, ctx: &Context) -> Result<Decimal, DecimalError> {
        ctx.check()?;
        let negative = if self.is_zero() && ctx.rounding != Rounding::Floor {
            false
        } else {
            !self.negative
        };
        finish(
            negative,
            self.coeff.clone(),
            i128::from(self.exp),
            false,
            ctx,
        )
    }

    /// `abs(x)` under `ctx`: the magnitude rounded to the context.
    pub fn abs(&self, ctx: &Context) -> Result<Decimal, DecimalError> {
        ctx.check()?;
        finish(false, self.coeff.clone(), i128::from(self.exp), false, ctx)
    }

    /// `self + other`, correctly rounded under `ctx`.
    pub fn add(&self, other: &Decimal, ctx: &Context) -> Result<Decimal, DecimalError> {
        self.add_signed(other, other.negative, ctx)
    }

    /// `self - other`, correctly rounded under `ctx`.
    pub fn sub(&self, other: &Decimal, ctx: &Context) -> Result<Decimal, DecimalError> {
        self.add_signed(other, !other.negative, ctx)
    }

    /// Adds `self` and `other` with its sign replaced by `other_negative`.
    fn add_signed(
        &self,
        other: &Decimal,
        other_negative: bool,
        ctx: &Context,
    ) -> Result<Decimal, DecimalError> {
        ctx.check()?;
        let prec = i128::from(ctx.prec);
        let a = (self.negative, &self.coeff, i128::from(self.exp));
        let b = (other_negative, &other.coeff, i128::from(other.exp));
        if a.1.is_zero() && b.1.is_zero() {
            let negative = if a.0 == b.0 {
                a.0
            } else {
                ctx.rounding == Rounding::Floor
            };
            return finish(negative, BigUint::zero(), a.2.min(b.2), false, ctx);
        }
        // `hi` has the larger exponent; the exact sum has `lo`'s exponent.
        let (hi, lo) = if a.2 >= b.2 { (a, b) } else { (b, a) };
        if hi.1.is_zero() {
            return finish(lo.0, lo.1.clone(), lo.2, false, ctx);
        }
        let hi_adjusted = hi.2 + ndigits(hi.1) as i128 - 1;
        let (lo_coeff, lo_exp) = if lo.1.is_zero() {
            // A zero only lowers the exponent, and digits more than prec + 1
            // places below `hi` are discarded as zeros anyway.
            (BigUint::zero(), lo.2.max(hi.2 - prec - 1))
        } else {
            // When `lo` lies wholly below both `hi`'s last digit and the
            // rounding position (which is at least hi_adjusted - prec, even
            // after a borrow), only its sign and nonzeroness matter: any value
            // in (0, 10^limit) rounds the same, so use 10^(limit - 1).
            let limit = (hi.2 - 1).min(hi_adjusted - prec - 1);
            let lo_adjusted = lo.2 + ndigits(lo.1) as i128 - 1;
            if lo_adjusted < limit {
                (BigUint::from(1u32), limit - 1)
            } else {
                (lo.1.clone(), lo.2)
            }
        };
        let hi_aligned = hi.1 * pow10_wide(hi.2 - lo_exp)?;
        if lo_coeff.is_zero() {
            return finish(hi.0, hi_aligned, lo_exp, false, ctx);
        }
        if hi.0 == lo.0 {
            return finish(hi.0, hi_aligned + lo_coeff, lo_exp, false, ctx);
        }
        match hi_aligned.cmp(&lo_coeff) {
            Ordering::Greater => finish(hi.0, hi_aligned - lo_coeff, lo_exp, false, ctx),
            Ordering::Less => finish(lo.0, lo_coeff - hi_aligned, lo_exp, false, ctx),
            Ordering::Equal => finish(
                ctx.rounding == Rounding::Floor,
                BigUint::zero(),
                lo_exp,
                false,
                ctx,
            ),
        }
    }

    /// `self * other`, correctly rounded under `ctx`.
    pub fn mul(&self, other: &Decimal, ctx: &Context) -> Result<Decimal, DecimalError> {
        ctx.check()?;
        let negative = self.negative != other.negative;
        let exp = i128::from(self.exp) + i128::from(other.exp);
        finish(negative, &self.coeff * &other.coeff, exp, false, ctx)
    }

    /// `self / other`, correctly rounded under `ctx`. An exact quotient takes
    /// the exponent closest to the ideal `self.exponent - other.exponent`.
    /// Fails with `DivisionByZero` for `x / 0` and `DivisionUndefined` for
    /// `0 / 0`.
    pub fn div(&self, other: &Decimal, ctx: &Context) -> Result<Decimal, DecimalError> {
        ctx.check()?;
        if other.is_zero() {
            return Err(if self.is_zero() {
                DecimalError::DivisionUndefined
            } else {
                DecimalError::DivisionByZero
            });
        }
        let negative = self.negative != other.negative;
        let ideal = i128::from(self.exp) - i128::from(other.exp);
        if self.is_zero() {
            return finish(negative, BigUint::zero(), ideal, false, ctx);
        }
        // Scale so the integer quotient has prec + 1 or prec + 2 digits:
        // at least one digit beyond the precision decides the rounding.
        let prec = i128::from(ctx.prec);
        let shift = prec + ndigits(&other.coeff) as i128 - ndigits(&self.coeff) as i128 + 1;
        let (mut q, r) = if shift >= 0 {
            (&self.coeff * pow10_wide(shift)?).div_rem(&other.coeff)
        } else {
            self.coeff.div_rem(&(&other.coeff * pow10_wide(-shift)?))
        };
        let mut exp = ideal - shift;
        let exact = r.is_zero();
        if exact {
            let ten = BigUint::from(10u32);
            while exp < ideal {
                let (tq, tr) = q.div_rem(&ten);
                if !tr.is_zero() {
                    break;
                }
                q = tq;
                exp += 1;
            }
        }
        finish(negative, q, exp, !exact, ctx)
    }

    /// Rounds to exponent `exp` under `ctx` (CPython's `quantize` with a
    /// mask of exponent `exp`). Fails with `InvalidOperation` when the result
    /// needs more than `prec` digits or `exp` is outside `Etiny..=Emax`.
    pub fn quantize(&self, exp: i64, ctx: &Context) -> Result<Decimal, DecimalError> {
        ctx.check()?;
        if exp > EMAX || exp < ctx.etiny() {
            return Err(DecimalError::InvalidOperation);
        }
        if self.is_zero() {
            return Ok(Decimal {
                negative: self.negative,
                coeff: BigUint::zero(),
                exp,
            });
        }
        let prec = i128::from(ctx.prec);
        let nd = ndigits(&self.coeff);
        let drop = i128::from(exp) - i128::from(self.exp);
        let coeff = if drop <= 0 {
            if nd as i128 - drop > prec {
                return Err(DecimalError::InvalidOperation);
            }
            &self.coeff * pow10_wide(-drop)?
        } else {
            let (mut q, d) = shift_right(&self.coeff, nd, drop, false);
            if rounds_away(ctx.rounding, d, self.negative, &q) {
                q += 1u32;
            }
            if ndigits(&q) as i128 > prec {
                return Err(DecimalError::InvalidOperation);
            }
            q
        };
        if !coeff.is_zero() && i128::from(exp) + ndigits(&coeff) as i128 - 1 > i128::from(EMAX) {
            return Err(DecimalError::InvalidOperation);
        }
        Ok(Decimal {
            negative: self.negative,
            coeff,
            exp,
        })
    }

    /// Sign of `(self - other).quantize(1E-13)` under `ctx`, the quantity
    /// every RP2 comparison inspects.
    fn rp2_compare(&self, other: &Decimal, ctx: &Context) -> Result<Ordering, DecimalError> {
        let diff = self.sub(other, ctx)?;
        Ok(diff.quantize(RP2_COMPARE_EXPONENT, ctx)?.signum())
    }

    /// RP2's `==`: `(self - other).quantize(1E-13) == 0` under `ctx`.
    pub fn rp2_eq(&self, other: &Decimal, ctx: &Context) -> Result<bool, DecimalError> {
        Ok(self.rp2_compare(other, ctx)? == Ordering::Equal)
    }

    /// RP2's `!=`: `not (self == other)`.
    pub fn rp2_ne(&self, other: &Decimal, ctx: &Context) -> Result<bool, DecimalError> {
        Ok(self.rp2_compare(other, ctx)? != Ordering::Equal)
    }

    /// RP2's `>`: `(self - other).quantize(1E-13) > 0` under `ctx`.
    pub fn rp2_gt(&self, other: &Decimal, ctx: &Context) -> Result<bool, DecimalError> {
        Ok(self.rp2_compare(other, ctx)? == Ordering::Greater)
    }

    /// RP2's `>=`: `(self - other).quantize(1E-13) >= 0` under `ctx`.
    pub fn rp2_ge(&self, other: &Decimal, ctx: &Context) -> Result<bool, DecimalError> {
        Ok(self.rp2_compare(other, ctx)? != Ordering::Less)
    }

    /// RP2's `<`: `not (self >= other)`.
    pub fn rp2_lt(&self, other: &Decimal, ctx: &Context) -> Result<bool, DecimalError> {
        Ok(!self.rp2_ge(other, ctx)?)
    }

    /// RP2's `<=`: `not (self > other)`.
    pub fn rp2_le(&self, other: &Decimal, ctx: &Context) -> Result<bool, DecimalError> {
        Ok(!self.rp2_gt(other, ctx)?)
    }

    /// The lossless canonical form `[-]<digits>E<exp>`, e.g. `-12345E-3` or
    /// `-0E0`, used at the JSON boundary.
    pub fn to_canonical(&self) -> String {
        let sign = if self.negative { "-" } else { "" };
        format!("{}{}E{}", sign, self.coeff.to_str_radix(10), self.exp)
    }

    /// CPython's `str(x)`: plain notation when the exponent is at most 0 and
    /// the adjusted exponent at least -6, scientific notation otherwise.
    fn to_sci_string(&self) -> String {
        let digits = self.coeff.to_str_radix(10);
        let mut out = String::with_capacity(digits.len() + 24);
        if self.negative {
            out.push('-');
        }
        let exp = i128::from(self.exp);
        let adjusted = exp + digits.len() as i128 - 1;
        if exp <= 0 && adjusted >= -6 {
            // Here -exp <= digits.len() + 6, so the padding is small.
            let int_len = digits.len() as i128 + exp;
            if exp == 0 {
                out.push_str(&digits);
            } else if int_len > 0 {
                let (int_part, frac_part) = digits.split_at(int_len as usize);
                out.push_str(int_part);
                out.push('.');
                out.push_str(frac_part);
            } else {
                out.push_str("0.");
                out.extend(core::iter::repeat('0').take((-int_len) as usize));
                out.push_str(&digits);
            }
        } else {
            let (first, rest) = digits.split_at(1);
            out.push_str(first);
            if !rest.is_empty() {
                out.push('.');
                out.push_str(rest);
            }
            out.push('E');
            if adjusted >= 0 {
                out.push('+');
            }
            out.push_str(&adjusted.to_string());
        }
        out
    }

    /// CPython's `format(x, 'f')`: fixed-point notation with every digit of
    /// the representation (`1E+3` gives `1000`, `-0.00` gives `-0.00`).
    /// Fails with `FormatTooLong` beyond [`MAX_FIXED_PADDING`] padding zeros.
    pub fn to_fixed_string(&self) -> Result<String, DecimalError> {
        let digits = self.coeff.to_str_radix(10);
        let exp = i128::from(self.exp);
        let int_len = digits.len() as i128 + exp;
        let padding = if exp >= 0 {
            if self.is_zero() {
                0
            } else {
                exp
            }
        } else if int_len > 0 {
            0
        } else {
            -int_len
        };
        if padding > i128::from(MAX_FIXED_PADDING) {
            return Err(DecimalError::FormatTooLong);
        }
        let padding = padding as usize;
        let mut out = String::with_capacity(digits.len() + padding + 3);
        if self.negative {
            out.push('-');
        }
        if self.is_zero() && exp >= 0 {
            out.push('0');
        } else if exp >= 0 {
            out.push_str(&digits);
            out.extend(core::iter::repeat('0').take(padding));
        } else if int_len > 0 {
            let (int_part, frac_part) = digits.split_at(int_len as usize);
            out.push_str(int_part);
            out.push('.');
            out.push_str(frac_part);
        } else {
            out.push_str("0.");
            out.extend(core::iter::repeat('0').take(padding));
            out.push_str(&digits);
        }
        Ok(out)
    }

    /// Compares magnitudes of two nonzero values.
    fn cmp_magnitude(&self, other: &Decimal) -> Ordering {
        let a_adj = i128::from(self.exp) + ndigits(&self.coeff) as i128;
        let b_adj = i128::from(other.exp) + ndigits(&other.coeff) as i128;
        if a_adj != b_adj {
            return a_adj.cmp(&b_adj);
        }
        // Equal adjusted exponents: the exponent gap equals the digit-count
        // gap, so aligning is cheap and the scale always fits in usize.
        let gap = i128::from(self.exp) - i128::from(other.exp);
        match pow10_wide(gap.abs()) {
            Ok(scale) if gap >= 0 => (&self.coeff * scale).cmp(&other.coeff),
            Ok(scale) => self.coeff.cmp(&(&other.coeff * scale)),
            Err(_) => Ordering::Equal,
        }
    }
}

macro_rules! from_signed {
    ($($t:ty),*) => {$(
        impl From<$t> for Decimal {
            /// Exact conversion with exponent 0, as `Decimal(int)`.
            fn from(v: $t) -> Self {
                Decimal {
                    negative: v < 0,
                    coeff: BigUint::from(v.unsigned_abs()),
                    exp: 0,
                }
            }
        }
    )*};
}

macro_rules! from_unsigned {
    ($($t:ty),*) => {$(
        impl From<$t> for Decimal {
            /// Exact conversion with exponent 0, as `Decimal(int)`.
            fn from(v: $t) -> Self {
                Decimal {
                    negative: false,
                    coeff: BigUint::from(v),
                    exp: 0,
                }
            }
        }
    )*};
}

from_signed!(i8, i16, i32, i64, i128, isize);
from_unsigned!(u8, u16, u32, u64, u128, usize);

impl FromStr for Decimal {
    type Err = DecimalError;

    /// Parses like `decimal.Decimal(str)` for finite numbers, exactly.
    /// Unicode decimal digits are accepted; surrounding whitespace is not.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Decimal::parse_python(s)
    }
}

impl fmt::Display for Decimal {
    /// Writes CPython's `str(x)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_sci_string())
    }
}

impl fmt::Debug for Decimal {
    /// Writes `Decimal(<canonical form>)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Decimal({})", self.to_canonical())
    }
}

impl PartialEq for Decimal {
    /// Exact numerical equality: `0 == -0` and `1.0 == 1.00`.
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Decimal {}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Decimal {
    /// Exact numerical ordering; zeros of either sign are equal.
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.signum(), other.signum()) {
            (Ordering::Equal, s) => s.reverse(),
            (s, Ordering::Equal) => s,
            (Ordering::Less, Ordering::Greater) => Ordering::Less,
            (Ordering::Greater, Ordering::Less) => Ordering::Greater,
            (Ordering::Less, _) => self.cmp_magnitude(other).reverse(),
            _ => self.cmp_magnitude(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RP2: Context = Context::RP2;

    fn d(s: &str) -> Decimal {
        match s.parse::<Decimal>() {
            Ok(v) => v,
            Err(e) => panic!("{s:?} did not parse: {e}"),
        }
    }

    fn canon(r: Result<Decimal, DecimalError>) -> String {
        match r {
            Ok(v) => v.to_canonical(),
            Err(e) => format!("!{}", e.name()),
        }
    }

    fn ctx(prec: u32, rounding: Rounding) -> Context {
        Context::new(prec, rounding)
    }

    #[test]
    fn parses_every_finite_python_form() {
        let cases = [
            ("0", "0E0"),
            ("-0", "-0E0"),
            ("+0", "0E0"),
            ("-0.00", "-0E-2"),
            (".5", "5E-1"),
            ("5.", "5E0"),
            ("-.0", "-0E-1"),
            ("1E+5", "1E5"),
            ("1e5", "1E5"),
            ("1e-5", "1E-5"),
            ("1E-0", "1E0"),
            ("00012E1", "12E1"),
            ("0.e1", "0E1"),
            (".0e1", "0E0"),
            ("12.3400", "123400E-4"),
            ("1_000", "1000E0"),
            ("_1__0.0_1_", "1001E-2"),
            ("1e1_0", "1E10"),
            ("-_0", "-0E0"),
            ("1E00000000000000000000000000005", "1E5"),
            ("0.001E+1000000000000000002", "1E999999999999999999"),
            (
                "12345678901234567890123456789012345",
                "12345678901234567890123456789012345E0",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(canon(input.parse()), expected, "parsing {input:?}");
        }
    }

    #[test]
    fn rejects_malformed_and_out_of_range_strings() {
        let syntax = [
            "", ".", "e5", "1e", "1e+", "+", "-", "++1", "1.2.3", "1E+-5", "0x10", "1,5", "_",
            "infinit", "NaN-1", ".e1",
        ];
        for input in syntax {
            assert_eq!(
                canon(input.parse()),
                "!ConversionSyntax",
                "parsing {input:?}"
            );
        }
        for input in ["Inf", "-infinity", "NaN", "nan_1", "-sNaN12", "In_f"] {
            assert_eq!(canon(input.parse()), "!NonFinite", "parsing {input:?}");
        }
        for input in [
            "1E1000000000000000000",
            "1E-1999999999999999998",
            "0E-1999999999999999998",
            "0E+1000000000000000000",
            "1E9223372036854775808",
            "1E-99999999999999999999999999999999999999999",
        ] {
            assert_eq!(
                canon(input.parse()),
                "!InvalidOperation",
                "parsing {input:?}"
            );
        }
        assert_eq!(
            canon("1E-1999999999999999997".parse()),
            "1E-1999999999999999997"
        );
        assert_eq!(
            canon("0E+999999999999999999".parse()),
            "0E999999999999999999"
        );
    }

    #[test]
    fn reads_unicode_decimal_digits_as_cpython_does() {
        let cases = [
            ("\u{661}\u{662}", "12E0"),
            ("1\u{662}", "12E0"),
            ("\u{ff11}", "1E0"),
            ("1e\u{661}", "1E1"),
            ("-\u{966}.\u{966}\u{966}", "-0E-2"),
            ("\u{661}_\u{662}.\u{665}E-\u{ff12}", "125E-3"),
            ("\u{1d7cf}\u{1d7d8}\u{1fbf9}", "109E0"),
            ("\u{1e950}\u{1e959}", "9E0"),
        ];
        for (input, expected) in cases {
            assert_eq!(canon(input.parse()), expected, "parsing {input:?}");
        }
        for input in ["nan\u{661}", "-sNaN\u{661}\u{662}", "NaN_\u{ff10}"] {
            assert_eq!(canon(input.parse()), "!NonFinite", "parsing {input:?}");
        }
        // Digit-like characters without a decimal value, look-alike signs and
        // letters, inner whitespace, and the neighbours of digit runs.
        for input in [
            "\u{b2}",
            "1\u{2460}",
            "\u{2170}",
            "\u{3007}",
            "1\u{66b}5",
            "\u{ff0d}1",
            "\u{2212}1",
            "1\u{ff45}5",
            "\u{ff4e}an",
            "\u{130}nf",
            "inf\u{661}",
            "1\u{3000}2",
            "\u{65f}",
            "\u{66a}",
            "\u{1fbfa}",
            "\u{feff}1",
            "1\u{0}",
        ] {
            assert_eq!(
                canon(input.parse()),
                "!ConversionSyntax",
                "parsing {input:?}"
            );
        }
    }

    #[test]
    fn unicode_digit_table_is_sorted_runs_of_ten() {
        assert!(UNICODE_DIGIT_ZEROS.windows(2).all(|w| w[0] + 10 <= w[1]));
        for &zero in &UNICODE_DIGIT_ZEROS {
            for (value, cp) in (zero..zero + 10).enumerate() {
                let c = char::from_u32(cp).expect("assigned scalar value");
                assert_eq!(unicode_digit_value(c), Some(value as u8), "U+{cp:04X}");
            }
        }
        for c in ['0', '9', 'a', '\u{65f}', '\u{66a}', '\u{1fbfa}', char::MAX] {
            assert_eq!(unicode_digit_value(c), None, "{c:?}");
        }
    }

    #[test]
    fn rejects_surrounding_whitespace_that_cpython_strips() {
        for input in [" 1", "1 ", "1\t", "\u{a0}1", "1\u{3000}", "\u{1c}1"] {
            assert_eq!(
                canon(input.parse()),
                "!ConversionSyntax",
                "parsing {input:?}"
            );
        }
    }

    #[test]
    fn canonical_form_round_trips_and_is_strict() {
        for s in ["0E0", "-0E0", "-0E-2", "-12345E-3", "1E5", "7E-1000030"] {
            let v = Decimal::from_canonical(s).expect("canonical");
            assert_eq!(v.to_canonical(), s);
        }
        for s in [
            "", "E0", "1E", "01E0", "1E01", "1E-0", "+1E0", "1E+5", "1.0E0", "1e0", "1E5 ",
        ] {
            assert_eq!(
                Decimal::from_canonical(s).map(|v| v.to_canonical()),
                Err(DecimalError::ConversionSyntax),
                "{s:?}"
            );
        }
    }

    #[test]
    fn integers_convert_exactly() {
        assert_eq!(Decimal::from(0i64).to_canonical(), "0E0");
        assert_eq!(Decimal::from(-5i32).to_canonical(), "-5E0");
        assert_eq!(
            Decimal::from(i128::MIN).to_canonical(),
            "-170141183460469231731687303715884105728E0"
        );
        assert_eq!(
            Decimal::from(u128::MAX).to_canonical(),
            "340282366920938463463374607431768211455E0"
        );
    }

    #[test]
    fn renders_str_and_fixed_forms() {
        let cases = [
            ("0", "0", "0"),
            ("-0", "-0", "-0"),
            ("-0.00", "-0.00", "-0.00"),
            ("0E-8", "0E-8", "0.00000000"),
            ("0E+2", "0E+2", "0"),
            ("-0E+5", "-0E+5", "-0"),
            ("1E+5", "1E+5", "100000"),
            ("123.45", "123.45", "123.45"),
            ("0.000001", "0.000001", "0.000001"),
            ("0.0000001", "1E-7", "0.0000001"),
            ("1.5E-10", "1.5E-10", "0.00000000015"),
            ("-12345E-3", "-12.345", "-12.345"),
            ("12345E3", "1.2345E+7", "12345000"),
        ];
        for (input, s, f) in cases {
            let v = d(input);
            assert_eq!(v.to_string(), s, "str({input})");
            assert_eq!(
                v.to_fixed_string().as_deref(),
                Ok(f),
                "format({input}, 'f')"
            );
        }
        assert_eq!(
            d("1E+999999999999999999").to_fixed_string(),
            Err(DecimalError::FormatTooLong)
        );
        assert_eq!(
            d("1E+999999999999999999").to_string(),
            "1E+999999999999999999"
        );
    }

    #[test]
    fn adds_and_subtracts_with_cpython_rounding_and_zero_signs() {
        assert_eq!(canon(d("0.1").add(&d("0.2"), &RP2)), "3E-1");
        assert_eq!(canon(d("1").sub(&d("1"), &RP2)), "0E0");
        assert_eq!(
            canon(d("1").sub(&d("1"), &ctx(32, Rounding::Floor))),
            "-0E0"
        );
        assert_eq!(canon(d("-0").add(&d("-0"), &RP2)), "-0E0");
        assert_eq!(canon(d("0").sub(&d("0"), &RP2)), "0E0");
        assert_eq!(canon(d("-0").sub(&d("0"), &RP2)), "-0E0");
        assert_eq!(canon(d("1.00").add(&d("-0E5"), &RP2)), "100E-2");
        // Exact sum needs 33 digits: a tie, broken to even.
        let big = d("12345678901234567890123456789012");
        assert_eq!(
            canon(big.add(&d("0.5"), &RP2)),
            "12345678901234567890123456789012E0"
        );
        assert_eq!(
            canon(d("12345678901234567890123456789013").add(&d("0.5"), &RP2)),
            "12345678901234567890123456789014E0"
        );
        // A tie plus a far smaller amount is no longer a tie.
        assert_eq!(
            canon(big.add(&d("0.5000000000000000000000000000001"), &RP2)),
            "12345678901234567890123456789013E0"
        );
        // Carry lengthens the coefficient and bumps the exponent.
        assert_eq!(
            canon(d("9.9999999999999999999999999999999").add(&d("5E-32"), &RP2)),
            "10000000000000000000000000000000E-30"
        );
        assert_eq!(
            canon(d("1E+999999").sub(&d("1E-999999"), &RP2)),
            "10000000000000000000000000000000E999968"
        );
        assert_eq!(canon(d("0E-999999999").add(&d("-0E5"), &RP2)), "0E-1000030");
        assert_eq!(
            canon(d("9E+999999").add(&d("9E+999999"), &RP2)),
            "!Overflow"
        );
    }

    #[test]
    fn huge_exponent_gaps_stay_cheap_and_correct() {
        let one = d("1");
        let tiny = d("1E-1999999999999999997");
        assert_eq!(
            canon(one.add(&tiny, &RP2)),
            "10000000000000000000000000000000E-31"
        );
        assert_eq!(
            canon(one.add(&tiny, &ctx(32, Rounding::Up))),
            "10000000000000000000000000000001E-31"
        );
        assert_eq!(
            canon(one.sub(&tiny, &RP2)),
            "10000000000000000000000000000000E-31"
        );
        assert_eq!(
            canon(one.sub(&tiny, &ctx(32, Rounding::Down))),
            "99999999999999999999999999999999E-32"
        );
        assert_eq!(
            canon(one.add(&d("0E-1999999999999999997"), &RP2)),
            "10000000000000000000000000000000E-31"
        );
        assert_eq!(
            canon(d("1E+999999999999999999").add(&one, &RP2)),
            "!Overflow"
        );
        assert_eq!(canon(tiny.quantize(-13, &RP2)), "0E-13");
        assert_eq!(
            canon(tiny.copy_negate().quantize(-13, &ctx(32, Rounding::Floor))),
            "-1E-13"
        );
        assert_eq!(canon(tiny.mul(&tiny, &RP2)), "0E-1000030");
        assert_eq!(canon(tiny.plus(&RP2)), "0E-1000030");
    }

    #[test]
    fn multiplies_with_exact_exponents_and_subnormals() {
        assert_eq!(canon(d("1.10").mul(&d("2.0"), &RP2)), "2200E-3");
        assert_eq!(canon(d("-0").mul(&d("5"), &RP2)), "-0E0");
        assert_eq!(canon(d("0E+999999").mul(&d("0E+999999"), &RP2)), "0E999999");
        assert_eq!(
            canon(d("1E-999999").mul(&d("1E-999999"), &RP2)),
            "0E-1000030"
        );
        assert_eq!(canon(d("1E-999999").mul(&d("1E-20"), &RP2)), "1E-1000019");
        assert_eq!(canon(d("1E999999").mul(&d("10"), &RP2)), "!Overflow");
        assert_eq!(
            canon(
                d("99999999999999999999999999999999")
                    .mul(&d("99999999999999999999999999999999"), &RP2)
            ),
            "99999999999999999999999999999998E32"
        );
    }

    #[test]
    fn divides_with_ideal_exponents_and_errors() {
        assert_eq!(
            canon(d("1").div(&d("3"), &RP2)),
            "33333333333333333333333333333333E-32"
        );
        assert_eq!(
            canon(d("2").div(&d("3"), &RP2)),
            "66666666666666666666666666666667E-32"
        );
        assert_eq!(canon(d("1").div(&d("4"), &RP2)), "25E-2");
        assert_eq!(canon(d("2.00").div(&d("2"), &RP2)), "100E-2");
        assert_eq!(canon(d("1E+10").div(&d("1"), &RP2)), "1E10");
        assert_eq!(
            canon(d("1000").div(&d("1"), &ctx(3, Rounding::HalfEven))),
            "100E1"
        );
        assert_eq!(canon(d("-0").div(&d("5"), &RP2)), "-0E0");
        assert_eq!(canon(d("0").div(&d("0"), &RP2)), "!DivisionUndefined");
        assert_eq!(canon(d("1").div(&d("-0"), &RP2)), "!DivisionByZero");
        assert_eq!(canon(d("1").div(&d("1E-999999"), &RP2)), "1E999999");
        assert_eq!(canon(d("10").div(&d("1E-999999"), &RP2)), "!Overflow");
    }

    #[test]
    fn quantizes_like_cpython() {
        let q = |s: &str, e: i64, c: &Context| canon(d(s).quantize(e, c));
        assert_eq!(q("1.23456789012345", -13, &RP2), "12345678901234E-13");
        assert_eq!(q("0.00000000000005", -13, &RP2), "0E-13");
        assert_eq!(q("0.00000000000015", -13, &RP2), "2E-13");
        assert_eq!(q("-0.00000000000001", -13, &RP2), "-0E-13");
        assert_eq!(
            q("0.00000000000005", -13, &ctx(32, Rounding::HalfUp)),
            "1E-13"
        );
        assert_eq!(
            q("9999999999999999999.9999999999999", -13, &RP2),
            "99999999999999999999999999999999E-13"
        );
        assert_eq!(
            q("9999999999999999999.99999999999995", -13, &RP2),
            "!InvalidOperation"
        );
        assert_eq!(q("1E19", -13, &RP2), "!InvalidOperation");
        assert_eq!(q("0", 1_000_000, &RP2), "!InvalidOperation");
        assert_eq!(q("0", -1_000_031, &RP2), "!InvalidOperation");
        assert_eq!(q("0", -1_000_030, &RP2), "0E-1000030");
        assert_eq!(q("1", 999_999, &RP2), "0E999999");
        assert_eq!(q("0.0001", 0, &ctx(32, Rounding::ZeroFiveUp)), "1E0");
        assert_eq!(q("-0.0001", 0, &ctx(32, Rounding::Floor)), "-1E0");
    }

    #[test]
    fn unary_operations_round_and_normalize_zero_signs() {
        assert_eq!(canon(d("0").neg(&RP2)), "0E0");
        assert_eq!(canon(d("-0").neg(&RP2)), "0E0");
        assert_eq!(canon(d("0").neg(&ctx(32, Rounding::Floor))), "-0E0");
        assert_eq!(canon(d("-0").neg(&ctx(32, Rounding::Floor))), "0E0");
        assert_eq!(canon(d("-0").abs(&ctx(32, Rounding::Floor))), "0E0");
        assert_eq!(canon(d("-0").plus(&RP2)), "0E0");
        assert_eq!(canon(d("-0").plus(&ctx(32, Rounding::Floor))), "-0E0");
        assert_eq!(canon(d("1.5").neg(&RP2)), "-15E-1");
        assert_eq!(
            canon(d("-1.234567890123456789012345678901234567").abs(&RP2)),
            "12345678901234567890123456789012E-31"
        );
        assert_eq!(canon(d("1E+1000000").neg(&RP2)), "!Overflow");
        assert_eq!(d("-0").copy_abs().to_canonical(), "0E0");
        assert_eq!(d("0").copy_negate().to_canonical(), "-0E0");
    }

    #[test]
    fn compares_values_exactly() {
        assert_eq!(d("0"), d("-0"));
        assert_eq!(d("0E+5"), d("-0E-5"));
        assert_eq!(d("1.0"), d("1.000"));
        assert_eq!(d("1E+5"), d("100000"));
        assert!(d("-1") < d("-0.5"));
        assert!(d("-0") < d("1E-1999999999999999997"));
        assert!(d("1E+999999999999999999") > d("999999999999999999999999"));
        assert!(d("0.1000000000000000000000000000000000001") > d("0.1"));
        assert!(!d("1.0").same_representation(&d("1")));
        assert_eq!(d("-0").signum(), Ordering::Equal);
        assert!(d("-0").is_sign_negative() && !d("-0").is_negative());
    }

    #[test]
    fn rp2_comparisons_quantize_the_difference() {
        let a = d("1.00000000000005");
        let b = d("1");
        let all = |x: &Decimal, y: &Decimal, c: &Context| {
            [
                x.rp2_eq(y, c),
                x.rp2_ne(y, c),
                x.rp2_gt(y, c),
                x.rp2_ge(y, c),
                x.rp2_lt(y, c),
                x.rp2_le(y, c),
            ]
        };
        // 0.5E-13 rounds to even (0): equal under RP2's context.
        assert_eq!(
            all(&a, &b, &RP2),
            [
                Ok(true),
                Ok(false),
                Ok(false),
                Ok(true),
                Ok(false),
                Ok(true)
            ]
        );
        // ROUND_HALF_UP turns the same tie into 1E-13: greater.
        let up = ctx(32, Rounding::HalfUp);
        assert_eq!(
            all(&a, &b, &up),
            [
                Ok(false),
                Ok(true),
                Ok(true),
                Ok(true),
                Ok(false),
                Ok(false)
            ]
        );
        // 1.5E-13 rounds to 2E-13; reversed it is less.
        let c = d("1.00000000000015");
        assert_eq!(
            all(&b, &c, &RP2),
            [
                Ok(false),
                Ok(true),
                Ok(false),
                Ok(false),
                Ok(true),
                Ok(true)
            ]
        );
        // A difference of 1E19 needs 33 digits at 13 places.
        let err = Err(DecimalError::InvalidOperation);
        assert_eq!(all(&d("1E19"), &d("0"), &RP2), [err; 6]);
        let ovf = Err(DecimalError::Overflow);
        assert_eq!(all(&d("9E+999999"), &d("-9E+999999"), &RP2), [ovf; 6]);
    }

    #[test]
    fn from_parts_enforces_cpython_range() {
        let one = || BigUint::from(1u32);
        assert!(Decimal::from_parts(true, BigUint::zero(), MIN_CONSTRUCTED_EXPONENT).is_ok());
        assert!(Decimal::from_parts(false, one(), MAX_CONSTRUCTED_ADJUSTED).is_ok());
        assert_eq!(
            Decimal::from_parts(false, BigUint::from(10u32), MAX_CONSTRUCTED_ADJUSTED).map(|_| ()),
            Err(DecimalError::InvalidOperation)
        );
        assert_eq!(
            Decimal::from_parts(false, one(), MIN_CONSTRUCTED_EXPONENT - 1).map(|_| ()),
            Err(DecimalError::InvalidOperation)
        );
        assert_eq!(
            Decimal::from_parts(false, BigUint::zero(), i64::MAX).map(|_| ()),
            Err(DecimalError::InvalidOperation)
        );
    }

    #[test]
    fn counts_digits_around_powers_of_ten() {
        for k in 1..=120usize {
            let p = pow10(k);
            assert_eq!(ndigits(&p), k + 1, "10^{k}");
            assert_eq!(ndigits(&(&p - 1u32)), k, "10^{k} - 1");
            assert_eq!(ndigits(&(&p + 1u32)), k + 1, "10^{k} + 1");
        }
        assert_eq!(ndigits(&BigUint::zero()), 1);
        assert_eq!(ndigits(&BigUint::from(u64::MAX)), 20);
        assert_eq!(ndigits(&(BigUint::from(u64::MAX) + 1u32)), 20);
    }

    #[test]
    fn invalid_context_is_an_error() {
        let bad = ctx(0, Rounding::HalfEven);
        assert_eq!(d("1").add(&d("1"), &bad), Err(DecimalError::InvalidContext));
        assert_eq!(d("1").quantize(0, &bad), Err(DecimalError::InvalidContext));
        assert_eq!(d("1").neg(&bad), Err(DecimalError::InvalidContext));
        // A huge precision must fail before it sizes any intermediate.
        let huge = ctx(u32::MAX, Rounding::HalfEven);
        assert_eq!(
            d("1").div(&d("1"), &huge),
            Err(DecimalError::InvalidContext)
        );
        let max = ctx(MAX_PRECISION, Rounding::HalfEven);
        assert_eq!(d("1").div(&d("1"), &max), Ok(d("1")));
    }

    #[test]
    fn precision_one_and_long_inputs_round_correctly() {
        let one = ctx(1, Rounding::HalfEven);
        assert_eq!(canon(d("9.5").plus(&one)), "1E1");
        assert_eq!(canon(d("8.5").plus(&one)), "8E0");
        assert_eq!(canon(d("0.95").add(&d("0"), &one)), "1E0");
        let long = d("1234567890123456789012345678901234567890");
        assert_eq!(
            canon(long.mul(&d("1"), &RP2)),
            "12345678901234567890123456789012E8"
        );
        assert_eq!(
            canon(long.div(&d("1"), &RP2)),
            "12345678901234567890123456789012E8"
        );
        assert_eq!(
            canon(d("1").div(&long, &RP2)),
            "81000000729000006633900060368491E-71"
        );
    }
}
