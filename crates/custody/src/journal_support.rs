//! Transitional graph facts for the journal review join; no ownership authority.
use crate::graph::{self, ownership::Index};
use serde_json::Value;

pub type Scope = (String, String, String, String);

pub fn require_tax_processing(profile: &Value) -> Result<(), (&'static str, String)> {
    crate::gates::require_tax_country(&profile["tax_country"])
}

pub fn onchain_scope(row: &Value) -> Option<Scope> {
    graph::onchain_scope(row)
}
pub fn parsed(row: &Value) -> Result<Option<Value>, String> {
    graph::parse(&row["raw_json"])
}
pub fn ownership_scope(
    row: &Value,
    parsed: &Value,
    index: &Value,
) -> Result<Option<Scope>, String> {
    graph::ownership::scope(row, Some(parsed), Index(Some(index)))
}
pub fn script_owners(
    script: &Value,
    scope: &(String, String),
    index: &Value,
) -> Result<std::collections::BTreeSet<String>, String> {
    Index(Some(index)).script_owners(script, scope)
}
pub fn scope_error_operand<'a>(parsed: &'a Value, index: &Value) -> &'a Value {
    let index = Index(Some(index));
    let entries = |key| parsed[key].as_array().map(Vec::as_slice).unwrap_or(&[]);
    for entry in entries("inputs") {
        let script = &entry["script"];
        if (!crate::codec::truth(&entry["outpoint"])
            || index.outpoint(&entry["outpoint"], None).is_empty())
            && crate::codec::truth(script)
            && !script.is_string()
        {
            return script;
        }
    }
    entries("outputs")
        .iter()
        .map(|entry| &entry["script"])
        .find(|script| crate::codec::truth(script) && !script.is_string())
        .unwrap_or(&Value::Null)
}
pub fn clear_memos() {
    crate::codec::clear_memos();
}

pub fn exact_operand(value: &kassiber_pyjson::Value) -> Result<Value, String> {
    use crate::codec;
    let converted = codec::to_json(value);
    if !codec::lossless(value)
        && codec::dumps(value, true)? != codec::dumps(&codec::from_json(&converted)?, true)?
    {
        return Err("journal operands cannot represent custody facts losslessly".into());
    }
    Ok(converted)
}

pub fn journal_operands(
    snapshot: &crate::snapshot::Snapshot,
    stages: &crate::Stages,
) -> Result<(Value, Vec<Value>), String> {
    use crate::{codec, interpretation};
    use serde_json::json;
    let raw = snapshot;
    let exact = exact_operand;
    let interpretation = exact(&stages.interpretation)?;
    let quantity = stages.quantity.json_with(true, exact)?;
    let finalized = stages
        .finalized_projection
        .as_ref()
        .ok_or("journal build requires finalized custody projection")?
        .json_with(&raw.transactions, exact)?;
    let (ignored, accounting_ignored) = interpretation::gap_ignored_ids(
        raw,
        &stages.components,
        &stages.evidence,
        &stages.interpretation,
    )?;
    let channels = interpretation::channels::compute(raw)?;
    let wallets: Vec<Value> = raw
        .wallets
        .iter()
        .map(|wallet| {
            Value::Object(
                wallet
                    .iter()
                    .filter(|(key, _)| {
                        !matches!(
                            key.as_str(),
                            "config_json" | "config_text_mentions_samourai"
                        )
                    })
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            )
        })
        .collect();
    let refs = Value::Object(
        wallets
            .iter()
            .map(|wallet| (codec::pystr(&wallet["id"]), wallet.clone()))
            .collect(),
    );
    let eligible = quantity["tax_eligibility"]["eligible_decisions"]
        .as_array()
        .ok_or("journal quantity eligibility is missing")?;
    let transfers = crate::quantity::transfers::canonical_internal_transfer_rows(
        &quantity["projection"],
        eligible,
        &refs,
    )?;
    let mut rows = raw.rows().to_vec();
    for row in &mut rows {
        row["_closed_confirmation_observation"] = interpretation::capture_confirmation(row)?;
    }
    Ok((
        json!({
            "rows": rows,
            "direct_payout_records": quantity["reviewed_direct_payouts"],
            "wallet_refs_by_id": refs,
            "owned_index": raw.owned_index,
            "loan_legs": raw.loan_legs,
            "channel_roles": channels["roles"],
            "active_components": stages.components["components"],
            "component_blockers": stages.components["blockers"],
            "interpretation": interpretation,
            "quantity_state": quantity,
            "custody_transfers": transfers,
            "gap_ignored_transaction_ids": ignored,
            "gap_accounting_ignored_transaction_ids": accounting_ignored,
            "finalized_tax_projection": finalized,
        }),
        wallets,
    ))
}

pub fn native_liquid_graph(raw: &kassiber_pyjson::Value) -> Option<kassiber_pyjson::Value> {
    let kassiber_pyjson::Value::Str(raw) = raw else {
        return None;
    };
    let decoded = crate::liquid::decode(&Value::String(raw.as_str()?.into()))?;
    crate::codec::from_json(&decoded).ok()
}

pub fn native_alphanumeric(c: char) -> bool {
    crate::python_text::isalnum(c)
}
