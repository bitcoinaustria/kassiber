//! Raw-stage finalized-projection adapter; no persistence authority.
use crate::evidence::canonical::LegRef;
use crate::{codec, graph::stored, interpretation, snapshot::Snapshot};
use kassiber_pyjson::Value as Py;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

fn exact(value: &Py) -> Result<Value, String> {
    let converted = codec::to_json(value);
    if !codec::lossless(value)
        && codec::dumps(value, true)? != codec::dumps(&codec::from_json(&converted)?, true)?
    {
        return Err("ValueError".into());
    }
    Ok(converted)
}

// Validate every supplied value, including fields the compiler does not read,
// before projecting only the operands it uses. The rare non-lossless case
// keeps the original exact round trip and its refusal ordering.
fn records(value: &Py, select: fn(&Py) -> Value) -> Result<Value, String> {
    if !matches!(value, Py::List(_)) || !codec::lossless(value) {
        return exact(value);
    }
    Ok(Value::Array(
        crate::par::map(stored::list(value), select)
            .into_iter()
            .map(crate::par::unwrap)
            .collect(),
    ))
}
fn fields(value: &Py, names: &[&str]) -> Value {
    let Py::Dict(items) = value else {
        return codec::to_json(value);
    };
    Value::Object(
        items
            .iter()
            .filter_map(|(key, value)| {
                let kassiber_pyjson::Key::Str(key) = key else {
                    return None;
                };
                let key = key.as_str()?;
                names
                    .contains(&key)
                    .then(|| (key.to_owned(), codec::to_json(value)))
            })
            .collect(),
    )
}
fn observation(value: &Py) -> Value {
    fields(
        value,
        &[
            "quantity_hash",
            "profile_id",
            "wallet_id",
            "asset",
            "direction",
            "occurred_at",
            "transaction_id",
            "anchor_transaction_id",
            "event_key",
            "amount_msat",
            "fee_msat",
            "authoritative_chain_observation",
            "fee_attribution",
        ],
    )
}
fn decision(value: &Py) -> Value {
    fields(
        value,
        &[
            "source",
            "target",
            "state",
            "selected_claim_id",
            "reason",
            "atomic_bundle_id",
            "component_id",
            "transfer_policy",
            "transfer_kind",
            "destination_kind",
            "external_economic_subtype",
        ],
    )
}
fn event(value: &Py) -> Value {
    let Py::Dict(items) = value else {
        return codec::to_json(value);
    };
    Value::Object(
        items
            .iter()
            .filter_map(|(key, value)| {
                let kassiber_pyjson::Key::Str(key) = key else {
                    return None;
                };
                let key = key.as_str()?;
                match key {
                    "event_key" => Some((key.to_owned(), codec::to_json(value))),
                    "legs" => Some((
                        key.to_owned(),
                        match value {
                            Py::List(legs) => Value::Array(
                                legs.iter()
                                    .map(|leg| fields(leg, &["occurred_at", "quantity_hash"]))
                                    .collect(),
                            ),
                            value => codec::to_json(value),
                        },
                    )),
                    _ => None,
                }
            })
            .collect(),
    )
}

fn pools(value: &Py) -> Result<Value, String> {
    let mut value = exact(value)?;
    for pair in value.as_array_mut().ok_or("TypeError")? {
        pair[0].as_object_mut().ok_or("TypeError")?.remove("$type");
    }
    Ok(value)
}

/// Unknown blocker ids cannot touch rows or move edges. Keep their diagnostics
/// losslessly, but do not narrow an irrelevant surrogate id through serde.
fn blocked_ids(interpreter: &Py, projection: &Py) -> Result<Value, String> {
    let anchors: std::collections::BTreeSet<_> =
        stored::list(stored::get(projection, "observations"))
            .iter()
            .filter_map(|o| match stored::get(o, "anchor_transaction_id") {
                Py::Str(id) => Some(id.clone()),
                _ => None,
            })
            .collect();
    let mut ids = Vec::new();
    for id in stored::list(stored::get(interpreter, "blocked_transaction_ids")) {
        match exact(id) {
            Ok(id) => ids.push(id),
            Err(_) if matches!(id, Py::Str(id) if !anchors.contains(id)) => (),
            Err(error) => return Err(error),
        }
    }
    Ok(Value::Array(ids))
}

/// Admission captures only confirmation facts, not a second copy of every row.
/// Closed evidence is still owned by the unported evidence stage, so only rows
/// carrying its version marker cross that neighbour's boundary here.
pub fn typed_confirmations(snapshot: &crate::snapshot::BookSnapshot) -> Result<Vec<Value>, String> {
    let captured = crate::par::map(&snapshot.transactions, |row| {
        if matches!(
            row.observation_authority_version.value(),
            crate::snapshot::SqlValue::Null
        ) {
            Ok(Value::Null)
        } else {
            interpretation::capture_confirmation(&row.json())
        }
    });
    captured.into_iter().map(crate::par::unwrap).collect()
}

/// `typed_confirmations` over loaded records: only rows carrying the version
/// marker are read as typed rows.
pub fn loaded_confirmations(snapshot: &Snapshot) -> Result<Vec<Value>, String> {
    snapshot
        .transactions
        .iter()
        .map(|record| match record.get("observation_authority_version") {
            None | Some(Value::Null) => Ok(Value::Null),
            Some(_) => interpretation::capture_confirmation(
                &record.typed::<crate::snapshot::TxRow>()?.json(),
            ),
        })
        .collect()
}

struct BorrowedRows<'a>(
    crate::fxhash::HashMap<std::borrow::Cow<'a, str>, (&'a crate::snapshot::Record, Value)>,
);
impl super::Rows for BorrowedRows<'_> {
    fn get(&self, id: &str) -> Option<super::row::RowRef<'_>> {
        self.0
            .get(id)
            .map(|(row, _)| super::row::RowRef::Record(row))
    }

    fn nonempty(&self, id: &str) -> bool {
        self.0.contains_key(id)
    }

    fn projected(&self, id: &str) -> super::row::FinalizedRow<'_> {
        let Some((row, confirmation)) = self.0.get(id) else {
            return super::row::FinalizedRow::borrowed(&Value::Null);
        };
        let mut projected = super::row::FinalizedRow::of(super::row::RowRef::Record(row));
        projected.set("_closed_confirmation_observation", confirmation.clone());
        projected
    }
}
fn confirmed_rows(snapshot: &Snapshot, captured: Vec<Value>) -> BorrowedRows<'_> {
    let mut rows =
        crate::fxhash::HashMap::with_capacity_and_hasher(captured.len(), Default::default());
    for (row, confirmation) in snapshot.transactions.iter().zip(captured) {
        let id = match row.get("id").unwrap_or(&Value::Null) {
            Value::String(id) => std::borrow::Cow::Borrowed(id.as_str()),
            id if codec::truth(id) => std::borrow::Cow::Owned(codec::pystr(id)),
            _ => std::borrow::Cow::Borrowed(""),
        };
        rows.insert(id, (row, confirmation));
    }
    BorrowedRows(rows)
}

/// Full row copies belong to the finalized stage's boundary, after admission.
pub fn stage_with_confirmations(
    snapshot: &Snapshot,
    captured: Vec<Value>,
    quantity: &Py,
    interpreter: &Py,
) -> Result<Py, super::Error> {
    stage_with_row_source(
        snapshot,
        &confirmed_rows(snapshot, captured),
        quantity,
        interpreter,
    )
}

/// `stage_with_confirmations` over the typed quantity stage.
pub fn stage_from_state(
    snapshot: &Snapshot,
    captured: Vec<Value>,
    quantity: &crate::quantity::runtime::QuantityState,
    interpreter: &Py,
) -> Result<super::FinalizedProjection, super::Error> {
    let span = crate::timing::span("finalized.input");
    let rows = confirmed_rows(snapshot, captured);
    let mut input = state_input(snapshot, quantity, interpreter)?;
    let blockers = state_valuation(snapshot, &rows, quantity, &mut input).map_err(validation)?;
    drop(span);
    let _span = crate::timing::span("finalized.compose");
    let parts =
        super::composition::evaluate_parts(&input, &diagnostics(interpreter, &blockers), &rows)
            .map_err(validation)?;
    let super::composition::Composition::Finalized(parts) = parts else {
        unreachable!("finalize composes a finalized projection")
    };
    let source = &snapshot.transactions;
    let canonical = quantity.canonical_arc();
    Ok(super::FinalizedProjection {
        rows: parts
            .rows
            .into_iter()
            .map(|row| row.detach(source, Some(&canonical)))
            .collect(),
        cross: parts.cross,
        intra: parts.intra,
        moves: parts.moves,
        quarantines: parts.quarantines,
        canonical: Some(canonical),
    })
}

fn validation(error: super::Error) -> super::Error {
    if error.code == "AppError" {
        super::Error {
            code: "validation".into(),
            message: error.message,
        }
    } else {
        error
    }
}

/// Assemble Python compiler operands from previously computed Rust stages.
/// Interpreter diagnostics bypass serde, including grouping and sorting.
pub fn stage_with_rows(
    snapshot: &Snapshot,
    rows: Value,
    quantity: &Py,
    interpreter: &Py,
) -> Result<Py, super::Error> {
    stage_with_row_source(snapshot, &rows, quantity, interpreter)
}

fn stage_with_row_source(
    snapshot: &Snapshot,
    rows: &dyn super::Rows,
    quantity: &Py,
    interpreter: &Py,
) -> Result<Py, super::Error> {
    let span = crate::timing::span("finalized.input");
    let mut input = py_input(snapshot, quantity, interpreter)?;
    let blockers = py_valuation(snapshot, rows, quantity, &mut input).map_err(validation)?;
    drop(span);
    let _span = crate::timing::span("finalized.compose");
    super::composition::evaluate_with_rows(
        &super::composition::Input::from_json(&input),
        &diagnostics(interpreter, &blockers),
        rows,
    )
    .map_err(validation)
}

/// The interpreter's quarantines, then the valuation holds, as Python's
/// compiler appends them.
fn diagnostics(interpreter: &Py, blockers: &[Py]) -> Vec<Py> {
    let mut diagnostics = stored::list(stored::get(interpreter, "quarantines")).to_vec();
    diagnostics.extend_from_slice(blockers);
    diagnostics
}

/// `with_tax_projection_blockers`' issues: each hold as an interpreter-blocked
/// quantity issue on its row. Only these fields reach the eligibility; the
/// issues are not part of any stage result.
fn hold_issues(blockers: &[Py], rows: &dyn super::Rows) -> Vec<Value> {
    blockers
        .iter()
        .map(|item| {
            let id = codec::pystr(&codec::to_json(stored::get(item, "transaction_id")));
            let row = rows.row(&id);
            let field = |key: &str| {
                if codec::truth(&row[key]) {
                    codec::pystr(&row[key])
                } else {
                    String::new()
                }
            };
            json!({"issue_type":"custody_interpreter_blocked","state":"conflicting",
                "asset":field("asset"),"amount_msat":null,"occurred_at":field("occurred_at"),
                "transaction_ids":[id],"reason":codec::to_json(stored::get(item, "reason"))})
        })
        .collect()
}

/// The rows an eligibility over `issues` may read, by id, as JSON objects.
fn eligibility_rows(issues: &[&[Value]], rows: &dyn super::Rows) -> BTreeMap<String, Value> {
    let mut values = BTreeMap::new();
    for issue in issues.iter().flat_map(|issues| issues.iter()) {
        for id in issue["transaction_ids"].as_array().into_iter().flatten() {
            let id = if codec::truth(id) {
                codec::pystr(id)
            } else {
                String::new()
            };
            if let Some(row) = rows.get(&id) {
                values.entry(id).or_insert_with(|| row.value().into_owned());
            }
        }
    }
    values
}

/// `compile_finalized_tax_projection`'s valuation holds over the typed
/// quantity stage: the held transactions are explicitly blocked and tax
/// eligibility is recomputed through the same pool basis barriers.
fn state_valuation<'a>(
    snapshot: &'a Snapshot,
    rows: &dyn super::Rows,
    quantity: &'a crate::quantity::runtime::QuantityState,
    input: &mut super::composition::Input<'a>,
) -> Result<Vec<Py>, super::Error> {
    let blockers = super::composition::holds(
        input,
        snapshot.transactions.iter().map(super::row::RowRef::Record),
        rows,
    )?;
    if blockers.is_empty() {
        return Ok(blockers);
    }
    let issues = hold_issues(&blockers, rows);
    let existing: Vec<Value> = quantity.issues().iter().map(codec::to_json).collect();
    let values = eligibility_rows(&[&existing, &issues], rows);
    let by_id: BTreeMap<String, &Value> = values.iter().map(|(k, v)| (k.clone(), v)).collect();
    let eligibility = quantity.eligibility_with(&issues, &by_id)?;
    apply_eligibility(input, &eligibility, &blockers)?;
    if !matches!(&*input.pool_barriers, Value::Array(barriers) if barriers.is_empty())
        && input.events.is_empty()
    {
        input.events = quantity.events().collect();
    }
    Ok(blockers)
}

/// `state_valuation` over the Python-value quantity stage.
fn py_valuation(
    snapshot: &Snapshot,
    rows: &dyn super::Rows,
    quantity: &Py,
    input: &mut Value,
) -> Result<Vec<Py>, super::Error> {
    let blockers = {
        let view = super::composition::Input::from_json(input);
        super::composition::holds(
            &view,
            snapshot.transactions.iter().map(super::row::RowRef::Record),
            rows,
        )?
    };
    if blockers.is_empty() {
        return Ok(blockers);
    }
    let mut issues: Vec<Value> = stored::list(stored::get(quantity, "issues"))
        .iter()
        .map(codec::to_json)
        .collect();
    let held = hold_issues(&blockers, rows);
    let values = eligibility_rows(&[&issues, &held], rows);
    let by_id: BTreeMap<String, &Value> = values.iter().map(|(k, v)| (k.clone(), v)).collect();
    issues.extend(held);
    let eligibility = crate::quantity::runtime::tax_eligibility(
        &codec::to_json(stored::get(quantity, "canonical_input")),
        &codec::to_json(stored::get(quantity, "projection")),
        &issues,
        &by_id,
    )?;
    input["eligible"] = Value::Array(
        eligibility["eligible_decisions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| d["source"].clone())
            .collect(),
    );
    for key in ["pool_barriers", "pool_barrier_roots"] {
        input[key] = without_pool_types(&eligibility[key])?;
    }
    if !matches!(&input["pool_barriers"], Value::Array(barriers) if barriers.is_empty()) {
        input["events"] = records(
            stored::get(stored::get(quantity, "canonical_input"), "events"),
            event,
        )?;
    }
    let ids = input["blocked_transaction_ids"]
        .as_array_mut()
        .ok_or("TypeError")?;
    ids.extend(super::composition::blocked_ids(&blockers));
    Ok(blockers)
}

/// A recomputed eligibility's pool list, as `pools` admits it.
fn without_pool_types(pools: &Value) -> Result<Value, String> {
    let mut pools = pools.clone();
    for pair in pools.as_array_mut().ok_or("TypeError")? {
        pair[0].as_object_mut().ok_or("TypeError")?.remove("$type");
    }
    Ok(pools)
}

/// The recomputed eligibility and explicit blocks, on typed operands.
fn apply_eligibility(
    input: &mut super::composition::Input<'_>,
    eligibility: &Value,
    blockers: &[Py],
) -> Result<(), String> {
    use std::borrow::Cow;
    input.eligible = eligibility["eligible_decisions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| super::composition::Eligible::Slice(Cow::Owned(d["source"].clone())))
        .collect();
    input.pool_barriers = Cow::Owned(without_pool_types(&eligibility["pool_barriers"])?);
    input.pool_barrier_roots = Cow::Owned(without_pool_types(&eligibility["pool_barrier_roots"])?);
    let mut ids = input.blocked_transaction_ids.to_vec();
    ids.extend(super::composition::blocked_ids(blockers));
    input.blocked_transaction_ids = Cow::Owned(ids);
    Ok(())
}

/// Issue-derived operands: blocked anchors and the compiler's issues.
fn issue_operands(issues: &[Py]) -> Result<(Vec<Value>, Vec<Value>), super::Error> {
    let mut blocked_anchors = Vec::new();
    for issue in issues {
        let component = match stored::get(issue, "reason") {
            Py::Str(reason) => reason
                .code_points()
                .take("custody_component_".len())
                .eq("custody_component_".chars().map(u32::from)),
            _ => return Err("AttributeError".into()),
        };
        if component
            || matches!(stored::get(issue, "issue_type"), Py::Str(kind) if matches!(kind.as_str(), Some("component_claim_compile_failed" | "quantity_claim_bundle_invalid")))
        {
            blocked_anchors.extend(
                stored::list(stored::get(issue, "transaction_ids"))
                    .iter()
                    .map(exact)
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
    }
    // Interpreter-blocked quantity issues are deliberately ignored by Python's
    // compiler. Their complete lossless records remain in interpreter quarantines.
    let issues = issues
        .iter()
        .filter(|i| stored::get(i, "issue_type") != &Py::Str("custody_interpreter_blocked".into()))
        .map(exact)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((blocked_anchors, issues))
}

/// The compiler's operands over the typed quantity stage. Canonical
/// observations, decisions and events are borrowed; every operand the
/// Python-value adapter admits exactly is admitted here in the same order.
fn state_input<'a>(
    snapshot: &Snapshot,
    quantity: &'a crate::quantity::runtime::QuantityState,
    interpreter: &Py,
) -> Result<super::composition::Input<'a>, super::Error> {
    use std::borrow::Cow;
    let blocked = quantity.blocked_eligibility();
    let decisions: Vec<crate::quantity::DecisionRef<'a>> = quantity.decisions().collect();
    let eligible = match blocked {
        Some(tax) => stored::list(stored::get(tax, "eligible_decisions"))
            .iter()
            .map(|d| {
                exact(stored::get(d, "source"))
                    .map(|source| super::composition::Eligible::Slice(Cow::Owned(source)))
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => decisions
            .iter()
            .filter(|d| {
                matches!(
                    d["state"].as_str(),
                    Some(
                        "internal_verified"
                            | "internal_reviewed"
                            | "external_confirmed"
                            | "external_presumed"
                    )
                )
            })
            .map(|d| super::composition::Eligible::Source(*d))
            .collect(),
    };
    let (blocked_anchors, issues) = issue_operands(quantity.issues())?;
    let mut non_events = exact(stored::get(interpreter, "non_event_transaction_ids"))?
        .as_array()
        .ok_or("TypeError")?
        .clone();
    non_events.extend(
        interpretation::channel_non_event_ids(snapshot)?
            .into_iter()
            .map(Value::String),
    );
    let observations: Vec<LegRef> = quantity.observations().collect();
    let (pool_barriers, pool_barrier_roots) = match blocked {
        Some(tax) => (
            pools(stored::get(tax, "pool_barriers"))?,
            pools(stored::get(tax, "pool_barrier_roots"))?,
        ),
        None => (json!([]), json!([])),
    };
    let events = if matches!(&pool_barriers, Value::Array(barriers) if barriers.is_empty()) {
        Vec::new()
    } else {
        quantity.events().collect()
    };
    let anchors: std::collections::BTreeSet<&str> = observations
        .iter()
        .filter_map(|o| o["anchor_transaction_id"].as_str())
        .collect();
    let mut blocked_ids = Vec::new();
    for id in stored::list(stored::get(interpreter, "blocked_transaction_ids")) {
        match exact(id) {
            Ok(id) => blocked_ids.push(id),
            Err(_) if matches!(id, Py::Str(id) if id.as_str().is_none_or(|id| !anchors.contains(id))) =>
                {}
            Err(error) => return Err(error.into()),
        }
    }
    let exact_value = |v: &Value| exact(&crate::codec::from_json(v)?);
    let payouts = exact_value(quantity.reviewed_direct_payouts())?;
    let reviewed = exact_value(quantity.reviewed_conversion_pairs())?;
    Ok(super::composition::Input {
        op: "finalize",
        profile: Cow::Owned(Value::Object(snapshot.profile.clone().into_map())),
        observations,
        decisions,
        eligible,
        events,
        pool_barriers: Cow::Owned(pool_barriers),
        pool_barrier_roots: Cow::Owned(pool_barrier_roots),
        issues: Cow::Owned(issues),
        blocked_anchor_ids: Cow::Owned(blocked_anchors),
        blocked_transaction_ids: Cow::Owned(blocked_ids),
        non_event_transaction_ids: Cow::Owned(non_events),
        payouts: Cow::Owned(payouts),
        reviewed: Cow::Owned(match reviewed {
            Value::Array(items) => items,
            _ => Vec::new(),
        }),
        source_rank: super::composition::source_rank(transaction_ids(snapshot)),
    })
}

/// The snapshot's transaction ids in its source order, keyed as
/// `confirmed_rows` keys them.
fn transaction_ids(snapshot: &Snapshot) -> impl Iterator<Item = String> + '_ {
    snapshot
        .transactions
        .iter()
        .map(|row| match row.get("id").unwrap_or(&Value::Null) {
            Value::String(id) => id.clone(),
            id if codec::truth(id) => codec::pystr(id),
            _ => String::new(),
        })
}

/// The Python-value adapter's operand object.
fn py_input(snapshot: &Snapshot, quantity: &Py, interpreter: &Py) -> Result<Value, super::Error> {
    let projection = stored::get(quantity, "projection");
    let eligibility = stored::get(quantity, "tax_eligibility");
    let eligible = stored::list(stored::get(eligibility, "eligible_decisions"))
        .iter()
        .map(|d| exact(stored::get(d, "source")))
        .collect::<Result<Vec<_>, _>>()?;
    let canonical = stored::get(quantity, "canonical_input");
    let (blocked_anchors, issues) = issue_operands(stored::list(stored::get(quantity, "issues")))?;
    let mut non_events = exact(stored::get(interpreter, "non_event_transaction_ids"))?
        .as_array()
        .ok_or("TypeError")?
        .clone();
    non_events.extend(
        interpretation::channel_non_event_ids(snapshot)?
            .into_iter()
            .map(Value::String),
    );
    // Operands move into the input object in their original evaluation
    // order, so the first refusal is unchanged and nothing is copied twice.
    let mut input = Map::new();
    input.insert("op".into(), json!("finalize"));
    input.insert(
        "row_order".into(),
        Value::Array(transaction_ids(snapshot).map(Value::String).collect()),
    );
    input.insert(
        "profile".into(),
        Value::Object(snapshot.profile.clone().into_map()),
    );
    input.insert(
        "observations".into(),
        records(stored::get(projection, "observations"), observation)?,
    );
    input.insert(
        "decisions".into(),
        records(stored::get(projection, "decisions"), decision)?,
    );
    input.insert("eligible".into(), Value::Array(eligible));
    input.insert(
        "events".into(),
        records(
            stored::get(canonical, "events"),
            if matches!(stored::get(eligibility, "pool_barriers"), Py::List(barriers) if barriers.is_empty()) {
                |_| Value::Null
            } else {
                event
            },
        )?,
    );
    input.insert(
        "pool_barriers".into(),
        pools(stored::get(eligibility, "pool_barriers"))?,
    );
    input.insert(
        "pool_barrier_roots".into(),
        pools(stored::get(eligibility, "pool_barrier_roots"))?,
    );
    input.insert("issues".into(), Value::Array(issues));
    input.insert("blocked_anchor_ids".into(), Value::Array(blocked_anchors));
    input.insert(
        "blocked_transaction_ids".into(),
        blocked_ids(interpreter, projection)?,
    );
    input.insert("non_event_transaction_ids".into(), Value::Array(non_events));
    input.insert(
        "payouts".into(),
        exact(stored::get(quantity, "reviewed_direct_payouts"))?,
    );
    input.insert(
        "reviewed".into(),
        exact(stored::get(quantity, "reviewed_conversion_pairs"))?,
    );
    Ok(Value::Object(input))
}

pub fn stage(snapshot: &Snapshot, quantity: &Py, interpreter: &Py) -> Result<Py, String> {
    let text = serde_json::to_string(snapshot).map_err(|e| e.to_string())?;
    let typed = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    stage_with_confirmations(
        snapshot,
        typed_confirmations(&typed)?,
        quantity,
        interpreter,
    )
    .map_err(|error| error.code)
}
