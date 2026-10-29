//! Comparison-only runtime composition. No persistence or authority boundary.
use super::{array, domain};
use crate::{codec, python_text::PythonText};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
type Order = [String; 5];
type Pool = [String; 4];
fn text(v: &Value) -> String {
    if codec::truth(v) {
        codec::pystr(v)
    } else {
        String::new()
    }
}
fn pool(o: &Value) -> Pool {
    let d = domain(o);
    [
        text(&o["profile_id"]),
        text(&d["network"]),
        text(&d["exposure"]),
        "msat".into(),
    ]
}
fn pool_value(p: &Pool) -> Value {
    json!({"$type":"TaxExposurePool","profile_id":p[0],"network":p[1],"exposure":p[2],"unit":p[3]})
}
fn row_pool(row: &Value) -> Option<Pool> {
    let (_, network) = crate::evidence::scope(row).ok()?;
    let asset = text(&row["asset"]).py_upper();
    if asset.is_empty() {
        return None;
    }
    let network = match network.as_str() {
        "liquidv1" => "main".into(),
        "liquidtestnet" => "test".into(),
        "elementsregtest" => "regtest".into(),
        _ => network,
    };
    Some([
        text(&row["profile_id"]),
        network,
        if matches!(asset.as_str(), "BTC" | "LBTC") {
            "bitcoin".into()
        } else {
            format!("asset:{asset}")
        },
        "msat".into(),
    ])
}
/// Deterministic barriers preserve finalized siblings, while equal-time distinct
/// events fail closed. Pool identity deliberately omits the rail.
pub fn tax_eligibility(
    canonical: &Value,
    projection: &Value,
    issues: &[Value],
    rows: &Value,
) -> Result<Value, String> {
    let mut orders = BTreeMap::new();
    let mut pools = BTreeMap::new();
    let mut aliases = BTreeMap::new();
    for event in array(&canonical["events"]) {
        let key = &event["event_key"];
        let order: Order = [
            array(&event["legs"])
                .iter()
                .map(|o| text(&o["occurred_at"]))
                .min()
                .unwrap_or_default(),
            text(&key["chain"]),
            text(&key["network"]),
            text(&key["native_namespace"]),
            text(&key["native_event_id"]),
        ];
        for alias in array(&event["observation_aliases"]) {
            aliases.insert(text(&alias[0]), text(&alias[1]));
        }
        for leg in array(&event["legs"]) {
            let hash = text(&leg["quantity_hash"]);
            orders.insert(hash.clone(), order.clone());
            pools.insert(hash, pool(leg));
        }
    }
    let mut barriers: BTreeMap<Pool, Order> = BTreeMap::new();
    let mut roots: BTreeMap<Pool, BTreeSet<String>> = BTreeMap::new();
    let mut direct = BTreeSet::new();
    let mut register = |p: Pool, order: Order, ids: BTreeSet<String>| match barriers.get(&p) {
        Some(current) if current < &order => {}
        Some(current) if current == &order => {
            roots.entry(p).or_default().extend(ids);
        }
        _ => {
            barriers.insert(p.clone(), order);
            roots.insert(p, ids);
        }
    };
    for issue in issues {
        if matches!(
            issue["issue_type"].as_str(),
            Some(
                "native_audit_evidence_invalid"
                    | "custody_gap_review_hold"
                    | "custody_gap_hold_compile_failed"
            )
        ) {
            for id in array(&issue["transaction_ids"]) {
                if let Some(hash) = aliases.get(&text(id)) {
                    direct.insert(hash.clone());
                }
            }
        }
        let known: Vec<_> = array(&issue["transaction_ids"])
            .iter()
            .filter_map(|id| {
                let hash = aliases.get(&text(id))?;
                Some((
                    pools.get(hash)?.clone(),
                    orders.get(hash)?.clone(),
                    text(id),
                ))
            })
            .collect();
        if !known.is_empty() {
            for p in known.iter().map(|v| v.0.clone()).collect::<BTreeSet<_>>() {
                let order = known
                    .iter()
                    .filter(|v| v.0 == p)
                    .map(|v| v.1.clone())
                    .min()
                    .unwrap();
                let ids = known
                    .iter()
                    .filter(|v| v.0 == p && v.1 == order)
                    .map(|v| v.2.clone())
                    .collect();
                register(p, order, ids);
            }
            continue;
        }
        if !codec::truth(&issue["occurred_at"]) {
            continue;
        }
        let order = [
            text(&issue["occurred_at"]),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ];
        let mut rejected: BTreeSet<_> = array(&issue["transaction_ids"])
            .iter()
            .filter_map(|id| rows.get(text(id)))
            .filter_map(row_pool)
            .collect();
        if rejected.is_empty() && codec::truth(&issue["asset"]) {
            for o in array(&projection["observations"]) {
                if o["asset"] == issue["asset"] {
                    rejected.insert(
                        pools
                            .get(&text(&o["quantity_hash"]))
                            .ok_or("KeyError")?
                            .clone(),
                    );
                }
            }
        }
        if rejected.is_empty() {
            rejected.extend(pools.values().cloned());
        }
        for p in rejected {
            register(
                p,
                order.clone(),
                array(&issue["transaction_ids"]).iter().map(text).collect(),
            );
        }
    }
    let barrier = barriers.values().min();
    let mut eligible = Vec::new();
    for d in array(&projection["decisions"]) {
        if matches!(
            d["state"].as_str(),
            Some("custody_suspense" | "conflicting")
        ) {
            continue;
        }
        let hash = text(&d["source"]["observation_hash"]);
        if direct.contains(&hash) {
            continue;
        }
        let p = pools.get(&hash).ok_or("KeyError")?;
        if let Some(b) = barriers.get(p) {
            let o = orders.get(&hash).ok_or("KeyError")?;
            if !(o[0] < b[0] || o == b) {
                continue;
            }
        }
        eligible.push(d.clone());
    }
    let mut ineligible: Vec<_> = array(&projection["decisions"])
        .iter()
        .filter(|d| !eligible.iter().any(|e| e["source"] == d["source"]))
        .map(|d| d["source"].clone())
        .collect();
    ineligible.sort_by(|a, b| {
        text(&a["observation_hash"])
            .cmp(&text(&b["observation_hash"]))
            .then_with(|| {
                super::exact(&a["start_msat"])
                    .unwrap()
                    .cmp(&super::exact(&b["start_msat"]).unwrap())
            })
            .then_with(|| {
                super::exact(&a["end_msat"])
                    .unwrap()
                    .cmp(&super::exact(&b["end_msat"]).unwrap())
            })
    });
    Ok(
        json!({"$type":"QuantityTaxEligibility","eligible_decisions":eligible,"ineligible_slices":ineligible,"blocked_from":barrier.map(|b|&b[0]),"barrier_event_key":barrier,"pool_barriers":barriers.iter().map(|(p,b)|json!([pool_value(p),b])).collect::<Vec<_>>(),"pool_barrier_roots":roots.iter().map(|(p,r)|json!([pool_value(p),r])).collect::<Vec<_>>()}),
    )
}
fn issue_id(parts: &[Value]) -> Result<String, String> {
    super::hashes::issue_id(
        &parts
            .iter()
            .map(codec::from_json)
            .collect::<Result<Vec<_>, _>>()?,
    )
    .map(|v| v.0)
}
fn issue(
    id: String,
    kind: &str,
    asset: Value,
    at: String,
    ids: Value,
    reason: &str,
    details: Value,
) -> Value {
    json!({"$type":"QuantityIssue","issue_id":id,"issue_type":kind,"state":"conflicting","asset":asset,"amount_msat":null,"occurred_at":at,"transaction_ids":ids,"reason":reason,"details":details})
}
fn observations(c: &Value) -> Result<Vec<(String, Value)>, String> {
    let hashes: BTreeMap<_, _> = array(&c["events"])
        .iter()
        .flat_map(|e| array(&e["legs"]))
        .map(|o| (text(&o["quantity_hash"]), o))
        .collect();
    let mut result = Vec::new();
    for event in array(&c["events"]) {
        for a in array(&event["observation_aliases"]) {
            let id = text(&a[0]);
            let o = (*hashes.get(&text(&a[1])).ok_or("KeyError")?).clone();
            if let Some((_, old)) = result.iter_mut().find(|(key, _)| key == &id) {
                *old = o;
            } else {
                result.push((id, o));
            }
        }
    }
    Ok(result)
}
fn tx(leg: &Value) -> String {
    text(if codec::truth(&leg["anchor_transaction_id"]) {
        &leg["anchor_transaction_id"]
    } else {
        &leg["transaction_id"]
    })
}
/// Compose authored-component diagnostics and fail-closed claims before native
/// augmentation. Compilation errors remain unquantified to avoid double counts.
pub fn component_claims(
    canonical: &Value,
    components: &[Value],
    evidence: &Value,
) -> Result<Value, String> {
    let obs = observations(canonical)?;
    let obs_value = json!(obs.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>());
    let mut components = components.to_vec();
    components.sort_by_key(|c| text(&c["id"]));
    let mut claims = Vec::new();
    let mut issues = Vec::new();
    let mut conversions = Vec::new();
    let mut payouts = Vec::new();
    for c in &components {
        let id = text(&c["id"]);
        let ids: BTreeSet<_> = array(&c["legs"])
            .iter()
            .map(tx)
            .filter(|id| !id.is_empty())
            .collect();
        let mut error = None;
        if c["effective_state"] != "active" {
            error = Some((
                "custody_component_authored_active_invalid".to_owned(),
                "authored active component is not locally effective".to_owned(),
                json!({"component_id":id,"evidence_status":c["evidence_status"]["status"]}),
            ));
        } else if !array(&evidence[&id]).is_empty() {
            let relevant: BTreeSet<_> = obs
                .iter()
                .filter(|(tx, _)| ids.contains(tx))
                .map(|(_, o)| text(&o["quantity_hash"]))
                .collect();
            let current: BTreeSet<_> = array(&canonical["events"])
                .iter()
                .flat_map(|e| array(&e["evidence_snapshots"]))
                .map(|s| text(&s["quantity_hash"]))
                .filter(|h| relevant.contains(h))
                .collect();
            let expected: BTreeSet<_> = array(&evidence[&id])
                .iter()
                .map(|s| text(&s["quantity_hash"]))
                .collect();
            if current != expected {
                error=Some(("custody_component_evidence_drift".into(),"component activation quantity or identity no longer matches current observations".into(),json!({"component_id":id,"drift_kind":"evidence_quantity_changed_or_missing","expected_snapshot_count":expected.len(),"current_snapshot_count":current.len()})));
            }
        }
        if error.is_none() {
            match super::reviewed::compile(c, &obs_value) {
                Ok(v) => {
                    claims.extend(array(&v["claims"]).iter().cloned());
                    conversions.extend(array(&v["reviewed_conversion_pairs"]).iter().cloned());
                    payouts.extend(array(&v["reviewed_direct_payouts"]).iter().cloned());
                    continue;
                }
                Err(e) => {
                    if !e.code.starts_with("custody_")
                        && !matches!(e.code.as_str(), "ValueError" | "TypeError")
                    {
                        return Err(e.code);
                    }
                    let code = if e.code.starts_with("custody_") {
                        e.code
                    } else {
                        "custody_component_claim_compile".into()
                    };
                    error = Some((code, e.message, e.details));
                }
            }
        }
        let (code, message, details) = error.unwrap();
        let mut sources: Vec<&Value> = Vec::new();
        for tx in &ids {
            if let Some((_, o)) = obs.iter().find(|(id, _)| id == tx) {
                if o["direction"] == "outbound"
                    && !sources
                        .iter()
                        .any(|s| s["quantity_hash"] == o["quantity_hash"])
                {
                    sources.push(o);
                }
            }
        }
        for o in &sources {
            if super::principal(o)? > kassiber_pyjson::BigInt::from(0) {
                claims.push(json!({"$type":"QuantityClaim","claim_id":format!("component:{id}:compile-failed:{}",text(&o["quantity_hash"])),"source":{"$type":"QuantitySlice","observation_hash":o["quantity_hash"],"start_msat":0,"end_msat":super::number(&super::principal(o)?)},"target":null,"state":"custody_suspense","priority":{"$enum":"ClaimPriority.REVIEWED_COMPONENT","value":10},"reason":code,"atomic_bundle_id":format!("component:{id}:compile-failed"),"destination_kind":null,"external_economic_subtype":null,"transfer_kind":null,"transfer_policy":null,"component_id":null,"allow_cross_rail":false}));
            }
        }
        issues.push(issue(
            issue_id(&[json!("component_compile"), json!(id)])?,
            if code == "custody_component_evidence_drift" {
                "custody_component_evidence_drift"
            } else {
                "component_claim_compile_failed"
            },
            sources
                .first()
                .map(|o| o["asset"].clone())
                .unwrap_or(Value::Null),
            sources
                .iter()
                .map(|o| text(&o["occurred_at"]))
                .min()
                .unwrap_or_default(),
            json!(ids),
            &code,
            json!({"component_id":id,"message":message,"compiler_details":details}),
        ));
    }
    Ok(json!([claims, issues, conversions, payouts]))
}
// Mirror the fixture serializer for nonfinite diagnostic values. The raw
// stage retains Py values directly; this helper belongs to the private seam.
fn diagnostic_json(v: &kassiber_pyjson::Value) -> Result<Value, String> {
    use kassiber_pyjson::Value as Py;
    Ok(match v {
        Py::Float(f) if !f.is_finite() => {
            json!({"$float":if f.is_nan() {"nan"} else if *f>0.0 {"inf"} else {"-inf"}})
        }
        Py::Str(s) if s.as_str().is_none() => return Err("ValueError".into()),
        Py::List(items) => Value::Array(
            items
                .iter()
                .map(diagnostic_json)
                .collect::<Result<_, _>>()?,
        ),
        Py::Dict(items) => Value::Object(
            items
                .iter()
                .map(|(k, v)| {
                    let kassiber_pyjson::Key::Str(k) = k else {
                        unreachable!("JSON key")
                    };
                    Ok((
                        k.as_str().ok_or("ValueError")?.to_owned(),
                        diagnostic_json(v)?,
                    ))
                })
                .collect::<Result<_, String>>()?,
        ),
        _ => codec::to_json(v),
    })
}
/// Assemble all issue families using Python's identity and admission ordering.
pub fn whole_issues(
    canonical: &Value,
    projection: &Value,
    rows: &Value,
    component: &[Value],
    gaps: &[Value],
    native: &[Value],
    blockers: &[Value],
) -> Result<Value, String> {
    let mut issues = Vec::new();
    for rejected in array(&canonical["rejected_events"]) {
        let key = &rejected["event_key"];
        let mut details = json!({"message":rejected["message"],"event_scope":{"chain":key["chain"],"network":key["network"],"namespace":key["native_namespace"]}});
        if let Some(d) = rejected["details"].as_object() {
            details.as_object_mut().unwrap().extend(d.clone());
        }
        issues.push(issue(
            super::hashes::rejected_event_issue_id(key, &text(&rejected["code"]))?.0,
            "canonical_event_rejected",
            Value::Null,
            array(&rejected["transaction_ids"])
                .iter()
                .filter_map(|id| rows.get(text(id)))
                .map(|r| text(&r["occurred_at"]))
                .min()
                .unwrap_or_default(),
            rejected["transaction_ids"].clone(),
            &text(&rejected["code"]),
            details,
        ));
    }
    issues.extend_from_slice(component);
    issues.extend_from_slice(gaps);
    for n in native {
        let mut n = n.clone();
        n["$type"] = json!("QuantityIssue");
        n["issue_type"] = json!("native_audit_evidence_invalid");
        n["state"] = json!("conflicting");
        issues.push(n);
    }
    let by_hash: BTreeMap<_, _> = array(&projection["observations"])
        .iter()
        .map(|o| (text(&o["quantity_hash"]), o))
        .collect();
    for e in array(&projection["claim_errors"]) {
        let involved: Vec<_> = array(&e["involved_observation_hashes"])
            .iter()
            .filter_map(|h| by_hash.get(&text(h)))
            .collect();
        let assets: BTreeSet<_> = involved.iter().map(|o| text(&o["asset"])).collect();
        issues.push(issue(issue_id(&[json!("claim_bundle_invalid"),e["bundle_id"].clone()])?,"quantity_claim_bundle_invalid",if assets.len()==1 {json!(assets.first().unwrap())} else {Value::Null},involved.iter().map(|o|text(&o["occurred_at"])).min().unwrap_or_default(),json!(involved.iter().map(|o|text(&o["anchor_transaction_id"])).collect::<BTreeSet<_>>()),"malformed_claim_bundle",json!({"bundle_id":e["bundle_id"],"claim_ids":e["claim_ids"],"validation_reasons":e["reasons"]})));
    }
    for (ordinal, b) in blockers.iter().enumerate() {
        let id = text(&b["transaction_id"]);
        let row = &rows[&id];
        let reason = if codec::truth(&b["reason"]) {
            text(&b["reason"])
        } else {
            "custody_interpreter_blocked".into()
        };
        let raw = if codec::truth(&b["detail_json"]) {
            b["detail_json"].clone()
        } else {
            json!("{}")
        };
        let details = match raw.as_str().and_then(|s| codec::loads(s).ok()) {
            Some(value @ kassiber_pyjson::Value::Dict(_)) => diagnostic_json(&value)?,
            Some(_) => json!({}),
            None => json!({"unparsed_detail":text(&b["detail_json"])}),
        };
        let asset = text(&row["asset"]).py_upper();
        let mut i = issue(
            issue_id(&[
                json!("custody_interpreter_blocked"),
                json!(id),
                json!(reason),
                json!(ordinal),
            ])?,
            "custody_interpreter_blocked",
            if asset.is_empty() {
                Value::Null
            } else {
                json!(asset)
            },
            text(&row["occurred_at"]),
            if id.is_empty() {
                json!([])
            } else {
                json!([id])
            },
            &reason,
            if details.is_object() {
                details
            } else {
                json!({})
            },
        );
        if codec::truth(row) {
            let amount = if codec::truth(&row["amount"]) {
                codec::integer(&row["amount"]).map_err(|_| {
                    if row["amount"].is_object() || row["amount"].is_array() {
                        "TypeError"
                    } else {
                        "ValueError"
                    }
                })?
            } else {
                json!(0)
            };
            if super::exact(&amount)? > kassiber_pyjson::BigInt::from(0) {
                i["amount_msat"] = amount;
            }
        }
        issues.push(i);
    }
    for d in array(&projection["decisions"]) {
        if matches!(
            d["state"].as_str(),
            Some("custody_suspense" | "conflicting")
        ) {
            let s = &d["source"];
            let o = by_hash
                .get(&text(&s["observation_hash"]))
                .ok_or("KeyError")?;
            let mut i = issue(
                issue_id(&[
                    json!("decision"),
                    s["observation_hash"].clone(),
                    s["start_msat"].clone(),
                    s["end_msat"].clone(),
                    d["state"].clone(),
                ])?,
                "unresolved_quantity",
                o["asset"].clone(),
                text(&o["occurred_at"]),
                json!([o["transaction_id"]]),
                &text(&d["reason"]),
                json!({"selected_claim_id":d["selected_claim_id"],"contender_claim_ids":d["contender_claim_ids"]}),
            );
            i["state"] = d["state"].clone();
            i["amount_msat"] =
                super::number(&(super::exact(&s["end_msat"])? - super::exact(&s["start_msat"])?));
            issues.push(i);
        }
    }
    issues.sort_by_key(|i| (text(&i["occurred_at"]), text(&i["issue_id"])));
    Ok(json!(issues))
}
/// Private typed runtime operands; raw snapshot v1 never contains these values.
pub fn build(v: &Value) -> Result<Value, String> {
    let canonical = &v["canonical"];
    let components = array(&v["components"]);
    let comp = component_claims(canonical, components, &v["evidence"])?;
    let component_ids: BTreeSet<_> = components
        .iter()
        .flat_map(|c| array(&c["legs"]))
        .map(tx)
        .filter(|id| !id.is_empty())
        .collect();
    let mut ignored = component_ids.clone();
    ignored.extend(
        array(&v["ignored"])
            .iter()
            .filter(|v| codec::truth(v))
            .map(codec::pystr),
    );
    let by_hash: BTreeMap<_, _> = array(&canonical["events"])
        .iter()
        .flat_map(|e| array(&e["legs"]))
        .map(|o| (text(&o["quantity_hash"]), o))
        .collect();
    for c in array(&v["claims"]) {
        for field in ["source", "target"] {
            if let Some(o) = by_hash.get(&text(&c[field]["observation_hash"])) {
                ignored.insert(text(&o["transaction_id"]));
            }
        }
    }
    let gaps = super::holds::compose(
        canonical,
        &ignored.into_iter().map(|s| json!(s)).collect::<Vec<_>>(),
        &v["dismissed"],
        &v["search"],
    )?;
    let native = super::native::compile(
        canonical,
        array(&v["native"]),
        &component_ids
            .into_iter()
            .map(|s| json!(s))
            .collect::<Vec<_>>(),
        &json!({}),
    )?;
    let canonical = &native["canonical_input"];
    let mut claims = array(&comp[0]).to_vec();
    claims.extend_from_slice(array(&gaps[0]));
    claims.extend_from_slice(array(&native["claims"]));
    claims.extend_from_slice(array(&v["claims"]));
    let observations: Vec<_> = array(&canonical["events"])
        .iter()
        .flat_map(|e| array(&e["legs"]))
        .cloned()
        .collect();
    let projection = super::project(&observations, &claims)?;
    let rows: serde_json::Map<_, _> = array(&v["rows"])
        .iter()
        .filter(|r| !r["id"].is_null() && r["id"] != "")
        .map(|r| (text(&r["id"]), r.clone()))
        .collect();
    let rows = Value::Object(rows);
    let issues = whole_issues(
        canonical,
        &projection,
        &rows,
        array(&comp[1]),
        array(&gaps[1]),
        array(&native["issues"]),
        array(&v["blockers"]),
    )?;
    let tax = tax_eligibility(canonical, &projection, array(&issues), &rows)?;
    Ok(
        json!({"$type":"CanonicalQuantityState","canonical_input":canonical,"projection":projection,"issues":issues,"tax_eligibility":tax,"gap_candidate_transaction_ids":gaps[2],"gap_holds":gaps[3],"reviewed_conversion_pairs":comp[2],"reviewed_direct_payouts":comp[3]}),
    )
}
/// Recompute quantity from preceding Rust stages and the raw snapshot only.
pub fn stage(
    snapshot: &crate::snapshot::Snapshot,
    components: &Value,
    evidence: &Value,
    interpretation: &kassiber_pyjson::Value,
    gaps: &Value,
) -> Result<kassiber_pyjson::Value, String> {
    use crate::graph::stored;
    use kassiber_pyjson::Value as Py;
    let (_, ignored) =
        crate::interpretation::gap_ignored_ids(snapshot, components, evidence, interpretation)?;
    // Quarantine payloads retain CPython nonfinite values and surrogate ids.
    // Only the already-typed compiler outputs cross the serde seam.
    let typed = [
        "claims",
        "native_audits",
        "native_component_reconciliations",
    ];
    let mut converted = serde_json::Map::new();
    for key in typed {
        let value = stored::get(interpretation, key);
        let json = codec::to_json(value);
        if codec::dumps(value, true)? != codec::dumps(&codec::from_json(&json)?, true)? {
            return Err("ValueError".into());
        }
        converted.insert(key.into(), json);
    }
    let converted = Value::Object(converted);
    let reconciled: BTreeSet<_> = array(&converted["native_component_reconciliations"])
        .iter()
        .map(|r| text(&r["component_id"]))
        .collect();
    let effective: Vec<_> = array(&components["components"])
        .iter()
        .filter(|c| !reconciled.contains(&text(&c["id"])))
        .cloned()
        .collect();
    let blocked = stored::list(stored::get(interpretation, "blocked_transaction_ids"));
    let transient = stored::list(stored::get(
        interpretation,
        "transient_blocked_transaction_ids",
    ));
    let blockers: Vec<_> = stored::list(stored::get(interpretation, "quarantines"))
        .iter()
        .filter(|q| {
            let id = stored::get(q, "transaction_id");
            blocked.contains(id) && !transient.contains(id)
        })
        .collect();
    fn string(v: &Py) -> Py {
        if let Py::Str(_) = v {
            v.clone()
        } else {
            Py::Str(crate::interpretation::python_value_string(v).into())
        }
    }
    let mut safe = Vec::new();
    let mut originals = Vec::new();
    for (ordinal, q) in blockers.iter().enumerate() {
        let raw_id = stored::get(q, "transaction_id");
        let id = if stored::truth(raw_id) {
            string(raw_id)
        } else {
            Py::Str("".into())
        };
        let raw_reason = stored::get(q, "reason");
        let reason = if stored::truth(raw_reason) {
            string(raw_reason)
        } else {
            Py::Str("custody_interpreter_blocked".into())
        };
        let as_utf8 = |v: &Py| {
            if let Py::Str(s) = v {
                s.as_str().unwrap_or("").to_owned()
            } else {
                unreachable!()
            }
        };
        let safe_id = as_utf8(&id);
        let safe_reason = as_utf8(&reason);
        // A surrogate id cannot equal any UTF-8 raw row id. The empty lookup
        // therefore has the same absence semantics for amounts and barriers.
        safe.push(json!({"transaction_id":safe_id,"reason":safe_reason,"detail_json":"{}"}));
        let actual_reason = if safe_reason.is_empty() {
            "custody_interpreter_blocked"
        } else {
            &safe_reason
        };
        let safe_hash = issue_id(&[
            json!("custody_interpreter_blocked"),
            json!(safe_id),
            json!(actual_reason),
            json!(ordinal),
        ])?;
        let exact_hash = super::hashes::issue_id(&[
            Py::Str("custody_interpreter_blocked".into()),
            id.clone(),
            reason.clone(),
            Py::Int(ordinal.into()),
        ])?
        .0;
        let raw = stored::get(q, "detail_json");
        let detail = if !stored::truth(raw) {
            Some(Py::Dict(vec![]))
        } else if let Py::Str(s) = raw {
            s.as_str().and_then(|s| codec::loads(s).ok())
        } else {
            None
        };
        let detail = match detail {
            Some(v @ Py::Dict(_)) => v,
            Some(_) => Py::Dict(vec![]),
            None => Py::Dict(vec![(
                kassiber_pyjson::Key::Str("unparsed_detail".into()),
                if stored::truth(raw) {
                    string(raw)
                } else {
                    Py::Str("".into())
                },
            )]),
        };
        originals.push((safe_hash, exact_hash, id, reason, detail));
    }
    let mut snapshots: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for row in &snapshot.evidence_snapshots {
        let row = json!(row);
        snapshots
            .entry(text(&row["subject_id"]))
            .or_default()
            .push(row);
    }
    let reviews: Vec<_> = snapshot.gap_reviews.iter().map(|r| json!(r)).collect();
    let value = build(
        &json!({"canonical":evidence["canonical_input"],"rows":evidence["enriched_rows"],"components":effective,"evidence":snapshots,"claims":converted["claims"],"native":converted["native_audits"],"blockers":safe,"search":gaps,"ignored":ignored,"dismissed":crate::gaps::dismissed_fingerprints(&reviews)}),
    )?;
    let mut value = codec::from_json(&value)?;
    let mut issues = stored::list(stored::get(&value, "issues")).to_vec();
    for (safe_hash, exact_hash, id, reason, detail) in originals {
        let q = issues
            .iter_mut()
            .find(|q| stored::get(q, "issue_id") == &Py::Str(safe_hash.clone().into()))
            .ok_or("KeyError")?;
        stored::put(q, "issue_id", Py::Str(exact_hash.into()));
        stored::put(
            q,
            "transaction_ids",
            Py::List(if stored::truth(&id) { vec![id] } else { vec![] }),
        );
        stored::put(q, "reason", reason);
        stored::put(q, "details", detail);
    }
    issues.sort_by_key(|q| {
        let field = |key| {
            if let Py::Str(s) = stored::get(q, key) {
                s.clone()
            } else {
                unreachable!()
            }
        };
        (field("occurred_at"), field("issue_id"))
    });
    stored::put(&mut value, "issues", Py::List(issues));
    Ok(value)
}
