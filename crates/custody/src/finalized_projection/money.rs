//! Decimal(str(value)) and the two context-rounded divisions in msat pricing.
//! Never combine these divisions: rounding the BTC denominator is observable.
use super::*;
use crate::python_text::{decimal_digit, PythonText};
use kassiber_pydecimal::{Context, Decimal, DecimalError};

fn error(e: DecimalError) -> String {
    match e {
        DecimalError::ConversionSyntax | DecimalError::DivisionUndefined => "InvalidOperation",
        _ => e.name(),
    }
    .into()
}

pub(super) fn empty(v: &Value) -> bool {
    v.is_null() || v == ""
}

#[derive(Clone)]
pub(super) enum Money {
    Finite(Decimal),
    Special(String),
}
impl Money {
    pub(super) fn parse(v: &Value) -> Result<Self, String> {
        let raw = string(v);
        let s = raw.py_strip();
        match Decimal::from_str(s) {
            Ok(d) => Ok(Self::Finite(d)),
            Err(DecimalError::NonFinite) => {
                let s: String = s
                    .chars()
                    .filter(|&c| c != '_')
                    .map(|c| decimal_digit(c).map(|d| char::from(b'0' + d)).unwrap_or(c))
                    .collect();
                let negative = s.starts_with('-');
                let unsigned = s.trim_start_matches(['+', '-']).to_ascii_lowercase();
                let name = if unsigned == "inf" || unsigned == "infinity" {
                    "Infinity".into()
                } else if let Some(payload) = unsigned.strip_prefix("snan") {
                    format!("sNaN{}", payload.trim_start_matches('0'))
                } else {
                    format!(
                        "NaN{}",
                        unsigned
                            .strip_prefix("nan")
                            .unwrap()
                            .trim_start_matches('0')
                    )
                };
                Ok(Self::Special(format!(
                    "{}{name}",
                    if negative { "-" } else { "" }
                )))
            }
            Err(e) => Err(error(e)),
        }
    }
    // finite_decimal rejects parse errors, Decimal specials, and legacy float overflow.
    pub(super) fn finite(v: &Value) -> Result<Self, String> {
        let value = Self::parse(v).map_err(|_| "AppError".to_owned())?;
        value.require_finite()?;
        Ok(match value {
            Self::Finite(d) if d.is_zero() => Self::Finite(Decimal::zero()),
            value => value,
        })
    }
    pub(super) fn require_finite(&self) -> Result<(), String> {
        match self {
            Self::Finite(d)
                if d.to_string().parse::<f64>().is_ok_and(f64::is_finite)
                    && (d.is_zero()
                        || (d.exponent().unsigned_abs() <= 308
                            && d.copy_abs() >= Decimal::from_str("1e-13").unwrap())) =>
            {
                Ok(())
            }
            _ => Err("AppError".into()),
        }
    }
    pub(super) fn positive(&self) -> Result<bool, String> {
        match self {
            Self::Finite(d) => Ok(d.is_positive()),
            Self::Special(s) if s.contains("NaN") => Err("InvalidOperation".into()),
            Self::Special(s) => Ok(!s.starts_with('-')),
        }
    }
    pub(super) fn fixed(&self) -> Result<String, String> {
        match self {
            Self::Finite(d) => d.to_fixed_string().map_err(error),
            Self::Special(s) => Ok(s.clone()),
        }
    }
    pub(super) fn float(&self) -> Result<Value, String> {
        let s = self.fixed()?;
        if s.contains("sNaN") {
            return Err("ValueError".into());
        }
        let f = if s.contains("NaN") {
            f64::NAN
        } else {
            s.parse::<f64>().map_err(|_| "ValueError".to_owned())?
        };
        Ok(if f.is_finite() {
            json!(f)
        } else {
            json!({"$float":if f.is_nan() {"nan"} else if f.is_sign_negative() {"-inf"} else {"inf"}})
        })
    }
    pub(super) fn rate(&self, msat: &BigInt) -> Result<Self, String> {
        let amount = Decimal::from_str(&msat.to_string()).map_err(error)?;
        let btc = amount
            .div(&Decimal::from_str("100000000000").unwrap(), &Context::RP2)
            .map_err(error)?;
        match self {
            Self::Finite(d) => Ok(Self::Finite(d.div(&btc, &Context::RP2).map_err(error)?)),
            Self::Special(s) if s.contains("sNaN") => Err("InvalidOperation".into()),
            Self::Special(s) => Ok(Self::Special(s.clone())),
        }
    }
}
