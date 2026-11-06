//! Quantity artifacts are storage projections of the completed custody stage.
use super::*;
use kassiber_store::rusqlite::types::ValueRef;
use std::borrow::Cow;
use std::collections::BTreeMap;

pub(super) fn key(v: &Value) -> Cow<'_, str> {
    v.as_str()
        .map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Owned(string(v)))
}

#[derive(Clone, Copy)]
struct Observation<'a> {
    row: ObservationRef<'a>,
    transaction: &'a Value,
    occurred: &'a Value,
}
impl<'a> Observation<'a> {
    fn new(row: ObservationRef<'a>) -> Self {
        Self {
            row,
            transaction: row.at("anchor_transaction_id"),
            occurred: row.at("occurred_at"),
        }
    }
}
impl std::ops::Index<&str> for Observation<'_> {
    type Output = Value;
    fn index(&self, key: &str) -> &Value {
        self.row.at(key)
    }
}

fn posting_fields(posting: PostingRef<'_>) -> [&Value; 7] {
    if let PostingRef::Json(posting) = posting {
        let mut fields = [&Value::Null; 7];
        if let Some(object) = posting.as_object() {
            for (key, value) in object {
                let index = match key.as_str() {
                    "posting_id" => 0,
                    "observation_hash" => 1,
                    "asset" => 2,
                    "location_kind" => 3,
                    "location_id" => 4,
                    "amount_msat" => 5,
                    "state" => 6,
                    _ => continue,
                };
                fields[index] = value;
            }
        }
        return fields;
    }
    [
        "posting_id",
        "observation_hash",
        "asset",
        "location_kind",
        "location_id",
        "amount_msat",
        "state",
    ]
    .map(|key| posting.at(key))
}

fn relation_source_id(relation: &Value) -> String {
    ["out_id", "out_transaction_id"]
        .iter()
        .map(|&field| &relation[field])
        .find(|value| truth(value))
        .map(string)
        .unwrap_or_default()
}

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
fn network<O: for<'k> std::ops::Index<&'k str, Output = Value> + ?Sized>(o: &O) -> Value {
    match o["event_key"]["network"].as_str() {
        Some("liquidv1") => json!("main"),
        Some("liquidtestnet") => json!("test"),
        Some("elementsregtest") => json!("regtest"),
        _ => o["event_key"]["network"].clone(),
    }
}
fn exposure<O: for<'k> std::ops::Index<&'k str, Output = Value> + ?Sized>(o: &O) -> String {
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
fn barrier<O: for<'k> std::ops::Index<&'k str, Output = Value> + ?Sized>(
    q: &Value,
    o: &O,
) -> Value {
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
    observations: &[ObservationRef<'_>],
    postings: &[PostingRef<'_>],
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
    w.observer.phase("quantity_postings_prepare");
    let observation_list = observations;
    let observations: HashMap<_, _> = observation_list
        .iter()
        .map(|o| (key(o.at("quantity_hash")), Observation::new(*o)))
        .collect();
    w.observer.phase("quantity_postings_bind");
    let input_postings = postings;
    let mut postings = Vec::with_capacity(input_postings.len());
    let mut balances = BTreeMap::<_, i128>::new();
    for p in input_postings {
        let [id, observation, asset, kind, location, amount, state] = posting_fields(*p);
        let o = observations
            .get(&key(observation))
            .copied()
            .unwrap_or_else(|| Observation::new(ObservationRef::Json(&Value::Null)));
        postings.push([
            borrowed(id)?,
            ValueRef::Text(wid.as_bytes()),
            ValueRef::Text(pid.as_bytes()),
            borrowed(o.transaction)?,
            borrowed(observation)?,
            borrowed(o.occurred)?,
            borrowed(asset)?,
            borrowed(kind)?,
            borrowed(location)?,
            borrowed(amount)?,
            borrowed(state)?,
            ValueRef::Text(at.as_bytes()),
        ]);
        let key = (key(kind), key(location), key(asset));
        let amount = amount.as_i64().ok_or_else(|| {
            Refusal::new(
                "OverflowError",
                "quantity posting does not fit SQLite INTEGER",
            )
        })?;
        *balances.entry(key).or_default() += i128::from(amount);
    }
    w.observer.phase("quantity_postings_write");
    storage::replace_borrowed_postings(w.conn, pid, &postings, w.observer)?;
    w.observer.phase("quantity_other");
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
        .filter(|d| {
            matches!(
                d["state"].as_str(),
                Some("internal_verified" | "internal_reviewed")
            ) && !d["target"].is_null()
        })
        .collect();
    let mut eligible_source_slices = HashSet::new();
    let has_relations = !array(&q["reviewed_conversion_pairs"]).is_empty()
        || !array(&q["reviewed_direct_payouts"]).is_empty();
    for d in array(&q["tax_eligibility"]["eligible_decisions"]) {
        let source = observations
            .get(&key(&d["source"]["observation_hash"]))
            .ok_or_else(|| Refusal::new("KeyError", "eligible source observation"))?;
        let amount = slice_amount(&d["source"])?;
        if has_relations {
            eligible_source_slices.insert((
                string(&source["anchor_transaction_id"]),
                amount,
                d["component_id"].clone(),
            ));
        }
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
            .get(&key(&d["source"]["observation_hash"]))
            .ok_or_else(|| Refusal::new("KeyError", "decision source observation"))?;
        let target = observations
            .get(&key(&d["target"]["observation_hash"]))
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
    let by_tx: HashMap<_, _> = observation_list
        .iter()
        .copied()
        .filter(|o| {
            (!array(&q["reviewed_conversion_pairs"]).is_empty()
                || !array(&q["reviewed_direct_payouts"]).is_empty())
                && truth(&o["anchor_transaction_id"])
        })
        .map(|o| (string(&o["anchor_transaction_id"]), o))
        .collect();
    let mut rows = Vec::new();
    for (kind, relations) in [
        ("conversion", &q["reviewed_conversion_pairs"]),
        ("direct_payout", &q["reviewed_direct_payouts"]),
    ] {
        for r in array(relations) {
            let source_id = relation_source_id(r);
            let target_id = &r["in_id"];
            let Some(source) = by_tx.get(&source_id) else {
                continue;
            };
            let target = by_tx
                .get(&string(target_id))
                .copied()
                .unwrap_or(ObservationRef::Json(&Value::Null));
            let absent = |v: &Value| v.is_null() || v == "";
            let mut source_amount = &r["out_amount"];
            if absent(source_amount) {
                source_amount = &r["out_amount_msat"];
            }
            let source_principal;
            if absent(source_amount) {
                source_principal = kassiber_custody::quantity::principal_msat(source)?;
                source_amount = &source_principal;
            }
            let mut target_amount = &r["in_amount"];
            if absent(target_amount) {
                target_amount = &r["payout_amount"];
            }
            let target_principal;
            if absent(target_amount) && !target.is_null() {
                target_principal = kassiber_custody::quantity::principal_msat(&target)?;
                target_amount = &target_principal;
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
                    source_id.clone(),
                    i128::from(amount),
                    r["component_id"].clone(),
                ))
            });
            rows.push(vec![
                hash(&json!([
                    "canonical-custody-economic-relation-v3",
                    kind,
                    source_id,
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
                t(&source_id),
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quantity_scalar_helpers_preserve_fallbacks_and_refuse_invalid_slices() {
        assert!(matches!(key(&json!("id")), Cow::Borrowed("id")));
        for (value, expected) in [
            (Value::Null, "None"),
            (json!(1), "1"),
            (json!(false), "False"),
        ] {
            assert_eq!(key(&value), expected);
        }
        for value in [
            Value::Null,
            json!(false),
            json!(0),
            json!(0.0),
            json!(""),
            json!([]),
            json!({}),
        ] {
            assert_eq!(nullable(&value).unwrap(), Sql::Null);
        }
        assert_eq!(nullable(&json!("id")).unwrap(), t("id"));
        assert_eq!(nullable(&json!(true)).unwrap(), Sql::Integer(1));
        assert_eq!(
            nullable(&json!([1])).unwrap_err().family,
            "ProgrammingError"
        );
        assert_eq!(
            hash(&json!({"b":2,"a":1})).unwrap(),
            t("43258cff783fe7036d8a43033f830adfc60ec037382473548ac742b888292777")
        );
        for (input, expected) in [
            ("liquidv1", "main"),
            ("liquidtestnet", "test"),
            ("elementsregtest", "regtest"),
            ("signet", "signet"),
        ] {
            assert_eq!(
                network(&json!({"event_key":{"network":input}})),
                json!(expected)
            );
        }
        assert_eq!(network(&json!({})), Value::Null);
        assert_eq!(network(&json!({"event_key":{"network":7}})), json!(7));
        for (asset, expected) in [
            ("BTC", "bitcoin"),
            ("LBTC", "bitcoin"),
            ("EUR", "asset:EUR"),
        ] {
            assert_eq!(exposure(&json!({"asset":asset})), expected);
        }
        assert_eq!(exposure(&json!({})), "asset:None");
        for (start, end) in [(0, 1), (7, -3), (i64::MIN, i64::MAX)] {
            assert_eq!(
                slice_amount(&json!({"start_msat":start,"end_msat":end})).unwrap(),
                i128::from(end) - i128::from(start)
            );
        }
        for field in ["start_msat", "end_msat"] {
            for invalid in [
                Value::Null,
                json!(true),
                json!(1.5),
                json!("1"),
                json!(u64::MAX),
            ] {
                let mut slice = json!({"start_msat":0,"end_msat":1});
                slice[field] = invalid;
                let error = slice_amount(&slice).unwrap_err();
                assert_eq!(error.family, "OverflowError");
                assert_eq!(
                    error.message,
                    format!(
                        "slice {} does not fit SQLite INTEGER",
                        if field == "start_msat" {
                            "start"
                        } else {
                            "end"
                        }
                    )
                );
            }
        }
    }

    #[test]
    fn pool_barriers_require_every_scope_field_and_keep_first_reason() {
        let observation =
            json!({"profile_id":"p","event_key":{"network":"liquidv1"},"asset":"LBTC"});
        let scope = json!({"profile_id":"p","network":"main","exposure":"bitcoin","unit":"msat"});
        let matching = json!([scope, ["first", "second"]]);
        assert_eq!(barrier(&json!({}), &observation), Value::Null);
        assert_eq!(
            barrier(
                &json!({"tax_eligibility":{"pool_barriers":[matching]}}),
                &observation
            ),
            json!("first")
        );
        for field in ["profile_id", "network", "exposure", "unit"] {
            let mut wrong = scope.clone();
            wrong[field] = json!("wrong");
            let q = json!({"tax_eligibility":{"pool_barriers":[[wrong,["unrelated"]], matching]}});
            assert_eq!(barrier(&q, &observation), json!("first"), "{field}");
        }
        assert_eq!(
            barrier(
                &json!({"tax_eligibility":{"pool_barriers":[[scope,[]],matching]}}),
                &observation
            ),
            Value::Null
        );
    }

    #[test]
    fn economic_relation_source_ids_use_first_truthy_id_or_empty_string() {
        for (relation, expected) in [
            (json!({}), ""),
            (json!({"out_id":null,"out_transaction_id":null}), ""),
            (json!({"out_id":false,"out_transaction_id":0}), ""),
            (
                json!({"out_id":"first","out_transaction_id":"second"}),
                "first",
            ),
            (json!({"out_id":"","out_transaction_id":"second"}), "second"),
            (json!({"out_id":null,"out_transaction_id":"None"}), "None"),
            (json!({"out_id":1,"out_transaction_id":"second"}), "1"),
        ] {
            assert_eq!(relation_source_id(&relation), expected);
        }
    }
}
