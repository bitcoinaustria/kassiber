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
pub fn clear_memos() {
    crate::codec::clear_memos();
}

pub fn journal_operands(
    snapshot: &crate::snapshot::BookSnapshot,
    stages: &crate::Stages,
) -> Result<(Value, Vec<Value>), String> {
    use crate::{codec, interpretation};
    use serde_json::json;
    let raw = snapshot.stages();
    let exact = |value: &kassiber_pyjson::Value| -> Result<Value, String> {
        let converted = codec::to_json(value);
        if !codec::lossless(value)
            && codec::dumps(value, true)? != codec::dumps(&codec::from_json(&converted)?, true)?
        {
            return Err("journal operands cannot represent custody facts losslessly".into());
        }
        Ok(converted)
    };
    let interpretation = exact(&stages.interpretation)?;
    let quantity = exact(&stages.quantity)?;
    let finalized = exact(
        stages
            .finalized_projection
            .as_ref()
            .ok_or("journal build requires finalized custody projection")?,
    )?;
    let (ignored, accounting_ignored) = interpretation::gap_ignored_ids(
        &raw,
        &stages.components,
        &stages.evidence,
        &stages.interpretation,
    )?;
    let channels = interpretation::channels::compute(&raw)?;
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
    let mut rows = raw.transactions.clone();
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
