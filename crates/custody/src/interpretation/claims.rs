//! Shared claim shape. Quantities are integer half-open msat slices; hashes
//! come exclusively from the canonical Rust evidence stage.
use crate::codec::wide;
use crate::evidence::canonical::Obs;
use serde_json::{json, Value};

/// The observation's principal, read by field so a typed leg builds no
/// object.
pub(super) fn principal<O: Obs + ?Sized>(observation: &O) -> i128 {
    let int = |v: &Value| v.as_i64().map_or_else(|| wide(v), i128::from);
    let amount = int(&observation["amount_msat"]);
    if observation["direction"] == "outbound" && observation["amount_includes_fee"] == true {
        amount - int(&observation["fee_msat"])
    } else {
        amount
    }
}

pub(super) fn slice(hash: &Value, start: i128, end: i128) -> Value {
    json!({"$type":"QuantitySlice", "observation_hash":hash, "start_msat":start, "end_msat":end})
}

pub(super) fn claim(id: String, source: Value, state: &str, exact: bool, reason: &str) -> Value {
    let (priority, number) = if exact {
        ("EXACT_NATIVE_EVENT", 20)
    } else {
        ("ACCOUNTING_CONVENTION", 40)
    };
    json!({"$type":"QuantityClaim", "claim_id":id, "source":source, "state":state,
        "priority":{"$enum":format!("ClaimPriority.{priority}"),"value":number},
        "reason":reason, "target":null, "atomic_bundle_id":null, "destination_kind":null,
        "external_economic_subtype":null, "transfer_kind":null, "transfer_policy":null,
        "component_id":null, "allow_cross_rail":false})
}
