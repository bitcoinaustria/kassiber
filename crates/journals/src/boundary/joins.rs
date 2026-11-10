use crate::integer::wide_or_zero as int;
use crate::*;
use kassiber_custody::journal_support as graph;
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap},
};
fn legacy_txid(v: &Value) -> String {
    let text = if truth(v) { string(v) } else { String::new() };
    if text.len() == 64 && text.bytes().all(|c| c.is_ascii_hexdigit()) {
        text.to_lowercase()
    } else {
        text
    }
}
/// `observations` and `postings` are the quantity projection's, as JSON
/// objects or views of them.
pub(super) fn differences(
    c: &super::RenderOperands<'_>,
    observations: &[ObservationRef<'_>],
    postings: &[PostingRef<'_>],
    holdings: &Holdings,
) -> Result<Value, Refusal> {
    fn text(v: &Value) -> Cow<'_, str> {
        v.as_str()
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(kassiber_custody::codec::text(v)))
    }
    fn current_text(v: &Value) -> Cow<'_, str> {
        v.as_str()
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(string(v)))
    }
    let mut current = BTreeMap::new();
    for (key, value) in &holdings.pairs {
        let amount = crate::render::msat(&value.quantity.0)?;
        let amount = amount
            .to_string()
            .parse::<BigInt>()
            .map_err(|_| Refusal::from("ValueError".to_owned()))?;
        current.insert((current_text(&key[0]), current_text(&key[3])), amount);
    }
    let mut reasons = json!({});
    for leg in c.loan_legs {
        if !leg["transaction_id"].is_null() {
            reasons[string(&leg["transaction_id"])] =
                json!(format!("loan_{}_non_event", string(&leg["role"])));
        }
    }
    if let Some(roles) = c.channel_roles.as_object() {
        for (id, role) in roles {
            reasons[id] = json!(format!("lightning_{}_non_event", string(role)));
        }
    }
    for component in c.components {
        if int(&component["validation"]["suspense_msat"])? <= BigInt::from(0) {
            continue;
        }
        for leg in array(&component["legs"]) {
            if leg["role"] == "source" && truth(&leg["transaction_id"]) {
                reasons[string(&leg["transaction_id"])] =
                    json!("custody_component_residual_suspense");
            }
        }
    }
    // Borrow identities rather than building an intermediate JSON current-balance
    // table and cloning every observation hash. Only named non-event reasons
    // need the observation join; postings remain authoritative for quantities.
    let observations: HashMap<_, _> = if reasons.as_object().is_some_and(|m| !m.is_empty()) {
        observations
            .iter()
            .map(|o| (text(o.at("quantity_hash")), o))
            .collect()
    } else {
        HashMap::new()
    };
    let mut canonical = BTreeMap::<_, BalanceSum>::new();
    let mut named = BTreeMap::<_, BTreeSet<String>>::new();
    for posting in postings {
        if *posting.at("location_kind") != "wallet" {
            continue;
        }
        let key = (text(posting.at("location_id")), text(posting.at("asset")));
        canonical
            .entry(key.clone())
            .or_default()
            .add(posting.at("amount_msat"))?;
        if let Some(observation) = (!observations.is_empty())
            .then(|| observations.get(&text(posting.at("observation_hash"))))
            .flatten()
        {
            let reason = &reasons[text(observation.at("transaction_id")).as_ref()];
            if truth(reason) {
                named.entry(key).or_default().insert(string(reason));
            }
        }
    }
    let keys: BTreeSet<_> = canonical.keys().chain(current.keys()).cloned().collect();
    let mut differences = vec![];
    for (wallet, asset) in keys {
        let key = (wallet.clone(), asset.clone());
        let a = canonical
            .get(&key)
            .map(BalanceSum::value)
            .unwrap_or_default();
        let b = current.get(&key).cloned().unwrap_or_default();
        if a == b {
            continue;
        }
        let reason = named
            .get(&key)
            .filter(|r| r.len() == 1)
            .and_then(|r| r.first());
        differences.push(
            json!({"$type":"WalletBalanceDifference", "wallet_id":wallet,
            "asset":asset, "canonical_msat":number(&a.to_string())?,
            "current_msat":number(&b.to_string())?, "reason":reason}),
        );
    }
    Ok(json!(differences))
}
// Most wallet totals fit i128. Promote before overflow, retaining arbitrary
// precision for historical quantities without allocating a BigInt per posting.
#[derive(Default)]
struct BalanceSum {
    small: i128,
    wide: Option<BigInt>,
}
impl BalanceSum {
    fn add(&mut self, value: &Value) -> Result<(), Refusal> {
        let small = value
            .as_i64()
            .map(i128::from)
            .or_else(|| value.as_u64().map(i128::from));
        if self.wide.is_none() {
            if let Some(n) = small.and_then(|n| self.small.checked_add(n)) {
                self.small = n;
                return Ok(());
            }
        }
        let amount = if let Some(n) = small {
            BigInt::from(n)
        } else {
            let s = value.to_string();
            if !value.is_number() || s.contains(['.', 'e', 'E']) {
                return Err(Refusal::from("ValueError".to_owned()));
            }
            s.parse::<BigInt>()
                .map_err(|_| Refusal::from("ValueError".to_owned()))?
        };
        *self.wide.get_or_insert_with(|| BigInt::from(self.small)) += amount;
        Ok(())
    }
    fn value(&self) -> BigInt {
        self.wide
            .clone()
            .unwrap_or_else(|| BigInt::from(self.small))
    }
}
fn downstream(reason: &str) -> bool {
    matches!(
        reason,
        "custody_basis_barrier"
            | "transfer_pair_dependency_blocked"
            | "basis_provenance_incomplete"
            | "derived_transfer_group_blocked"
            | "at_swap_basis_carry_unresolved"
            | "bitcoin_rail_carry_basis_unresolved"
    )
}
fn pairable(reason: &str) -> bool {
    matches!(
        reason,
        "owned_fanout_unresolved"
            | "ownership_transfer_destination_ambiguous"
            | "ownership_transfer_source_ambiguous"
    )
}
fn graph_refusal(error: String, script: &Value) -> Refusal {
    match error.as_str() {
        "OverflowError" => {
            Refusal::new("OverflowError", "cannot convert float infinity to integer")
        }
        "AttributeError" => {
            let kind = match script {
                Value::Null => "NoneType",
                Value::Bool(_) => "bool",
                Value::Array(_) => "list",
                Value::Object(_) => "dict",
                Value::Number(n) if n.to_string().contains(['.', 'e', 'E']) => "float",
                Value::Number(_) => "int",
                Value::String(_) => "str",
            };
            Refusal::new(
                "AttributeError",
                format!("'{kind}' object has no attribute 'lower'"),
            )
        }
        _ => Refusal::from(error),
    }
}
pub(crate) fn ownership_review(
    c: &super::RenderOperands<'_>,
    qs: &[QuarantineRow],
) -> Result<(Value, Value), Refusal> {
    struct Memos;
    impl Drop for Memos {
        fn drop(&mut self) {
            graph::clear_memos();
        }
    }
    let _memos = Memos;
    let mut blocked = BTreeMap::new();
    for q in qs {
        let entry = blocked
            .entry(string(&q.transaction_id))
            .or_insert_with(|| q.reason.clone());
        if downstream(entry) && !downstream(&q.reason) {
            *entry = q.reason.clone();
        }
    }
    if blocked.is_empty() {
        return Ok((json!({"total":0,"by_reason":{}}), json!([])));
    }
    let rows = c.rows;
    if !blocked.values().any(|reason| pairable(reason)) {
        // Python still converts every inbound amount before selecting sources.
        // Retain those conversions even when no quarantine can produce a card.
        for row in rows {
            if *field(row, "direction") == "inbound" {
                let amount = field(row, "amount");
                if amount.as_i64().is_none() && amount.as_u64().is_none() {
                    let _ = int(amount)?;
                }
            }
        }
        return Ok((json!({"total":0,"by_reason":{}}), json!([])));
    }
    let by_id: HashMap<_, _> = rows.iter().map(|r| (string(&r["id"]), r)).collect();
    let mut active = BTreeSet::new();
    let mut unavailable = BTreeSet::new();
    for component in c.components {
        let legs: HashMap<_, _> = array(&component["legs"])
            .iter()
            .map(|l| (string(&l["id"]), l))
            .collect();
        for a in array(&component["allocations"]) {
            let source = legs
                .get(&string(&a["source_leg_id"]))
                .copied()
                .unwrap_or(&Value::Null);
            let sink = legs
                .get(&string(&a["sink_leg_id"]))
                .copied()
                .unwrap_or(&Value::Null);
            let out = fallback(&source["anchor_transaction_id"], &source["transaction_id"]);
            let inbound = fallback(&sink["anchor_transaction_id"], &sink["transaction_id"]);
            if out.is_null() || out == "" {
                continue;
            }
            let out = string(out);
            let inbound = if inbound.is_null() || inbound == "" {
                String::new()
            } else {
                string(inbound)
            };
            active.insert((out.clone(), inbound.clone()));
            if inbound.is_empty() {
                unavailable.insert(out);
                continue;
            }
            let o = by_id.get(&out).copied().unwrap_or(&Value::Null);
            let i = by_id.get(&inbound).copied().unwrap_or(&Value::Null);
            let kind = component["component_type"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("manual");
            let policy = component["conversion_policy"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("carrying-value");
            if o.is_null()
                || i.is_null()
                || string(&o["asset"]).to_uppercase() != string(&i["asset"]).to_uppercase()
                || policy != "carrying-value"
                || !matches!(kind, "manual" | "coinjoin" | "whirlpool")
            {
                unavailable.insert(out);
                unavailable.insert(inbound);
            }
        }
    }
    let mut inbounds: HashMap<String, Vec<&Value>> = HashMap::new();
    let mut groups: HashMap<String, Vec<&Value>> = HashMap::new();
    // Scopes inspect and decode raw provider payloads. Most quarantines cannot
    // offer ownership cards, so compute scopes only for rows a card examines.
    let mut scopes = HashMap::<usize, Option<graph::Scope>>::new();
    fn row_scope(
        scopes: &mut HashMap<usize, Option<graph::Scope>>,
        row: &Value,
    ) -> Option<graph::Scope> {
        scopes
            .entry(row as *const Value as usize)
            .or_insert_with(|| graph::onchain_scope(row))
            .clone()
    }
    let mut physical_groups_built = false;
    let mut legacy_groups_built = false;
    for r in rows {
        if r["direction"] == "inbound" && int(&r["amount"])? > BigInt::from(0) {
            inbounds.entry(string(&r["wallet_id"])).or_default().push(r);
        }
    }
    let mut candidates = vec![];
    let mut counts = BTreeMap::<String, usize>::new();
    for (id, reason) in blocked {
        if !pairable(&reason) {
            continue;
        }
        let Some(source) = by_id.get(&id).copied() else {
            continue;
        };
        if unavailable.contains(&id)
            || source["direction"] != "outbound"
            || int(&source["amount"])? <= BigInt::from(0)
        {
            continue;
        }
        let parsed = graph::parsed(source)
            .map_err(|error| graph_refusal(error, &Value::Null))?
            .filter(|parsed| {
                array(&parsed["outputs"])
                    .iter()
                    .all(|output| !output["value_sats"].is_null())
            });
        let mut scope = None;
        let mut destinations = BTreeMap::<String, BigInt>::new();
        if let Some(parsed) = &parsed {
            scope = graph::ownership_scope(source, parsed, c.owned_index).map_err(|error| {
                graph_refusal(error, graph::scope_error_operand(parsed, c.owned_index))
            })?;
            let Some(s) = &scope else {
                continue;
            };
            let mut ambiguous = false;
            for output in array(&parsed["outputs"]) {
                let owners = graph::script_owners(
                    &output["script"],
                    &(s.0.clone(), s.1.clone()),
                    c.owned_index,
                )
                .map_err(|error| graph_refusal(error, &output["script"]))?;
                if owners.contains(&string(&source["wallet_id"])) {
                    continue;
                }
                if owners.len() > 1 {
                    ambiguous = true;
                    break;
                }
                if let Some(owner) = owners.first() {
                    *destinations.entry(owner.clone()).or_default() +=
                        int(&output["value_sats"])? * 1000;
                }
            }
            if ambiguous {
                continue;
            }
        } else if reason == "owned_fanout_unresolved" {
            scope = row_scope(&mut scopes, source);
            if scope.is_some() && !physical_groups_built {
                for row in rows {
                    if let Some(s) = row_scope(&mut scopes, row) {
                        groups
                            .entry(format!("physical:{s:?}"))
                            .or_default()
                            .push(row);
                    }
                }
                physical_groups_built = true;
            }
            let legacy = legacy_txid(&source["external_id"]);
            if scope.is_none() && legacy.is_empty() {
                continue;
            }
            if scope.is_none() && !legacy_groups_built {
                for row in rows {
                    let legacy = legacy_txid(&row["external_id"]);
                    if !legacy.is_empty() {
                        groups
                            .entry(format!(
                                "legacy:{legacy}:{}",
                                string(&row["asset"]).to_uppercase()
                            ))
                            .or_default()
                            .push(row);
                    }
                }
                legacy_groups_built = true;
            }
            let key = scope
                .as_ref()
                .map(|s| format!("physical:{s:?}"))
                .unwrap_or_else(|| {
                    format!(
                        "legacy:{legacy}:{}",
                        string(&source["asset"]).to_uppercase()
                    )
                });
            let group = groups.get(&key).map(Vec::as_slice).unwrap_or(&[]);
            let mut outs = vec![];
            for row in group {
                if row["direction"] == "outbound" && int(&row["amount"])? > BigInt::from(0) {
                    outs.push(row);
                }
            }
            if outs.len() != 1 || outs[0]["id"] != source["id"] {
                continue;
            }
            for inbound in group {
                if inbound["direction"] == "inbound"
                    && int(&inbound["amount"])? > BigInt::from(0)
                    && string(&inbound["wallet_id"]) != string(&source["wallet_id"])
                {
                    *destinations
                        .entry(string(&inbound["wallet_id"]))
                        .or_default() += int(&inbound["amount"])?;
                }
            }
        }
        let txid = parsed
            .as_ref()
            .map(|p| fallback(&p["txid"], &source["external_id"]))
            .unwrap_or(&source["external_id"]);
        let txid = kassiber_pytext::strip(&string(txid)).to_lowercase();
        let scope = scope.or_else(|| row_scope(&mut scopes, source));
        for (wallet, amount) in destinations {
            let mut exact = vec![];
            let mut compatible = vec![];
            for inbound in inbounds.get(&wallet).into_iter().flatten() {
                let iid = string(&inbound["id"]);
                if string(&inbound["asset"]).to_uppercase()
                    != string(&source["asset"]).to_uppercase()
                    || unavailable.contains(&iid)
                    || active.contains(&(id.clone(), iid.clone()))
                {
                    continue;
                }
                let received = int(&inbound["amount"])?;
                let diff = &received - &amount;
                let abs = if diff < BigInt::from(0) { -diff } else { diff };
                let magnitude = if amount < BigInt::from(0) {
                    -amount.clone()
                } else {
                    amount.clone()
                };
                let tolerance = std::cmp::max(magnitude / 100, BigInt::from(2_500_000));
                if abs > tolerance {
                    continue;
                }
                let iscope = row_scope(&mut scopes, inbound);
                if scope.is_some() && iscope.is_some() && scope != iscope {
                    continue;
                }
                if txid.len() == 64
                    && txid.bytes().all(|b| b.is_ascii_hexdigit())
                    && iscope.as_ref().is_some_and(|s| s.2 != txid)
                {
                    continue;
                }
                if parsed.is_some()
                    && reason != "ownership_transfer_source_ambiguous"
                    && scope.is_some()
                    && iscope == scope
                    && int(&source["amount"])? == amount
                    && received == amount
                {
                    exact.push(*inbound);
                } else {
                    compatible.push(*inbound);
                }
            }
            let (selected, confidence) = if exact.is_empty() {
                (compatible, "strong")
            } else {
                (exact, "exact")
            };
            let size = selected.len();
            for inbound in selected {
                candidates.push(candidate(
                    source,
                    inbound,
                    &amount,
                    &reason,
                    &format!("ownership-review:{id}:{wallet}"),
                    confidence,
                    size,
                )?);
                *counts.entry(reason.clone()).or_default() += 1;
            }
        }
    }
    Ok((
        json!({"total":candidates.len(),"by_reason":counts}),
        json!(candidates),
    ))
}
fn candidate(
    out: &Value,
    inbound: &Value,
    amount: &BigInt,
    reason: &str,
    conflict: &str,
    confidence: &str,
    size: usize,
) -> Result<Value, Refusal> {
    let received = int(&inbound["amount"])?;
    let mut v = json!({"out_id":string(&out["id"]),"in_id":string(&inbound["id"]),"out_amount_msat":number(&amount.to_string())?,"in_amount_msat":number(&received.to_string())?,"confidence":confidence,"method":"ownership_graph","swap_fee_msat":number(&(amount-received).to_string())?,"swap_fee_kind":"ownership_graph_delta","default_kind":"manual","default_policy":"carrying-value","conflict_set_id":conflict,"conflict_size":size,"evidence_provider":"ownership_graph","evidence_id":reason,"evidence_kind":"owned_output","evidence_conflicts":[],"evidence_send_amount_msat":null,"evidence_receive_amount_msat":null});
    for (prefix, row) in [("out", out), ("in", inbound)] {
        for key in [
            "asset",
            "wallet_id",
            "wallet_label",
            "wallet_kind",
            "occurred_at",
        ] {
            let value = if key == "wallet_kind" && !truth(&row[key]) {
                String::new()
            } else {
                string(&row[key])
            };
            v[format!("{prefix}_{key}")] = json!(value);
        }
    }
    for k in [
        "evidence_status",
        "evidence_version",
        "evidence_taproot",
        "evidence_cooperative",
        "evidence_spend_path",
    ] {
        v[k] = json!("");
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `differences` reading the observations and postings from `c`.
    fn differences(c: &Value, holdings: &Holdings) -> Result<Value, Refusal> {
        let operands = super::super::RenderOperands::from_json(c);
        let (observations, postings) = operands.quantity_refs();
        super::differences(&operands, &observations, &postings, holdings)
    }

    #[test]
    fn balance_current_keys_preserve_python_stringification() {
        for wallet in [Value::Null, json!(false), json!(0), json!(""), json!("w")] {
            let quantity = "1".parse().unwrap();
            let holdings = Holdings {
                pairs: vec![(
                    vec![wallet.clone(), json!("label"), json!("code"), json!("BTC")],
                    Holding {
                        quantity: JournalDecimal(quantity),
                        cost_basis: JournalDecimal("0".parse().unwrap()),
                    },
                )],
            };
            let c = json!({"quantity_state":{"projection":{"postings":[
                {"location_kind":"wallet", "location_id":wallet, "asset":"BTC", "amount_msat":1}
            ]}}});
            let expected = kassiber_custody::quantity::balances::compare_wallet_balances(
                &c["quantity_state"]["projection"],
                &[json!([string(&wallet), "BTC", 100_000_000_000i64])],
                &json!({}),
            )
            .unwrap();
            assert_eq!(differences(&c, &holdings).unwrap(), expected);
        }
    }

    #[test]
    fn balance_join_matches_existing_comparator_and_promotes_before_overflow() {
        let mut sum = BalanceSum {
            small: i128::MAX,
            wide: None,
        };
        sum.add(&json!(1)).unwrap();
        assert_eq!(sum.value(), BigInt::from(i128::MAX) + BigInt::from(1));
        sum.add(&number("-340282366920938463463374607431768211456").unwrap())
            .unwrap();
        assert_eq!(sum.value(), -(BigInt::from(i128::MAX) + BigInt::from(1)));
        let mut negative = BalanceSum {
            small: i128::MIN,
            wide: None,
        };
        negative.add(&json!(-1)).unwrap();
        assert_eq!(negative.value(), BigInt::from(i128::MIN) - BigInt::from(1));
        let mut unsigned = BalanceSum::default();
        unsigned.add(&json!(u64::MAX)).unwrap();
        assert_eq!(unsigned.value(), BigInt::from(u64::MAX));
        for invalid in [json!(true), json!("12"), json!(1.5), Value::Null] {
            assert_eq!(
                BalanceSum::default().add(&invalid).unwrap_err().message,
                "ValueError"
            );
        }
        let holdings = Holdings { pairs: vec![] };
        for with_reasons in [false, true] {
            let mut c = json!({"quantity_state":{"projection":{"observations":[],"postings":[]}},"loan_legs":[]});
            let projection = &mut c["quantity_state"]["projection"];
            for n in 0..100 {
                projection["observations"].as_array_mut().unwrap().push(
                    json!({"quantity_hash":format!("h{n}"),"transaction_id":format!("t{n}")}),
                );
                projection["postings"].as_array_mut().unwrap().push(json!({"location_kind":if n % 9 == 0 {"outside"} else {"wallet"}, "location_id":format!("w{}", n%3),"asset":"BTC","amount_msat":if n%2==0 {n*999} else {-n*111},"observation_hash":format!("h{n}")}));
            }
            let mut reasons = json!({});
            if with_reasons {
                c["loan_legs"] = json!([{"transaction_id":"t1","role":"principal"},{"transaction_id":"t4","role":"collateral"},{"transaction_id":"t2","role":"interest"}]);
                reasons = json!({"t1":"loan_principal_non_event","t4":"loan_collateral_non_event","t2":"loan_interest_non_event"});
            }
            let expected = kassiber_custody::quantity::balances::compare_wallet_balances(
                &c["quantity_state"]["projection"],
                &[],
                &reasons,
            )
            .unwrap();
            assert_eq!(differences(&c, &holdings).unwrap(), expected);
        }
    }
}
