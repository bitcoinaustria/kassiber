//! `review_workflow._tax_delta`: after minus before for every listed tax year
//! that changes. Apply recomputes it from an artifact's own `before` and
//! `after`, which are untrusted, so this follows Python's evaluation on any
//! JSON value: what raises `KeyError`, `TypeError`, `ValueError` or an
//! `ArithmeticError` there is [`Fault::Malformed`]; an `AttributeError`
//! escapes as [`Fault::Internal`].
use crate::review_artifact::equal;
use kassiber_envelope::{dict, get, s, set, truthy, type_name};
use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_pyjson::{BigInt, Key, PyStr, Value};
use std::cmp::Ordering;

#[derive(Debug, PartialEq)]
pub enum Fault {
    Malformed,
    Internal(String),
}
type Result<T> = std::result::Result<T, Fault>;

const CONTEXT: Context = Context::new(32, Rounding::HalfEven);
const CENTS: Context = Context::new(32, Rounding::HalfUp);

fn subscript<'a>(value: &'a Value, key: &str) -> Result<&'a Value> {
    match value {
        Value::Dict(_) => get(value, key).ok_or(Fault::Malformed),
        _ => Err(Fault::Malformed),
    }
}

fn key_value(key: &Key) -> Value {
    match key {
        Key::Null => Value::Null,
        Key::Bool(v) => Value::Bool(*v),
        Key::Int(v) => Value::Int(v.clone()),
        Key::Float(v) => Value::Float(*v),
        Key::Str(v) => Value::Str(v.clone()),
    }
}

/// `iter(value)`: a dict's keys, a list's items, a string's characters.
fn iterate(value: &Value) -> Result<Vec<Value>> {
    match value {
        Value::List(items) => Ok(items.clone()),
        Value::Dict(fields) => Ok(fields.iter().map(|(key, _)| key_value(key)).collect()),
        Value::Str(text) => Ok(text
            .code_points()
            .map(|point| Value::Str(PyStr::from_code_points([point]).expect("a code point")))
            .collect()),
        _ => Err(Fault::Malformed),
    }
}

/// Hash equality: `1 == 1.0 == True`; lists and dicts are unhashable.
fn hashable(value: &Value) -> Result<()> {
    match value {
        Value::List(_) | Value::Dict(_) => Err(Fault::Malformed),
        _ => Ok(()),
    }
}

/// A mapping with Python's key semantics: the first key object stays, the
/// last value wins.
#[derive(Default)]
struct Mapping(Vec<(Value, Value)>);
impl Mapping {
    fn insert(&mut self, key: Value, item: Value) -> Result<()> {
        hashable(&key)?;
        match self.0.iter_mut().find(|(known, _)| equal(known, &key)) {
            Some((_, slot)) => *slot = item,
            None => self.0.push((key, item)),
        }
        Ok(())
    }
    fn get(&self, key: &Value) -> Option<&Value> {
        self.0
            .iter()
            .find(|(known, _)| equal(known, key))
            .map(|(_, item)| item)
    }
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Float(v) => Some(*v),
        _ => None,
    }
}
fn integer(value: &Value) -> Option<BigInt> {
    match value {
        Value::Int(v) => Some(v.clone()),
        Value::Bool(v) => Some(u8::from(*v).into()),
        _ => None,
    }
}

/// Exact comparison of two Python numbers.
fn compare_numbers(a: &Value, b: &Value) -> Ordering {
    match (integer(a), integer(b), number(a), number(b)) {
        (Some(a), Some(b), _, _) => a.cmp(&b),
        (_, _, Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
        (Some(a), None, None, Some(b)) => {
            compare_numbers(&Value::Float(b), &Value::Int(a)).reverse()
        }
        (None, Some(b), Some(a), None) => {
            let floor = a.floor();
            let whole: BigInt = format!("{floor:.0}").parse().expect("a finite float");
            whole.cmp(&b).then(if a > floor {
                Ordering::Greater
            } else {
                Ordering::Equal
            })
        }
        _ => Ordering::Equal,
    }
}

/// `sorted(values)`: numbers with numbers, strings with strings; any other
/// pair raises once two values must be ordered.
fn sorted(mut values: Vec<Value>) -> Result<Vec<Value>> {
    let numeric = |v: &Value| matches!(v, Value::Int(_) | Value::Bool(_) | Value::Float(_));
    if values.len() > 1
        && !values.iter().all(numeric)
        && !values.iter().all(|v| matches!(v, Value::Str(_)))
    {
        return Err(Fault::Malformed);
    }
    values.sort_by(|a, b| match (a, b) {
        (Value::Str(a), Value::Str(b)) => a.cmp(b),
        _ => compare_numbers(a, b),
    });
    Ok(values)
}

/// `int(value)` as `sorted(..., key=int)` calls it.
fn int(value: &Value) -> Result<BigInt> {
    match value {
        Value::Int(_) | Value::Bool(_) => Ok(integer(value).expect("an integer")),
        Value::Float(v) => format!("{:.0}", v.trunc())
            .parse()
            .map_err(|_| Fault::Malformed),
        Value::Str(text) => text
            .as_str()
            .and_then(|text| kassiber_pytext::parse_int(text).ok())
            .ok_or(Fault::Malformed),
        _ => Err(Fault::Malformed),
    }
}

/// `a - b` for JSON numbers; anything else raises `TypeError`.
fn subtract(a: &Value, b: &Value) -> Result<Value> {
    if let (Some(a), Some(b)) = (integer(a), integer(b)) {
        return Ok(Value::Int(a - b));
    }
    let float = |v: &Value| match v {
        Value::Float(v) => Ok(*v),
        _ => integer(v)
            .ok_or(Fault::Malformed)
            .and_then(|v| v.to_string().parse::<f64>().map_err(|_| Fault::Malformed))
            .and_then(|v| {
                if v.is_finite() {
                    Ok(v)
                } else {
                    Err(Fault::Malformed)
                }
            }),
    };
    if matches!(a, Value::Float(_)) || matches!(b, Value::Float(_)) {
        return Ok(Value::Float(float(a)? - float(b)?));
    }
    Err(Fault::Malformed)
}

/// A `Decimal` as the delta can meet one: finite, a quiet NaN (which
/// propagates without a signal) or an infinity (which every quantize refuses).
enum Number {
    Finite(Decimal),
    NaN { negative: bool, payload: String },
    Infinite,
}

fn exact_float(value: f64) -> Result<Decimal> {
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if value == 0.0 {
        return format!("{sign}0").parse().map_err(|_| Fault::Malformed);
    }
    let bits = value.to_bits();
    let raw_exponent = ((bits >> 52) & 0x7ff) as i64;
    let fraction = bits & ((1 << 52) - 1);
    let (mut mantissa, mut exponent) = if raw_exponent == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), raw_exponent - 1075)
    };
    while mantissa % 2 == 0 && exponent < 0 {
        mantissa /= 2;
        exponent += 1;
    }
    let (coefficient, exponent) = if exponent >= 0 {
        (BigInt::from(mantissa) << exponent as usize, 0)
    } else {
        let places = u32::try_from(-exponent).expect("a float exponent");
        (
            BigInt::from(mantissa) * BigInt::from(5u8).pow(places),
            exponent,
        )
    };
    format!("{sign}{coefficient}E{exponent}")
        .parse()
        .map_err(|_| Fault::Malformed)
}

/// `Decimal((sign, digits, exponent))`, which also takes a list.
fn tuple_form(items: &[Value]) -> Result<Number> {
    let [sign, digits, exponent] = items else {
        return Err(Fault::Malformed);
    };
    let negative = match integer(sign) {
        Some(sign) if sign == 0.into() => false,
        Some(sign) if sign == 1.into() => true,
        _ => return Err(Fault::Malformed),
    };
    let Value::List(digits) = digits else {
        return Err(Fault::Malformed);
    };
    let mut text = String::new();
    for digit in digits {
        match integer(digit) {
            Some(digit) if digit >= 0.into() && digit <= 9.into() => {
                text.push_str(&digit.to_string())
            }
            _ => return Err(Fault::Malformed),
        }
    }
    match exponent {
        Value::Str(marker) if marker.is("F") => Ok(Number::Infinite),
        Value::Str(marker) if marker.is("n") => Ok(Number::NaN {
            negative,
            payload: text.trim_start_matches('0').to_owned(),
        }),
        Value::Int(_) | Value::Bool(_) => {
            let exponent = integer(exponent).expect("an integer");
            let digits = if text.is_empty() { "0" } else { text.as_str() };
            let sign = if negative { "-" } else { "" };
            format!("{sign}{digits}E{exponent}")
                .parse()
                .map(Number::Finite)
                .map_err(|_| Fault::Malformed)
        }
        // A signalling NaN ('N') raises on the subtraction.
        _ => Err(Fault::Malformed),
    }
}

/// `Decimal(value)`.
fn decimal(value: &Value) -> Result<Number> {
    match value {
        Value::List(items) => tuple_form(items),
        Value::Int(_) | Value::Bool(_) => integer(value)
            .expect("an integer")
            .to_string()
            .parse()
            .map(Number::Finite)
            .map_err(|_| Fault::Malformed),
        Value::Float(v) => exact_float(*v).map(Number::Finite),
        Value::Str(text) => {
            let text = text.as_str().ok_or(Fault::Malformed)?;
            let text = kassiber_pytext::strip(text);
            if let Ok(number) = text.parse::<Decimal>() {
                return Ok(Number::Finite(number));
            }
            let lower = text.to_ascii_lowercase();
            let (negative, body) = match lower.as_bytes().first() {
                Some(b'-') => (true, &lower[1..]),
                Some(b'+') => (false, &lower[1..]),
                _ => (false, lower.as_str()),
            };
            if body == "inf" || body == "infinity" {
                return Ok(Number::Infinite);
            }
            // A signalling NaN raises on the subtraction, so refuse it here.
            match body.strip_prefix("nan") {
                Some(digits) if digits.bytes().all(|b| b.is_ascii_digit()) => Ok(Number::NaN {
                    negative,
                    payload: digits.trim_start_matches('0').to_owned(),
                }),
                _ => Err(Fault::Malformed),
            }
        }
        _ => Err(Fault::Malformed),
    }
}

/// `_money(Decimal(new) - Decimal(old))`.
fn money_difference(new: &Value, old: &Value) -> Result<String> {
    let (new, old) = (decimal(new)?, decimal(old)?);
    match (new, old) {
        (Number::NaN { negative, payload }, _) | (_, Number::NaN { negative, payload }) => {
            // `_fix_nan` keeps the payload's last 32 digits.
            let start = payload.len().saturating_sub(32);
            let payload = payload[start..].trim_start_matches('0');
            Ok(format!("{}NaN{payload}", if negative { "-" } else { "" }))
        }
        (Number::Finite(new), Number::Finite(old)) => {
            let difference = new.sub(&old, &CONTEXT).map_err(|_| Fault::Malformed)?;
            money(&difference)
        }
        _ => Err(Fault::Malformed),
    }
}

/// `_money`: an exact cent string, half up, without a negative zero.
pub fn money(value: &Decimal) -> Result<String> {
    let rounded = value.quantize(-2, &CENTS).map_err(|_| Fault::Malformed)?;
    if rounded.is_zero() {
        return Ok("0.00".into());
    }
    rounded.to_fixed_string().map_err(|_| Fault::Malformed)
}

fn attribute(value: &Value) -> Fault {
    Fault::Internal(format!(
        "'{}' object has no attribute 'get'",
        type_name(value)
    ))
}

/// `mapping.get(key, default)` where `mapping` must be a dict.
fn lookup(mapping: &Value, key: &Value, default: Value) -> Result<Value> {
    let Value::Dict(fields) = mapping else {
        return Err(attribute(mapping));
    };
    Ok(fields
        .iter()
        .find(|(known, _)| equal(&key_value(known), key))
        .map_or(default, |(_, item)| item.clone()))
}

fn years(effects: &Value) -> Result<Mapping> {
    let mut years = Mapping::default();
    for item in iterate(subscript(subscript(effects, "tax")?, "years")?)? {
        let year = subscript(&item, "year")?.clone();
        years.insert(year, item)?;
    }
    Ok(years)
}

fn kennzahl_delta(a: &Value, b: &Value, item: &mut Value) -> Result<bool> {
    let empty = dict::<&str>([]);
    let old = get(a, "kennzahlen").unwrap_or(&empty);
    let new = get(b, "kennzahlen").unwrap_or(&empty);
    let mut codes: Vec<Value> = Vec::new();
    for code in iterate(old)?.into_iter().chain(iterate(new)?) {
        hashable(&code)?;
        if !codes.iter().any(|known| equal(known, &code)) {
            codes.push(code);
        }
    }
    let mut keyed = codes
        .into_iter()
        .map(|code| Ok((int(&code)?, code)))
        .collect::<Result<Vec<_>>>()?;
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    let mut changes = Vec::new();
    for (_, code) in keyed {
        let after = lookup(new, &code, Value::Int(0.into()))?;
        let before = lookup(old, &code, Value::Int(0.into()))?;
        let change = subtract(&after, &before)?;
        if truthy(&change) {
            changes.push((code, change));
        }
    }
    let changed = !changes.is_empty();
    let mut kennzahlen = Value::Dict(Vec::new());
    if let Value::Dict(fields) = &mut kennzahlen {
        for (code, change) in changes {
            fields.push((value_key(code), change));
        }
    }
    set(item, "kennzahlen", kennzahlen);
    let old_tax = get(a, "indicative_tax")
        .filter(|v| truthy(v))
        .cloned()
        .unwrap_or_else(|| empty.clone());
    let new_tax = get(b, "indicative_tax")
        .filter(|v| truthy(v))
        .cloned()
        .unwrap_or_else(|| empty.clone());
    let field = s("estimated_tax_eur_cents");
    let old_cents = lookup(&old_tax, &field, Value::Null)?;
    let new_cents = lookup(&new_tax, &field, Value::Null)?;
    // Withheld on either side means no figure to compare, not zero.
    let estimate = if matches!(old_cents, Value::Null) || matches!(new_cents, Value::Null) {
        Value::Null
    } else {
        subtract(&new_cents, &old_cents)?
    };
    let estimate_changed = truthy(&estimate);
    set(item, "estimated_tax_eur_cents", estimate);
    let status = s("status");
    set(
        item,
        "indicative_status",
        lookup(&new_tax, &status, s("indicative"))?,
    );
    Ok(changed
        || estimate_changed
        || !equal(
            &lookup(&old_tax, &status, Value::Null)?,
            &lookup(&new_tax, &status, Value::Null)?,
        ))
}

fn value_key(value: Value) -> Key {
    match value {
        Value::Null => Key::Null,
        Value::Bool(v) => Key::Bool(v),
        Value::Int(v) => Key::Int(v),
        Value::Float(v) => Key::Float(v),
        Value::Str(v) => Key::Str(v),
        _ => unreachable!("hashable keys"),
    }
}

/// After minus before for every listed tax year that changes.
pub fn tax_delta(before: &Value, after: &Value) -> Result<Value> {
    let old = years(before)?;
    let new = years(after)?;
    let mut union: Vec<Value> = old.0.iter().map(|(year, _)| year.clone()).collect();
    for (year, _) in &new.0 {
        if !union.iter().any(|known| equal(known, year)) {
            union.push(year.clone());
        }
    }
    let empty = dict::<&str>([]);
    let mut result = Vec::new();
    for year in sorted(union)? {
        let a = old.get(&year).unwrap_or(&empty);
        let b = new.get(&year).unwrap_or(&empty);
        let mut item = dict([("year", year.clone())]);
        let mut changed = false;
        for key in ["realized_gain_loss", "income"] {
            let zero = s("0");
            let difference =
                money_difference(get(b, key).unwrap_or(&zero), get(a, key).unwrap_or(&zero))?;
            changed |= difference != "0.00";
            set(&mut item, key, s(difference));
        }
        if get(a, "kennzahlen").is_some() || get(b, "kennzahlen").is_some() {
            changed = kennzahl_delta(a, b, &mut item)? || changed;
        }
        if changed {
            result.push(item);
        }
    }
    Ok(dict([("years", Value::List(result))]))
}
