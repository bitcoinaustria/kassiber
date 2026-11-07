//! Decimal(str(value)) and the two context-rounded divisions in msat pricing.
//! Never combine these divisions: rounding the BTC denominator is observable.
use super::*;
use crate::python_text::{decimal_digit, PythonText};
use kassiber_pydecimal::{Context, Decimal, DecimalError};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finalized_price_errors_keep_field_code_and_message() {
        for (raw, reason) in [
            ("1e999", "must be a finite decimal number"),
            ("-1e999", "must be a finite decimal number"),
            ("\"legacy scalar\"", "must be a decimal number"),
            ("\"1e-14\"", "is outside the supported decimal range"),
        ] {
            let value: Value = serde_json::from_str(raw).unwrap();
            let error = Money::finite_field(&value, "transaction sample: fiat_rate")
                .err()
                .unwrap();
            assert_eq!(error.code, "validation");
            assert_eq!(
                error.message,
                format!("transaction sample: fiat_rate {reason}")
            );
        }
    }

    #[test]
    fn finalized_prices_keep_exact_exponent_bound() {
        for sign in ["", "-"] {
            let accepted = json!(format!("{sign}1.{}", "0".repeat(308)));
            let refused = json!(format!("{sign}1.{}", "0".repeat(309)));
            assert!(Money::finite(&accepted).is_ok());
            assert_eq!(Money::finite(&refused).err().as_deref(), Some("AppError"));
        }
        for zero in ["0e-999", "-0e-999", "0e999"] {
            assert_eq!(Money::finite(&json!(zero)).unwrap().fixed().unwrap(), "0");
        }
    }

    #[test]
    fn finalized_plain_prices_are_admitted() {
        for raw in [
            "\"47454\"",
            "\"-0\"",
            "\"0.0000000000001\"",
            "\"958.09436184\"",
            "12",
            "-3",
            "\"00012.5\"",
            "47454.0",
            "1.5",
            "1e-12",
            "-0.0",
            "1e300",
        ] {
            let value: Value = serde_json::from_str(raw).unwrap();
            assert!(plain_finite(&value), "{raw}");
            assert!(Money::finite_field(&value, "field").is_ok(), "{raw}");
        }
        for raw in [
            "\"0.00000000000001\"",
            "\"1e5\"",
            "\" 1\"",
            "\"1.\"",
            "\".5\"",
            "\"1_0\"",
            "\"\"",
            "true",
            "\"\\u0661\"",
            "1e-13",
            "1e301",
            "18446744073709551616",
        ] {
            assert!(!plain_finite(&serde_json::from_str(raw).unwrap()), "{raw}");
        }
    }

    #[test]
    fn finalized_prices_keep_exact_precision_floor() {
        for accepted in ["1e-13", "-1e-13", "0", "-0"] {
            assert!(Money::finite(&json!(accepted)).is_ok(), "{accepted}");
        }
        for refused in [
            "1e-14",
            "-1e-14",
            "0.000000000000099999999999999999999999999999999",
            "-0.000000000000099999999999999999999999999999999",
        ] {
            assert_eq!(
                Money::finite(&json!(refused)).err().as_deref(),
                Some("AppError"),
                "{refused}"
            );
        }
    }
}

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

/// Whether `Money::finite_field` admits `v` without parsing it: a plain
/// decimal spelling with at most 300 integer digits and 13 fraction digits
/// is finite, a finite float, and zero or within the supported range.
pub(super) fn plain_finite(v: &Value) -> bool {
    let spelling = match v {
        Value::String(s) => s.as_str(),
        Value::Number(n) => {
            // str() of an integer is its digits. A float's repr is finite,
            // and within these bounds its exponent and magnitude are in range.
            if n.as_i64().is_some() {
                return true;
            }
            let spelling = n.as_str();
            if !spelling.contains(['.', 'e', 'E']) {
                return false;
            }
            return spelling
                .parse::<f64>()
                .is_ok_and(|f| f == 0.0 || (1e-12..=1e300).contains(&f.abs()));
        }
        _ => return false,
    };
    let unsigned = spelling.strip_prefix('-').unwrap_or(spelling);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    !whole.is_empty()
        && whole.len() <= 300
        && whole.bytes().all(|b| b.is_ascii_digit())
        && (fraction.is_empty() && !unsigned.ends_with('.')
            || !fraction.is_empty()
                && fraction.len() <= 13
                && fraction.bytes().all(|b| b.is_ascii_digit()))
}

#[derive(Clone)]
pub(crate) enum Money {
    Finite(Decimal),
    Special(String),
}
impl Money {
    pub(super) fn finite_field(v: &Value, field: &str) -> Result<Self, super::Error> {
        let refused = |reason| super::Error {
            code: "validation".into(),
            message: format!("{field} {reason}"),
        };
        let value = Self::parse(v).map_err(|_| refused("must be a decimal number"))?;
        let Self::Finite(number) = &value else {
            return Err(refused("must be a finite decimal number"));
        };
        if !number.to_string().parse::<f64>().is_ok_and(f64::is_finite) {
            return Err(refused("must be a finite decimal number"));
        }
        if !number.is_zero()
            && (number.exponent().unsigned_abs() > 308
                || number.copy_abs() < Decimal::from_str("1e-13").unwrap())
        {
            return Err(refused("is outside the supported decimal range"));
        }
        Ok(if number.is_zero() {
            Self::Finite(Decimal::zero())
        } else {
            value
        })
    }

    pub(crate) fn parse(v: &Value) -> Result<Self, String> {
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
    pub(crate) fn positive(&self) -> Result<bool, String> {
        match self {
            Self::Finite(d) => Ok(d.is_positive()),
            Self::Special(s) if s.contains("NaN") => Err("InvalidOperation".into()),
            Self::Special(s) => Ok(!s.starts_with('-')),
        }
    }
    pub(crate) fn fixed(&self) -> Result<String, String> {
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
            Self::Finite(d) => {
                let rate = d.div(&btc, &Context::RP2).map_err(error)?;
                // Python finite_decimal canonicalizes the derived rate too.
                Ok(Self::Finite(if rate.is_zero() {
                    Decimal::zero()
                } else {
                    rate
                }))
            }
            Self::Special(s) if s.contains("sNaN") => Err("InvalidOperation".into()),
            Self::Special(s) => Ok(Self::Special(s.clone())),
        }
    }
}
