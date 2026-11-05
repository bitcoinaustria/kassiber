//! Comparison-only runtime composition. No persistence or authority boundary.
use super::{array, domain, Canon, QEvent};
use crate::evidence::canonical::{EventRef, LegRef, Obs};
use crate::{codec, python_text::PythonText};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
type Order = [String; 5];
type Pool = [String; 4];
fn text(v: &Value) -> String {
    if codec::truth(v) {
        codec::pystr(v)
    } else {
        String::new()
    }
}
fn pool<O: Obs + ?Sized>(o: &O) -> Pool {
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
/// Rows by transaction id: a JSON object, or borrowed rows keyed the same way.
pub trait Rows {
    fn get(&self, id: String) -> Option<&Value>;
}
impl Rows for Value {
    fn get(&self, id: String) -> Option<&Value> {
        Value::get(self, id)
    }
}
impl Rows for BTreeMap<String, &Value> {
    fn get(&self, id: String) -> Option<&Value> {
        BTreeMap::get(self, &id).copied()
    }
}

/// Deterministic barriers preserve finalized siblings, while equal-time distinct
/// events fail closed. Pool identity deliberately omits the rail.
pub fn tax_eligibility(
    canonical: &Value,
    projection: &Value,
    issues: &[Value],
    rows: &impl Rows,
) -> Result<Value, String> {
    tax_eligibility_events(
        &crate::evidence::canonical::json_events(canonical),
        &super::arbitration::View::of(projection),
        issues,
        rows,
    )
}
fn tax_eligibility_events(
    events: &[EventRef],
    projection: &super::arbitration::View,
    issues: &[Value],
    rows: &impl Rows,
) -> Result<Value, String> {
    let mut orders = BTreeMap::new();
    let mut pools = BTreeMap::new();
    let mut aliases = BTreeMap::new();
    for event in events {
        let [chain, network, namespace, event_id] =
            event.key_texts().map(std::borrow::Cow::into_owned);
        let order: Order = [
            event
                .legs()
                .map(|o| text(&o["occurred_at"]))
                .min()
                .unwrap_or_default(),
            chain,
            network,
            namespace,
            event_id,
        ];
        match event.typed_aliases() {
            Some(typed) => {
                for alias in typed {
                    let hash = text(&event.leg(alias.leg)["quantity_hash"]);
                    aliases.insert(alias.transaction_id.clone(), hash);
                }
            }
            None => {
                for alias in event.aliases() {
                    aliases.insert(text(&alias[0]), text(&alias[1]));
                }
            }
        }
        for leg in event.legs() {
            let hash = text(&leg["quantity_hash"]);
            orders.insert(hash.clone(), order.clone());
            pools.insert(hash, pool(&leg));
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
            for o in &projection.observations {
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
    for d in &projection.decisions {
        if !matches!(
            d["state"].as_str(),
            Some(
                "internal_verified"
                    | "internal_reviewed"
                    | "external_confirmed"
                    | "external_presumed"
            )
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
        eligible.push((*d).clone());
    }
    // serde_json's encoding of a Value is injective, so text equality is
    // Value equality; this keeps the membership test linear.
    let eligible_sources: std::collections::HashSet<String> =
        eligible.iter().map(|e| e["source"].to_string()).collect();
    let mut ineligible: Vec<_> = projection
        .decisions
        .iter()
        .filter(|d| !eligible_sources.contains(&d["source"].to_string()))
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
fn observations(c: Canon<'_>) -> Result<Vec<(String, LegRef<'_>)>, String> {
    observations_needed(c, true)
}
fn observations_needed(c: Canon<'_>, needed: bool) -> Result<Vec<(String, LegRef<'_>)>, String> {
    let events = c.events();
    // A typed alias names the leg whose quantity hash it holds, so every
    // alias resolves without a hash index.
    if events.iter().all(|event| event.typed_aliases().is_some()) {
        let mut result: Vec<(String, LegRef)> = Vec::new();
        if !needed {
            return Ok(result);
        }
        let mut positions: crate::fxhash::HashMap<&str, usize> = Default::default();
        for event in &events {
            for alias in event.typed_aliases().expect("typed aliases") {
                let o = event.leg(alias.leg);
                let id = alias.transaction_id.as_str();
                // Insertion order is the first occurrence; a later alias replaces it.
                if let Some(&at) = positions.get(id) {
                    result[at].1 = o;
                } else {
                    positions.insert(id, result.len());
                    result.push((id.to_owned(), o));
                }
            }
        }
        return Ok(result);
    }
    if !needed
        && events.iter().all(|event| {
            event.aliases().iter().all(|alias| {
                alias[1].as_str().is_some_and(|hash| {
                    event
                        .legs()
                        .take(8)
                        .any(|leg| leg["quantity_hash"].as_str() == Some(hash))
                })
            })
        })
    {
        return Ok(Vec::new());
    }
    let hashes: HashMap<_, _> = events
        .iter()
        .flat_map(|e| e.legs())
        .map(|o| (super::text_key(o.at("quantity_hash")), o))
        .collect();
    let mut result: Vec<(String, LegRef)> = Vec::new();
    let mut positions: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for event in &events {
        for a in event.aliases() {
            let o = *hashes
                .get(super::text_key(&a[1]).as_ref())
                .ok_or("KeyError")?;
            if !needed {
                continue;
            }
            let id = text(&a[0]);
            // Insertion order is the first occurrence; a later alias replaces it.
            if let Some(&at) = positions.get(&id) {
                result[at].1 = o;
            } else {
                positions.insert(id.clone(), result.len());
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
    let canonical = Canon::Json(canonical);
    component_claims_observed(canonical, components, evidence, &observations(canonical)?)
}
fn component_claims_observed(
    canonical: Canon<'_>,
    components: &[Value],
    evidence: &Value,
    obs: &[(String, LegRef<'_>)],
) -> Result<Value, String> {
    if components.is_empty() {
        return Ok(json!([[], [], [], []]));
    }
    let observed =
        super::reviewed::Observations::new(obs.iter().map(|(k, v)| (k.clone(), *v)).collect());
    let mut first_observation: HashMap<&str, LegRef> = HashMap::new();
    let mut hashes_by_tx: HashMap<&str, Vec<String>> = HashMap::new();
    for (tx, o) in obs {
        first_observation.entry(tx.as_str()).or_insert(*o);
        hashes_by_tx
            .entry(tx.as_str())
            .or_default()
            .push(text(&o["quantity_hash"]));
    }
    let snapshot_hashes: BTreeSet<_> = canonical
        .events()
        .into_iter()
        .flat_map(|e| e.snapshot_quantity_hashes())
        .collect();
    let mut components: Vec<&Value> = components.iter().collect();
    components.sort_by_cached_key(|c| text(&c["id"]));
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
            let relevant: BTreeSet<_> = ids
                .iter()
                .filter_map(|tx| hashes_by_tx.get(tx.as_str()))
                .flatten()
                .cloned()
                .collect();
            let current: BTreeSet<_> = relevant
                .iter()
                .filter(|h| snapshot_hashes.contains(h.as_str()))
                .cloned()
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
            match super::reviewed::compile_observed(c, Some(&observed)) {
                Ok(mut v) => {
                    // The compilation is this caller's; move its lists out.
                    let mut take = |key: &str| match v[key].take() {
                        Value::Array(items) => items,
                        _ => Vec::new(),
                    };
                    claims.extend(take("claims"));
                    conversions.extend(take("reviewed_conversion_pairs"));
                    payouts.extend(take("reviewed_direct_payouts"));
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
        let mut sources: Vec<LegRef> = Vec::new();
        for tx in &ids {
            if let Some(&o) = first_observation.get(tx.as_str()) {
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
    rows: &impl Rows,
    component: &[Value],
    gaps: &[Value],
    native: &[Value],
    blockers: &[Value],
) -> Result<Value, String> {
    whole_issues_rejected(
        array(&canonical["rejected_events"]),
        &super::arbitration::View::of(projection),
        rows,
        component,
        gaps,
        native,
        blockers,
    )
}
fn whole_issues_rejected(
    rejected: &[Value],
    projection: &super::arbitration::View,
    rows: &impl Rows,
    component: &[Value],
    gaps: &[Value],
    native: &[Value],
    blockers: &[Value],
) -> Result<Value, String> {
    let mut issues = Vec::new();
    for rejected in rejected {
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
    let needs_observations = !projection.errors.is_empty()
        || projection.decisions.iter().any(|d| {
            matches!(
                d["state"].as_str(),
                Some("custody_suspense" | "conflicting")
            )
        });
    let by_hash: HashMap<_, _> = if needs_observations {
        projection
            .observations
            .iter()
            .map(|o| (super::text_key(&o["quantity_hash"]), o))
            .collect()
    } else {
        HashMap::new()
    };
    for e in projection.errors {
        let involved: Vec<_> = array(&e["involved_observation_hashes"])
            .iter()
            .filter_map(|h| by_hash.get(super::text_key(h).as_ref()))
            .collect();
        let assets: BTreeSet<_> = involved.iter().map(|o| text(&o["asset"])).collect();
        issues.push(issue(issue_id(&[json!("claim_bundle_invalid"),e["bundle_id"].clone()])?,"quantity_claim_bundle_invalid",if assets.len()==1 {json!(assets.first().unwrap())} else {Value::Null},involved.iter().map(|o|text(&o["occurred_at"])).min().unwrap_or_default(),json!(involved.iter().map(|o|text(&o["anchor_transaction_id"])).collect::<BTreeSet<_>>()),"malformed_claim_bundle",json!({"bundle_id":e["bundle_id"],"claim_ids":e["claim_ids"],"validation_reasons":e["reasons"]})));
    }
    for (ordinal, b) in blockers.iter().enumerate() {
        let id = text(&b["transaction_id"]);
        let row = rows.get(id.clone()).unwrap_or(&Value::Null);
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
    for d in &projection.decisions {
        if matches!(
            d["state"].as_str(),
            Some("custody_suspense" | "conflicting")
        ) {
            let s = &d["source"];
            let o = by_hash
                .get(super::text_key(&s["observation_hash"]).as_ref())
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
    issues.sort_by_cached_key(|i| (text(&i["occurred_at"]), text(&i["issue_id"])));
    Ok(json!(issues))
}
/// Private typed runtime operands by reference, indexed by their operand
/// names; an unknown name is null, as in a JSON object.
struct Operands<'a> {
    canonical: Canon<'a>,
    rows: RowSource<'a>,
    components: &'a Value,
    evidence: &'a Value,
    claims: &'a Value,
    native: &'a Value,
    blockers: &'a Value,
    search: &'a Value,
    ignored: &'a Value,
    dismissed: &'a Value,
}
impl<'a> Operands<'a> {
    fn of(v: &'a Value) -> Self {
        Self {
            canonical: Canon::Json(&v["canonical"]),
            rows: RowSource::Values(array(&v["rows"])),
            components: &v["components"],
            evidence: &v["evidence"],
            claims: &v["claims"],
            native: &v["native"],
            blockers: &v["blockers"],
            search: &v["search"],
            ignored: &v["ignored"],
            dismissed: &v["dismissed"],
        }
    }
}
impl std::ops::Index<&str> for Operands<'_> {
    type Output = Value;
    fn index(&self, key: &str) -> &Value {
        static NULL: Value = Value::Null;
        match key {
            "components" => self.components,
            "evidence" => self.evidence,
            "claims" => self.claims,
            "native" => self.native,
            "blockers" => self.blockers,
            "search" => self.search,
            "ignored" => self.ignored,
            "dismissed" => self.dismissed,
            _ => &NULL,
        }
    }
}
/// The computed parts of a `CanonicalQuantityState`.
struct State<'a> {
    native: super::native::Compilation<'a>,
    projection: super::arbitration::ProjectionState,
    issues: Value,
    /// `None` when no issue blocks eligibility; see `unblocked_eligibility`.
    tax: Option<kassiber_pyjson::Value>,
    gaps: Value,
    comp: Value,
}
impl State<'_> {
    /// `codec::from_json(&self.json())`, converting each part in place:
    /// the JSON object holds its keys in sorted order.
    fn py(self) -> Result<kassiber_pyjson::Value, String> {
        use kassiber_pyjson::{Key, Value as Py};
        let events = &self.native.events;
        let projection = self
            .projection
            .py(|(e, l)| events[e as usize].get().leg(l as usize))?;
        let tax = match self.tax {
            Some(tax) => tax,
            None => unblocked_eligibility(&projection),
        };
        let canonical = Py::Dict(vec![
            (
                Key::Str("$type".into()),
                Py::Str("CanonicalQuantityInput".into()),
            ),
            (
                Key::Str("events".into()),
                Py::List(
                    self.native
                        .events
                        .iter()
                        .map(|e| super::borrowed_py(&e.get().value()))
                        .collect::<Result<_, _>>()?,
                ),
            ),
            (
                Key::Str("rejected_events".into()),
                Py::List(
                    self.native
                        .rejected
                        .iter()
                        .map(super::borrowed_py)
                        .collect::<Result<_, _>>()?,
                ),
            ),
        ]);
        let mut gaps = self.gaps;
        let mut comp = self.comp;
        let parts = [
            ("$type", Py::Str("CanonicalQuantityState".into())),
            ("canonical_input", canonical),
            (
                "gap_candidate_transaction_ids",
                super::into_py(gaps[2].take())?,
            ),
            ("gap_holds", super::into_py(gaps[3].take())?),
            ("issues", super::into_py(self.issues)?),
            ("projection", projection),
            ("reviewed_conversion_pairs", super::into_py(comp[2].take())?),
            ("reviewed_direct_payouts", super::into_py(comp[3].take())?),
            ("tax_eligibility", tax),
        ];
        Ok(Py::Dict(
            parts
                .into_iter()
                .map(|(key, value)| (Key::Str(key.into()), value))
                .collect(),
        ))
    }
}
/// Private typed runtime operands; raw snapshot v1 never contains these values.
pub fn build(v: &Value) -> Result<Value, String> {
    compose(&Operands::of(v))?.py().map(|v| codec::to_json(&v))
}
fn compose<'a>(v: &Operands<'a>) -> Result<State<'a>, String> {
    let canonical = v.canonical;
    let components = array(&v["components"]);
    let span = crate::timing::span("quantity.components");
    let aliases = observations_needed(
        canonical,
        !components.is_empty() || !array(&v["native"]).is_empty(),
    )?;
    let mut comp = component_claims_observed(canonical, components, &v["evidence"], &aliases)?;
    drop(span);
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
    let canonical_events = canonical.events();
    let by_hash: HashMap<_, _> = if array(&v["claims"]).is_empty() {
        HashMap::new()
    } else {
        canonical_events
            .iter()
            .flat_map(|e| e.legs())
            .map(|o| (super::text_key(o.at("quantity_hash")), o))
            .collect()
    };
    for c in array(&v["claims"]) {
        for field in ["source", "target"] {
            if let Some(o) = by_hash.get(super::text_key(&c[field]["observation_hash"]).as_ref()) {
                ignored.insert(text(&o["transaction_id"]));
            }
        }
    }
    let span = crate::timing::span("quantity.holds");
    let mut gaps = super::holds::compose_canon(
        canonical,
        &ignored.into_iter().map(|s| json!(s)).collect::<Vec<_>>(),
        &v["dismissed"],
        &v["search"],
    )?;
    drop(span);
    let span = crate::timing::span("quantity.native");
    let audits = array(&v["native"]);
    let key = |event: &EventRef| match event {
        // A typed key holds its fields as text.
        EventRef::Typed(_) => event.key_texts().map(std::borrow::Cow::into_owned),
        EventRef::Json(_) => {
            let key = event.key();
            ["chain", "network", "native_namespace", "native_event_id"]
                .map(|field| super::text_key(&key[field]).into_owned())
        }
    };
    // Without audits, sorted canonical events are the compilation's events;
    // otherwise the native compiler reads the JSON input.
    let mut native = if audits.is_empty()
        && canonical_events
            .windows(2)
            .all(|pair| key(&pair[0]) < key(&pair[1]))
    {
        super::native::Compilation {
            events: canonical_events.iter().copied().map(QEvent::Ref).collect(),
            rejected: canonical.rejected(),
            claims: vec![],
            issues: vec![],
        }
    } else {
        let shared = matches!(canonical, Canon::Json(_)).then_some(aliases.as_slice());
        super::native::compile_borrowed(
            canonical.json(),
            audits,
            &component_ids
                .into_iter()
                .map(|s| json!(s))
                .collect::<Vec<_>>(),
            &json!({}),
            shared,
        )?
    };
    let mut claims = match comp[0].take() {
        Value::Array(claims) => claims,
        _ => vec![],
    };
    if let Value::Array(hold_claims) = gaps[0].take() {
        claims.extend(hold_claims);
    }
    claims.append(&mut native.claims);
    claims.extend_from_slice(array(&v["claims"]));
    let events: Vec<EventRef> = native.events.iter().map(QEvent::get).collect();
    let observations: Vec<LegRef> = events.iter().flat_map(|e| e.legs()).collect();
    drop(span);
    let span = crate::timing::span("quantity.project");
    let projection = super::arbitration::arbitrate(&observations, &claims)?;
    let view = projection.issues_view();
    drop(span);
    let _span = crate::timing::span("quantity.issues");
    // Rows by id, as the JSON object of copies would hold them.
    let rows = IndexedRows {
        rows: v.rows,
        index: std::cell::OnceCell::new(),
        values: std::cell::OnceCell::new(),
    };
    let issues = whole_issues_rejected(
        native.rejected,
        &view,
        &rows,
        array(&comp[1]),
        array(&gaps[1]),
        &native.issues,
        array(&v["blockers"]),
    )?;
    let tax = if array(&issues).is_empty() {
        None
    } else {
        Some(super::into_py(tax_eligibility_events(
            &events,
            &projection.view(),
            array(&issues),
            &rows,
        )?)?)
    };
    // Locate each observation by its event slot and leg, so the result
    // owns no borrow of the events it reads.
    let mut located = HashMap::new();
    for (slot, event) in events.iter().enumerate() {
        for (leg, observation) in event.legs().enumerate() {
            located.insert(identity(observation), (slot as u32, leg as u32));
        }
    }
    let projection = projection.located(|o| located[&identity(o)]);
    drop(events);
    Ok(State {
        native,
        projection,
        issues,
        tax,
        gaps,
        comp,
    })
}

/// An observation's address, the identity `located` maps.
fn identity(observation: LegRef) -> usize {
    match observation {
        LegRef::Typed(leg) => leg as *const _ as usize,
        LegRef::Json(value) => value as *const _ as usize,
    }
}
/// Recompute quantity from preceding Rust stages and the raw snapshot only.
/// The components that stay effective: every one not reconciled. The stage's
/// own array serves, uncopied, when none is.
fn effective_components<'a>(
    all: &'a Value,
    reconciled: &BTreeSet<String>,
) -> std::borrow::Cow<'a, Value> {
    let kept = |c: &&Value| !reconciled.contains(&text(&c["id"]));
    if all.is_array() && array(all).iter().all(|c| kept(&c)) {
        return std::borrow::Cow::Borrowed(all);
    }
    std::borrow::Cow::Owned(Value::Array(
        array(all).iter().filter(kept).cloned().collect(),
    ))
}

pub fn stage(
    snapshot: &crate::snapshot::Snapshot,
    components: &Value,
    evidence: &crate::evidence::Evidence,
    interpretation: &kassiber_pyjson::Value,
    gaps: &Value,
) -> Result<QuantityState, String> {
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
        if !codec::lossless(value)
            && codec::dumps(value, true)? != codec::dumps(&codec::from_json(&json)?, true)?
        {
            return Err("ValueError".into());
        }
        converted.insert(key.into(), json);
    }
    let converted = Value::Object(converted);
    let reconciled: BTreeSet<_> = array(&converted["native_component_reconciliations"])
        .iter()
        .map(|r| text(&r["component_id"]))
        .collect();
    let effective = effective_components(&components["components"], &reconciled);
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
    // The operands `build` would read from one JSON object, without copying.
    let components = &*effective;
    let snapshots = Value::Object(
        snapshots
            .into_iter()
            .map(|(k, v)| (k, Value::Array(v)))
            .collect(),
    );
    let blockers = Value::Array(safe);
    let ignored = Value::Array(ignored.into_iter().map(Value::String).collect());
    let dismissed = crate::gaps::dismissed_fingerprints(&reviews);
    let state = compose(&Operands {
        canonical: Canon::Typed(evidence.canonical_input()),
        rows: RowSource::Enriched(evidence.enriched(), &snapshot.transactions),
        components,
        evidence: &snapshots,
        claims: &converted["claims"],
        native: &converted["native_audits"],
        blockers: &blockers,
        search: gaps,
        ignored: &ignored,
        dismissed: &dismissed,
    })?;
    let span = crate::timing::span("quantity.state");
    let State {
        native,
        projection,
        issues,
        tax,
        mut gaps,
        mut comp,
    } = state;
    let canonical = evidence.canonical_arc();
    let typed: HashMap<usize, usize> = canonical
        .events()
        .iter()
        .enumerate()
        .map(|(i, event)| (event as *const _ as usize, i))
        .collect();
    // A native recompilation reads the canonical input's built JSON.
    let json: HashMap<usize, usize> = canonical
        .built_value()
        .map(|value| array(&value["events"]))
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .map(|(i, event)| (event as *const _ as usize, i))
        .collect();
    let events = native
        .events
        .into_iter()
        .map(|event| match event {
            QEvent::Ref(EventRef::Typed(event)) => {
                Slot::Typed(typed[&(event as *const _ as usize)])
            }
            QEvent::Ref(EventRef::Json(event)) => Slot::Json(json[&(event as *const _ as usize)]),
            QEvent::Owned(event) => Slot::Owned(event),
        })
        .collect();
    let mut issues = match super::into_py(issues)? {
        Py::List(issues) => issues,
        _ => unreachable!("issue list"),
    };
    drop(span);
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
    issues.sort_by_cached_key(|q| {
        let field = |key| {
            if let Py::Str(s) = stored::get(q, key) {
                s.clone()
            } else {
                unreachable!()
            }
        };
        (field("occurred_at"), field("issue_id"))
    });
    Ok(QuantityState {
        canonical,
        events,
        projection,
        gap_candidate_transaction_ids: gaps[2].take(),
        gap_holds: gaps[3].take(),
        issues,
        reviewed_conversion_pairs: comp[2].take(),
        reviewed_direct_payouts: comp[3].take(),
        tax,
    })
}

/// A JSON object of moved parts; `json!` would copy each part again.
fn object<const N: usize>(parts: [(&str, Value); N]) -> Value {
    Value::Object(parts.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

/// Whether `value` converts to Python and back to exactly itself: no float
/// and only canonically spelled integers.
pub(crate) fn plain(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(_) | Value::String(_) => true,
        Value::Number(n) => {
            if n.as_i64().is_some() {
                return true;
            }
            let s = n.as_str();
            let digits = s.strip_prefix('-').unwrap_or(s);
            !digits.is_empty()
                && digits.bytes().all(|b| b.is_ascii_digit())
                && !digits.starts_with('0')
        }
        Value::Array(items) => items.iter().all(plain),
        Value::Object(items) => items.values().all(plain),
    }
}

/// A quantity event: a canonical event shared with evidence, or one native
/// augmentation rewrote.
enum Slot {
    /// A typed canonical event.
    Typed(usize),
    /// An event of the canonical input's built JSON.
    Json(usize),
    Owned(Value),
}

/// The quantity stage. Observations stay in their canonical events; its
/// Python value is built only on request.
pub struct QuantityState {
    canonical: std::sync::Arc<crate::evidence::canonical::Canonical>,
    events: Vec<Slot>,
    projection: super::arbitration::ProjectionState,
    gap_candidate_transaction_ids: Value,
    gap_holds: Value,
    issues: Vec<kassiber_pyjson::Value>,
    reviewed_conversion_pairs: Value,
    reviewed_direct_payouts: Value,
    /// `None` when no issue blocks eligibility; see `unblocked_eligibility`.
    tax: Option<kassiber_pyjson::Value>,
}
impl QuantityState {
    pub(crate) fn event(&self, slot: usize) -> EventRef<'_> {
        match &self.events[slot] {
            Slot::Typed(at) => EventRef::Typed(&self.canonical.events()[*at]),
            Slot::Json(at) => EventRef::Json(&array(&self.canonical.value()["events"])[*at]),
            Slot::Owned(event) => EventRef::Json(event),
        }
    }
    pub(crate) fn events(&self) -> impl Iterator<Item = EventRef<'_>> {
        (0..self.events.len()).map(|slot| self.event(slot))
    }
    fn located(&self, (event, leg): super::arbitration::Located) -> LegRef<'_> {
        self.event(event as usize).leg(leg as usize)
    }
    /// Projection observations, in quantity-hash order.
    pub(crate) fn observations(&self) -> impl Iterator<Item = LegRef<'_>> {
        self.projection
            .observations
            .iter()
            .map(|at| self.located(*at))
    }
    /// Arbitrated decisions as their complete JSON objects.
    /// The canonical input the state's typed events belong to.
    pub(crate) fn canonical_arc(&self) -> std::sync::Arc<crate::evidence::canonical::Canonical> {
        self.canonical.clone()
    }
    pub(crate) fn decisions(&self) -> impl Iterator<Item = super::arbitration::DecisionRef<'_>> {
        self.projection.decisions.iter().map(|d| d.as_ref())
    }
    pub(crate) fn issues(&self) -> &[kassiber_pyjson::Value] {
        &self.issues
    }
    pub(crate) fn reviewed_conversion_pairs(&self) -> &Value {
        &self.reviewed_conversion_pairs
    }
    pub(crate) fn reviewed_direct_payouts(&self) -> &Value {
        &self.reviewed_direct_payouts
    }
    /// The blocked tax eligibility, or `None` when every decision in an
    /// eligible state is eligible.
    pub(crate) fn blocked_eligibility(&self) -> Option<&kassiber_pyjson::Value> {
        self.tax.as_ref()
    }
    /// `convert(&self.py()?)` for a converter that maps every Python value to
    /// its JSON equivalent: plain JSON parts are cloned, every other part is
    /// converted, in the order `convert` would traverse the whole value.
    /// Without `canonical`, the canonical input, which no journal reads and
    /// whose conversion cannot fail, is left out.
    pub fn json_with<E: From<String>>(
        &self,
        canonical: bool,
        convert: impl Fn(&kassiber_pyjson::Value) -> Result<Value, E>,
    ) -> Result<Value, E> {
        let exact = |v: &Value| -> Result<Value, E> {
            if plain(v) {
                Ok(v.clone())
            } else {
                convert(&super::borrowed_py(v)?)
            }
        };
        let canonical = if canonical {
            let events = self
                .events()
                .map(|e| exact(&e.value()))
                .collect::<Result<Vec<_>, E>>()?;
            Some(object([
                ("$type", json!("CanonicalQuantityInput")),
                ("events", Value::Array(events)),
                (
                    "rejected_events",
                    Value::Array(
                        self.canonical
                            .rejected()
                            .iter()
                            .map(exact)
                            .collect::<Result<_, E>>()?,
                    ),
                ),
            ]))
        } else {
            None
        };
        let gap_candidates = exact(&self.gap_candidate_transaction_ids)?;
        let gap_holds = exact(&self.gap_holds)?;
        let issues = self
            .issues
            .iter()
            .map(&convert)
            .collect::<Result<Vec<_>, E>>()?;
        let errors = exact(&Value::Array(self.projection.errors.clone()))?;
        let decisions = self
            .projection
            .decisions
            .iter()
            .map(|d| exact(d.value()))
            .collect::<Result<Vec<_>, E>>()?;
        let tax = match &self.tax {
            Some(_) => None,
            None => {
                let mut eligible = Vec::new();
                let mut ineligible = Vec::new();
                for d in &decisions {
                    if matches!(
                        d["state"].as_str(),
                        Some(
                            "internal_verified"
                                | "internal_reviewed"
                                | "external_confirmed"
                                | "external_presumed"
                        )
                    ) {
                        eligible.push(d.clone());
                    } else {
                        ineligible.push(d["source"].clone());
                    }
                }
                Some(object([
                    ("$type", json!("QuantityTaxEligibility")),
                    ("barrier_event_key", Value::Null),
                    ("blocked_from", Value::Null),
                    ("eligible_decisions", Value::Array(eligible)),
                    ("ineligible_slices", Value::Array(ineligible)),
                    ("pool_barrier_roots", json!([])),
                    ("pool_barriers", json!([])),
                ]))
            }
        };
        // A typed observation's JSON is built once and moved when plain.
        let owned = |v: std::borrow::Cow<'_, Value>| -> Result<Value, E> {
            if plain(&v) {
                Ok(v.into_owned())
            } else {
                convert(&super::borrowed_py(&v)?)
            }
        };
        let observations = self
            .observations()
            .map(|o| owned(o.value()))
            .collect::<Result<Vec<_>, E>>()?;
        let postings = self
            .projection
            .postings_json(|at| self.located(at), &exact)?;
        let projection = object([
            ("$type", json!("QuantityProjection")),
            ("claim_errors", errors),
            ("decisions", Value::Array(decisions)),
            ("observations", Value::Array(observations)),
            ("postings", Value::Array(postings)),
        ]);
        let pairs = exact(&self.reviewed_conversion_pairs)?;
        let payouts = exact(&self.reviewed_direct_payouts)?;
        let tax = match (&self.tax, tax) {
            (Some(tax), _) => convert(tax)?,
            (None, tax) => tax.expect("unblocked eligibility"),
        };
        let mut state = object([("$type", json!("CanonicalQuantityState"))]);
        if let Some(canonical) = canonical {
            state["canonical_input"] = canonical;
        }
        let Value::Object(state) = &mut state else {
            unreachable!("object")
        };
        state.extend(
            [
                ("gap_candidate_transaction_ids", gap_candidates),
                ("gap_holds", gap_holds),
                ("issues", Value::Array(issues)),
                ("projection", projection),
                ("reviewed_conversion_pairs", pairs),
                ("reviewed_direct_payouts", payouts),
                ("tax_eligibility", tax),
            ]
            .map(|(k, v)| (k.to_owned(), v)),
        );
        Ok(Value::Object(std::mem::take(state)))
    }

    /// The stage's Python value, as Python's composition returns it.
    pub fn py(&self) -> Result<kassiber_pyjson::Value, String> {
        use kassiber_pyjson::{Key, Value as Py};
        let projection = self.projection.py(|at| self.located(at))?;
        let tax = match &self.tax {
            Some(tax) => tax.clone(),
            None => unblocked_eligibility(&projection),
        };
        let canonical = Py::Dict(vec![
            (
                Key::Str("$type".into()),
                Py::Str("CanonicalQuantityInput".into()),
            ),
            (
                Key::Str("events".into()),
                Py::List(
                    self.events()
                        .map(|e| super::borrowed_py(&e.value()))
                        .collect::<Result<_, _>>()?,
                ),
            ),
            (
                Key::Str("rejected_events".into()),
                kassiber_pyjson::Value::List(
                    self.canonical
                        .rejected()
                        .iter()
                        .map(super::borrowed_py)
                        .collect::<Result<_, _>>()?,
                ),
            ),
        ]);
        let parts = [
            ("$type", Py::Str("CanonicalQuantityState".into())),
            ("canonical_input", canonical),
            (
                "gap_candidate_transaction_ids",
                super::borrowed_py(&self.gap_candidate_transaction_ids)?,
            ),
            ("gap_holds", super::borrowed_py(&self.gap_holds)?),
            ("issues", Py::List(self.issues.clone())),
            ("projection", projection),
            (
                "reviewed_conversion_pairs",
                super::borrowed_py(&self.reviewed_conversion_pairs)?,
            ),
            (
                "reviewed_direct_payouts",
                super::borrowed_py(&self.reviewed_direct_payouts)?,
            ),
            ("tax_eligibility", tax),
        ];
        Ok(Py::Dict(
            parts
                .into_iter()
                .map(|(key, value)| (Key::Str(key.into()), value))
                .collect(),
        ))
    }
}

fn unblocked_eligibility(projection: &kassiber_pyjson::Value) -> kassiber_pyjson::Value {
    use crate::graph::stored;
    use kassiber_pyjson::Value as Py;
    let mut eligible = Vec::new();
    let mut ineligible = Vec::new();
    for decision in stored::list(stored::get(projection, "decisions")) {
        let state = stored::get(decision, "state");
        if matches!(state, Py::Str(s) if s.is("internal_verified") || s.is("internal_reviewed") || s.is("external_confirmed") || s.is("external_presumed"))
        {
            eligible.push(decision.clone());
        } else {
            ineligible.push(stored::get(decision, "source").clone());
        }
    }
    super::py_dict([
        ("$type", Py::Str("QuantityTaxEligibility".into())),
        ("barrier_event_key", Py::Null),
        ("blocked_from", Py::Null),
        ("eligible_decisions", Py::List(eligible)),
        ("ineligible_slices", Py::List(ineligible)),
        ("pool_barrier_roots", Py::List(vec![])),
        ("pool_barriers", Py::List(vec![])),
    ])
}

/// Enriched rows, as JSON values or as typed evidence over snapshot rows.
#[derive(Clone, Copy)]
enum RowSource<'a> {
    Values(&'a [Value]),
    Enriched(
        &'a [crate::evidence::Enriched],
        &'a [crate::snapshot::Record],
    ),
}

struct IndexedRows<'a> {
    rows: RowSource<'a>,
    index: std::cell::OnceCell<BTreeMap<String, usize>>,
    /// Typed rows' JSON objects, built when an issue reads them.
    values: std::cell::OnceCell<Vec<std::cell::OnceCell<Value>>>,
}
impl Rows for IndexedRows<'_> {
    fn get(&self, id: String) -> Option<&Value> {
        use crate::fields::Fields;
        let index = self.index.get_or_init(|| {
            let mut index = BTreeMap::new();
            let mut add = |at: usize, id: &Value| {
                if !id.is_null() && id != "" {
                    index.insert(text(id), at);
                }
            };
            match self.rows {
                RowSource::Values(rows) => {
                    rows.iter().enumerate().for_each(|(i, r)| add(i, &r["id"]))
                }
                RowSource::Enriched(rows, records) => rows
                    .iter()
                    .enumerate()
                    .for_each(|(i, r)| add(i, r.view(records).at("id"))),
            }
            index
        });
        let at = *index.get(&id)?;
        Some(match self.rows {
            RowSource::Values(rows) => &rows[at],
            RowSource::Enriched(rows, records) => self.values.get_or_init(|| {
                (0..rows.len())
                    .map(|_| std::cell::OnceCell::new())
                    .collect()
            })[at]
                .get_or_init(|| rows[at].view(records).value()),
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn typed_aliases_resolve_as_json_aliases() {
        let row = |id: &str, event: &str, wallet: &str| json!({"id":id,"profile_id":"p","wallet_id":wallet,"wallet_kind":"descriptor","direction":"inbound","asset":"BTC","amount":1000,"fee":0,"occurred_at":"2024-01-01T00:00:00+00:00","source_ref":"src","external_id":event,"raw_json":"{}","network":"regtest"});
        // A transaction id in two events, and an event with two slots.
        let rows = [
            row("t1", "e1", "w"),
            row("t2", "e2", "w"),
            row("t3", "e2", "v"),
            row("t1", "e3", "w"),
        ];
        let typed = crate::evidence::canonical_of_rows(&rows).unwrap();
        let json = typed.value().clone();
        let pairs = |c: Canon<'_>| -> Vec<(String, Value)> {
            observations(c)
                .unwrap()
                .into_iter()
                .map(|(id, o)| (id, o.value().into_owned()))
                .collect()
        };
        let expected = pairs(Canon::Json(&json));
        assert_eq!(expected.len(), 3);
        assert_eq!(pairs(Canon::Typed(&typed)), expected);
        // Each row is its own slot's observation, so it is its alias's leg.
        for (id, o) in &expected {
            assert_eq!(o["transaction_id"], json!(id));
        }
    }

    use super::*;

    #[test]
    fn quantity_plain_json_is_exact() {
        for text in [
            "0",
            "-0",
            "1",
            "-12",
            "1.5",
            "1e5",
            "100000.0",
            "18446744073709551616",
            "-18446744073709551617",
            "[1,{\"a\":-0}]",
            "{\"a\":[null,true,\"x\"]}",
        ] {
            let value: Value = serde_json::from_str(text).unwrap();
            let converted = codec::to_json(&super::super::borrowed_py(&value).unwrap());
            // Plain values are cloned, so they must convert to themselves.
            if plain(&value) {
                assert_eq!(converted.to_string(), value.to_string(), "{text}");
            }
        }
        for text in ["1e5", "1.5", "-0.0", "[1,{\"a\":1e5}]"] {
            assert!(!plain(&serde_json::from_str(text).unwrap()), "{text}");
        }
    }

    #[test]
    fn quantity_unused_alias_validation() {
        let mut canonical = json!({"events":[
            {"legs":[{"quantity_hash":"a"}],"observation_aliases":[["tx-a","a"]]},
            {"legs":[{"quantity_hash":"b"}],"observation_aliases":[["tx-b","b"]]}
        ]});
        assert!(observations_needed(Canon::Json(&canonical), false)
            .unwrap()
            .is_empty());
        canonical["events"][0]["observation_aliases"][0][1] = json!("b");
        assert!(observations_needed(Canon::Json(&canonical), false)
            .unwrap()
            .is_empty());
        let indexed = observations_needed(Canon::Json(&canonical), true).unwrap();
        assert_eq!(indexed[0].1["quantity_hash"], "b");
        canonical["events"][0]["observation_aliases"][0][1] = json!("missing");
        assert_eq!(
            observations_needed(Canon::Json(&canonical), false).unwrap_err(),
            "KeyError"
        );
        canonical["events"][0]["legs"][0]["quantity_hash"] = json!(0);
        canonical["events"][0]["observation_aliases"][0][1] = json!("");
        assert!(observations_needed(Canon::Json(&canonical), false)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn reconciled_components_leave_the_effective_population() {
        let all = json!([{"id": "a"}, {"id": "b"}, {"id": 3}]);
        let none = BTreeSet::new();
        let kept = effective_components(&all, &none);
        assert!(matches!(kept, std::borrow::Cow::Borrowed(_)));
        assert_eq!(*kept, all);
        let reconciled = BTreeSet::from(["b".to_owned(), "3".to_owned()]);
        assert_eq!(
            *effective_components(&all, &reconciled),
            json!([{"id": "a"}])
        );
        assert_eq!(*effective_components(&Value::Null, &none), json!([]));
    }
}
