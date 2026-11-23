//! Exact BTCPay Bark on-chain sends: a Bark send (`send-onchain`, `offboard`,
//! or `onchain-send`) naming the txid of an owned wallet's observed receipt,
//! for exactly its one owned output, moves basis without review.
//! The same-txid candidate is promoted only with a proof bound to both
//! quantities and the matched output; the pair compiler revalidates it.
use super::{
    anchor,
    claims::principal,
    exchange::{self, ExchangeTxidProof, Proofs},
    pairs::source_name,
    Context,
};
use crate::{
    codec::{self, text, truth},
    evidence,
    graph::{self, ownership::Index, BARK_PART_PRINCIPAL, BARK_RAW_SOURCE},
    python_text::PythonText,
    snapshot::Snapshot,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const SOURCE: &str = "bark_txid_exact";
/// Bark on-chain send kinds whose `transaction_hash` names the on-chain Bitcoin
/// transaction and whose principal (`-amount - fee`) is the exact on-chain
/// output value sent outside the Bark wallet.
const SEND_KINDS: [&str; 3] = ["send-onchain", "offboard", "onchain-send"];

pub(super) fn pair_id(txid: &str, vout: &str) -> String {
    format!("bark-txid:{txid}:{vout}")
}

/// Row-bound Bark sync provenance loaded from SQLite
/// (`transaction_id -> (observed_hash, graph_hash)`).
fn bark_provenance(snapshot: &Snapshot) -> BTreeMap<String, (String, String)> {
    snapshot
        .bark_sync_provenance
        .iter()
        .flatten()
        .filter_map(|record| {
            let id = text(record.get("transaction_id")?);
            let observed = text(record.get("observed_hash")?);
            let graph = text(record.get("graph_hash")?);
            (!id.is_empty() && !observed.is_empty() && !graph.is_empty())
                .then_some((id, (observed, graph)))
        })
        .collect()
}

/// Whether `row` is a BTCPay Bark principal on-chain send naming `expected_txid`
/// with row-bound provenance established by the Bark sync path.
fn is_bark_send(
    row: &Value,
    out_id: &str,
    expected_txid: &str,
    provenance: &BTreeMap<String, (String, String)>,
) -> Result<bool, String> {
    if text(&row["wallet_kind"]).py_strip().py_lower() != "custom"
        || truth(&row["amount_includes_fee"])
    {
        return Ok(false);
    }
    let Some((expected_observed, expected_graph)) = provenance.get(out_id) else {
        return Ok(false);
    };
    let observed = evidence::observed_hash(row).map_err(|e| e.to_string())?.0;
    let raw_hash = evidence::graph_hash(&row["raw_json"])
        .map_err(|e| e.to_string())?
        .0;
    if observed != *expected_observed || raw_hash != *expected_graph {
        return Ok(false);
    }
    let raw = graph::transfer_object(&row["raw_json"]);
    Ok(
        text(&raw["source"]).py_strip().py_lower() == BARK_RAW_SOURCE
            && text(&raw["part"]).py_strip().py_lower() == BARK_PART_PRINCIPAL
            && SEND_KINDS.contains(&text(&raw["kind"]).py_strip().py_lower().as_str())
            && graph::strict_txid(&raw["transaction_hash"]).as_deref() == Some(expected_txid),
    )
}

/// Promote exact BTCPay Bark on-chain sends among the same-txid candidates.
/// Anything less exact keeps its row-matched label, and with it the
/// authority-conflict hold; a pair dismissal undoes the promotion.
pub(super) fn promote(
    pairs: Vec<Value>,
    context: &Context,
    snapshot: &Snapshot,
    dismissed: &BTreeSet<(String, String)>,
) -> Result<(Vec<Value>, Proofs), String> {
    let index = Index(snapshot.owned_index.as_ref());
    let provenance = bark_provenance(snapshot);
    let mut promoted = Vec::with_capacity(pairs.len());
    let mut proofs = Proofs::new();
    for mut pair in pairs {
        let (out_id, in_id) = (anchor(&pair["out"]), anchor(&pair["in"]));
        let scope = graph::onchain_scope(&pair["in"]);
        let observed = (
            context.observations.get(&out_id),
            context.observations.get(&in_id),
        );
        let (Some(source), Some(target), Some(scope)) = (observed.0, observed.1, scope) else {
            promoted.push(pair);
            continue;
        };
        if source_name(&pair) != "row_matched"
            || context.excluded.contains(&out_id)
            || context.excluded.contains(&in_id)
            || dismissed.contains(&(out_id.clone(), in_id.clone()))
            || !is_bark_send(&pair["out"], &out_id, &scope.2, &provenance)?
            || !exchange::shape(source, target)
        {
            promoted.push(pair);
            continue;
        }
        let Some(vout) = exchange::owned_output(&pair["in"], target, &scope, index)? else {
            promoted.push(pair);
            continue;
        };
        let id = pair_id(&scope.2, &vout);
        proofs.insert(
            (out_id, in_id),
            ExchangeTxidProof::bark(
                codec::text(&source["quantity_hash"]),
                codec::text(&target["quantity_hash"]),
                scope.2.clone(),
                vout,
                principal(source),
            ),
        );
        pair["source"] = json!(SOURCE);
        pair["pair_id"] = json!(id);
        promoted.push(pair);
    }
    Ok((promoted, proofs))
}
