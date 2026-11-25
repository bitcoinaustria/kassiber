//! Exact exchange withdrawals: a platform row naming the txid of an owned
//! receipt, for exactly its one owned output, moves basis without review.
//! The same-txid candidate is promoted only with a proof bound to both
//! quantities and the matched output; the pair compiler revalidates it.
use super::{anchor, claims::principal, pairs::source_name, Context};
use crate::{
    codec::{self, text, truth},
    graph::{
        ownership::{Index, PhysicalScope, TransferScope},
        stored as py,
    },
    python_text::PythonText,
    snapshot::Snapshot,
};
use kassiber_pyjson::{BigInt, Value as Py};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const SOURCE: &str = "exchange_txid_exact";
/// Custodial platforms whose withdrawal exports name the on-chain txid and
/// report the amount sent on chain apart from the platform's fee.
const WALLET_KINDS: [&str; 2] = ["river", "strike"];

pub(super) fn pair_id(txid: &str, vout: &str) -> String {
    format!("exchange-txid:{txid}:{vout}")
}

/// A platform's graphless withdrawal row and the observed owned receipt.
pub(super) fn shape(source: &Value, target: &Value) -> bool {
    !truth(&source["authoritative_chain_observation"])
        && truth(&target["authoritative_chain_observation"])
        && source["direction"] == "outbound"
        && target["direction"] == "inbound"
        && source["event_key"] == target["event_key"]
        && source["event_key"]["native_namespace"] == "chain"
        && source["event_key"]["chain"] == "bitcoin"
        && source["asset"] == target["asset"]
        && source["wallet_id"] != target["wallet_id"]
        && principal(source) > 0
        && principal(source) == principal(target)
}

/// Which automatic txid-exact producer created a bound proof.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum ProofSource {
    #[default]
    Exchange,
    Bark,
}

/// A platform withdrawal or Bark on-chain send bound to the one owned output it paid.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct ExchangeTxidProof {
    #[serde(default)]
    source: ProofSource,
    source_quantity_hash: String,
    target_quantity_hash: String,
    txid: String,
    vout: String,
    amount_msat: i128,
}
impl ExchangeTxidProof {
    pub(super) fn bark(
        source_quantity_hash: String,
        target_quantity_hash: String,
        txid: String,
        vout: String,
        amount_msat: i128,
    ) -> Self {
        Self {
            source: ProofSource::Bark,
            source_quantity_hash,
            target_quantity_hash,
            txid,
            vout,
            amount_msat,
        }
    }
    pub(super) fn validates_for(
        &self,
        expected: ProofSource,
        pair: &Value,
        source: &Value,
        target: &Value,
    ) -> bool {
        if self.source != expected {
            return false;
        }
        let expected_pair_id = match self.source {
            ProofSource::Exchange => pair_id(&self.txid, &self.vout),
            ProofSource::Bark => {
                if truth(&source["amount_includes_fee"]) {
                    return false;
                }
                super::bark::pair_id(&self.txid, &self.vout)
            }
        };
        source["quantity_hash"] == self.source_quantity_hash
            && target["quantity_hash"] == self.target_quantity_hash
            && shape(source, target)
            && principal(source) == self.amount_msat
            && pair["pair_id"] == expected_pair_id
    }
}

pub(super) type Proofs = BTreeMap<(String, String), ExchangeTxidProof>;

fn valid_script_hex(value: &Py) -> bool {
    let Py::Str(raw) = value else {
        return false;
    };
    let Some(text) = raw.as_str() else {
        return false;
    };
    let trimmed = text.py_strip();
    !trimmed.is_empty()
        && trimmed.len().is_multiple_of(2)
        && trimmed.bytes().all(|b| b.is_ascii_hexdigit())
}

fn exact_nonneg_sats(value: &Py) -> bool {
    matches!(value, Py::Int(n) if *n >= BigInt::from(0))
}

/// Reject raw leg payloads where `stored::parse` silently dropped non-dict
/// elements or truncated a fractional `value_sats` into an integer.
fn valid_raw_legs(raw_json: &Value, expected_inputs: usize, expected_outputs: usize) -> bool {
    let raw = py::object(raw_json);
    let payload = [py::get(&raw, "ownership_graph"), py::get(&raw, "tx"), &raw]
        .into_iter()
        .find(|candidate| {
            matches!(
                (py::get(candidate, "vin"), py::get(candidate, "vout")),
                (Py::List(_), Py::List(_))
            )
        });
    let Some(payload) = payload else {
        return true;
    };
    let (Py::List(vin), Py::List(vout)) = (py::get(payload, "vin"), py::get(payload, "vout"))
    else {
        return false;
    };
    if vin.len() != expected_inputs || vout.len() != expected_outputs {
        return false;
    }
    let integer_sats_field = |entry: &Py| -> bool {
        match py::get(entry, "value_sats") {
            Py::Null => true,
            Py::Int(n) => *n >= BigInt::from(0),
            Py::Str(s) => s.as_str().is_some_and(|text| {
                let trimmed = text.py_strip();
                !trimmed.is_empty() && trimmed.bytes().all(|b| b.is_ascii_digit())
            }),
            _ => false,
        }
    };
    vin.iter().all(|entry| {
        matches!(entry, Py::Dict(_)) && {
            let prevout = py::get(entry, "prevout");
            matches!(prevout, Py::Dict(_))
                && integer_sats_field(prevout)
                && integer_sats_field(entry)
        }
    }) && vout
        .iter()
        .all(|entry| matches!(entry, Py::Dict(_)) && integer_sats_field(entry))
}

/// The receipt's one owned output, or `None` when the match is not exact:
/// a fully valued graph with no input owned by the book and exactly one
/// output owned in its network, by the receiving wallet alone, for the whole
/// receipt.
pub(super) fn owned_output(
    row: &Value,
    target: &Value,
    scope: &TransferScope,
    index: Index<'_>,
) -> Result<Option<String>, String> {
    if index.0.is_none() {
        return Ok(None);
    }
    let Some(graph) = py::parse(&row["raw_json"])? else {
        return Ok(None);
    };
    let outputs = py::list(py::get(&graph, "outputs"));
    if outputs
        .iter()
        .any(|output| !matches!(py::get(output, "value_sats"), Py::Int(_)))
        || py::truth(py::get(&graph, "evidence_conflicts"))
    {
        return Ok(None);
    }
    if super::recorded::canonical_hash(&py::project(py::get(&graph, "txid"))).as_deref()
        != Some(scope.2.as_str())
    {
        return Ok(None);
    }
    let inputs = py::list(py::get(&graph, "inputs"));
    if inputs.is_empty()
        || outputs.is_empty()
        || !valid_raw_legs(&row["raw_json"], inputs.len(), outputs.len())
        || inputs.iter().any(|entry| {
            !valid_script_hex(py::get(entry, "script"))
                || !exact_nonneg_sats(py::get(entry, "value_sats"))
        })
        || outputs.iter().any(|output| {
            !valid_script_hex(py::get(output, "script"))
                || !exact_nonneg_sats(py::get(output, "value_sats"))
        })
    {
        return Ok(None);
    }
    let physical: PhysicalScope = (scope.0.clone(), scope.1.clone());
    for entry in inputs {
        if !index
            .input_owners(&py::project(entry), &physical)?
            .is_empty()
        {
            return Ok(None);
        }
    }
    let mut owned = Vec::new();
    for output in outputs {
        let owners = index.script_owners(&py::project(py::get(output, "script")), &physical)?;
        if !owners.is_empty() {
            owned.push((output, owners));
        }
    }
    let [(output, owners)] = owned.as_slice() else {
        return Ok(None);
    };
    if *owners != BTreeSet::from([text(&target["wallet_id"])]) {
        return Ok(None);
    }
    let (Py::Int(value), Py::Int(vout)) = (py::get(output, "value_sats"), py::get(output, "n"))
    else {
        return Ok(None);
    };
    if value.clone() * BigInt::from(1000) != BigInt::from(principal(target)) {
        return Ok(None);
    }
    Ok(Some(vout.to_string()))
}

/// Promote exact platform withdrawals among the same-txid candidates.
/// Anything less exact keeps its row-matched label, and with it the
/// authority-conflict hold; a pair dismissal undoes the promotion.
pub(super) fn promote(
    pairs: Vec<Value>,
    context: &Context,
    snapshot: &Snapshot,
    dismissed: &BTreeSet<(String, String)>,
) -> Result<(Vec<Value>, Proofs), String> {
    let index = Index(snapshot.owned_index.as_ref());
    let mut promoted = Vec::with_capacity(pairs.len());
    let mut proofs = Proofs::new();
    for mut pair in pairs {
        let (out_id, in_id) = (anchor(&pair["out"]), anchor(&pair["in"]));
        let scope = crate::graph::onchain_scope(&pair["in"]);
        let observed = (
            context.observations.get(&out_id),
            context.observations.get(&in_id),
        );
        let (Some(source), Some(target), Some(scope)) = (observed.0, observed.1, scope) else {
            promoted.push(pair);
            continue;
        };
        let kind = text(&pair["out"]["wallet_kind"]).py_strip().py_lower();
        if source_name(&pair) != "row_matched"
            || context.excluded.contains(&out_id)
            || context.excluded.contains(&in_id)
            || dismissed.contains(&(out_id.clone(), in_id.clone()))
            || !WALLET_KINDS.contains(&kind.as_str())
            || !shape(source, target)
        {
            promoted.push(pair);
            continue;
        }
        let Some(vout) = owned_output(&pair["in"], target, &scope, index)? else {
            promoted.push(pair);
            continue;
        };
        let id = pair_id(&scope.2, &vout);
        proofs.insert(
            (out_id, in_id),
            ExchangeTxidProof {
                source: ProofSource::Exchange,
                source_quantity_hash: codec::text(&source["quantity_hash"]),
                target_quantity_hash: codec::text(&target["quantity_hash"]),
                txid: scope.2.clone(),
                vout,
                amount_msat: principal(source),
            },
        );
        pair["source"] = json!(SOURCE);
        pair["pair_id"] = json!(id);
        promoted.push(pair);
    }
    Ok((promoted, proofs))
}
