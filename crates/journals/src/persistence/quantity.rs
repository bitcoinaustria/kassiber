//! Quantity artifacts are storage projections of the completed custody stage.
use super::*;
use std::collections::BTreeMap;

fn nullable(v: &Value) -> Result<Sql, Refusal> {
    if truth(v) {
        bind(v)
    } else {
        Ok(Sql::Null)
    }
}
fn hash(v: &Value) -> Result<Sql, Refusal> {
    Ok(t(kassiber_custody::codec::sha256(&pyjson(v, true)?)))
}
fn network(o: &Value) -> Value {
    match o["event_key"]["network"].as_str() {
        Some("liquidv1") => json!("main"),
        Some("liquidtestnet") => json!("test"),
        Some("elementsregtest") => json!("regtest"),
        _ => o["event_key"]["network"].clone(),
    }
}
fn exposure(o: &Value) -> String {
    if matches!(o["asset"].as_str(), Some("BTC" | "LBTC")) {
        "bitcoin".into()
    } else {
        format!("asset:{}", string(&o["asset"]))
    }
}
fn slice_amount(slice: &Value) -> Result<i128, Refusal> {
    let start = slice["start_msat"]
        .as_i64()
        .ok_or_else(|| Refusal::new("OverflowError", "slice start does not fit SQLite INTEGER"))?;
    let end = slice["end_msat"]
        .as_i64()
        .ok_or_else(|| Refusal::new("OverflowError", "slice end does not fit SQLite INTEGER"))?;
    Ok(i128::from(end) - i128::from(start))
}
fn barrier(q: &Value, o: &Value) -> Value {
    array(&q["tax_eligibility"]["pool_barriers"])
        .iter()
        .find(|p| {
            p[0]["profile_id"] == o["profile_id"]
                && p[0]["network"] == network(o)
                && p[0]["exposure"] == exposure(o)
                && p[0]["unit"] == "msat"
        })
        .map(|p| p[1][0].clone())
        .unwrap_or(Value::Null)
}

pub(super) fn store<O: storage::WriteObserver>(
    w: &mut Writer<'_, O>,
    wid: &str,
    pid: &str,
    q: &Value,
    at: &str,
) -> Result<HashMap<&'static str, usize>, Refusal> {
    let mut counts = HashMap::from([
        ("postings", 0),
        ("issues", 0),
        ("balances", 0),
        ("decisions", 0),
    ]);
    if q.is_null() {
        return Ok(counts);
    }
    for table in [
        "journal_quantity_issues",
        "journal_quantity_balances",
        "journal_custody_decisions",
        "journal_custody_economic_relations",
    ] {
        w.clear(table, pid)?;
    }
    let observations: HashMap<_, _> = array(&q["projection"]["observations"])
        .iter()
        .map(|o| (string(&o["quantity_hash"]), o))
        .collect();
    let mut postings = Vec::new();
    let mut balances = BTreeMap::<(String, String, String), i128>::new();
    for p in array(&q["projection"]["postings"]) {
        let o = observations
            .get(&string(&p["observation_hash"]))
            .copied()
            .unwrap_or(&Value::Null);
        postings.push([
            bind(&p["posting_id"])?,
            t(wid),
            t(pid),
            bind(&o["anchor_transaction_id"])?,
            bind(&p["observation_hash"])?,
            bind(&o["occurred_at"])?,
            bind(&p["asset"])?,
            bind(&p["location_kind"])?,
            bind(&p["location_id"])?,
            bind(&p["amount_msat"])?,
            bind(&p["state"])?,
            t(at),
        ]);
        let key = (
            string(&p["location_kind"]),
            string(&p["location_id"]),
            string(&p["asset"]),
        );
        let amount = p["amount_msat"].as_i64().ok_or_else(|| {
            Refusal::new(
                "OverflowError",
                "quantity posting does not fit SQLite INTEGER",
            )
        })?;
        *balances.entry(key).or_default() += i128::from(amount);
    }
    storage::replace_quantity_postings(w.conn, pid, &postings, w.observer)?;
    counts.insert("postings", postings.len());
    let mut rows = Vec::new();
    for issue in array(&q["issues"]) {
        rows.push(vec![
            bind(&issue["issue_id"])?,
            t(wid),
            t(pid),
            bind(&issue["issue_type"])?,
            bind(&issue["state"])?,
            bind(&issue["asset"])?,
            bind(&issue["amount_msat"])?,
            nullable(&issue["occurred_at"])?,
            t(pyjson(&issue["transaction_ids"], false)?),
            bind(&issue["reason"])?,
            t(pyjson(&issue["details"], false)?),
            bind(&q["tax_eligibility"]["blocked_from"])?,
            t(at),
        ]);
    }
    counts.insert("issues", rows.len());
    w.insert(
        "journal_quantity_issues",
        &[
            "issue_id",
            "workspace_id",
            "profile_id",
            "issue_type",
            "state",
            "asset",
            "amount_msat",
            "occurred_at",
            "transaction_ids_json",
            "reason",
            "detail_json",
            "blocks_from",
            "created_at",
        ],
        rows,
    )?;
    let rows = balances
        .into_iter()
        .filter(|(_, n)| *n != 0)
        .map(|((kind, id, asset), n)| {
            let n = i64::try_from(n).map_err(|_| {
                Refusal::new(
                    "OverflowError",
                    "quantity balance does not fit SQLite INTEGER",
                )
            })?;
            Ok(vec![
                t(wid),
                t(pid),
                t(kind),
                t(id),
                t(asset),
                Sql::Integer(n),
                t(at),
            ])
        })
        .collect::<Result<Vec<_>, Refusal>>()?;
    counts.insert("balances", rows.len());
    w.insert(
        "journal_quantity_balances",
        &[
            "workspace_id",
            "profile_id",
            "location_kind",
            "location_id",
            "asset",
            "amount_msat",
            "created_at",
        ],
        rows,
    )?;
    let eligible: HashSet<&Value> = array(&q["tax_eligibility"]["eligible_decisions"])
        .iter()
        .collect();
    let mut eligible_source_slices = HashSet::new();
    for d in &eligible {
        let source = observations
            .get(&string(&d["source"]["observation_hash"]))
            .ok_or_else(|| Refusal::new("KeyError", "eligible source observation"))?;
        eligible_source_slices.insert((
            string(&source["anchor_transaction_id"]),
            slice_amount(&d["source"])?,
            d["component_id"].clone(),
        ));
    }
    let mut rows = Vec::new();
    for d in array(&q["projection"]["decisions"]) {
        if !matches!(
            d["state"].as_str(),
            Some("internal_verified" | "internal_reviewed")
        ) || d["target"].is_null()
        {
            continue;
        }
        let source = observations
            .get(&string(&d["source"]["observation_hash"]))
            .ok_or_else(|| Refusal::new("KeyError", "decision source observation"))?;
        let target = observations
            .get(&string(&d["target"]["observation_hash"]))
            .ok_or_else(|| Refusal::new("KeyError", "decision target observation"))?;
        let id = hash(&json!([
            "canonical-custody-decision-v1",
            d["source"]["observation_hash"],
            d["source"]["start_msat"],
            d["source"]["end_msat"],
            d["target"]["observation_hash"],
            d["target"]["start_msat"],
            d["target"]["end_msat"],
            d["state"],
            d["reason"],
            d["atomic_bundle_id"],
            d["component_id"]
        ]))?;
        rows.push(vec![
            id,
            t(wid),
            t(pid),
            bind(&source["anchor_transaction_id"])?,
            bind(&target["anchor_transaction_id"])?,
            bind(&d["source"]["observation_hash"])?,
            bind(&d["source"]["start_msat"])?,
            bind(&d["source"]["end_msat"])?,
            bind(&d["target"]["observation_hash"])?,
            bind(&d["target"]["start_msat"])?,
            bind(&d["target"]["end_msat"])?,
            bind(&source["wallet_id"])?,
            bind(&target["wallet_id"])?,
            bind(&network(source))?,
            bind(&network(target))?,
            bind(&source["event_key"]["chain"])?,
            bind(&target["event_key"]["chain"])?,
            bind(&source["asset"])?,
            bind(&target["asset"])?,
            bind(&d["state"])?,
            t(if eligible.contains(d) {
                "eligible"
            } else {
                "blocked_by_prior_custody_basis"
            }),
            bind(&barrier(q, source))?,
            bind(&d["reason"])?,
            bind(&d["atomic_bundle_id"])?,
            bind(&d["component_id"])?,
            nullable(&source["occurred_at"])?,
            nullable(&target["occurred_at"])?,
            t(at),
        ]);
    }
    counts.insert("decisions", rows.len());
    w.insert(
        "journal_custody_decisions",
        &[
            "decision_id",
            "workspace_id",
            "profile_id",
            "source_transaction_id",
            "target_transaction_id",
            "source_observation_hash",
            "source_start_msat",
            "source_end_msat",
            "target_observation_hash",
            "target_start_msat",
            "target_end_msat",
            "source_wallet_id",
            "target_wallet_id",
            "source_network",
            "target_network",
            "source_rail",
            "target_rail",
            "source_asset",
            "target_asset",
            "state",
            "basis_state",
            "basis_barrier_at",
            "reason",
            "atomic_group_id",
            "component_id",
            "occurred_at",
            "target_occurred_at",
            "created_at",
        ],
        rows,
    )?;
    let by_tx: HashMap<_, _> = array(&q["projection"]["observations"])
        .iter()
        .filter(|o| truth(&o["anchor_transaction_id"]))
        .map(|o| (string(&o["anchor_transaction_id"]), o))
        .collect();
    let mut rows = Vec::new();
    for (kind, relations) in [
        ("conversion", &q["reviewed_conversion_pairs"]),
        ("direct_payout", &q["reviewed_direct_payouts"]),
    ] {
        for r in array(relations) {
            let source_id = crate::fallback(&r["out_id"], &r["out_transaction_id"]);
            let target_id = &r["in_id"];
            let Some(source) = by_tx.get(&string(source_id)) else {
                continue;
            };
            let target = by_tx
                .get(&string(target_id))
                .copied()
                .unwrap_or(&Value::Null);
            let absent = |v: &Value| v.is_null() || v == "";
            let mut source_amount = &r["out_amount"];
            if absent(source_amount) {
                source_amount = &r["out_amount_msat"];
            }
            if absent(source_amount) {
                source_amount = &source["principal_msat"];
            }
            let mut target_amount = &r["in_amount"];
            if absent(target_amount) {
                target_amount = &r["payout_amount"];
            }
            if absent(target_amount) && !target.is_null() {
                target_amount = &target["principal_msat"];
            }
            let source_amount = kassiber_custody::codec::integer(source_amount)?;
            let target_amount = kassiber_custody::codec::integer_or_zero(target_amount)?;
            if source_amount.to_string().starts_with('-')
                || source_amount == 0
                || target_amount.to_string().starts_with('-')
                || target_amount == 0
            {
                continue;
            }
            let target_id = if absent(target_id) {
                Value::Null
            } else {
                json!(string(target_id))
            };
            let asset = if !target.is_null() {
                &target["asset"]
            } else {
                &r["payout_asset"]
            };
            let eligible = source_amount.as_i64().is_some_and(|amount| {
                eligible_source_slices.contains(&(
                    string(source_id),
                    i128::from(amount),
                    r["component_id"].clone(),
                ))
            });
            rows.push(vec![
                hash(&json!([
                    "canonical-custody-economic-relation-v3",
                    kind,
                    string(source_id),
                    target_id,
                    r["component_id"],
                    source["asset"],
                    asset,
                    source_amount,
                    target_amount
                ]))?,
                t(wid),
                t(pid),
                t(kind),
                t(string(source_id)),
                bind(&target_id)?,
                bind(&r["component_id"])?,
                bind(&source["asset"])?,
                bind(asset)?,
                bind(&source_amount)?,
                bind(&target_amount)?,
                t(if eligible {
                    "eligible"
                } else {
                    "blocked_by_prior_custody_basis"
                }),
                nullable(&source["occurred_at"])?,
                bind(if !target.is_null() {
                    &target["occurred_at"]
                } else {
                    &r["payout_occurred_at"]
                })?,
                t(at),
            ]);
        }
    }
    counts.insert("economic_relations", rows.len());
    w.insert(
        "journal_custody_economic_relations",
        &[
            "relation_id",
            "workspace_id",
            "profile_id",
            "relation_kind",
            "source_transaction_id",
            "target_transaction_id",
            "component_id",
            "source_asset",
            "target_asset",
            "source_amount_msat",
            "target_amount_msat",
            "basis_state",
            "occurred_at",
            "target_occurred_at",
            "created_at",
        ],
        rows,
    )?;
    Ok(counts)
}

pub(super) fn reconciliations<O: storage::WriteObserver>(
    w: &mut Writer<'_, O>,
    profile: &Value,
    state: &JournalState<'_>,
    at: &str,
) -> Result<(), Refusal> {
    let pid = string(&profile["id"]);
    w.clear("journal_custody_reconciliations", &pid)?;
    if state.custody_quantity.is_null() {
        return Ok(());
    }
    let mut selected = HashMap::<String, i128>::new();
    for d in array(&state.custody_quantity["projection"]["decisions"]) {
        if d["state"] == "internal_verified"
            && !d["target"].is_null()
            && truth(&d["selected_claim_id"])
        {
            *selected.entry(string(&d["selected_claim_id"])).or_default() +=
                slice_amount(&d["source"])?;
        }
    }
    let mut rows = Vec::new();
    for proof in array(&state.custody_native_reconciliations) {
        let expected = array(&proof["native_claim_amounts"]);
        if expected.is_empty()
            || expected.iter().any(|pair| {
                selected.get(&string(&pair[0])).copied().unwrap_or(0)
                    != i128::from(pair[1].as_i64().unwrap_or(-1))
            })
        {
            continue;
        }
        let mut select = w
            .conn
            .prepare("SELECT revision,state FROM custody_components WHERE profile_id=? AND id=?")?;
        let mut found = select.query(params![pid, string(&proof["component_id"])])?;
        let Some(component) = found.next()? else {
            continue;
        };
        if component.get::<_, String>(1)? != "active"
            || component.get::<_, i64>(0)? != proof["component_revision"].as_i64().unwrap_or(-1)
        {
            continue;
        }
        rows.push(vec![
            t(&pid),
            bind(&proof["component_id"])?,
            bind(&profile["journal_input_version"])?,
            bind(&proof["component_revision"])?,
            t(pyjson(proof, true)?),
            t(at),
        ]);
    }
    w.insert(
        "journal_custody_reconciliations",
        &[
            "profile_id",
            "component_id",
            "input_version",
            "component_revision",
            "proof_json",
            "created_at",
        ],
        rows,
    )
}
