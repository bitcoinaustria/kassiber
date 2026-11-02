//! Shared pure validation and activation evidence for authoring services.
//! Storage callers hydrate the same raw snapshot as journal composition.
use super::*;

/// Validate a stored draft, including raw anchors, routes and revision links.
/// External raw-active overlaps are checked even when stored memberships are absent.
pub fn validate_activation(snapshot: &Snapshot, component: &Component) -> Result<Value, Error> {
    let mut quantity_legs = Vec::new();
    let mut quantity_allocations = Vec::new();
    for c in &snapshot.components {
        if (c.header["state"] == "active" || c.header["id"] == component.header["id"])
            && c.header["conservation_mode"] == "quantity"
        {
            quantity_legs.extend(legs(c)?);
            quantity_allocations.extend(allocations(c)?);
        }
    }
    let routes = scope::profile_routes(snapshot, &quantity_legs, &quantity_allocations)?;
    let shared = Shared::new(snapshot);
    let native = commitments::Native::new(Vec::new());
    let mut result = materialize(snapshot, &shared, component, &routes, &native)?;
    let conflicts: Vec<_> = membership(&shared, component)
        .into_iter()
        .filter(|conflict| {
            snapshot.components.iter().any(|other| {
                other.header["id"] == conflict["component_id"]
                    && other.header["lineage_id"] != component.header["lineage_id"]
            })
        })
        .collect();
    if !conflicts.is_empty() {
        result["validation"]["activatable"] = json!(false);
        result["validation"]["issues"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "code":"active_transaction_membership_conflict",
                "message":"a transaction belongs to another active component", "conflicts":conflicts
            }));
    }
    Ok(result["validation"].clone())
}

pub struct ActivationEvidence {
    pub snapshots: Vec<Value>,
    pub commitments: Vec<(String, String)>,
}

/// Author-bound raw snapshots and payload-free commitment pairs. No clock or I/O.
pub fn activation_evidence(
    snapshot: &Snapshot,
    component: &Component,
) -> Result<ActivationEvidence, Error> {
    let shared = Shared::new(snapshot);
    let mut snapshots = commitments::current(
        &shared,
        &Value::Object(component.header.clone().into_map()),
        &legs(component)?,
    )?;
    snapshots.sort_by_key(|s| (pystr(&s["quantity_hash"]), pystr(&s["detail_hash"])));
    let pairs = commitments::commitment_pairs(&snapshots)?;
    Ok(ActivationEvidence {
        snapshots,
        commitments: pairs,
    })
}

/// Evidence recheck also applies to superseded revisions used for corrections.
pub fn component_evidence_status(
    snapshot: &Snapshot,
    component: &Component,
) -> Result<Value, Error> {
    let shared = Shared::new(snapshot);
    commitments::evidence_status(
        &shared,
        &Value::Object(component.header.clone().into_map()),
        &legs(component)?,
        &component
            .commitments
            .iter()
            .map(|r| Value::Object(r.clone().into_map()))
            .collect::<Vec<_>>(),
    )
}

/// Shared imported-boundary arithmetic for guided review plans.
pub fn boundary_amounts(row: &Value) -> Result<Value, Error> {
    let principal = anchors::principal(row)?;
    let fee = exact(&codec::integer_or_zero(&row["fee"])?)?;
    let outbound = text(&row["direction"]).py_strip().py_lower() == "outbound";
    Ok(json!({
        "principal_msat": number(&principal),
        "fee_msat": number(&fee),
        "wallet_movement_msat": number(&if outbound { principal + fee } else { principal }),
    }))
}

/// Python's protocol scope includes Lightning as a rail above Bitcoin.
pub fn protocol_scope(row: &Value) -> (String, Option<String>, Option<String>) {
    match crate::evidence::scope(row) {
        Ok((rail, network)) => (
            rail.clone(),
            Some(if rail == "lightning" {
                "bitcoin".into()
            } else {
                rail
            }),
            Some(network),
        ),
        Err(_) => ("external".into(), None, None),
    }
}

/// Read-only matching adapters reuse the custody identity and timestamp rules.
pub fn matching_scope(row: &Value) -> Option<(String, String, String, String)> {
    crate::graph::onchain_scope(row)
}

pub fn matching_output_sats(output: &Value) -> Option<i64> {
    crate::graph::sats(output)
}

pub fn matching_instant(value: &Value) -> Option<f64> {
    time::instant(value).ok().flatten().map(|us| {
        // datetime.timestamp() rounds the complete integer microsecond count.
        (us - 62_135_596_800_000_000) as f64 / 1_000_000.0
    })
}

pub fn canonical_timestamp(value: &Value) -> Result<String, Error> {
    let raw = pystr(value);
    let raw = raw.py_strip();
    let Some(us) = time::instant(&json!(raw))? else {
        return Err(Error {
            code: "validation",
            message: format!("Invalid occurred_at/date value '{raw}'"),
        });
    };
    let day = us / 86_400_000_000;
    let mut lo = 1;
    let mut hi = 10000;
    let year_day = |year: i64| {
        let y = year - 1;
        y * 365 + y / 4 - y / 100 + y / 400
    };
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if year_day(mid) <= day {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let mut remaining = day - year_day(lo);
    let mut month = 1;
    for length in [
        31,
        if lo % 4 == 0 && (lo % 100 != 0 || lo % 400 == 0) {
            29
        } else {
            28
        },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ] {
        if remaining < length {
            break;
        }
        remaining -= length;
        month += 1;
    }
    let seconds = us / 1_000_000 % 86400;
    Ok(format!(
        "{lo:04}-{month:02}-{:02}T{:02}:{:02}:{:02}Z",
        remaining + 1,
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    ))
}

/// Write-boundary network evidence distinguishes absent metadata from conflicts.
pub fn network_domain_evidence(row: &Value) -> (Option<String>, bool) {
    if let Some((chain, network, _, _)) = crate::graph::onchain_scope(row) {
        let domain = match (chain.as_str(), network.as_str()) {
            ("bitcoin", _) => Some(network),
            (_, "liquidv1") => Some("main".into()),
            (_, "liquidtestnet") => Some("test".into()),
            (_, "elementsregtest") => Some("regtest".into()),
            _ => None,
        };
        let valid = domain.is_some();
        return (domain, valid);
    }
    matching_domain_fallback(row)
}

/// Network metadata after a caller has already refused on-chain scope.
pub fn matching_domain_fallback(row: &Value) -> (Option<String>, bool) {
    let raw = crate::graph::transfer_object(&row["raw_json"]);
    let config = crate::graph::transfer_object(if truth(&row["config_json"]) {
        &row["config_json"]
    } else {
        &row["wallet_config_json"]
    });
    let mut families = BTreeSet::new();
    let mut domains = BTreeSet::new();
    for payload in [&raw, &config] {
        let value = &payload["chain"];
        if !value.is_null() && value != "" {
            let family = text(value).py_strip().py_lower().replace('_', "-");
            let family = match family.as_str() {
                "bitcoin" | "btc" | "onchain" => "bitcoin",
                "liquid" | "lbtc" | "elements" => "liquid",
                "core-ln" | "core-lightning" => "bitcoin",
                k if crate::evidence::lightning(k) => "bitcoin",
                _ => return (None, false),
            };
            families.insert(family);
        }
        for key in ["network", "bitcoin_network", "chain_network"] {
            let value = &payload[key];
            if value.is_null() || value == "" {
                continue;
            }
            if text(value).py_strip().is_empty() {
                return (None, false);
            }
            let Ok(n) = crate::evidence::network("bitcoin", value)
                .or_else(|_| crate::evidence::network("liquid", value))
            else {
                return (None, false);
            };
            domains.insert(match n.as_str() {
                "liquidv1" => "main".into(),
                "liquidtestnet" => "test".into(),
                "elementsregtest" => "regtest".into(),
                _ => n,
            });
        }
    }
    let asset = text(&row["asset"])
        .py_strip()
        .py_upper()
        .replace("L-BTC", "LBTC");
    if families.len() > 1
        || (asset == "BTC" && families.contains("liquid"))
        || (asset == "LBTC" && families.contains("bitcoin"))
        || domains.len() > 1
    {
        return (None, false);
    }
    (domains.into_iter().next(), true)
}

/// Preserve Python's public validation field order without interpreting facts.
/// The issue field lists are generated by the transfer oracle from the reference.
pub fn validation_projection(value: &Value) -> Result<kassiber_pyjson::Value, String> {
    use kassiber_pyjson::{Key, Value as Py};
    use std::sync::OnceLock;
    static ISSUES: OnceLock<BTreeMap<String, Vec<String>>> = OnceLock::new();
    let issues = ISSUES.get_or_init(|| {
        serde_json::from_str(include_str!("review_field_order.json"))
            .expect("checked-in validation field order")
    });
    fn project(
        value: &Value,
        context: &str,
        issues: &BTreeMap<String, Vec<String>>,
    ) -> Result<Py, String> {
        match value {
            Value::Array(items) => Ok(Py::List(
                items
                    .iter()
                    .map(|v| project(v, context, issues))
                    .collect::<Result<_, _>>()?,
            )),
            Value::Object(fields) => {
                let context = if context == "groups" && fields.contains_key("valuation_unit") {
                    "by_valuation_unit"
                } else {
                    context
                };
                let order = match context {
                    "validation" => "conservation_mode activatable issues suspense_msat source_msat owned_destination_msat by_asset by_conservation_unit by_valuation_unit allocations warnings anchors",
                    "by_asset" => "asset source_msat destination_msat fee_msat external_msat retained_msat suspense_msat sink_msat residual_msat",
                    "by_conservation_unit" | "groups" => "exposure conservation_unit source_msat destination_msat fee_msat external_msat retained_msat suspense_msat sink_msat residual_msat",
                    "by_valuation_unit" => "valuation_unit source_amount destination_amount fee_amount external_amount retained_amount suspense_amount sink_amount residual_amount",
                    "allocations" => "valid issues edges",
                    "edges" => "id source_leg_id sink_leg_id source_amount_msat sink_amount_msat",
                    "anchors" => "valid issues warnings transaction_coverage",
                    "transaction_coverage" => "transaction_id direction partial_review raw_economic_msat reviewed_component_msat reviewed_minus_raw_msat",
                    "conflicts" => "transaction_id component_id",
                    "source_scope" | "sink_scope" | "fee_scope" => "valid rail chain network domain",
                    _ => "",
                };
                let mut keys: Vec<&str> = if let Some(order) = fields
                    .get("code")
                    .and_then(Value::as_str)
                    .and_then(|code| issues.get(code))
                {
                    order.iter().map(String::as_str).collect()
                } else {
                    order.split_whitespace().collect()
                };
                for key in fields.keys() {
                    if !keys.contains(&key.as_str()) {
                        keys.push(key);
                    }
                }
                Ok(Py::Dict(
                    keys.into_iter()
                        .filter_map(|key| fields.get(key).map(|v| (key, v)))
                        .map(|(key, value)| {
                            Ok((Key::Str(key.into()), project(value, key, issues)?))
                        })
                        .collect::<Result<_, String>>()?,
                ))
            }
            _ => codec::from_json(value),
        }
    }
    project(value, "validation", issues)
}

/// Python's `validate_component_plan` stages for unsaved rows: conservation and
/// raw anchors. Planned legs carry no component identity, so profile route
/// issues do not apply; active-membership arbitration is a storage read.
pub fn plan_validation(
    snapshot: &Snapshot,
    legs: &[Value],
    allocations: &[Value],
    header: &Value,
) -> Result<Value, Error> {
    let mut validation = conservation::validate(legs, allocations, header)?;
    let anchors = anchors::validate(
        snapshot,
        legs,
        allocations,
        &[],
        &text(&header["conservation_mode"]),
        &[],
    )?;
    if !array(&anchors["issues"]).is_empty() {
        validation["activatable"] = json!(false);
        let mut issues = array(&validation["issues"]).to_vec();
        issues.extend(array(&anchors["issues"]).iter().cloned());
        validation["issues"] = json!(issues);
    }
    if !array(&anchors["warnings"]).is_empty() {
        let mut warnings = array(&validation["warnings"]).to_vec();
        warnings.extend(array(&anchors["warnings"]).iter().cloned());
        validation["warnings"] = json!(warnings);
    }
    validation["anchors"] = anchors;
    Ok(validation)
}

/// Route continuity across existing authored-active and planned active quantity
/// rows (Python's `validate_planned_active_batch`). Issues naming components
/// keep only those touching a planned component.
pub fn planned_batch_routes(
    snapshot: &Snapshot,
    legs: &[Value],
    allocations: &[Value],
    planned: &BTreeSet<String>,
) -> Result<Vec<Value>, Error> {
    Ok(scope::profile_routes(snapshot, legs, allocations)?
        .into_iter()
        .filter(|issue| {
            let ids = array(&issue["component_ids"]);
            ids.is_empty() || ids.iter().any(|id| planned.contains(&pystr(id)))
        })
        .collect())
}

/// One stored component as Python's `get_component` materializes it, without
/// local evidence. Cross-component routes apply only when another authored-active
/// quantity component shares the profile.
pub fn materialize_component(snapshot: &Snapshot, component: &Component) -> Result<Value, Error> {
    let h = &component.header;
    let own = legs(component)?;
    let mut routes = Vec::new();
    if h["conservation_mode"] == "quantity" && !own.is_empty() {
        let mut others: Vec<_> = snapshot
            .components
            .iter()
            .filter(|c| {
                c.header["profile_id"] == h["profile_id"]
                    && c.header["state"] == "active"
                    && c.header["conservation_mode"] == "quantity"
                    && c.header["id"] != h["id"]
            })
            .collect();
        others.sort_by_key(|c| pystr(&c.header["id"]));
        let mut other_legs = Vec::new();
        let mut other_allocations = Vec::new();
        for c in others {
            other_legs.extend(legs(c)?);
            other_allocations.extend(allocations(c)?);
        }
        if !other_legs.is_empty() {
            let mut combined = own.clone();
            combined.extend(other_legs);
            let mut combined_allocations = allocations(component)?;
            combined_allocations.extend(other_allocations);
            routes = scope::profile_routes(snapshot, &combined, &combined_allocations)?;
        }
    }
    let native = if h["state"] == "active" {
        commitments::native_context(snapshot, None)?
    } else {
        Vec::new()
    };
    let shared = Shared::new(snapshot);
    let native = commitments::Native::new(native);
    let mut result = materialize(snapshot, &shared, component, &routes, &native)?;
    if h["state"] == "superseded" {
        result["effective_state"] = json!("superseded");
    } else if h["state"] != "active" {
        result["effective_state"] = json!("draft");
    }
    Ok(result)
}
