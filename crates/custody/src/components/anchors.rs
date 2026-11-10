use super::scope::*;
use super::*;

pub(super) fn validate(
    snapshot: &Snapshot,
    legs: &[Value],
    allocs: &[Value],
    terms: &[Value],
    mode: &str,
    routes: &[Value],
) -> Result<Value, Error> {
    let mut issues = Vec::new();
    let mut warnings = Vec::new();
    let mut by_transaction: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    let mut rows: BTreeMap<String, &Value> = BTreeMap::new();
    let mut excluded = BTreeSet::new();
    for l in legs {
        let tx = &l["transaction_id"];
        let anchor = or(&l["anchor_transaction_id"], tx);
        let role = text(&l["role"]);
        let reviewed = role == "retained"
            && text(&l["rail"]) == "untracked"
            && (truth(&l["location_ref"])
                || text(&l["notes"]) == "reviewed_residual:retained_custody");
        if (role == "source" || owned(l))
            && big(&l["amount_msat"]) > BigInt::from(0)
            && !truth(&l["wallet_id"])
            && !reviewed
        {
            issues.push(json!({"code":"owned_leg_wallet_missing","leg_id":l["id"],"role":role}));
        }
        if tx.is_null() {
            if !anchor.is_null() {
                issues.push(json!({"code":"anchor_transaction_retracted","message":"an imported transaction anchor was removed; re-import it or author a reviewed replacement revision","leg_id":l["id"],"transaction_id":pystr(anchor)}));
                continue;
            }
            if (role == "source" || owned(l)) && big(&l["amount_msat"]) > BigInt::from(0) {
                if !truth(&l["wallet_id"]) && !reviewed {
                    issues.push(json!({"code":"transactionless_leg_wallet_missing","leg_id":l["id"],"role":role}));
                }
                if !truth(&l["occurred_at"]) {
                    issues.push(
                        json!({"code":"leg_occurred_at_missing","leg_id":l["id"],"role":role}),
                    );
                }
            }
            continue;
        }
        let id = pystr(tx);
        if !anchor.is_null() && pystr(anchor) != id {
            issues.push(json!({"code":"anchor_transaction_identity_mismatch","leg_id":l["id"],"transaction_id":id,"anchor_transaction_id":pystr(anchor)}));
        }
        by_transaction.entry(id.clone()).or_default().push(l);
        let Some(row) = snapshot.anchors.get(&id).filter(|r| !r.is_null()) else {
            issues.push(json!({"code":"anchor_transaction_missing","transaction_id":id}));
            continue;
        };
        rows.insert(id.clone(), row);
        issues.extend(anchor_scope_issues(l, row));
        if truth(&row["excluded"]) && excluded.insert(id.clone()) {
            issues.push(json!({"code":"anchor_transaction_excluded","message":"an imported transaction anchor is excluded from journals; supersede the component or include the evidence again","transaction_id":id}));
        }
        if role == "source" && row["direction"] != "outbound" {
            issues.push(json!({"code":"source_anchor_direction_mismatch","leg_id":l["id"],"transaction_id":id,"direction":row["direction"]}));
        }
        if owned(l) && row["direction"] != "inbound" {
            issues.push(json!({"code":"destination_anchor_direction_mismatch","leg_id":l["id"],"transaction_id":id,"direction":row["direction"]}));
        }
        if matches!(role.as_str(), "fee" | "external") && row["direction"] != "outbound" {
            issues.push(json!({"code":"loss_anchor_direction_mismatch","leg_id":l["id"],"transaction_id":id,"direction":row["direction"],"role":role}));
        }
        if asset(&l["asset"]) != asset(&row["asset"]) {
            issues.push(json!({"code":"anchor_asset_mismatch","leg_id":l["id"],"transaction_id":id,"leg_asset":l["asset"],"transaction_asset":row["asset"]}));
        }
        if !l["wallet_id"].is_null() && l["wallet_id"] != row["wallet_id"] {
            issues.push(json!({"code":"anchor_wallet_mismatch","leg_id":l["id"],"transaction_id":id,"leg_wallet_id":l["wallet_id"],"transaction_wallet_id":row["wallet_id"]}));
        }
        let lt = time::instant(&l["occurred_at"])?;
        let rt = time::instant(&row["occurred_at"])?;
        if lt.is_none() {
            issues.push(
                json!({"code":"leg_occurred_at_invalid","leg_id":l["id"],"transaction_id":id}),
            );
        } else if rt.is_none() || lt != rt {
            let mut mismatch = json!({"leg_id":l["id"],"transaction_id":id,"leg_occurred_at":l["occurred_at"],"transaction_occurred_at":row["occurred_at"]});
            if rt.is_some() && crate::evidence::current_observation(row)? {
                mismatch["code"] = json!("anchor_observer_chronology_changed");
                mismatch["severity"] = json!("warning");
                mismatch["review_required"] = json!(true);
                mismatch["message"]=json!("an authoritative observer refined this anchor's chronology without changing its event identity or quantity");
                mismatch["authority_version"] =
                    codec::integer(&row["observation_authority_version"]).map_err(Error::value)?;
                mismatch["application_revision"] = row["observation_application_revision"].clone();
                warnings.push(mismatch);
            } else {
                mismatch["code"] = json!("anchor_occurred_at_mismatch");
                issues.push(mismatch);
            }
        }
    }
    // Observed fee allocated per source transaction, and legacy pair fee
    // allocations that may consume principal as well as the observed fee.
    let mut fee_by_transaction: BTreeMap<String, BigInt> = BTreeMap::new();
    for a in allocs {
        let (Some(source), Some(sink)) = (
            by_id(legs, &a["source_leg_id"]),
            by_id(legs, &a["sink_leg_id"]),
        ) else {
            continue;
        };
        if sink["role"] != "fee" {
            continue;
        }
        let id = pystr(&source["transaction_id"]);
        *fee_by_transaction.entry(id.clone()).or_default() +=
            big(&integer(or(&a["source_amount_msat"], &json!(0)))?);
        let Some(row) = rows.get(&id) else {
            continue;
        };
        if mode != "quantity" {
            continue;
        }
        let fee = big(&integer(or(&row["fee"], &json!(0)))?);
        let principal = principal(row)?;
        let movement = if text(&row["direction"]).py_strip().py_lower() == "outbound" {
            &principal + &fee
        } else {
            big(&integer(or(&row["amount"], &json!(0)))?)
        };
        if text(&sink["notes"]) == "reviewed_residual:network_fee"
            && fee > BigInt::from(0)
            && big(&integer(or(&source["amount_msat"], &json!(0)))?) < movement
        {
            issues.push(json!({"code":"legacy_pair_fee_allocation_review_required","message":"legacy pair fee allocation may consume principal as well as the observed fee; revise the complete debit and residual","source_leg_id":source["id"],"fee_leg_id":sink["id"],"review_required":true}));
        }
    }
    let partial: BTreeSet<_> = terms
        .iter()
        .flat_map(|t| [pystr(&t["source_leg_id"]), pystr(&t["target_leg_id"])])
        .collect();
    let mut coverage = Vec::new();
    for (id, ls) in &by_transaction {
        let Some(row) = rows.get(id) else {
            continue;
        };
        let actual: BigInt = ls
            .iter()
            .filter(|l| {
                (row["direction"] == "outbound" && l["role"] == "source")
                    || (row["direction"] == "inbound" && owned(l))
            })
            .map(|l| big(&l["amount_msat"]))
            .sum();
        let principal = principal(row)?;
        let movement = if text(&row["direction"]).py_strip().py_lower() == "outbound" {
            &principal + big(&integer(or(&row["fee"], &json!(0)))?)
        } else {
            big(&integer(or(&row["amount"], &json!(0)))?)
        };
        let partial_review = ls.iter().any(|l| partial.contains(&pystr(&l["id"])));
        // A partial review covers the principal plus the observed fee it
        // actually allocates, never more than the observed fee.
        let allocated = fee_by_transaction.get(id).cloned().unwrap_or_default();
        let fee = big(&integer(or(&row["fee"], &json!(0)))?);
        let expected = if partial_review {
            principal + allocated.min(fee)
        } else {
            movement
        };
        let mut c = json!({"transaction_id":id,"direction":row["direction"],"partial_review":partial_review,"raw_economic_msat":number(&expected),"reviewed_component_msat":number(&actual),"reviewed_minus_raw_msat":number(&(&actual-&expected))});
        coverage.push(c.clone());
        if (partial_review && !(BigInt::from(0) < actual && actual <= expected))
            || (!partial_review && actual != expected)
        {
            c["code"] = json!("anchor_coverage_mismatch");
            issues.push(c);
        }
    }
    for (source_id, allocs) in group(allocs.iter().cloned(), |a| pystr(&a["source_leg_id"])) {
        let Some(s) = by_id(legs, &json!(source_id)) else {
            continue;
        };
        if !allocs
            .iter()
            .any(|a| by_id(legs, &a["sink_leg_id"]).is_some_and(|l| l["role"] == "suspense"))
        {
            continue;
        }
        let tx = &s["transaction_id"];
        let Some(row) = rows.get(&pystr(tx)).filter(|_| truth(tx)) else {
            continue;
        };
        let allocated: BigInt = allocs
            .iter()
            .filter(|a| by_id(legs, &a["sink_leg_id"]).is_some_and(|l| l["role"] == "fee"))
            .map(|a| big(&a["source_amount_msat"]))
            .sum();
        let fee = big(&integer(or(&row["fee"], &json!(0)))?);
        if allocated != fee {
            issues.push(json!({"code":"custody_suspense_fee_coverage_mismatch","message":"an observed miner fee must stay separate from custody suspense","source_leg_id":source_id,"transaction_id":pystr(tx),"observed_fee_msat":number(&fee),"allocated_fee_msat":number(&allocated)}));
        }
    }
    if mode == "quantity" {
        let scoped = scoped_legs(snapshot, legs)?;
        // Unlike the Python helper's extensible issue list, every preceding
        // issue here is anchor/coverage-specific. None has allocation_network_
        // identity, so its existing_scope_keys dedup is currently unreachable.
        issues.extend(allocation_scope_issues(&scoped, allocs, mode));
        let mut additional = chronology(&scoped, allocs, mode)?;
        additional.extend(connectivity(&scoped, allocs, mode));
        let ids: BTreeSet<_> = legs
            .iter()
            .filter(|l| !absent(&l["component_id"]))
            .map(|l| pystr(&l["component_id"]))
            .collect();
        additional.extend(
            routes
                .iter()
                .filter(|r| {
                    array(&r["component_ids"])
                        .iter()
                        .any(|id| ids.contains(&pystr(id)))
                })
                .cloned(),
        );
        let mut seen: BTreeSet<_> = issues.iter().map(Value::to_string).collect();
        for issue in additional {
            if seen.insert(issue.to_string()) {
                issues.push(issue);
            }
        }
    }
    if by_transaction.is_empty() {
        issues.push(json!({"code":"transaction_anchor_missing","message":"an effective component requires at least one imported transaction anchor"}));
    }
    // The lists move into the result; `json!` would copy them.
    let mut result = serde_json::Map::new();
    result.insert("valid".into(), Value::Bool(issues.is_empty()));
    result.insert("issues".into(), Value::Array(issues));
    result.insert("warnings".into(), Value::Array(warnings));
    result.insert("transaction_coverage".into(), Value::Array(coverage));
    Ok(Value::Object(result))
}
pub(super) fn principal(row: &Value) -> Result<BigInt, Error> {
    let amount = big(&integer(or(&row["amount"], &json!(0)))?);
    let fee = big(&integer(or(&row["fee"], &json!(0)))?);
    if text(&row["direction"]).py_strip().py_lower() == "outbound"
        && truth(&row["amount_includes_fee"])
    {
        Ok(amount - fee)
    } else {
        Ok(amount)
    }
}
