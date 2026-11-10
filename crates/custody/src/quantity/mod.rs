//! Exact custody primitives. Runtime stage admission remains separate: these
//! comparison-only artifacts never write or grant accounting authority.
pub mod allocation;
mod arbitration;
pub(crate) use arbitration::DecisionRef;
pub use arbitration::PostingView;
pub mod balances;
pub mod hashes;
pub mod holds;
pub mod native;
pub mod reviewed;
pub mod runtime;
pub mod transfers;

use crate::{
    codec,
    evidence::canonical::{EventRef, LegRef, Obs},
    python_text::PythonText,
};
pub use arbitration::project;
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::str::FromStr;

pub(super) fn array(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}

/// The canonical input: typed from evidence, or a JSON operand.
#[derive(Clone, Copy)]
pub(super) enum Canon<'a> {
    Typed(&'a crate::evidence::canonical::Canonical),
    Json(&'a Value),
}
impl<'a> Canon<'a> {
    pub fn events(self) -> Vec<EventRef<'a>> {
        match self {
            Self::Typed(c) => c.events().iter().map(EventRef::Typed).collect(),
            Self::Json(v) => crate::evidence::canonical::json_events(v),
        }
    }
    pub fn rejected(self) -> &'a [Value] {
        match self {
            Self::Typed(c) => c.rejected(),
            Self::Json(v) => array(&v["rejected_events"]),
        }
    }
    /// The JSON object; a typed input builds it on first request.
    pub fn json(self) -> &'a Value {
        match self {
            Self::Typed(c) => c.value(),
            Self::Json(v) => v,
        }
    }
}

/// A quantity event: a canonical event, or one native augmentation rewrote.
pub(super) enum QEvent<'a> {
    Ref(EventRef<'a>),
    Owned(Value),
}
impl QEvent<'_> {
    pub fn get(&self) -> EventRef<'_> {
        match self {
            Self::Ref(event) => *event,
            Self::Owned(event) => EventRef::Json(event),
        }
    }
}
pub(super) fn number(n: &BigInt) -> Value {
    if let Ok(n) = i64::try_from(n) {
        return Value::from(n);
    }
    if let Ok(n) = u64::try_from(n) {
        return Value::from(n);
    }
    serde_json::from_str(&n.to_string()).expect("computed integer")
}
pub(super) fn exact(v: &Value) -> Result<BigInt, String> {
    if let Some(n) = v.as_i64() {
        return Ok(BigInt::from(n));
    }
    if let Some(n) = v.as_u64() {
        return Ok(BigInt::from(n));
    }
    if v.as_number()
        .is_none_or(|n| n.as_str().contains(['.', 'e', 'E']))
    {
        return Err("ValueError".into());
    }
    BigInt::from_str(&v.to_string()).map_err(|_| "ValueError".into())
}
pub fn principal_msat<O: Obs + ?Sized>(v: &O) -> Result<Value, String> {
    principal(v).map(|n| number(&n))
}

pub(super) fn principal<O: Obs + ?Sized>(v: &O) -> Result<BigInt, String> {
    let amount = exact(&v["amount_msat"])?;
    Ok(
        if v["direction"] == "outbound" && codec::truth(&v["amount_includes_fee"]) {
            amount - exact(&v["fee_msat"])?
        } else {
            amount
        },
    )
}
pub(super) fn delta<O: Obs + ?Sized>(v: &O) -> Result<BigInt, String> {
    let amount = exact(&v["amount_msat"])?;
    Ok(if v["direction"] == "inbound" {
        amount
    } else if codec::truth(&v["amount_includes_fee"]) {
        -amount
    } else {
        -amount - exact(&v["fee_msat"])?
    })
}
pub(super) struct Domain {
    pub network: String,
    pub exposure: String,
    pub rail: Value,
}
impl Domain {
    pub fn of<O: Obs + ?Sized>(v: &O) -> Self {
        let (network, exposure) = Self::key(v);
        Self {
            network: network.into_owned(),
            exposure: exposure.into_owned(),
            rail: v["event_key"]["chain"].clone(),
        }
    }
    /// [`Self::of`] an observation, reading a typed one's event key.
    pub fn of_leg(v: LegRef<'_>) -> Self {
        let (network, exposure) = Self::leg_key(v);
        Self {
            network: network.into_owned(),
            exposure: exposure.into_owned(),
            rail: match v {
                LegRef::Typed(leg) => Value::String(leg.event().chain.clone()),
                LegRef::Json(_) => v["event_key"]["chain"].clone(),
            },
        }
    }
    /// The domain's network and exposure, borrowed where the observation
    /// spells them.
    pub fn key<O: Obs + ?Sized>(v: &O) -> (Cow<'_, str>, Cow<'_, str>) {
        Self::spelled(text_key(&v["event_key"]["network"]), text_key(&v["asset"]))
    }
    /// [`Self::key`] of an observation, reading a typed one's event key.
    pub fn leg_key(v: LegRef<'_>) -> (Cow<'_, str>, Cow<'_, str>) {
        let network = match v {
            LegRef::Typed(leg) => Cow::Borrowed(leg.event().network.as_str()),
            LegRef::Json(_) => text_key(&v.at("event_key")["network"]),
        };
        Self::spelled(network, text_key(v.at("asset")))
    }
    fn spelled<'a>(network: Cow<'a, str>, asset: Cow<'a, str>) -> (Cow<'a, str>, Cow<'a, str>) {
        let mapped = match network.as_ref() {
            "liquidv1" => Some("main"),
            "liquidtestnet" => Some("test"),
            "elementsregtest" => Some("regtest"),
            _ => None,
        };
        let network = mapped.map_or(network, Cow::Borrowed);
        let exposure = if matches!(asset.as_ref(), "BTC" | "LBTC") {
            Cow::Borrowed("bitcoin")
        } else {
            Cow::Owned(format!("asset:{asset}"))
        };
        (network, exposure)
    }
    pub fn py(&self) -> Result<kassiber_pyjson::Value, String> {
        use kassiber_pyjson::Value as Py;
        Ok(py_dict([
            ("$type", Py::Str("QuantityDomain".into())),
            ("exposure", Py::Str(self.exposure.as_str().into())),
            ("network", Py::Str(self.network.as_str().into())),
            ("rail", borrowed_py(&self.rail)?),
            ("unit", Py::Str("msat".into())),
        ]))
    }
}
pub(super) fn domain<O: Obs + ?Sized>(v: &O) -> Value {
    let d = Domain::of(v);
    json!({"$type":"QuantityDomain","network":d.network,"exposure":d.exposure,"unit":"msat","rail":d.rail})
}
pub(super) fn validate_observation<O: Obs + ?Sized>(v: &O) -> Result<(), String> {
    let fail = || Err("ValueError".into());
    for key in ["transaction_id", "anchor_transaction_id", "wallet_id"] {
        if !codec::truth(&v[key]) {
            return fail();
        }
    }
    for key in ["quantity_hash", "evidence_detail_hash"] {
        if let Some(s) = v[key].as_str() {
            if s.len() == 64
                && s.bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            {
                continue;
            }
        }
        let s = codec::pystr(&v[key]);
        if s.chars().count() != 64 || s != s.py_lower() {
            return fail();
        }
        // Hashes arriving from the canonical stage are ASCII SHA-256. The
        // general observation constructor also admits Python int(..., 16)
        // spellings (Unicode decimal digits, signs, underscores and spaces).
        // int(text, 16) transforms Unicode decimal digits and spaces, but its
        // ASCII parser rejects the four C0 separators accepted by str.strip.
        if s.chars().any(|c| matches!(c, '\u{1c}'..='\u{1f}')) {
            return fail();
        }
        let normalized: String = s
            .py_strip()
            .chars()
            .map(|c| {
                crate::python_text::decimal_digit(c)
                    .map(|digit| char::from(b'0' + digit))
                    .unwrap_or(c)
            })
            .collect();
        let stripped = normalized.as_str();
        let digits = stripped.strip_prefix(['+', '-']).unwrap_or(stripped);
        let digits = digits.strip_prefix("0x").unwrap_or(digits);
        let mut previous_digit = false;
        for (i, c) in digits.chars().enumerate() {
            if c == '_' {
                if !previous_digit && !(i == 0 && stripped.contains("0x")) {
                    return fail();
                }
                previous_digit = false;
            } else if c.is_ascii_hexdigit() {
                previous_digit = true;
            } else {
                return fail();
            }
        }
        if !previous_digit {
            return fail();
        }
    }
    if !matches!(v["direction"].as_str(), Some("inbound" | "outbound"))
        || !codec::truth(&v["asset"])
    {
        return fail();
    }
    let inclusive = codec::truth(&v["amount_includes_fee"]);
    let outbound = v["direction"] == "outbound";
    // Machine integers decide without big integers.
    let valid = match (v["amount_msat"].as_i64(), v["fee_msat"].as_i64()) {
        (Some(amount), Some(fee)) => quantity_valid(outbound, inclusive, amount, fee),
        _ => quantity_valid(
            outbound,
            inclusive,
            exact(&v["amount_msat"])?,
            exact(&v["fee_msat"])?,
        ),
    };
    if valid {
        Ok(())
    } else {
        fail()
    }
}
/// The amount and fee rules of a quantity observation.
fn quantity_valid<T: Ord + From<i8>>(outbound: bool, inclusive: bool, amount: T, fee: T) -> bool {
    let zero = || T::from(0);
    !(amount < zero()
        || fee < zero()
        || !outbound && inclusive
        || amount == zero() && !(outbound && fee > zero())
        || inclusive && fee > amount)
}

pub(super) fn into_py(value: Value) -> Result<kassiber_pyjson::Value, String> {
    use kassiber_pyjson::{Key, Value as Py};
    Ok(match value {
        Value::Null => Py::Null,
        Value::Bool(v) => Py::Bool(v),
        Value::String(s) => Py::Str(s.into()),
        Value::Number(n) => {
            if let Some(n) = n.as_i64() {
                Py::Int(n.into())
            } else if let Some(n) = n.as_u64() {
                Py::Int(n.into())
            } else {
                codec::from_json(&Value::Number(n))?
            }
        }
        Value::Array(v) => {
            let mut items = Vec::with_capacity(v.len());
            for value in v {
                items.push(into_py(value)?);
            }
            Py::List(items)
        }
        Value::Object(v) => {
            let mut items = Vec::with_capacity(v.len());
            for (key, value) in v {
                items.push((Key::Str(key.into()), into_py(value)?));
            }
            Py::Dict(items)
        }
    })
}

pub(super) fn borrowed_py(value: &Value) -> Result<kassiber_pyjson::Value, String> {
    use kassiber_pyjson::{Key, Value as Py};
    Ok(match value {
        Value::Number(n) if n.as_i64().is_some() => Py::Int(n.as_i64().unwrap().into()),
        Value::Number(n) if n.as_u64().is_some() => Py::Int(n.as_u64().unwrap().into()),
        Value::Array(v) => {
            let mut items = Vec::with_capacity(v.len());
            for value in v {
                items.push(borrowed_py(value)?);
            }
            Py::List(items)
        }
        Value::Object(v) => {
            let mut items = Vec::with_capacity(v.len());
            for (key, value) in v {
                items.push((Key::Str(key.as_str().into()), borrowed_py(value)?));
            }
            Py::Dict(items)
        }
        _ => codec::from_json(value)?,
    })
}

pub(super) fn text_key(v: &Value) -> std::borrow::Cow<'_, str> {
    if let Some(s) = v.as_str() {
        std::borrow::Cow::Borrowed(s)
    } else if codec::truth(v) {
        std::borrow::Cow::Owned(codec::pystr(v))
    } else {
        std::borrow::Cow::Borrowed("")
    }
}

pub(super) fn py_dict<const N: usize>(
    parts: [(&str, kassiber_pyjson::Value); N],
) -> kassiber_pyjson::Value {
    kassiber_pyjson::Value::Dict(
        parts
            .into_iter()
            .map(|(k, v)| (kassiber_pyjson::Key::Str(k.into()), v))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantity_rules_match_for_machine_and_big_integers() {
        let hash = "a".repeat(64);
        let big = "100000000000000000000";
        let cases = [
            ("outbound", "5", "5", true, true),
            ("outbound", "5", "6", true, false),
            ("outbound", "0", "1", false, true),
            ("outbound", "0", "0", false, false),
            ("inbound", "5", "0", true, false),
            ("inbound", "5", "0", false, true),
            ("inbound", "-1", "0", false, false),
            ("outbound", big, big, true, true),
            ("outbound", big, "100000000000000000001", true, false),
        ];
        for (direction, amount, fee, inclusive, valid) in cases {
            let observation = json!({
                "transaction_id": "t", "anchor_transaction_id": "t", "wallet_id": "w",
                "quantity_hash": hash, "evidence_detail_hash": hash, "asset": "BTC",
                "direction": direction, "amount_includes_fee": inclusive,
                "amount_msat": serde_json::from_str::<Value>(amount).unwrap(),
                "fee_msat": serde_json::from_str::<Value>(fee).unwrap(),
            });
            assert_eq!(
                validate_observation(&observation).is_ok(),
                valid,
                "{direction} {amount} {fee} {inclusive}"
            );
        }
    }
}
