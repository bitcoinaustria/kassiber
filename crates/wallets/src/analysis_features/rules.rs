use super::{big, BigInt, Value, RULE_VERSION};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

struct Rules<'a> {
    observed: BTreeMap<&'a str, &'a Value>,
    findings: Vec<Value>,
}
impl Rules<'_> {
    fn value(&self, code: &str) -> &Value {
        self.observed
            .get(code)
            .map_or(&Value::Null, |r| &r["value"])
    }
    fn add(
        &mut self,
        code: &str,
        keys: &[&str],
        message: &str,
        authority: &str,
        assumptions: &[&str],
        contradictions: &[&str],
    ) {
        if keys.iter().any(|k| !self.observed.contains_key(k)) {
            return;
        }
        let evidence: Vec<_> = keys
            .iter()
            .flat_map(|k| {
                self.observed[k]["evidence"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .cloned()
            })
            .collect();
        self.findings.push(json!({"code":code,"rule_version":RULE_VERSION,"severity":"info","authority":authority,"message":message,"feature_codes":keys,"evidence":evidence,"assumptions":assumptions,"contradictions":contradictions}));
    }
}

/// Evaluate only evidence-bound categories. Collaboration suppresses incompatible
/// ownership hypotheses; findings themselves never confer accounting authority.
pub fn evaluate(snapshot: &Value, collaboration: Option<&Value>) -> Vec<Value> {
    let mut r = Rules {
        observed: BTreeMap::new(),
        findings: Vec::new(),
    };
    for row in snapshot["features"].as_array().into_iter().flatten() {
        if matches!(row["availability"].as_str(), Some("observed" | "partial")) {
            if let Some(code) = row["code"].as_str() {
                r.observed.insert(code, row);
            }
        }
    }
    let version = r.value("transaction_version");
    if !version.is_null() && version != 1 && version != 2 {
        r.add(
            "unusual_transaction_version",
            &["transaction_version"],
            "The transaction uses an uncommon version; this does not identify a wallet.",
            "observed",
            &[],
            &[],
        );
    }
    let lock = r.value("absolute_locktime");
    if crate::descriptors::truthy(&lock["value"]) && lock["enabled"] == false {
        r.add(
            "ineffective_absolute_locktime",
            &["absolute_locktime", "sequences"],
            "All input sequences are final, so the nonzero absolute locktime is disabled.",
            "observed",
            &[],
            &[],
        );
    }
    if crate::descriptors::truthy(&r.value("sequences")["signals_bip125"]) {
        r.add("explicit_rbf_signal",&["sequences"],"At least one input explicitly signals BIP125 replacement; relay and replacement depend on node policy.","observed",&[],&[]);
    }
    if crate::descriptors::truthy(&r.value("output_script_types")["counts"]["op_return"]) {
        r.add("op_return_output",&["output_script_types"],"An observed output carries an OP_RETURN script; the payload is not included in analysis.","observed",&[],&[]);
    }
    let rate = r.value("fee_rate");
    if let (Some(fee), Some(size)) = (
        big(&rate["fee_sats"]),
        big(&rate["vsize"]).filter(|n| *n > BigInt::from(0)),
    ) {
        let rounded: BigInt = (&fee + &size / 2_u32) / &size;
        let delta = &fee - &rounded * &size;
        let delta = if delta < BigInt::from(0) {
            -delta
        } else {
            delta
        };
        if [1, 2, 3, 5, 10, 15, 20, 25, 50, 100]
            .into_iter()
            .any(|n| rounded == BigInt::from(n))
            && delta * 100_u32 < size
            && collaboration.is_none()
        {
            r.add("rounded_fee_rate",&["fee_rate"],"The observed fee rate is close to a common integer rate; many wallets and manual settings share it.","hypothesis",&["fee_rate_is_not_wallet_identity"],&["manual_fee_selection","shared_node_estimator"]);
        }
    }
    if crate::descriptors::truthy(&r.value("sequences")["relative_locks"]) {
        r.add("relative_lock_constraints",&["sequences","transaction_version"],"Input sequences encode relative lock constraints; maturity needs prevout confirmation context.","observed",&[],&[]);
    }
    for side in ["input", "output"] {
        let code = format!("{side}_script_types");
        let known = r.value(&code)["counts"].as_object().map_or(0, |m| {
            m.keys()
                .filter(|k| !matches!(k.as_str(), "unknown" | "op_return"))
                .count()
        });
        if known > 1 {
            r.add(&format!("mixed_{side}_script_types"),&[&code],&format!("The {side} scripts use multiple templates. This is a structural distinction, not an ownership link."),"observed",&[],&[]);
        }
    }
    let equal = crate::descriptors::truthy(&r.value("output_value_pattern")["equal_groups"]);
    if equal {
        r.add("equal_output_groups",&["output_value_pattern"],"Repeated output amounts provide a CoinJoin hypothesis, but also occur in ordinary batches.","hypothesis",&["protocol_cannot_be_established_from_equal_amounts"],&["ordinary_batch_payments"]);
    }
    let card = r.value("cardinality");
    let ins = card["inputs"].as_u64().unwrap_or(0);
    let outs = card["outputs"].as_u64().unwrap_or(0);
    if ins >= 3 && outs == 1 && collaboration.is_none() {
        r.add("consolidation_shape",&["cardinality"],"Several inputs create one output, consistent with consolidation or a jointly funded payment.","hypothesis",&["single_owner_not_established"],&["collaborative_payment"]);
    }
    if outs >= 4 && !equal && collaboration.is_none() {
        r.add(
            "batch_payment_shape",
            &["cardinality"],
            "Many outputs are consistent with a payment batch; output roles remain unknown.",
            "hypothesis",
            &[],
            &["coinjoin_or_multiple_owners"],
        );
    }
    let sigs = r.value("signature_encodings")["observations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let ecdsa: Vec<_> = sigs.iter().filter(|s| s["algorithm"] == "ecdsa").collect();
    if ecdsa.len() >= 2
        && ecdsa
            .iter()
            .all(|s| crate::descriptors::truthy(&s["low_r"]))
    {
        r.add("consistent_low_r_encoding",&["signature_encodings"],"All observed ECDSA signatures use low-R encodings. Several wallets share this behavior and chance can produce it.","hypothesis",&["observed_signatures_only"],&["random_low_r_signatures","multiple_wallets_share_encoding"]);
    }
    let defaults: BTreeSet<_> = [0_u64, 1].into_iter().collect();
    if sigs
        .iter()
        .any(|s| !s["sighash"].as_u64().is_some_and(|n| defaults.contains(&n)))
    {
        r.add("nondefault_sighash",&["signature_encodings"],"An observed signature uses a sighash other than DEFAULT or ALL; its commitment scope differs.","observed",&[],&[]);
    }
    r.findings
}
