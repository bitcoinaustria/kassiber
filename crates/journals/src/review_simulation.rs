use crate::review_workflow::Error;
use crate::JournalState;
use kassiber_custody::codec;
use kassiber_envelope::{dict, get, s, set, AppError};
use kassiber_pyjson::{PyStr, Value};
use kassiber_store::rusqlite::Connection;
use kassiber_transactions::metadata::{self, Command, GroupedUpdate, PricingUpdate};
use kassiber_transfers::review_support::sql;
use kassiber_transfers::review_support::{text, value};
use kassiber_transfers::{review_cases, review_support, Scope, Sources};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(crate) struct Cases {
    quarantined: BTreeSet<String>,
    suspense_components: BTreeSet<String>,
}

impl Cases {
    pub(crate) fn from_state(state: &JournalState<'_>) -> Self {
        Self {
            quarantined: state
                .quarantines
                .iter()
                .map(|row| codec::pystr(&row.transaction_id))
                .collect(),
            suspense_components: state.custody_quantity["projection"]["decisions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|row| {
                    row["state"] == "custody_suspense"
                        && row["reason"] == "reviewed_residual_suspense"
                })
                .filter_map(|row| {
                    row["component_id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned)
                })
                .collect(),
        }
    }

    fn holds_suspense(&self, pair: &Value) -> bool {
        ["out_transaction_id", "in_transaction_id"]
            .iter()
            .any(|key| self.quarantined.contains(&text(pair, key)))
            && self
                .suspense_components
                .contains(&text(pair, "component_id"))
    }
}

struct Baseline {
    pairs: BTreeMap<PyStr, Value>,
    fingerprints: BTreeMap<PyStr, String>,
}

fn exact_text(row: &Value, key: &str) -> PyStr {
    match get(row, key) {
        Some(Value::Str(raw)) => raw.clone(),
        _ => PyStr::new(),
    }
}

impl Baseline {
    fn read(
        conn: &Connection,
        scope: &Scope<'_>,
        unpaired: &BTreeSet<PyStr>,
    ) -> Result<Self, Error> {
        let pairs: BTreeMap<_, _> = review_cases::current_pairs(conn, scope)?
            .into_iter()
            .map(|pair| (exact_text(&pair, "id"), pair))
            .collect();
        let fingerprints = unpaired
            .iter()
            .filter_map(|id| pairs.get(id).map(|pair| (id, pair)))
            .map(|(id, pair)| Ok((id.clone(), review_cases::pair_fingerprint(conn, pair)?)))
            .collect::<Result<_, AppError>>()?;
        Ok(Self {
            pairs,
            fingerprints,
        })
    }

    fn confirmed(&self, operation: &Value) -> Result<&Value, Error> {
        let id = exact_text(operation, "pair_id");
        let pair = self
            .pairs
            .get(&id)
            .ok_or_else(|| AppError::new("not_found", "Review pair was not found"))?;
        if self.fingerprints.get(&id).map(|print| s(print.as_str()))
            != get(operation, "expected_fingerprint").cloned()
        {
            return Err(AppError::new(
                "review_case_changed",
                "Review pair changed since it was confirmed; inspect it again",
            )
            .into());
        }
        Ok(pair)
    }
}

fn metadata_operation(
    conn: &Connection,
    scope: &Scope<'_>,
    operation: &Value,
    cases: &Cases,
    source: &str,
) -> Result<Value, Error> {
    let raw_id = value(operation, "transaction_id");
    let bound_id = review_support::reference_bind(&raw_id, 1)?;
    let tx = sql::rows(
        conn,
        "SELECT * FROM transactions WHERE id=? AND profile_id=?",
        kassiber_store::rusqlite::params![bound_id, scope.profile_id],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::new("not_found", "Review transaction was not found"))?;
    let id = text(&tx, "id");
    let kind = text(operation, "type");
    let mut update = GroupedUpdate {
        transaction: id.clone(),
        reason: match get(operation, "reason") {
            Some(Value::Str(reason)) => Some(reason.clone()),
            _ => None,
        },
        source: if source == "user" { "gui" } else { source }.into(),
        ..Default::default()
    };
    if kind == "kind_override" {
        if value(&tx, "direction") != s("inbound") {
            return Err(AppError::new(
                "validation",
                "Acquisition review requires an inbound transaction",
            )
            .into());
        }
        if !sql::rows(conn, "SELECT 1 FROM custody_component_legs leg JOIN custody_components component ON component.id=leg.component_id WHERE component.profile_id=? AND component.state='active' AND COALESCE(leg.anchor_transaction_id,leg.transaction_id)=? LIMIT 1", [scope.profile_id, &id])?.is_empty() {
            return Err(AppError::new("acquisition_custody_review_required", "This transaction belongs to an active custody review")
                .with_hint("Reopen or supersede that custody interpretation before classifying the acquisition.").into());
        }
        update.kind = Some(value(operation, "kind"));
    } else {
        if !cases.quarantined.contains(&id) {
            return Err(AppError::new(
                "review_case_changed",
                "Review transaction is not quarantined",
            )
            .into());
        }
        match kind.as_str() {
            "exclude" => update.excluded = Some(Value::Bool(true)),
            "price_override" => {
                let pricing = PricingUpdate {
                    fiat_rate: value(operation, "fiat_rate"),
                    fiat_value: value(operation, "fiat_value"),
                    source_kind: s("manual_override"),
                    quality: s("exact"),
                    method: s("quarantine_price_override"),
                    ..Default::default()
                };
                if value(&tx, "amount") != Value::Int(0.into())
                    && get(operation, "fiat_value").is_some_and(|v| {
                        kassiber_pydecimal::finite_price(&kassiber_envelope::py_str(v))
                            .is_ok_and(|n| n.is_zero())
                    })
                {
                    return Err(AppError::new(
                        "validation",
                        "Fiat rate must be positive and finite",
                    )
                    .into());
                }
                update.pricing = Some(pricing);
            }
            _ => unreachable!("normalized metadata operation"),
        }
    }
    let updated = metadata::execute(
        conn,
        &kassiber_transactions::Scope {
            workspace_id: scope.workspace_id,
            profile_id: scope.profile_id,
        },
        &Command::Update(Box::new(update)),
    )?;
    Ok(dict([
        ("transaction_id", s(id)),
        ("history_event_id", value(&updated, "history_event_id")),
        ("updated", value(&updated, "updated")),
    ]))
}

pub(crate) fn apply(
    conn: &Connection,
    scope: &Scope<'_>,
    operations: &[Value],
    sources: &dyn Sources,
    source: &str,
    mut state: Cases,
    mut rebuild: impl FnMut(&Connection) -> Result<Cases, Error>,
) -> Result<Value, Error> {
    let case_ids = Cases {
        quarantined: state.quarantined.clone(),
        ..Default::default()
    };
    let unpaired = operations
        .iter()
        .filter(|op| text(op, "type") == "unpair")
        .map(|op| exact_text(op, "pair_id"))
        .collect();
    let mut baseline = Baseline::read(conn, scope, &unpaired)?;
    for operation in operations {
        if text(operation, "type") == "unpair"
            && baseline
                .pairs
                .contains_key(&exact_text(operation, "pair_id"))
        {
            baseline.confirmed(operation)?;
        }
    }
    let mut changed = false;
    let mut results = Vec::new();
    for operation in operations {
        let kind = text(operation, "type");
        let result = match kind.as_str() {
            "custody_component" => {
                let mut request = value(operation, "request");
                let version = sql::rows(
                    conn,
                    "SELECT journal_input_version FROM profiles WHERE id=?",
                    [scope.profile_id],
                )?
                .remove(0);
                set(
                    &mut request,
                    "expected_input_version",
                    review_support::input_version(&version).into(),
                );
                set(&mut request, "authored_source", s(source));
                kassiber_transfers::execute_with_sources(
                    conn,
                    scope,
                    "transfers.components.apply",
                    &request,
                    sources,
                )?
            }
            "unpair" => {
                let id = exact_text(operation, "pair_id");
                if !review_cases::current_pairs(conn, scope)?
                    .iter()
                    .any(|pair| exact_text(pair, "id") == id)
                {
                    return Err(AppError::new("not_found", "Review pair was not found").into());
                }
                if changed {
                    state = rebuild(conn)?;
                    baseline = Baseline::read(conn, scope, &unpaired)?;
                    changed = false;
                }
                let pair = baseline.confirmed(operation)?;
                if !state.holds_suspense(pair) {
                    return Err(AppError::new(
                        "review_case_changed",
                        "Review pair no longer holds a suspense case; inspect it again",
                    )
                    .into());
                }
                kassiber_transfers::execute_with_sources(
                    conn,
                    scope,
                    "transfers.unpair",
                    &dict([
                        ("pair_id", Value::Str(id.clone())),
                        ("authored_source", s(source)),
                    ]),
                    sources,
                )?;
                dict([
                    ("pair_id", s(id)),
                    (
                        "transaction_ids",
                        Value::List(vec![
                            s(text(pair, "out_transaction_id")),
                            s(text(pair, "in_transaction_id")),
                        ]),
                    ),
                ])
            }
            _ => metadata_operation(conn, scope, operation, &case_ids, source)?,
        };
        results.push(dict([("type", s(kind.as_str())), ("result", result)]));
        if kind != "unpair" {
            changed = true;
        }
    }
    Ok(Value::List(results))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervening_metadata_requires_refreshed_canonical_suspense_decisions() {
        let fixture =
            kassiber_pytext::loads(include_str!("../../transfers/tests/review_cases.json"))
                .unwrap();
        let plans = kassiber_pytext::loads(include_str!("../../transfers/tests/review_plans.json"))
            .unwrap();
        let list = |value| match value {
            Value::List(rows) => rows,
            _ => panic!("fixture list"),
        };
        let scenario = list(value(&fixture, "scenarios"))
            .into_iter()
            .find(|row| text(row, "name") == "pair")
            .unwrap();
        let plan = list(plans)
            .into_iter()
            .find(|row| text(row, "name") == "refresh-after-metadata")
            .unwrap();
        let scope = Scope {
            workspace_id: "w",
            profile_id: "p",
        };
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=OFF; PRAGMA temp_store=MEMORY")
            .unwrap();
        for statements in [
            value(&fixture, "schema"),
            value(&scenario, "sql"),
            value(&fixture, "objects"),
        ] {
            for statement in list(statements) {
                conn.execute_batch(&kassiber_envelope::py_str(&statement))
                    .unwrap();
            }
        }
        conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        let _transaction = conn.unchecked_transaction().unwrap();
        let profile = sql::rows(&conn, "SELECT * FROM profiles WHERE id='p'", [])
            .unwrap()
            .remove(0);
        let prepared =
            crate::pipeline::Prepared::load(&conn, profile, "2026-10-31T12:00:00Z").unwrap();
        let state = prepared.review_state(&conn, || "derived".into()).unwrap();
        let cases = Cases::from_state(&state);
        let operations = list(value(&plan, "operations"));
        let mut builds = 0;
        let error = apply(
            &conn,
            &scope,
            &operations,
            &kassiber_transfers::SystemSources,
            "user",
            cases,
            |_| {
                builds += 1;
                Ok(Cases::default())
            },
        )
        .unwrap_err();
        assert_eq!(builds, 1);
        assert!(
            matches!(error, Error::Service(error) if error.code == "review_case_changed" && error.message == "Review pair no longer holds a suspense case; inspect it again")
        );
    }
}
