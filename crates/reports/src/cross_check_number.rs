//! The reference parser admits Decimal specials; finite arithmetic stays shared.
use super::*;

#[derive(Clone)]
pub(super) enum Number {
    Finite(Decimal),
    Nan(String),
    Signaling,
    Infinity(bool),
}
fn invalid() -> AppError {
    AppError::new("internal_error", "[<class 'decimal.InvalidOperation'>]")
}
impl From<Decimal> for Number {
    fn from(value: Decimal) -> Self {
        Self::Finite(value)
    }
}
impl std::str::FromStr for Number {
    type Err = kassiber_pydecimal::DecimalError;
    fn from_str(raw: &str) -> std::result::Result<Self, Self::Err> {
        match raw.parse::<Decimal>() {
            Ok(n) => Ok(Self::Finite(n)),
            Err(kassiber_pydecimal::DecimalError::NonFinite) => {
                let normalized = kassiber_pytext::strip(raw)
                    .chars()
                    .filter(|c| *c != '_')
                    .map(|c| {
                        kassiber_pytext::decimal_value(c as u32)
                            .map(|n| char::from(b'0' + n as u8))
                            .unwrap_or(c)
                    })
                    .collect::<String>()
                    .to_ascii_lowercase();
                let unsigned = normalized.trim_start_matches(['+', '-']);
                Ok(if unsigned.starts_with("snan") {
                    Self::Signaling
                } else if let Some(payload) = unsigned.strip_prefix("nan") {
                    Self::Nan(format!(
                        "{}NaN{}",
                        if normalized.starts_with('-') { "-" } else { "" },
                        payload.trim_start_matches('0')
                    ))
                } else {
                    Self::Infinity(normalized.starts_with('-'))
                })
            }
            Err(e) => Err(e),
        }
    }
}
impl Number {
    pub fn zero() -> Self {
        Decimal::zero().into()
    }
    pub fn from_value(value: &Value) -> Result<Self> {
        if matches!(value, Value::Null) || value == &s("") {
            return Ok(Self::zero());
        }
        let raw = py_str(value);
        raw.parse()
            .map_err(|_| AppError::new("app_error", format!("Invalid decimal value: {raw}")))
    }
    pub fn is_zero(&self) -> bool {
        matches!(self, Self::Finite(n) if n.is_zero())
    }
    pub fn abs(&self) -> Result<Self> {
        Ok(match self {
            Self::Finite(n) => n.abs(&DECIMAL).map_err(arithmetic)?.into(),
            Self::Signaling => return Err(invalid()),
            Self::Infinity(_) => Self::Infinity(false),
            Self::Nan(n) => Self::Nan(n.clone()),
        })
    }
    pub fn add(&self, other: &Self) -> Result<Self> {
        Ok(match (self, other) {
            (Self::Signaling, _) | (_, Self::Signaling) => return Err(invalid()),
            (Self::Nan(n), _) | (_, Self::Nan(n)) => Self::Nan(n.clone()),
            (Self::Infinity(a), Self::Infinity(b)) if a != b => return Err(invalid()),
            (Self::Infinity(a), _) | (_, Self::Infinity(a)) => Self::Infinity(*a),
            (Self::Finite(a), Self::Finite(b)) => a.add(b, &DECIMAL).map_err(arithmetic)?.into(),
        })
    }
    pub fn sub(&self, other: &Self) -> Result<Self> {
        let negated = match other {
            Self::Finite(n) => Self::Finite(n.neg(&DECIMAL).map_err(arithmetic)?),
            Self::Infinity(sign) => Self::Infinity(!sign),
            n => n.clone(),
        };
        self.add(&negated)
    }
    pub fn greater(&self, other: &Self) -> Result<bool> {
        Ok(match (self, other) {
            (Self::Nan(_) | Self::Signaling, _) | (_, Self::Nan(_) | Self::Signaling) => {
                return Err(invalid())
            }
            (Self::Infinity(a), Self::Infinity(b)) => !a && *b,
            (Self::Infinity(a), _) => !a,
            (_, Self::Infinity(b)) => *b,
            (Self::Finite(a), Self::Finite(b)) => a > b,
        })
    }
    pub fn tolerance(&self) -> Result<Self> {
        match self.abs()? {
            Self::Finite(n) => Ok(n
                .mul(&"0.01".parse().unwrap(), &DECIMAL)
                .map_err(arithmetic)?
                .max(Decimal::from(1))
                .into()),
            Self::Nan(_) => Err(invalid()),
            n => Ok(n),
        }
    }
    pub fn money(&self) -> Result<Value> {
        match self {
            Self::Finite(n) => n
                .quantize(-2, &DECIMAL)
                .map_err(arithmetic)?
                .to_fixed_string()
                .map(s)
                .map_err(arithmetic),
            Self::Nan(n) => Ok(s(n.as_str())),
            _ => Err(invalid()),
        }
    }
    pub fn msat(&self) -> Result<kassiber_pyjson::BigInt> {
        match self {
            Self::Finite(n) => super::cross_check::msat(n),
            Self::Nan(_) => Err(AppError::new(
                "internal_error",
                "cannot convert NaN to integer",
            )),
            Self::Signaling => Err(invalid()),
            Self::Infinity(_) => Err(AppError::new(
                "internal_error",
                "cannot convert Infinity to integer",
            )),
        }
    }
}
