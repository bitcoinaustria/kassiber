//! Private finalized-projection compiler and comparison primitives.
//! Snapshot stage admission remains skipped; no result grants persistence authority.
use crate::{codec, evidence, graph, interpretation::python_value_string};
use kassiber_pyjson::{BigInt, Value as Py};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::str::FromStr;
mod composition;
mod conversions;
mod fees;
mod money;
mod payouts;
mod principal;
mod quarantines;
pub use fees::FeeAllocation;
pub use principal::slice_row;

fn string(value: &Value) -> String {
    python_value_string(&codec::from_json(value).expect("JSON primitive input"))
}
fn text(value: &Value) -> String {
    if codec::truth(value) {
        string(value)
    } else {
        String::new()
    }
}

/// Python's constructors deliberately validate rows before move-pair references.
pub fn validate(rows: &[Value], pairs: &[Value]) -> Result<(), (&'static str, &'static str)> {
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for row in rows {
        if !codec::truth(&row["custody_finalized_tax_projection"]) {
            return Err((
                "TypeError",
                "tax engines accept only finalized custody projection rows",
            ));
        }
        let id = text(&row["id"]);
        if id.is_empty() || !ids.insert(id) {
            return Err((
                "ValueError",
                "finalized tax projection rows need unique ids",
            ));
        }
    }
    for pair in pairs {
        let (out, input) = (&pair["out"], &pair["in"]);
        if !out.is_object() || !input.is_object() {
            return Err((
                "ValueError",
                "finalized move pairs require projected out and in rows",
            ));
        }
        if !ids.contains(&string(&out["id"])) || !ids.contains(&string(&input["id"])) {
            return Err((
                "ValueError",
                "finalized move pairs cannot reference raw/unprojected rows",
            ));
        }
    }
    Ok(())
}

pub fn slice_row_id(hash: &str, start: &BigInt, end: &BigInt, side: &str) -> String {
    format!("custody-tax:{hash}:{start}:{end}:{side}")
}
pub fn fee_row_id(hash: &str, side: &str, amount: &BigInt) -> String {
    format!("custody-tax:{hash}:{side}:{amount}")
}
pub fn payout_row_id(id: &str, side: &str) -> String {
    format!("direct-payout:{id}:{side}")
}

// Match the raw hash function's int(amount or 0), then int(fee or 0).
// Container failures are TypeError, numeric spelling failures are ValueError.
fn observed_hash(row: &Value) -> Result<(String, String), String> {
    for key in ["amount", "fee"] {
        crate::interpretation::integer_or_zero(&row[key])?;
    }
    evidence::observed_hash(row).map_err(|e| e.code.to_owned())
}

/// Capture closed authority on the original row, before its id/amount changes.
fn chain_order(row: &Value) -> Result<Value, String> {
    if codec::integer(&row["observation_authority_version"]).ok() != Some(json!(1)) {
        return Ok(Value::Null);
    }
    if text(&row["observation_graph_hash"])
        != evidence::graph_hash(&row["raw_json"])
            .map_err(|e| e.code.to_owned())?
            .0
        || text(&row["observation_quantity_hash"]) != observed_hash(row)?.0
    {
        return Ok(Value::Null);
    }
    let Some((chain, network, txid, asset)) = graph::onchain_scope(row) else {
        return Ok(Value::Null);
    };
    let raw = if let Some(raw) = row["raw_json"].as_str() {
        codec::loads(raw).unwrap_or(Py::Null)
    } else {
        codec::from_json(&row["raw_json"])?
    };
    let mut parents = BTreeSet::new();
    let get = graph::stored::get;
    for graph in [&raw, get(&raw, "tx"), get(&raw, "ownership_graph")] {
        for entry in graph::stored::list(get(graph, "vin")) {
            let Py::Int(vout) = get(entry, "vout") else {
                continue;
            };
            if *vout < BigInt::from(0) {
                continue;
            }
            if let Some(parent) = graph::strict_txid(&codec::to_json(get(entry, "txid"))) {
                parents.insert((chain.clone(), network.clone(), parent, asset.clone()));
            }
        }
    }
    Ok(
        json!({"$type":"ChainOrderEvidence", "scope":[chain,network,txid,asset], "parents":{"$set":parents}}),
    )
}

pub fn base_row(observation: &Value, rows: &Value) -> Result<Value, String> {
    let source = &rows[text(&observation["transaction_id"])];
    let source = if codec::truth(source) {
        source
    } else {
        &rows[text(&observation["anchor_transaction_id"])]
    };
    let mut result = if codec::truth(source) {
        source.clone()
    } else {
        json!({})
    };
    for field in [
        "fiat_value",
        "fiat_value_exact",
        "fiat_rate",
        "fiat_rate_exact",
    ] {
        if !money::empty(&result[field]) {
            money::Money::finite(&result[field])?;
        }
    }
    result["custody_chain_order"] = chain_order(&result)?;
    for key in ["wallet_id", "asset", "direction", "occurred_at"] {
        result[key] = observation[key].clone();
    }
    if !codec::truth(&result["external_id"]) {
        result["external_id"] = observation["event_key"]["native_event_id"].clone();
    }
    if !codec::truth(&result["raw_json"]) {
        result["raw_json"] = json!({});
    }
    result["amount_includes_fee"] = json!(false);
    Ok(result)
}

pub fn fee_row(
    observation: &Value,
    amount: &BigInt,
    rows: &Value,
    side: &str,
) -> Result<Value, String> {
    let mut base = base_row(observation, rows)?;
    if codec::truth(&observation["authoritative_chain_observation"])
        && observation["fee_attribution"] == "exact"
    {
        base["privacy_boundary"] = Value::Null;
    }
    base["id"] = json!(fee_row_id(
        &string(&observation["quantity_hash"]),
        side,
        amount
    ));
    base["journal_transaction_id"] = observation["anchor_transaction_id"].clone();
    base["amount"] = json!(0);
    base["fee"] = serde_json::from_str(&amount.to_string()).expect("exact computed integer");
    base["fiat_value"] = Value::Null;
    base["fiat_value_exact"] = Value::Null;
    base["amount_includes_fee"] = json!(false);
    base["custody_finalized_tax_projection"] = json!(true);
    base["custody_quantity_hash"] = observation["quantity_hash"].clone();
    base["custody_slice_start_msat"] = json!(0);
    base["custody_slice_end_msat"] = json!(0);
    Ok(base)
}

fn integer_big(v: &Value) -> Result<BigInt, String> {
    let value = crate::interpretation::integer_or_zero(v)?;
    BigInt::from_str(&value.to_string()).map_err(|_| "ValueError".into())
}
fn number(n: &BigInt) -> Value {
    serde_json::from_str(&n.to_string()).expect("exact integer")
}

/// Private fixture operation, not snapshot admission or a product interface.
pub fn primitive(value: &Value) -> Value {
    let integer =
        |key: &str| BigInt::from_str(&value[key].to_string()).expect("integer fixture operand");
    let result = match value["op"].as_str().unwrap() {
        "validate" => {
            return match validate(
                value["rows"].as_array().unwrap(),
                value["pairs"].as_array().unwrap(),
            ) {
                Ok(()) => json!({"expected":true}),
                Err((error, message)) => json!({"error":error,"message":message}),
            }
        }
        "ids" => Ok(json!([
            slice_row_id(
                &string(&value["hash"]),
                &integer("start"),
                &integer("end"),
                &string(&value["side"])
            ),
            fee_row_id(
                &string(&value["hash"]),
                &string(&value["side"]),
                &integer("amount")
            ),
            payout_row_id(&string(&value["payout_id"]), &string(&value["side"]))
        ])),
        "slice" => slice_row(
            &value["observation"],
            &value["quantity"],
            &value["rows"],
            &string(&value["side"]),
            &integer("fee"),
        ),
        "payout_valid" => payouts::valid(&value["records"], &value["rows"]),
        "payout_time" => payouts::time(&value["payout"]),
        "payout_settlement" => {
            payouts::settlement(&value["observation"], &value["payout"], &value["rows"])
        }
        "payout_price" => {
            payouts::price(value["row"].clone(), &value["payout"], &integer("amount"))
        }
        "fees" => fees::evaluate(value),
        "conversions" => conversions::evaluate(value),
        "finalize" | "compose" | "connected" | "basis" => composition::evaluate(value),
        "base" => base_row(&value["observation"], &value["rows"]),
        "fee" => fee_row(
            &value["observation"],
            &integer("amount"),
            &value["rows"],
            &string(&value["side"]),
        ),
        _ => panic!("unknown private projection operation"),
    };
    match result {
        Ok(expected) => json!({"expected":expected}),
        Err(error) => json!({"error":error}),
    }
}
