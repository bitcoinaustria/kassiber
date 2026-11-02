//! Exact conservation and explicit/inferred allocation validation.
use super::scope::*;
use super::*;

fn balances(legs: &[Value], keys: &[&str], amount: &str, skip_missing: bool) -> Vec<Value> {
    let mut buckets: BTreeMap<Vec<String>, BTreeMap<String, BigInt>> = BTreeMap::new();
    for l in legs {
        if (skip_missing && keys.iter().any(|k| l[k].is_null())) || l[amount].is_null() {
            continue;
        }
        let key = keys
            .iter()
            .map(|k| {
                if l[k].is_null() {
                    String::new()
                } else {
                    pystr(&l[k])
                }
            })
            .collect();
        *buckets
            .entry(key)
            .or_default()
            .entry(pystr(&l["role"]))
            .or_default() += big(&l[amount]);
    }
    let mut rows = Vec::new();
    for (key, totals) in buckets {
        let mut row = Map::new();
        for (k, v) in keys.iter().zip(key) {
            row.insert((*k).into(), json!(v));
        }
        let suffix = if amount == "amount_msat" {
            "msat"
        } else {
            "amount"
        };
        let mut sinks = BigInt::from(0);
        for role in [
            "source",
            "destination",
            "fee",
            "external",
            "retained",
            "suspense",
        ] {
            let n = totals.get(role).cloned().unwrap_or_default();
            row.insert(format!("{role}_{suffix}"), number(&n));
            if role != "source" {
                sinks += n;
            }
        }
        row.insert(format!("sink_{suffix}"), number(&sinks));
        row.insert(
            format!("residual_{suffix}"),
            number(&(totals.get("source").cloned().unwrap_or_default() - sinks)),
        );
        rows.push(Value::Object(row));
    }
    rows
}
fn fee_issues(source: &Value, fee: &Value, allocation: Option<&Value>, mode: &str) -> Vec<Value> {
    let mut issues = Vec::new();
    let base = |code: &str| {
        let mut value = json!({"code":code,"source_leg_id":source["id"],"fee_leg_id":fee["id"]});
        if let Some(a) = allocation {
            value["allocation_id"] = a["id"].clone();
        }
        value
    };
    if asset(&source["asset"]) != asset(&fee["asset"]) {
        let mut v = base("fee_source_asset_mismatch");
        v["source_asset"] = source["asset"].clone();
        v["fee_asset"] = fee["asset"].clone();
        issues.push(v);
    }
    if !fee["wallet_id"].is_null()
        && !source["wallet_id"].is_null()
        && pystr(&fee["wallet_id"]) != pystr(&source["wallet_id"])
    {
        let mut v = base("fee_source_wallet_mismatch");
        v["source_wallet_id"] = source["wallet_id"].clone();
        v["fee_wallet_id"] = fee["wallet_id"].clone();
        issues.push(v);
    }
    let ss = network_scope(source);
    let fs = network_scope(fee);
    if !truth(&ss["valid"])
        || !truth(&fs["valid"])
        || ss["rail"] != fs["rail"]
        || (!ss["domain"].is_null() && !fs["domain"].is_null() && ss["domain"] != fs["domain"])
    {
        let mut v = base("fee_source_scope_mismatch");
        v["source_scope"] = ss;
        v["fee_scope"] = fs;
        issues.push(v);
    }
    if let Some(a) = allocation.filter(|_| mode == "conversion") {
        let sa = big(&a["source_amount_msat"]);
        let ta = big(&a["sink_amount_msat"]);
        if sa != ta {
            let mut v = base("conversion_fee_quantity_mismatch");
            v["source_amount_msat"] = number(&sa);
            v["fee_amount_msat"] = number(&ta);
            issues.push(v);
        }
        let sv = &source["valuation_amount"];
        let fv = &fee["valuation_amount"];
        let amount = big(&source["amount_msat"]);
        if !sv.is_null()
            && !fv.is_null()
            && source["valuation_unit"] == fee["valuation_unit"]
            && amount > BigInt::from(0)
            && big(fv) * amount != big(sv) * &sa
        {
            issues.push(base("conversion_fee_valuation_mismatch"));
        }
    }
    issues
}
fn allocations(legs: &[Value], allocs: &[Value], mode: &str) -> Value {
    let sources: Vec<_> = legs
        .iter()
        .filter(|l| l["role"] == "source" && big(&l["amount_msat"]) > BigInt::from(0))
        .collect();
    let sinks: Vec<_> = legs
        .iter()
        .filter(|l| sink(l) && big(&l["amount_msat"]) > BigInt::from(0))
        .collect();
    let mut issues = Vec::new();
    let mut edges = Vec::new();
    let mut source_coverage: BTreeMap<String, BigInt> = BTreeMap::new();
    let mut sink_coverage: BTreeMap<String, BigInt> = BTreeMap::new();
    let mut fee_sources = BTreeSet::new();
    let mut material_sources = BTreeSet::new();
    for a in allocs {
        let endpoints = by_id(legs, &a["source_leg_id"]).zip(by_id(legs, &a["sink_leg_id"]));
        let Some((s, t)) =
            endpoints.filter(|(s, t)| s["role"] == "source" && t["role"] != "source")
        else {
            issues.push(json!({"code":"allocation_leg_invalid","allocation_id":a["id"]}));
            continue;
        };
        let sa = big(&a["source_amount_msat"]);
        let ta = big(&a["sink_amount_msat"]);
        *source_coverage.entry(pystr(&s["id"])).or_default() += &sa;
        *sink_coverage.entry(pystr(&t["id"])).or_default() += &ta;
        if sa > BigInt::from(0) {
            if t["role"] == "fee" {
                fee_sources.insert(pystr(&s["id"]));
            } else {
                material_sources.insert(pystr(&s["id"]));
            }
        }
        if t["role"] == "fee" {
            issues.extend(fee_issues(s, t, Some(a), mode));
        }
        if mode == "quantity"
            && (s["exposure"] != t["exposure"]
                || s["conservation_unit"] != t["conservation_unit"]
                || sa != ta)
        {
            issues.push(json!({"code":"allocation_quantity_mismatch","allocation_id":a["id"],"source_leg_id":s["id"],"sink_leg_id":t["id"]}));
        }
        edges.push(json!({"id":a["id"],"source_leg_id":s["id"],"sink_leg_id":t["id"],"source_amount_msat":number(&sa),"sink_amount_msat":number(&ta)}));
    }
    if !allocs.is_empty() {
        for s in &sources {
            let covered = source_coverage
                .get(&pystr(&s["id"]))
                .cloned()
                .unwrap_or_default();
            if covered != big(&s["amount_msat"]) {
                issues.push(json!({"code":"allocation_source_coverage_mismatch","leg_id":s["id"],"expected_msat":s["amount_msat"],"covered_msat":number(&covered)}));
            }
        }
        let orphans: Vec<_> = fee_sources.difference(&material_sources).collect();
        if !orphans.is_empty() {
            issues.push(json!({"code":"custody_component_fee_orphaned","message":"a fee source needs an owned transfer or external disposal row to carry it into journals","source_leg_ids":orphans}));
        }
        for t in &sinks {
            let covered = sink_coverage
                .get(&pystr(&t["id"]))
                .cloned()
                .unwrap_or_default();
            if covered != big(&t["amount_msat"]) {
                issues.push(json!({"code":"allocation_sink_coverage_mismatch","leg_id":t["id"],"expected_msat":t["amount_msat"],"covered_msat":number(&covered)}));
            }
        }
    } else {
        let owned_count = sinks.iter().filter(|l| owned(l)).count();
        let attributed = sinks
            .iter()
            .any(|l| matches!(l["role"].as_str(), Some("fee" | "external" | "suspense")));
        let unambiguous = if mode == "conversion" {
            sources.len() == 1 && sinks.len() == 1
        } else {
            sources.len() <= 1 || (owned_count == 1 && !attributed)
        };
        if !sources.is_empty() && !sinks.is_empty() && !unambiguous {
            issues.push(json!({"code":"allocation_required","message":"N:M custody flow requires explicit source-to-sink allocation","source_leg_ids":sources.iter().map(|l|&l["id"]).collect::<Vec<_>>(),"sink_leg_ids":sinks.iter().map(|l|&l["id"]).collect::<Vec<_>>()}));
        }
        if sources.len() == 1 {
            for t in &sinks {
                if t["role"] == "fee" {
                    issues.extend(fee_issues(sources[0], t, None, mode));
                }
            }
        }
    }
    issues.extend(allocation_scope_issues(legs, allocs, mode));
    json!({"valid":issues.is_empty(),"issues":issues,"edges":edges})
}
pub(super) fn validate(legs: &[Value], allocs: &[Value], h: &Value) -> Result<Value, Error> {
    let mode = text(&h["conservation_mode"]).py_strip().to_owned();
    let mut issues = Vec::new();
    let roles: BTreeSet<_> = legs
        .iter()
        .filter(|l| {
            !matches!(
                l["role"].as_str(),
                Some("source" | "destination" | "fee" | "external" | "retained" | "suspense")
            )
        })
        .map(|l| text(&l["role"]))
        .collect();
    if !roles.is_empty() {
        return Ok(
            json!({"conservation_mode":mode,"activatable":false,"issues":[{"code":"custody_component_leg_role_unknown","message":"component contains a leg role unsupported by this application","roles":roles}],"suspense_msat":0,"source_msat":0,"owned_destination_msat":0,"by_asset":[],"by_conservation_unit":[],"by_valuation_unit":[],"allocations":{"valid":false,"issues":[],"edges":[]}}),
        );
    }
    let source_total: BigInt = legs
        .iter()
        .filter(|l| l["role"] == "source")
        .map(|l| big(&l["amount_msat"]))
        .sum();
    let owned_total: BigInt = legs
        .iter()
        .filter(|l| owned(l))
        .map(|l| big(&l["amount_msat"]))
        .sum();
    let suspense: Vec<_> = legs
        .iter()
        .filter(|l| l["role"] == "suspense" && big(&l["amount_msat"]) > BigInt::from(0))
        .collect();
    let suspense_total: BigInt = suspense.iter().map(|l| big(&l["amount_msat"])).sum();
    if legs.is_empty() {
        issues.push(json!({"code":"no_legs","message":"component has no legs"}));
    }
    if source_total <= BigInt::from(0) {
        issues.push(json!({"code":"missing_source","message":"component has no positive source"}));
    }
    if owned_total <= BigInt::from(0) {
        issues.push(json!({"code":"missing_owned_destination","message":"component has no positive destination or retained-custody leg"}));
    }
    if !suspense.is_empty() {
        if mode != "quantity" {
            issues.push(json!({"code":"custody_suspense_quantity_mode_required","message":"custody suspense is supported only for exact quantity conservation"}));
        }
        if h["component_type"] != "manual_bridge"
            || text(&h["evidence_grade"]).py_lower() != "reviewed"
        {
            issues.push(json!({"code":"custody_suspense_review_required","message":"custody suspense requires an explicitly reviewed manual bridge"}));
        }
        if allocs.is_empty() {
            issues.push(json!({"code":"custody_suspense_allocation_required","message":"custody suspense requires exact explicit source allocations"}));
        }
        for l in &suspense {
            let forbidden: Vec<_> = ["wallet_id", "transaction_id", "anchor_transaction_id"]
                .into_iter()
                .filter(|k| !absent(&l[*k]))
                .collect();
            if !forbidden.is_empty() {
                issues.push(json!({"code":"custody_suspense_anchor_invalid","message":"a suspense sink cannot claim a wallet or transaction location","leg_id":l["id"],"fields":forbidden}));
            }
            if !truth(&l["occurred_at"]) {
                issues.push(json!({"code":"custody_suspense_occurred_at_missing","message":"a suspense sink begins at its observed source debit time","leg_id":l["id"]}));
            }
        }
        for a in allocs {
            let Some((s, t)) = by_id(legs, &a["source_leg_id"])
                .zip(by_id(legs, &a["sink_leg_id"]))
                .filter(|(_, t)| t["role"] == "suspense")
            else {
                continue;
            };
            let base = |code: &str, message: &str| json!({"code":code,"message":message,"allocation_id":a["id"],"source_leg_id":s["id"],"suspense_leg_id":t["id"]});
            if absent(&s["transaction_id"]) {
                issues.push(base(
                    "custody_suspense_observed_source_required",
                    "a suspense allocation must begin at an observed outbound debit",
                ));
            }
            if asset(&s["asset"]) != asset(&t["asset"]) {
                issues.push(base(
                    "custody_suspense_asset_mismatch",
                    "a suspense residual must retain its source asset",
                ));
            }
            if s["exposure"] != t["exposure"]
                || s["conservation_unit"] != t["conservation_unit"]
                || big(&a["source_amount_msat"]) != big(&a["sink_amount_msat"])
            {
                issues.push(base(
                    "custody_suspense_quantity_mismatch",
                    "a suspense residual must preserve exact source quantity units",
                ));
            }
            let ss = network_scope(s);
            let ts = network_scope(t);
            if !truth(&ss["valid"])
                || !truth(&ts["valid"])
                || ss["domain"].is_null()
                || ss["domain"] != ts["domain"]
            {
                let mut v = base(
                    "custody_suspense_network_mismatch",
                    "a suspense residual must remain in the source network domain",
                );
                v["source_scope"] = ss;
                v["suspense_scope"] = ts;
                issues.push(v);
            }
            if time::instant(&s["occurred_at"])?.is_none()
                || time::instant(&t["occurred_at"])? != time::instant(&s["occurred_at"])?
            {
                let mut v = base(
                    "custody_suspense_time_mismatch",
                    "suspense must begin at the exact source debit time",
                );
                v["source_occurred_at"] = s["occurred_at"].clone();
                v["suspense_occurred_at"] = t["occurred_at"].clone();
                issues.push(v);
            }
        }
    }
    let value_only: Vec<_> = legs
        .iter()
        .filter(|l| {
            matches!(l["role"].as_str(), Some("fee" | "external"))
                && big(&l["amount_msat"]) == BigInt::from(0)
                && big(or(&l["valuation_amount"], &json!(0))) > BigInt::from(0)
        })
        .map(|l| {
            if truth(&l["id"]) {
                pystr(&l["id"])
            } else {
                wide(&l["ordinal"]).to_string()
            }
        })
        .collect();
    if !value_only.is_empty() {
        issues.push(json!({"code":"custody_component_value_only_loss_unsupported","message":"a fiat-only fee or external loss needs a positive quantity leg before it can be projected into journals","leg_ids":value_only}));
    }
    let missing: Vec<_> = legs
        .iter()
        .filter(|l| {
            l["transaction_id"].is_null()
                && (l["role"] == "source" || owned(l))
                && big(&l["amount_msat"]) > BigInt::from(0)
                && !truth(&l["occurred_at"])
        })
        .map(|l| wide(&l["ordinal"]))
        .collect();
    if !missing.is_empty() {
        issues.push(json!({"code":"leg_occurred_at_missing","message":"transaction-less custody legs require occurred_at","ordinals":missing}));
    }
    let mut invalid = Vec::new();
    for l in legs {
        if !absent(&l["occurred_at"]) && time::instant(&l["occurred_at"])?.is_none() {
            invalid.push(wide(&l["ordinal"]));
        }
    }
    if !invalid.is_empty() {
        issues.push(json!({"code":"leg_occurred_at_invalid","message":"custody leg timestamps must be valid RFC3339 values","ordinals":invalid}));
    }
    issues.extend(chronology(legs, allocs, &mode)?);
    issues.extend(location_issues(legs)?);
    let by_asset = balances(legs, &["asset"], "amount_msat", false);
    let by_unit = balances(
        legs,
        &["exposure", "conservation_unit"],
        "amount_msat",
        false,
    );
    let by_valuation = balances(legs, &["valuation_unit"], "valuation_amount", true);
    if mode == "quantity" {
        let unbalanced: Vec<_> = by_unit
            .iter()
            .filter(|r| big(&r["residual_msat"]) != BigInt::from(0))
            .collect();
        if !unbalanced.is_empty() {
            issues.push(json!({"code":"unbalanced_quantity","message":"source and sink quantities do not conserve","groups":unbalanced}));
        }
    } else {
        let sources: Vec<_> = legs
            .iter()
            .filter(|l| l["role"] == "source" && big(&l["amount_msat"]) > BigInt::from(0))
            .collect();
        let mut outputs: Vec<_> = legs
            .iter()
            .filter(|l| l["role"] == "destination" && big(&l["amount_msat"]) > BigInt::from(0))
            .collect();
        let retained: Vec<_> = legs
            .iter()
            .filter(|l| l["role"] == "retained" && big(&l["amount_msat"]) > BigInt::from(0))
            .collect();
        let mut continuations = Vec::new();
        if sources.len() == 1 {
            let s = sources[0];
            for l in retained {
                if asset(&l["asset"]) == asset(&s["asset"])
                    && l["exposure"] == s["exposure"]
                    && l["conservation_unit"] == s["conservation_unit"]
                {
                    continuations.push(l);
                } else {
                    outputs.push(l);
                }
            }
        }
        if outputs.is_empty() && !continuations.is_empty() {
            outputs.push(continuations.remove(0));
        }
        if sources.len() != 1 || outputs.len() != 1 {
            issues.push(json!({"code":"conversion_topology_unsupported","message":"reviewed conversions currently require exactly one quantity source and one conversion output; exact same-asset retained continuations may be additional sinks","source_leg_ids":sources.iter().map(|l|&l["id"]).collect::<Vec<_>>(),"destination_leg_ids":outputs.iter().map(|l|&l["id"]).collect::<Vec<_>>()}));
        }
        let invalid: Vec<_> = allocs
            .iter()
            .filter(|a| {
                by_id(legs, &a["sink_leg_id"]).is_some_and(|l| continuations.contains(&l))
                    && big(&a["source_amount_msat"]) != big(&a["sink_amount_msat"])
            })
            .map(|a| &a["id"])
            .collect();
        if !invalid.is_empty() {
            issues.push(json!({"code":"conversion_retained_quantity_mismatch","message":"a retained conversion residual must preserve exact quantity","allocation_ids":invalid}));
        }
        if !truth(&h["conversion_reviewed"]) {
            issues.push(json!({"code":"conversion_not_reviewed","message":"conversion requires explicit review"}));
        }
        if !h["conversion_policy"]
            .as_str()
            .is_some_and(|s| !s.py_strip().is_empty())
        {
            issues.push(json!({"code":"conversion_policy_missing","message":"conversion requires an explicit policy"}));
        }
        let missing: Vec<_> = legs
            .iter()
            .filter(|l| {
                (big(&l["amount_msat"]) > BigInt::from(0)
                    || big(or(&l["valuation_amount"], &json!(0))) > BigInt::from(0))
                    && (l["valuation_unit"].is_null() || l["valuation_amount"].is_null())
            })
            .map(|l| wide(&l["ordinal"]))
            .collect();
        if !missing.is_empty() {
            issues.push(json!({"code":"conversion_valuation_missing","message":"every material conversion leg requires an exact valuation","ordinals":missing}));
        }
        if by_valuation.is_empty() {
            issues.push(json!({"code":"conversion_valuation_missing","message":"conversion has no exact valuation basis"}));
        }
        let unbalanced: Vec<_> = by_valuation
            .iter()
            .filter(|r| big(&r["residual_amount"]) != BigInt::from(0))
            .collect();
        if !unbalanced.is_empty() {
            issues.push(json!({"code":"unbalanced_conversion_valuation","message":"conversion valuations do not conserve","groups":unbalanced}));
        }
    }
    let allocation = allocations(legs, allocs, &mode);
    issues.extend(array(&allocation["issues"]).iter().cloned());
    issues.extend(connectivity(legs, allocs, &mode));
    // The parts move into the result; `json!` would copy each one.
    let mut result = serde_json::Map::new();
    result.insert("conservation_mode".into(), json!(mode));
    result.insert("activatable".into(), json!(issues.is_empty()));
    result.insert("issues".into(), Value::Array(issues));
    result.insert("suspense_msat".into(), number(&suspense_total));
    result.insert("source_msat".into(), number(&source_total));
    result.insert("owned_destination_msat".into(), number(&owned_total));
    result.insert("by_asset".into(), Value::Array(by_asset));
    result.insert("by_conservation_unit".into(), Value::Array(by_unit));
    result.insert("by_valuation_unit".into(), Value::Array(by_valuation));
    result.insert("allocations".into(), allocation);
    Ok(Value::Object(result))
}
