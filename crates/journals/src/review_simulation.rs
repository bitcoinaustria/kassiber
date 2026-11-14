use crate::review_workflow::Error;
use crate::JournalState;
use kassiber_custody::codec;
use kassiber_envelope::{dict, get, s, set, AppError};
use kassiber_pyjson::{PyStr, Value};
use kassiber_store::rusqlite::Connection;
use kassiber_transactions::metadata::{self, Command, GroupedUpdate, PricingUpdate};
use kassiber_transfers::review_support::sql;
use kassiber_transfers::review_support::{text, value};
use kassiber_transfers::{review_cases, review_operations, review_support, Scope, Sources};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const COARSE_PRICE_REASON: &str = "pricing_review_required";

#[derive(Default)]
pub(crate) struct Cases {
    quarantined: BTreeSet<String>,
    /// Held because a coarse price waits for the owner.
    coarse_review: BTreeSet<String>,
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
            coarse_review: state
                .quarantines
                .iter()
                .filter(|row| row.reason == COARSE_PRICE_REASON)
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
                // A reviewed price is in the book's currency, which the
                // journals read it in, whatever an unpriced row was imported with.
                let book = sql::rows(
                    conn,
                    "SELECT fiat_currency FROM profiles WHERE id=?",
                    [scope.profile_id],
                )?;
                let pricing = PricingUpdate {
                    fiat_currency: book
                        .first()
                        .map_or(Value::Null, |row| value(row, "fiat_currency")),
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

fn current_version(conn: &Connection, scope: &Scope<'_>) -> Result<i64, Error> {
    let row = sql::rows(
        conn,
        "SELECT journal_input_version FROM profiles WHERE id=?",
        [scope.profile_id],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::new("not_found", "Review profile was not found"))?;
    Ok(review_support::input_version(&row))
}

/// `_msat.msat_to_btc` formatted as Python's `format(..., "f")`: the quotient
/// at the 32-digit context, in fixed notation.
fn btc_text(msat: &Value) -> Result<String, Error> {
    use kassiber_pydecimal::{Context, Decimal, Rounding};
    let arithmetic = |error: kassiber_pydecimal::DecimalError| {
        AppError::new("internal_error", error.to_string())
    };
    let amount: Decimal = kassiber_envelope::py_str(msat)
        .parse()
        .map_err(|_| AppError::new("internal_error", "invalid msat amount"))?;
    let btc = amount
        .div(
            &Decimal::from(100_000_000_000_i64),
            &Context::new(32, Rounding::HalfEven),
        )
        .map_err(arithmetic)?;
    Ok(btc.to_fixed_string().map_err(arithmetic)?)
}

/// Pair creation and candidate dismissal or its undo, over the existing
/// services; custody-gap actions through the gap review.
fn term_operation(
    conn: &Connection,
    scope: &Scope<'_>,
    operation: &Value,
    sources: &dyn Sources,
    source: &str,
) -> Result<Value, Error> {
    let kind = text(operation, "type");
    if kind == "custody_gap" {
        let mut request = dict([
            ("action", value(operation, "action")),
            ("gap_id", value(operation, "gap_id")),
            ("reason", value(operation, "reason")),
            (
                "expected_input_version",
                current_version(conn, scope)?.into(),
            ),
            ("authored_source", s(source)),
        ]);
        if let Some(classification) = get(operation, "classification") {
            set(&mut request, "classification", classification.clone());
        }
        return Ok(kassiber_transfers::execute_with_sources(
            conn,
            scope,
            "transfers.gaps.apply",
            &request,
            sources,
        )?);
    }
    let mut legs = Vec::new();
    for key in ["out_transaction_id", "in_transaction_id"] {
        legs.push(
            sql::rows(
                conn,
                "SELECT * FROM transactions WHERE id=? AND profile_id=?",
                kassiber_store::rusqlite::params![
                    review_support::reference_bind(&value(operation, key), 1)?,
                    scope.profile_id
                ],
            )?
            .into_iter()
            .next(),
        );
    }
    let (Some(out_row), Some(in_row)) = (legs[0].take(), legs[1].take()) else {
        return Err(AppError::new("not_found", "Review transaction was not found").into());
    };
    let ids = Value::List(vec![value(&out_row, "id"), value(&in_row, "id")]);
    let legs = |extra: &[(&str, Value)]| {
        let mut args = dict([
            ("tx_out", value(&out_row, "id")),
            ("tx_in", value(&in_row, "id")),
        ]);
        for (key, item) in extra {
            set(&mut args, key, item.clone());
        }
        args
    };
    match kind.as_str() {
        "undismiss_pair" => {
            let removed = kassiber_transfers::execute_with_sources(
                conn,
                scope,
                "transfers.dismissals.delete",
                &legs(&[]),
                sources,
            )?;
            Ok(dict([
                ("dismissal_id", value(&removed, "deleted")),
                ("transaction_ids", ids),
            ]))
        }
        "dismiss_pair" => {
            let row = kassiber_transfers::execute_with_sources(
                conn,
                scope,
                "transfers.dismiss",
                &legs(&[
                    ("reason", value(operation, "reason")),
                    (
                        "expires_in_days",
                        get(operation, "expires_in_days")
                            .cloned()
                            .unwrap_or(90.into()),
                    ),
                ]),
                sources,
            )?;
            Ok(dict([
                ("dismissal_id", value(&row, "id")),
                ("transaction_ids", ids),
                ("expires_at", value(&row, "expires_at")),
            ]))
        }
        _ => {
            if value(&out_row, "direction") != s("outbound")
                || value(&in_row, "direction") != s("inbound")
            {
                return Err(AppError::new(
                    "validation",
                    "Pair review requires an outbound and an inbound transaction",
                )
                .into());
            }
            let amount = match get(operation, "out_amount_msat") {
                Some(msat) => s(btc_text(msat)?),
                None => Value::Null,
            };
            let pair = kassiber_transfers::execute_with_sources(
                conn,
                scope,
                "transfers.pair",
                &legs(&[
                    (
                        "kind",
                        get(operation, "pair_kind")
                            .cloned()
                            .unwrap_or_else(|| s("manual")),
                    ),
                    ("policy", value(operation, "policy")),
                    ("notes", value(operation, "reason")),
                    ("out_amount", amount),
                    ("authored_source", s(source)),
                ]),
                sources,
            )?;
            Ok(dict([
                ("pair_id", value(&pair, "id")),
                ("transaction_ids", ids),
                ("kind", value(&pair, "kind")),
                ("policy", value(&pair, "policy")),
            ]))
        }
    }
}

/// Held transactions whose own coarse price waits for the owner. A transfer's
/// other leg held for the same reason has no coarse price of its own to
/// accept; it clears when the priced leg is accepted.
fn coarse_price_cases(
    conn: &Connection,
    scope: &Scope<'_>,
    cases: &Cases,
) -> Result<Vec<String>, Error> {
    let coarse: BTreeSet<String> = sql::rows(
        conn,
        "SELECT id FROM transactions WHERE profile_id=? AND pricing_quality=?",
        [scope.profile_id, "coarse_fallback"],
    )?
    .iter()
    .map(|row| text(row, "id"))
    .collect();
    Ok(cases
        .coarse_review
        .iter()
        .filter(|id| coarse.contains(*id))
        .cloned()
        .collect())
}

fn accept_coarse_prices(
    conn: &Connection,
    scope: &Scope<'_>,
    operation: &mut Value,
    cases: &Cases,
    resolve: bool,
    source: &str,
) -> Result<Value, Error> {
    if resolve && get(operation, "transaction_ids").is_none() {
        let ids = coarse_price_cases(conn, scope, cases)?;
        if ids.is_empty() {
            return Err(AppError::new(
                "review_case_changed",
                "No transaction waits on a coarse price review",
            )
            .into());
        }
        if ids.len() > review_operations::MAX_ACCEPTED_PRICES {
            return Err(AppError::new(
                "validation",
                format!(
                    "More than {} coarse prices wait; name them in batches",
                    review_operations::MAX_ACCEPTED_PRICES
                ),
            )
            .into());
        }
        set(
            operation,
            "transaction_ids",
            Value::List(ids.into_iter().map(s).collect()),
        );
    }
    let Some(Value::List(named)) = get(operation, "transaction_ids").cloned() else {
        return Err(AppError::new(
            "review_artifact_invalid",
            "accept_coarse_prices must name its transactions",
        )
        .into());
    };
    let coarse: BTreeSet<String> = coarse_price_cases(conn, scope, cases)?
        .into_iter()
        .collect();
    let missing: Vec<Value> = named
        .iter()
        .filter(|id| !coarse.contains(&kassiber_envelope::py_str(id)))
        .take(20)
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(AppError::new(
            "review_case_changed",
            "A named transaction no longer waits on a coarse price review",
        )
        .with_details(dict([("transaction_ids", Value::List(missing))]))
        .into());
    }
    for id in &named {
        metadata::execute(
            conn,
            &kassiber_transactions::Scope {
                workspace_id: scope.workspace_id,
                profile_id: scope.profile_id,
            },
            &Command::Update(Box::new(GroupedUpdate {
                transaction: kassiber_envelope::py_str(id),
                accept_coarse_price: true,
                reason: match get(operation, "reason") {
                    Some(Value::Str(reason)) => Some(reason.clone()),
                    _ => None,
                },
                source: if source == "user" { "gui" } else { source }.into(),
                ..Default::default()
            })),
        )?;
    }
    Ok(dict([
        ("transaction_ids", Value::List(named.clone())),
        ("accepted", named.len().into()),
    ]))
}

/// Apply normalized operations in order. With `resolve_acceptance` (planning)
/// an `accept_coarse_prices` that names no transaction is filled in place
/// with the current cases, so the artifact names each one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply(
    conn: &Connection,
    scope: &Scope<'_>,
    operations: &mut [Value],
    sources: &dyn Sources,
    source: &str,
    resolve_acceptance: bool,
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
    for operation in operations.iter() {
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
    for (index, operation) in operations.iter_mut().enumerate() {
        let kind = text(operation, "type");
        let result = match kind.as_str() {
            "pair" | "dismiss_pair" | "undismiss_pair" | "custody_gap" => {
                term_operation(conn, scope, operation, sources, source)?
            }
            "accept_coarse_prices" => {
                // Earlier operations can expose or clear coarse-price holds;
                // target expansion and validation use this batch position.
                if index > 0 {
                    state = rebuild(conn)?;
                }
                accept_coarse_prices(conn, scope, operation, &state, resolve_acceptance, source)?
            }
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
            "repair_legacy_pair" => {
                let id = exact_text(operation, "pair_id");
                let changed_case = || {
                    AppError::new(
                        "review_case_changed",
                        "Legacy pair changed; inspect it again",
                    )
                };
                let Some(pair) = review_cases::current_pairs(conn, scope)?
                    .into_iter()
                    .find(|pair| exact_text(pair, "id") == id)
                else {
                    return Err(
                        AppError::new("review_case_changed", "Review pair was not found").into(),
                    );
                };
                let guidance =
                    kassiber_transfers::pair_guidance::pair_review_guidance(conn, scope, &pair)?;
                let offered = kassiber_transfers::pair_guidance::repair(&guidance)
                    .map(|repair| value(repair, "preview_operation"));
                if review_cases::pair_fingerprint(conn, &pair)?
                    != text(operation, "expected_fingerprint")
                    || offered != Some(s("repair_legacy_pair"))
                {
                    return Err(changed_case().into());
                }
                let prior = value(&pair, "notes");
                let notes = format!(
                    "{}\nLegacy fee review: {}",
                    if kassiber_envelope::truthy(&prior) {
                        kassiber_envelope::py_str(&prior)
                    } else {
                        String::new()
                    },
                    text(operation, "reason")
                );
                kassiber_transfers::execute_with_sources(
                    conn,
                    scope,
                    "transfers.update",
                    &dict([
                        ("pair_id", Value::Str(id.clone())),
                        ("notes", s(kassiber_pytext::strip(&notes))),
                        ("authored_source", s(source)),
                    ]),
                    sources,
                )?
            }
            "confirm_distinct_import_rows" => {
                // Judged on the held group itself: another reason may head a
                // member's case.
                let review_id = sources.uuid(conn)?;
                let now = sources.now(conn)?;
                let confirmation = kassiber_wallets::import_identity::confirm_distinct(
                    conn,
                    scope.workspace_id,
                    scope.profile_id,
                    &text(operation, "transaction_id"),
                    &text(operation, "expected_fingerprint"),
                    &text(operation, "reason"),
                    source,
                    &review_id,
                    &now,
                )?;
                dict([
                    ("review_id", s(confirmation.review_id)),
                    ("group_fingerprint", s(confirmation.group_fingerprint)),
                    (
                        "transaction_ids",
                        Value::List(confirmation.transaction_ids.into_iter().map(s).collect()),
                    ),
                ])
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
        let cases =
            crate::pipeline::load_review_state(&conn, profile, "2026-10-31T12:00:00Z", |state| {
                Cases::from_state(&state)
            })
            .unwrap();
        let mut operations = list(value(&plan, "operations"));
        let mut builds = 0;
        let error = apply(
            &conn,
            &scope,
            &mut operations,
            &kassiber_transfers::SystemSources,
            "user",
            false,
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
