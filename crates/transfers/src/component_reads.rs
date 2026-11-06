//! Stored component reads reuse the same materialization as authoring previews.
use super::component_rows as cr;
use super::*;
use kassiber_envelope::{s, set, truthy};
use kassiber_store::rusqlite::params_from_iter;

fn transaction(c: &Connection, scope: &Scope<'_>, reference: &Value) -> Result<Value> {
    kassiber_transactions::lookup::resolve_transaction(c, scope.profile_id, reference, None)
        .map(|row| value(&row, "id"))
}

pub(super) fn reference_bind(
    v: &Value,
    index: usize,
) -> Result<kassiber_store::rusqlite::types::Value> {
    if let Value::Str(text) = v {
        let points: Vec<_> = text.code_points().collect();
        if let Some(start) = points.iter().position(|p| (0xd800..=0xdfff).contains(p)) {
            let count = points[start..]
                .iter()
                .take_while(|p| (0xd800..=0xdfff).contains(*p))
                .count();
            let message = if count == 1 {
                format!("'utf-8' codec can't encode character '\\u{:04x}' in position {start}: surrogates not allowed",points[start])
            } else {
                format!("'utf-8' codec can't encode characters in position {start}-{}: surrogates not allowed",start+count-1)
            };
            return Err(AppError::new("internal_error", message));
        }
    }
    sql::bind(v, index)
}

fn component(c: &Connection, scope: &Scope<'_>, id: &str, local: bool) -> Result<Value> {
    let result = cr::get_component(c, scope, id, true)?;
    with_local_evidence(c, result, local)
}

fn with_local_evidence(c: &Connection, mut result: Value, local: bool) -> Result<Value> {
    if local {
        let id = text(&result, "id");
        let stored = rows(
            c,
            "SELECT evidence_json,conversion_metadata_json FROM custody_components WHERE id=?",
            [&id],
        )?
        .remove(0);
        if let Some(Value::List(legs)) = get(&result, "legs").cloned() {
            let mut projected = Vec::new();
            for mut leg in legs {
                let location = rows(
                    c,
                    "SELECT location_ref FROM custody_component_legs WHERE id=?",
                    [text(&leg, "id")],
                )?
                .remove(0);
                if let Value::Dict(fields) = &mut leg {
                    let before = fields
                        .iter()
                        .position(|(k, _)| *k == "notes".into())
                        .unwrap_or(fields.len());
                    fields.insert(
                        before,
                        ("location_ref".into(), value(&location, "location_ref")),
                    );
                }
                projected.push(leg);
            }
            set(&mut result, "legs", Value::List(projected));
        }
        set(
            &mut result,
            "evidence",
            cr::json_object(&value(&stored, "evidence_json"), "evidence_json")?,
        );
        set(
            &mut result,
            "conversion_metadata",
            cr::json_object(
                &value(&stored, "conversion_metadata_json"),
                "conversion_metadata_json",
            )?,
        );
    }
    Ok(result)
}

pub(super) fn execute(
    c: &Connection,
    scope: &Scope<'_>,
    operation: &str,
    args: &Value,
) -> Result<Value> {
    let local = get(args, "include_local_evidence").is_some_and(truthy);
    if operation == "transfers.components.show" {
        if let Some(id) = get(args, "component_id") {
            reference_bind(id, 1)?;
        }
        return component(c, scope, &text(args, "component_id"), local);
    }
    // CLI and desktop resolve transaction references before the core list checks.
    let selected_transaction = get(args, "transaction")
        .filter(|v| !matches!(v, Value::Null))
        .map(|v| transaction(c, scope, v))
        .transpose()?;
    if get(args, "desktop_kind") == Some(&s("ui.transfers.components.list"))
        && get(args, "effective_only").is_some_and(|v| !matches!(v, Value::Bool(_)))
    {
        return Err(AppError::new(
            "validation",
            "ui.transfers.components.list effective_only must be a boolean",
        ));
    }
    let limit = get(args, "limit").cloned().unwrap_or(200.into());
    let Value::Int(ref n) = limit else {
        return Err(cr::fail(
            "custody_component_validation",
            "limit must be between 1 and 1001",
            vec![("limit", limit)],
        ));
    };
    if n < &1.into() || n > &1001.into() {
        return Err(cr::fail(
            "custody_component_validation",
            "limit must be between 1 and 1001",
            vec![("limit", limit)],
        ));
    }
    let count = usize::try_from(n.clone()).unwrap();
    let effective = get(args, "effective_only").is_some_and(truthy);
    let mut conditions = vec!["c.profile_id=?"];
    let mut bound = vec![s(scope.profile_id)];
    if let Some(state) = get(args, "state").filter(|v| !matches!(v, Value::Null)) {
        let state = cr::required_text(state, "state", true)?;
        if ![s("draft"), s("active"), s("superseded")].contains(&state) {
            return Err(cr::fail(
                "custody_component_validation",
                "state is not supported",
                vec![],
            ));
        }
        conditions.push("c.state=?");
        bound.push(state);
    }
    if let Some(kind) = get(args, "component_type").filter(|v| !matches!(v, Value::Null)) {
        conditions.push("c.component_type=?");
        bound.push(cr::component_type(kind)?);
    }
    if let Some(id) = selected_transaction {
        conditions.push("EXISTS (SELECT 1 FROM custody_component_legs l WHERE l.component_id=c.id AND COALESCE(l.anchor_transaction_id,l.transaction_id)=?)");
        bound.push(id);
    }
    let limit_sql = if effective {
        ""
    } else {
        bound.push(limit);
        " LIMIT ?"
    };
    let bound = bound
        .iter()
        .enumerate()
        .map(|(i, v)| sql::bind(v, i + 1))
        .collect::<Result<Vec<_>>>()?;
    let selected = rows(c,&format!("SELECT c.* FROM custody_components c WHERE {} ORDER BY c.created_at DESC,c.revision DESC,c.id DESC{limit_sql}",conditions.join(" AND ")),params_from_iter(bound))?;
    let snapshot = super::component_store::snapshot(c, scope)?;
    let mut result = Vec::new();
    for row in selected {
        let item = with_local_evidence(c, cr::materialize_read(c, &snapshot, &row)?, local)?;
        if effective && text(&item, "effective_state") != "active" {
            continue;
        }
        result.push(item);
        if result.len() == count {
            break;
        }
    }
    Ok(Value::List(result))
}
