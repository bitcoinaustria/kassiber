//! Generic pair compilation. A source label never substitutes for endpoint
//! authority. Native proof production supplies bound compiler-only proofs.
use super::{
    anchor,
    claims::{claim, principal, slice},
    exchange, recorded, Context,
};
use crate::{
    codec::{self, text, truth},
    components::time,
    snapshot::Snapshot,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

// Internal compiler input, never a field of Snapshot. The native HTLC
// producer constructs this from independently recomputed matcher facts.
#[derive(serde::Deserialize)]
pub(super) struct NativeTransitionProof {
    candidate: Value,
    source_quantity_hash: String,
    target_quantity_hash: String,
    policy: Value,
}
impl NativeTransitionProof {
    fn validates(&self, pair: &Value, source: &Value, target: &Value) -> bool {
        self.candidate["confidence"] == "exact"
            && numeric_equal(&self.candidate["conflict_size"], 1)
            && matches!(
                self.candidate["method"].as_str(),
                Some("payment_hash" | "htlc_refund")
            )
            && source["quantity_hash"] == self.source_quantity_hash
            && target["quantity_hash"] == self.target_quantity_hash
            && pair["kind"] == self.candidate["default_kind"]
            && pair["policy"] == self.policy
            && numeric_equal(&pair["out_amount_msat"], principal(source))
    }
}

fn numeric_equal(value: &Value, expected: i128) -> bool {
    if !value.is_number() && !value.is_boolean() {
        return false;
    }
    if value.is_f64() && value.as_f64().is_some_and(|n| n.fract() != 0.0) {
        return false;
    }
    integer(value).ok() == Some(expected)
}

pub(super) fn source_name(pair: &Value) -> String {
    text(if truth(&pair["source"]) {
        &pair["source"]
    } else {
        &pair["pair_source"]
    })
}
fn absent(value: &Value) -> bool {
    value.is_null() || value == ""
}
fn requested(pair: &Value) -> &Value {
    if absent(&pair["out_amount_msat"]) {
        &pair["out_amount"]
    } else {
        &pair["out_amount_msat"]
    }
}
fn integer(value: &Value) -> Result<i128, String> {
    let parsed = super::integer(value)?.to_string();
    // Python ints are unbounded; requests are clamped against signed-64
    // canonical principal before any addition. Saturating only this comparison
    // operand preserves huge positive/negative requests without truncation.
    Ok(parsed.parse().unwrap_or_else(|_| {
        if parsed.starts_with('-') {
            i128::MIN
        } else {
            i128::MAX
        }
    }))
}
fn exact_fanout(
    pairs: &[&Value],
    context: &Context,
    scopes: &BTreeMap<String, (String, String, String, String)>,
) -> Result<BTreeSet<String>, String> {
    let mut groups: Vec<(String, Vec<&Value>)> = Vec::new();
    let mut index = crate::fxhash::HashMap::default();
    for &pair in pairs {
        let group = text(&pair["group_id"]);
        if !group.is_empty() {
            crate::codec::push_grouped(&mut groups, &mut index, group, pair);
        }
    }
    let mut exact = BTreeSet::new();
    for (group, members) in groups {
        if members.len() < 2 || members.iter().any(|p| source_name(p) != "recorded_fanout") {
            continue;
        }
        let sources: BTreeSet<_> = members.iter().map(|p| anchor(&p["out"])).collect();
        let targets: Vec<_> = members.iter().map(|p| anchor(&p["in"])).collect();
        if sources.len() != 1
            || sources.contains("")
            || targets.iter().collect::<BTreeSet<_>>().len() != members.len()
            || targets.iter().any(String::is_empty)
        {
            continue;
        }
        let source_id = sources.first().unwrap();
        let (Some(source), Some(scope)) =
            (context.observations.get(source_id), scopes.get(source_id))
        else {
            continue;
        };
        if source["direction"] != "outbound"
            || !truth(&source["authoritative_chain_observation"])
            || source["event_key"]["native_namespace"] != "chain"
            || !matches!(
                source["event_key"]["chain"].as_str(),
                Some("bitcoin" | "liquid")
            )
        {
            continue;
        }
        // Group admission precedes every raw amount conversion in Python.
        let mut wallets = BTreeSet::new();
        let mut typed_targets = Vec::new();
        for target_id in &targets {
            let Some(target) = context.observations.get(target_id) else {
                break;
            };
            if !truth(&target["authoritative_chain_observation"])
                || target["direction"] != "inbound"
                || target["event_key"] != source["event_key"]
                || target["asset"] != source["asset"]
                || target["wallet_id"] == source["wallet_id"]
                || scopes.get(target_id) != Some(scope)
            {
                break;
            }
            wallets.insert(text(&target["wallet_id"]));
            typed_targets.push(target);
        }
        if typed_targets.len() != members.len() || wallets.len() != members.len() {
            continue;
        }
        let mut amount = 0;
        let mut valid = true;
        for (pair, target) in members.iter().zip(typed_targets) {
            let requested = if absent(requested(pair)) {
                principal(target)
            } else {
                integer(requested(pair))?
            };
            let synthetic = integer(&super::integer_or_zero(&pair["out"]["amount"])?)?;
            if requested <= 0 || requested != principal(target) || synthetic != principal(target) {
                valid = false;
                break;
            }
            amount += requested;
        }
        if valid && amount == principal(source) {
            exact.insert(group);
        }
    }
    Ok(exact)
}
pub(super) fn detail_json(value: &Value) -> Result<String, String> {
    kassiber_pyjson::dumps(
        &codec::from_json(value)?,
        &kassiber_pyjson::Options::new().sort_keys(true),
    )
    .map_err(|e| e.to_string())
}
fn quarantine(row: &Value, id: &str, reason: &str, detail: &Value) -> Result<Value, String> {
    Ok(
        json!({"transaction_id":id,"workspace_id":row["workspace_id"],"profile_id":row["profile_id"],"reason":reason,"detail_json":detail_json(detail)?}),
    )
}
fn block_pair(
    pair: &Value,
    reason: &str,
    detail: Value,
    blocked: &mut BTreeSet<String>,
    quarantines: &mut Vec<Value>,
) -> Result<(), String> {
    for (side, paired) in [("out", false), ("in", true)] {
        let row = &pair[side];
        let id = anchor(row);
        let mut detail = detail.clone();
        if paired {
            detail["paired_leg"] = json!(true);
        }
        quarantines.push(quarantine(row, &id, reason, &detail)?);
        blocked.insert(id);
    }
    Ok(())
}
fn exposure(observation: &Value) -> String {
    if matches!(observation["asset"].as_str(), Some("BTC" | "LBTC")) {
        "bitcoin".into()
    } else {
        format!("asset:{}", text(&observation["asset"]))
    }
}
fn btc(amount: i128) -> f64 {
    // Python divides Decimal(int) at precision 32 before converting to float.
    // Snapshot quantities are signed-64: the exact quotient has <= 30 digits.
    format!(
        "{}{}.{:011}",
        if amount < 0 { "-" } else { "" },
        amount.abs() / 100_000_000_000,
        amount.abs() % 100_000_000_000
    )
    .parse()
    .unwrap()
}
pub(super) fn compile(
    pairs: &[Value],
    context: &Context,
    snapshot: &Snapshot,
) -> Result<Value, String> {
    let pairs: Vec<&Value> = pairs.iter().collect();
    compile_with_proofs(
        &pairs,
        context,
        snapshot,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
}
pub(super) fn compile_with_proofs(
    pairs: &[&Value],
    context: &Context,
    snapshot: &Snapshot,
    proofs: &BTreeMap<(String, String), NativeTransitionProof>,
    exchange_proofs: &exchange::Proofs,
) -> Result<Value, String> {
    if pairs.is_empty() {
        return compile_with_scopes(
            pairs,
            context,
            &BTreeMap::new(),
            &BTreeMap::new(),
            proofs,
            exchange_proofs,
        );
    }
    let endpoints: BTreeSet<_> = pairs
        .iter()
        .flat_map(|pair| [anchor(&pair["out"]), anchor(&pair["in"])])
        .collect();
    // The snapshot records hold the rows' fields and share their scopes.
    let scopes = snapshot
        .transactions
        .iter()
        .filter(|r| endpoints.contains(&anchor(*r)))
        .filter_map(|r| Some((anchor(r), crate::fields::Row::onchain_scope(r)?)))
        .collect();
    compile_with_scopes(
        pairs,
        context,
        &snapshot
            .wallets
            .iter()
            .map(|w| (text(&w["id"]), json!(w)))
            .collect(),
        &scopes,
        proofs,
        exchange_proofs,
    )
}
pub(super) fn compile_with_scopes(
    pairs: &[&Value],
    context: &Context,
    wallets: &BTreeMap<String, Value>,
    scopes: &BTreeMap<String, (String, String, String, String)>,
    proofs: &BTreeMap<(String, String), NativeTransitionProof>,
    exchange_proofs: &exchange::Proofs,
) -> Result<Value, String> {
    let _span = crate::timing::span("interpretation.pairs");
    let fanout = exact_fanout(pairs, context, scopes)?;
    let mut ordered: Vec<&Value> = pairs.to_vec();
    ordered.sort_by_key(|p| {
        (
            text(&p["out"]["occurred_at"]),
            text(&p["out"]["id"]),
            text(&p["in"]["id"]),
            text(&p["pair_id"]),
        )
    });
    let last: BTreeMap<_, _> = ordered
        .iter()
        .enumerate()
        .map(|(i, p)| (anchor(&p["out"]), i))
        .collect();
    let mut sources = BTreeMap::new();
    let mut targets = BTreeMap::new();
    let mut implicit = BTreeMap::new();
    let mut claims = Vec::new();
    let mut blocked = BTreeSet::new();
    let mut quarantines = Vec::new();
    for (ordinal, pair) in ordered.into_iter().enumerate() {
        let out = &pair["out"];
        let incoming = &pair["in"];
        let (out_id, in_id) = (anchor(out), anchor(incoming));
        if out_id.is_empty()
            || in_id.is_empty()
            || context.excluded.contains(&out_id)
            || context.excluded.contains(&in_id)
        {
            continue;
        }
        let (Some(source), Some(target)) = (
            context.observations.get(&out_id),
            context.observations.get(&in_id),
        ) else {
            continue;
        };
        if source["direction"] != "outbound" || target["direction"] != "inbound" {
            continue;
        }
        let name = source_name(pair);
        let exact_transition = name == "native_htlc_transition"
            && proofs
                .get(&(out_id.clone(), in_id.clone()))
                .is_some_and(|p| p.validates(pair, source, target));
        let exact_exchange = name == exchange::SOURCE
            && exchange_proofs
                .get(&(out_id.clone(), in_id.clone()))
                .is_some_and(|p| p.validates(pair, source, target));
        let exact_native = recorded::exact_native(source, target);
        let exact_recorded = recorded::exact_recorded(pair, source, target);
        let exact_fanout = name == "recorded_fanout" && fanout.contains(&text(&pair["group_id"]));
        let chain_candidate = matches!(name.as_str(), "row_matched" | "recorded_fanout")
            && source["event_key"] == target["event_key"]
            && source["event_key"]["native_namespace"] == "chain"
            && matches!(
                source["event_key"]["chain"].as_str(),
                Some("bitcoin" | "liquid")
            );
        if chain_candidate
            && source["authoritative_chain_observation"]
                != target["authoritative_chain_observation"]
        {
            let (id, row, observation, authoritative) =
                if truth(&source["authoritative_chain_observation"]) {
                    (&in_id, incoming, target, &out_id)
                } else {
                    (&out_id, out, source, &in_id)
                };
            quarantines.push(quarantine(row,id,"recorded_transfer_authority_conflict",&json!({"pairing_source":name,"authoritative_transaction_id":authoritative,"observation_authority_reason":observation["observation_authority_reason"],"required_for":"authoritative_same_event_wallet_observation"}))?);
            blocked.insert(id.clone());
            continue;
        }
        if name != "channel_lifecycle"
            && !exact_recorded
            && !exact_fanout
            && !exact_native
            && !exact_transition
            && !exact_exchange
        {
            continue;
        }
        let source_when = time::instant(&source["occurred_at"]).map_err(|e| e.code.to_owned())?;
        let target_when = time::instant(&target["occurred_at"]).map_err(|e| e.code.to_owned())?;
        if !exact_native
            && source_when
                .zip(target_when)
                .map(|(s, t)| time::skewed(t).map(|t| s > t))
                .transpose()
                .map_err(|e| e.code.to_owned())?
                .unwrap_or(false)
        {
            block_pair(
                pair,
                "transfer_pair_chronology_mismatch",
                json!({"source_occurred_at":source["occurred_at"],"target_occurred_at":target["occurred_at"],"maximum_clock_skew_seconds":172800,"required_for":"chronological_custody_continuity"}),
                &mut blocked,
                &mut quarantines,
            )?;
            continue;
        }
        let sh = text(&source["quantity_hash"]);
        let th = text(&target["quantity_hash"]);
        let ss = *sources.get(&sh).unwrap_or(&0);
        let ts = *targets.get(&th).unwrap_or(&0);
        let available_source = principal(source) - ss;
        let available_target = principal(target) - ts;
        let transition_kind = matches!(
            text(&pair["kind"]).as_str(),
            "chain-swap"
                | "peg-in"
                | "peg-out"
                | "reverse-submarine-swap"
                | "submarine-swap"
                | "swap-refund"
        );
        let requested = if absent(requested(pair)) {
            if transition_kind {
                available_source
            } else {
                available_source.min(available_target)
            }
        } else {
            integer(requested(pair))?
        };
        let amount = requested.min(available_source).min(available_target);
        if amount <= 0 {
            continue;
        }
        let pair_id = if truth(&pair["pair_id"]) {
            text(&pair["pair_id"])
        } else {
            format!("{out_id}:{in_id}:{ordinal}")
        };
        let bundle = if truth(&pair["group_id"]) {
            format!("pair-group:{}", text(&pair["group_id"]))
        } else {
            format!("pair:{pair_id}")
        };
        if exposure(source) != exposure(target) {
            continue;
        }
        let sn = recorded::domain_name(source["event_key"]["network"].as_str().unwrap_or(""));
        let tn = recorded::domain_name(target["event_key"]["network"].as_str().unwrap_or(""));
        if sn != tn {
            block_pair(
                pair,
                "transfer_network_mismatch",
                json!({"source_network":sn,"target_network":tn,"required_for":"compatible_custody_domains"}),
                &mut blocked,
                &mut quarantines,
            )?;
            continue;
        }
        let cross_rail = source["event_key"]["chain"] != target["event_key"]["chain"];
        if cross_rail && text(&pair["policy"]) != "carrying-value" && !exact_transition {
            continue;
        }
        let fee = (requested.min(available_source) - amount).max(0);
        let unclaimed = available_source - amount - fee;
        let tolerance = (principal(source) / 100).max(2_500_000);
        if !exact_native
            && !truth(&pair["allow_unclaimed_residual"])
            && last.get(&out_id) == Some(&ordinal)
            && unclaimed > tolerance
        {
            let label = |o: &Value| {
                wallets
                    .get(&text(&o["wallet_id"]))
                    .and_then(|w| w.get("label"))
                    .cloned()
                    .unwrap_or_else(|| o["wallet_id"].clone())
            };
            block_pair(
                pair,
                "transfer_fee_implausible",
                json!({"from_wallet":label(source),"to_wallet":label(target),"sent":btc(principal(source)),"received":btc(amount),"implied_fee":btc(unclaimed),"fee_ceiling":btc(tolerance),"required_for":"complete_transfer_component"}),
                &mut blocked,
                &mut quarantines,
            )?;
            let mut hold = claim(
                format!("pair:{pair_id}:implausible-fee"),
                slice(&source["quantity_hash"], ss, principal(source)),
                "custody_suspense",
                true,
                "transfer_fee_implausible",
            );
            hold["atomic_bundle_id"] = json!(bundle);
            claims.push(hold);
            sources.insert(sh, principal(source));
            continue;
        }
        let mut movement = claim(
            format!("pair:{pair_id}:slice:{ss}:{amount}"),
            slice(&source["quantity_hash"], ss, ss + amount),
            "internal_verified",
            true,
            if name.is_empty() {
                "row_matched"
            } else {
                &name
            },
        );
        movement["target"] = slice(&target["quantity_hash"], ts, ts + amount);
        movement["atomic_bundle_id"] = json!(bundle);
        movement["allow_cross_rail"] = json!(cross_rail);
        for key in ["kind", "policy", "component_id"] {
            let value = text(&pair[key]);
            if !value.is_empty() {
                movement[match key {
                    "kind" => "transfer_kind",
                    "policy" => "transfer_policy",
                    _ => "component_id",
                }] = json!(value);
            }
        }
        claims.push(movement);
        sources.insert(sh.clone(), ss + amount);
        targets.insert(th, ts + amount);
        if source["fee_attribution"] == "implicit_wallet_delta" {
            implicit.insert(sh.clone(), source);
        }
        if fee > 0 && source["fee_attribution"] != "implicit_wallet_delta" {
            let start = ss + amount;
            let mut loss = claim(
                format!("pair:{pair_id}:fee:{start}:{fee}"),
                slice(&source["quantity_hash"], start, start + fee),
                "external_confirmed",
                true,
                "verified_transfer_fee",
            );
            loss["atomic_bundle_id"] = json!(bundle);
            loss["destination_kind"] = json!("fee");
            claims.push(loss);
            sources.insert(sh, start + fee);
        }
    }
    for (hash, source) in implicit {
        let start = *sources.get(&hash).unwrap_or(&0);
        if start < principal(source) {
            claims.push(claim(
                format!("implicit-wallet-delta:{hash}:{start}"),
                slice(&source["quantity_hash"], start, principal(source)),
                "custody_suspense",
                false,
                "implicit_wallet_delta_unallocated",
            ));
        }
    }
    // The lists move into the result; `json!` would copy them.
    let mut result = json!({"blocked_transaction_ids":blocked});
    result["claims"] = Value::Array(claims);
    result["quarantines"] = Value::Array(quarantines);
    Ok(result)
}
fn sum(a: &Value, b: &Value) -> Value {
    use kassiber_pyjson::BigInt;
    let a: BigInt = a.to_string().parse().expect("converted integer");
    let b: BigInt = b.to_string().parse().expect("converted integer");
    serde_json::from_str(&(a + b).to_string()).expect("integer sum")
}
pub(super) fn native_audits<P: std::borrow::Borrow<Value>>(
    pairs: &[P],
    context: &Context,
) -> Result<Vec<Value>, String> {
    let mut audits = Vec::new();
    for pair in pairs {
        let pair = pair.borrow();
        let name = source_name(pair);
        if !matches!(
            name.as_str(),
            "ownership_derived"
                | "multi_source_consolidation"
                | "recorded_fanout"
                | "channel_lifecycle"
        ) {
            continue;
        }
        let out = &pair["out"];
        let incoming = &pair["in"];
        let (oa, ia) = (anchor(out), anchor(incoming));
        let (oi, ii) = (text(&out["id"]), text(&incoming["id"]));
        if name == "channel_lifecycle"
            && !context.observations.contains_key(&oi)
            && context
                .observations
                .get(&ia)
                .is_some_and(|o| o["direction"] == "inbound")
        {
            let target = &context.observations[&ia];
            let received = super::integer_or_zero(&incoming["amount"])?;
            let fee = super::integer_or_zero(&out["fee"])?;
            if integer(&received)? > 0 {
                audits.push(json!({"pairing_source":name,"virtual_source":true,"out_id":oi,"in_id":ii,"out_anchor_transaction_id":oi,"in_anchor_transaction_id":ia,"from_wallet_id":text(&out["wallet_id"]),"to_wallet_id":target["wallet_id"],"asset":target["asset"],"occurred_at":target["occurred_at"],"crypto_received_msat":received,"crypto_fee_msat":fee,"crypto_sent_msat":sum(&received, &fee)}));
            }
            continue;
        }
        let Some(source) = context
            .observations
            .get(&oa)
            .filter(|_| !context.excluded.contains(&oa))
        else {
            continue;
        };
        if name != "channel_lifecycle" && !truth(&source["authoritative_chain_observation"]) {
            continue;
        }
        if source["direction"] != "outbound"
            || context.observations.contains_key(&ii)
            || (ia != oa && context.observations.contains_key(&ia))
        {
            continue;
        }
        let received = super::integer_or_zero(&incoming["amount"])?;
        let fee = super::integer_or_zero(&out["fee"])?;
        if integer(&received)? <= 0 {
            continue;
        }
        use crate::python_text::PythonText;
        let asset = if truth(&incoming["asset"]) {
            text(&incoming["asset"])
        } else {
            text(&source["asset"])
        }
        .py_upper();
        audits.push(json!({"pairing_source":name,"out_id":oi,"in_id":ii,"out_anchor_transaction_id":oa,"in_anchor_transaction_id":ia,"from_wallet_id":source["wallet_id"],"to_wallet_id":text(&incoming["wallet_id"]),"asset":asset,"occurred_at":source["occurred_at"],"crypto_received_msat":received,"crypto_fee_msat":fee,"crypto_sent_msat":sum(&received, &fee)}));
    }
    Ok(audits)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(case: &str) {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_pair_cases.json"
        ))
        .unwrap();
        let vector = vectors.iter().find(|v| v["case"] == case).unwrap();
        let snapshot: Snapshot = serde_json::from_value(vector["input"].clone()).unwrap();
        snapshot.validate().unwrap();
        let mut context = Context::load(&snapshot).unwrap();
        context.excluded = super::super::array(&vector["excluded"])
            .iter()
            .map(text)
            .collect();
        let pairs = super::super::array(&vector["pairs"]);
        let scopes = snapshot
            .transactions
            .iter()
            .filter_map(|r| {
                let r = json!(r);
                Some((anchor(&r), crate::graph::onchain_scope(&r)?))
            })
            .collect();
        let wallets = snapshot
            .wallets
            .iter()
            .map(|w| (text(&w["id"]), json!(w)))
            .collect();
        let proofs = super::super::array(&vector["proofs"])
            .iter()
            .map(|p| {
                (
                    (text(&p["out_id"]), text(&p["in_id"])),
                    serde_json::from_value(p["proof"].clone()).unwrap(),
                )
            })
            .collect();
        let pairs: &[&Value] = &pairs.iter().collect::<Vec<_>>();
        let compiled = compile_with_scopes(
            pairs,
            &context,
            &wallets,
            &scopes,
            &proofs,
            &BTreeMap::new(),
        )
        .unwrap_or_else(|code| json!({"error_code":code}));
        assert_eq!(compiled, vector["expected"], "{case}");
        assert_eq!(
            exact_fanout(pairs, &context, &scopes)
                .map(|v| json!(v))
                .unwrap_or_else(|code| json!({"error_code":code})),
            vector["fanout_groups"],
            "{case} fanout"
        );
        assert_eq!(
            native_audits(pairs, &context)
                .map(|v| json!(v))
                .unwrap_or_else(|code| json!({"error_code":code})),
            vector["native_audits"],
            "{case} audits"
        );
    }
    macro_rules! cases {
        ($($name:ident => $case:literal),* $(,)?) => { $(#[test] fn $name() { check($case); })* };
    }
    cases! {
        pair_audit_raw_strings => "audit_raw_strings",
        pair_audit_raw_huge => "audit_raw_huge",
        pair_audit_bad_amount => "audit_bad_amount",
        pair_audit_bad_fee => "audit_bad_fee",
        pair_audit_null_fee => "audit_null_fee",
        pair_audit_skipped_bad_amount => "audit_skipped_bad_amount",
        pair_audit_virtual_raw => "audit_virtual_raw",
        pair_fanout_raw_string => "fanout_raw_string",
        pair_fanout_bad_amount => "fanout_bad_amount",
        pair_fanout_conversion_order => "fanout_conversion_order",
        pair_fanout_wallet_admission_order => "fanout_wallet_admission_order",
        pair_fanout_target_admission_order => "fanout_target_admission_order",
        pair_fanout_group_admission_order => "fanout_group_admission_order",
        pair_implicit_source_order => "implicit_source_order",
        pair_ordinary => "ordinary",
        pair_source_fallback => "source_fallback",
        pair_forged_label => "forged_label",
        pair_excluded_source => "excluded_source",
        pair_excluded_target => "excluded_target",
        pair_rowless => "rowless",
        pair_rowless_untrusted => "rowless_untrusted",
        pair_rowless_reserved => "rowless_reserved",
        pair_observed_target => "observed_target",
        pair_virtual_channel_source => "virtual_channel_source",
        pair_wrong_direction => "wrong_direction",
        pair_empty_anchor => "empty_anchor",
        pair_chronology => "chronology",
        pair_skew_boundary => "skew_boundary",
        pair_bad_timestamp => "bad_timestamp",
        pair_time_overflow => "time_overflow",
        pair_network_mismatch => "network_mismatch",
        pair_cross_rail => "cross_rail",
        pair_cross_rail_unreviewed => "cross_rail_unreviewed",
        pair_unlike_exposure => "unlike_exposure",
        pair_explicit_fee => "explicit_fee",
        pair_implicit_fee => "implicit_fee",
        pair_implausible_fee => "implausible_fee",
        pair_allowed_residual => "allowed_residual",
        pair_fee_floor_boundary => "fee_floor_boundary",
        pair_fee_percent_boundary => "fee_percent_boundary",
        pair_fee_percent_above => "fee_percent_above",
        pair_request_zero => "request_zero",
        pair_request_negative => "request_negative",
        pair_request_unicode => "request_unicode",
        pair_request_invalid => "request_invalid",
        pair_request_float => "request_float",
        pair_request_huge => "request_huge",
        pair_request_huge_negative => "request_huge_negative",
        pair_request_type_error => "request_type_error",
        pair_request_false => "request_false",
        pair_request_precedence => "request_precedence",
        pair_request_fallback => "request_fallback",
        pair_exact_msat => "exact_msat",
        pair_metadata => "metadata",
        pair_pair_order => "pair_order",
        pair_source_cursor => "source_cursor",
        pair_target_cursor => "target_cursor",
        pair_same_native_skew => "same_native_skew",
        pair_forged_native_transition => "forged_native_transition",
        pair_fanout => "fanout",
        pair_fanout_forged => "fanout_forged",
        pair_fanout_duplicate_wallet => "fanout_duplicate_wallet",
        pair_fanout_duplicate_target => "fanout_duplicate_target",
        pair_fanout_amount => "fanout_amount",
        pair_fanout_nonconserving => "fanout_nonconserving",
        pair_rowless_alias => "rowless_alias",
        pair_virtual_channel_excluded => "virtual_channel_excluded",
        pair_rowless_target_alias => "rowless_target_alias",
        pair_proof_exact => "proof_exact",
        pair_proof_refund => "proof_refund",
        pair_proof_confidence => "proof_confidence",
        pair_proof_conflict => "proof_conflict",
        pair_proof_method => "proof_method",
        pair_proof_source_hash => "proof_source_hash",
        pair_proof_target_hash => "proof_target_hash",
        pair_proof_kind => "proof_kind",
        pair_proof_policy => "proof_policy",
        pair_proof_amount => "proof_amount",
        pair_proof_float_amount => "proof_float_amount",
        pair_proof_string_amount => "proof_string_amount",
    }
}
