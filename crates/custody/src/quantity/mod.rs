//! Exact custody primitives. Runtime stage admission remains separate: these
//! comparison-only artifacts never write or grant accounting authority.
pub mod allocation;
mod arbitration;
pub mod balances;
pub mod hashes;
pub mod holds;
pub mod native;
pub mod reviewed;
pub mod runtime;
pub mod transfers;

use crate::{codec, python_text::PythonText};
pub use arbitration::project;
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::str::FromStr;

pub(super) fn array(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
pub(super) fn number(n: &BigInt) -> Value {
    serde_json::from_str(&n.to_string()).expect("computed integer")
}
pub(super) fn exact(v: &Value) -> Result<BigInt, String> {
    if !v.is_number() || v.to_string().contains(['.', 'e', 'E']) {
        return Err("ValueError".into());
    }
    BigInt::from_str(&v.to_string()).map_err(|_| "ValueError".into())
}
pub(super) fn principal(v: &Value) -> Result<BigInt, String> {
    let amount = exact(&v["amount_msat"])?;
    Ok(
        if v["direction"] == "outbound" && codec::truth(&v["amount_includes_fee"]) {
            amount - exact(&v["fee_msat"])?
        } else {
            amount
        },
    )
}
pub(super) fn delta(v: &Value) -> Result<BigInt, String> {
    let amount = exact(&v["amount_msat"])?;
    Ok(if v["direction"] == "inbound" {
        amount
    } else if codec::truth(&v["amount_includes_fee"]) {
        -amount
    } else {
        -amount - exact(&v["fee_msat"])?
    })
}
pub(super) fn domain(v: &Value) -> Value {
    let network = codec::text(&v["event_key"]["network"]);
    let network = match network.as_str() {
        "liquidv1" => "main",
        "liquidtestnet" => "test",
        "elementsregtest" => "regtest",
        _ => &network,
    };
    let asset = codec::text(&v["asset"]);
    let exposure = if matches!(asset.as_str(), "BTC" | "LBTC") {
        "bitcoin".into()
    } else {
        format!("asset:{asset}")
    };
    json!({"$type":"QuantityDomain","network":network,"exposure":exposure,"unit":"msat","rail":v["event_key"]["chain"]})
}
pub(super) fn validate_observation(v: &Value) -> Result<(), String> {
    let fail = || Err("ValueError".into());
    for key in ["transaction_id", "anchor_transaction_id", "wallet_id"] {
        if !codec::truth(&v[key]) {
            return fail();
        }
    }
    for key in ["quantity_hash", "evidence_detail_hash"] {
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
    let amount = exact(&v["amount_msat"])?;
    let fee = exact(&v["fee_msat"])?;
    if amount < BigInt::from(0) || fee < BigInt::from(0) {
        return fail();
    }
    let inclusive = codec::truth(&v["amount_includes_fee"]);
    if v["direction"] == "inbound" && inclusive
        || amount == BigInt::from(0) && !(v["direction"] == "outbound" && fee > BigInt::from(0))
        || inclusive && fee > amount
    {
        return fail();
    }
    Ok(())
}
