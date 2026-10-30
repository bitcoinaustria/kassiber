//! Append-only gap review history, correction context, and filed-report impacts.
use super::component_rows::s;
use super::*;
use super::{component_rows as cr, component_store as cs};
use kassiber_custody::{codec, components::review};
use kassiber_envelope::{set, truthy};
use kassiber_pyjson::BigInt;
use std::collections::BTreeSet;

pub(super) fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    get(v, key).unwrap_or(&Value::Null)
}
pub(super) fn fallback(v: &Value, default: &str) -> Value {
    if truthy(v) {
        v.clone()
    } else {
        s(default)
    }
}
pub(super) fn n(v: &Value, key: &str) -> i64 {
    integer(field(v, key))
        .ok()
        .and_then(|n| i64::try_from(n).ok())
        .unwrap_or(0)
}
pub(super) fn uuid(name: &str) -> String {
    super::component_review::uuid5(name)
}
pub(super) fn fail(code: &str, message: &str) -> AppError {
    AppError::new(code, message)
}
fn object(raw: &Value) -> Value {
    parse_json(&py_str(raw))
        .ok()
        .filter(|v| matches!(v, Value::Dict(_)))
        .unwrap_or_else(|| dict::<&str>([]))
}

pub(super) fn context(c: &Connection, scope: &Scope<'_>, gap: &str, active: bool) -> Result<Value> {
    let found=rows(c,"SELECT * FROM custody_gap_reviews WHERE profile_id=? AND gap_id=? AND revision=(SELECT MAX(revision) FROM custody_gap_reviews WHERE profile_id=? AND gap_id=?) ORDER BY created_at,id",[scope.profile_id,gap,scope.profile_id,gap])?;
    if found.len() != 1 {
        return Err(fail(
            "custody_gap_review_conflict",
            "Custody bridge review history is missing or conflicting",
        )
        .with_hint("Resolve concurrent review history before correcting this bridge."));
    }
    let r = &found[0];
    let id = text(r, "component_id");
    let missing = || {
        fail(
            "custody_gap_review_conflict",
            "Custody bridge component is missing",
        )
    };
    if id.is_empty() {
        return Err(missing());
    }
    let mut component = cr::get_component(c, scope, &id, false).map_err(|_| missing())?;
    let snapshot = cs::snapshot(c, scope)?;
    let stored = snapshot
        .components
        .iter()
        .find(|v| v.header["id"] == id)
        .ok_or_else(missing)?;
    let evidence = review::component_evidence_status(&snapshot, stored)
        .map_err(|e| fail(e.code, &e.message))?;
    if evidence["valid"] != true {
        return Err(fail(
            "custody_gap_stale",
            "Custody bridge evidence changed after review",
        )
        .with_hint("Re-import or restore the reviewed evidence before correcting it.")
        .with_details(dict([(
            "evidence_status",
            codec::from_json(&evidence["status"]).map_err(|e| fail("internal_error", &e))?,
        )])));
    }
    let event = fallback(field(r, "event_kind"), "review_decision");
    let valid = if active {
        text(r, "action") == "resolved"
            && event != s("bridge_reopened")
            && text(&component, "effective_state") == "active"
    } else {
        event == s("bridge_reopened") && text(&component, "state") == "superseded"
    };
    if !valid {
        return Err(fail(
            "custody_gap_review_state",
            "Custody bridge is not in the required correction state",
        )
        .with_details(dict([
            (
                "required_state",
                s(if active { "active" } else { "reopened" }),
            ),
            ("event_kind", event),
            ("component_state", value(&component, "effective_state")),
        ])));
    }
    let snap = parse_json(&py_str(&fallback(field(r, "snapshot_json"), "{}")))
        .ok()
        .filter(|v| matches!(v, Value::Dict(_)))
        .ok_or_else(|| {
            fail(
                "custody_gap_review_conflict",
                "Custody bridge review snapshot is invalid",
            )
        })?;
    set(
        &mut component,
        "legs",
        Value::List(rows(
            c,
            "SELECT * FROM custody_component_legs WHERE component_id=? ORDER BY ordinal,id",
            [&id],
        )?),
    );
    for (out, stored) in [
        ("evidence", "evidence_json"),
        ("conversion_metadata", "conversion_metadata_json"),
    ] {
        let row = rows(
            c,
            &format!("SELECT {stored} FROM custody_components WHERE id=?"),
            [&id],
        )?;
        set(&mut component, out, object(field(&row[0], stored)));
    }
    Ok(dict([
        ("review", r.clone()),
        ("component", component),
        ("snapshot", snap),
    ]))
}

pub(super) fn correction(
    c: &Connection,
    context: &Value,
    operation: &str,
    details: Value,
) -> Result<String> {
    let component = field(context, "component");
    let r = field(context, "review");
    let commitments=rows(c,"SELECT ordinal,quantity_hash,detail_hash FROM custody_component_evidence_commitments WHERE component_id=? ORDER BY ordinal,id",[text(component,"id")])?;
    cs::hash(
        &dict([
            ("schema_version", 1.into()),
            ("operation", s(operation)),
            ("gap_id", value(r, "gap_id")),
            ("review_id", value(r, "id")),
            ("review_revision", value(r, "revision")),
            (
                "review_event_kind",
                fallback(field(r, "event_kind"), "review_decision"),
            ),
            ("component_id", value(component, "id")),
            ("component_lineage_id", value(component, "lineage_id")),
            ("component_revision", value(component, "revision")),
            ("evidence_commitments", Value::List(commitments)),
            ("details", details),
        ]),
        true,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn append(
    c: &Connection,
    scope: &Scope<'_>,
    plan: &Value,
    component: &Value,
    snapshot: &Value,
    fingerprint: &str,
    event: &str,
    action: &str,
    reason: Value,
    sources: &dyn Sources,
) -> Result<Value> {
    let gap = text(plan, "gap_id");
    let id = sources.uuid(c)?;
    let now = sources.now(c)?;
    let revision=value(&rows(c,"SELECT COALESCE(MAX(revision),0)+1 AS revision FROM custody_gap_reviews WHERE profile_id=? AND gap_id=?",[scope.profile_id,&gap])?[0],"revision");
    let candidate = field(plan, "candidate");
    let mut relations = BTreeSet::new();
    if !matches!(candidate, Value::Null) {
        for (role, key) in [("source", "source_ids"), ("return", "return_ids")] {
            for tx in cs::list(candidate, key) {
                relations.insert((role.to_string(), py_str(tx)));
            }
        }
    } else {
        let mut prior=rows(c,"SELECT x.role,x.transaction_id FROM custody_gap_review_transactions x JOIN custody_gap_reviews r ON r.id=x.review_id WHERE r.profile_id=? AND r.gap_id=? AND r.revision=(SELECT MAX(revision) FROM custody_gap_reviews WHERE profile_id=? AND gap_id=?) ORDER BY r.created_at DESC,r.id DESC,CASE x.role WHEN 'source' THEN 0 ELSE 1 END,x.transaction_id,x.id",[scope.profile_id,&gap,scope.profile_id,&gap])?;
        if prior.is_empty() && truthy(field(component, "id")) {
            prior=rows(c,"SELECT CASE role WHEN 'destination' THEN 'return' ELSE 'source' END AS role,COALESCE(anchor_transaction_id,transaction_id) AS transaction_id FROM custody_component_legs WHERE profile_id=? AND component_id=? AND role IN ('source','fee','destination') AND COALESCE(anchor_transaction_id,transaction_id) IS NOT NULL ORDER BY ordinal,id",[scope.profile_id,&text(component,"id")])?;
        }
        for r in prior {
            relations.insert((text(&r, "role"), text(&r, "transaction_id")));
        }
    }
    let row = dict([
        ("id", s(&id)),
        ("workspace_id", s(scope.workspace_id)),
        ("profile_id", s(scope.profile_id)),
        ("gap_id", s(&gap)),
        ("revision", revision),
        ("candidate_fingerprint", s(fingerprint)),
        ("action", s(action)),
        ("event_kind", s(event)),
        ("component_id", value(component, "id")),
        ("authored_source", value(plan, "authored_source")),
        ("reason", reason),
        ("snapshot_json", s(cs::json(snapshot)?)),
        ("created_at", s(&now)),
    ]);
    cs::insert(c, "custody_gap_reviews", &row)?;
    let set_id = cs::hash(
        &Value::List(vec![s("custody-gap-review-relation-set-v1"), s(&id)]),
        true,
    )?;
    cs::insert(
        c,
        "custody_gap_review_relation_sets",
        &dict([
            ("id", s(set_id)),
            ("review_id", s(&id)),
            ("workspace_id", s(scope.workspace_id)),
            ("profile_id", s(scope.profile_id)),
            (
                "expected_source_count",
                Value::Int(
                    relations
                        .iter()
                        .filter(|(role, _)| role == "source")
                        .count()
                        .into(),
                ),
            ),
            (
                "expected_return_count",
                Value::Int(
                    relations
                        .iter()
                        .filter(|(role, _)| role == "return")
                        .count()
                        .into(),
                ),
            ),
            ("created_at", s(&now)),
        ]),
    )?;
    for (role, tx) in relations {
        let key = cs::hash(
            &Value::List(vec![
                s("custody-gap-review-transaction-v2"),
                s(&id),
                s(&role),
                s(&tx),
            ]),
            true,
        )?;
        cs::insert(
            c,
            "custody_gap_review_transactions",
            &dict([
                ("id", s(key)),
                ("review_id", s(&id)),
                ("workspace_id", s(scope.workspace_id)),
                ("profile_id", s(scope.profile_id)),
                ("role", s(role)),
                ("transaction_id", s(tx)),
                ("created_at", s(&now)),
            ]),
        )?;
    }
    Ok(row)
}

const WARNING:&str="Reviewed custody evidence overlaps this saved/filed report period. Rebuild journals and review whether an amended filing is required.";
pub(super) fn summary(
    snapshot: &Value,
    action: &str,
    classification: &str,
    residual: &BigInt,
) -> Result<Value> {
    let quantity = |key| {
        integer(
            get(snapshot, key)
                .filter(|v| truthy(v))
                .unwrap_or(&Value::Int(0.into())),
        )
    };
    let mut buckets = Vec::new();
    if action == "reopen" {
        let total = quantity("source_total_msat")?;
        if total != BigInt::from(0) {
            buckets.push(("custody_review_reopened".to_owned(), total));
        }
    } else {
        for (key, amount) in [
            ("internal_retained".to_owned(), quantity("retained_msat")?),
            (
                if action == "classify_residual" && classification != "suspense_continuation" {
                    format!("reviewed_{classification}")
                } else {
                    "custody_suspense".into()
                },
                if action == "classify_residual" {
                    residual.clone()
                } else {
                    quantity("residual_msat")?
                },
            ),
            ("network_fee".into(), quantity("source_fee_msat")?),
        ] {
            if amount != BigInt::from(0) {
                buckets.push((key, amount));
            }
        }
    }
    buckets.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(dict(buckets.into_iter().map(|(key, amount)| {
        (
            key,
            dict([("count", 1.into()), ("amount_msat", Value::Int(amount))]),
        )
    })))
}

pub(super) fn preview(
    c: &Connection,
    scope: &Scope<'_>,
    snapshot: &Value,
    after: &Value,
) -> Result<Value> {
    let mut years = BTreeSet::new();
    for key in ["started_at", "ended_at"] {
        let text = text(snapshot, key);
        if let Some(year) =
            kassiber_custody::gaps::year_prefix(&text).filter(|_| text.chars().count() >= 4)
        {
            years.insert(i64::try_from(integer(&s(year))?).expect("four decimal digits"));
        }
    }
    for year in cs::list(field(snapshot, "downstream"), "affected_years") {
        let y = integer(year)?;
        if y < 1900.into() || y > 9999.into() {
            return Err(fail(
                "filed_report_snapshot_validation",
                "affected_year must be between 1900 and 9999",
            )
            .with_details(dict([("field", s("affected_year"))])));
        }
        years.insert(i64::try_from(y).expect("bounded calendar year"));
    }
    if years.iter().any(|year| !(1900..=9999).contains(year)) {
        return Err(fail(
            "filed_report_snapshot_validation",
            "affected_year must be between 1900 and 9999",
        )
        .with_details(dict([("field", s("affected_year"))])));
    }
    let mut impacts = Vec::new();
    for r in rows(c,"SELECT * FROM filed_report_snapshots WHERE profile_id=? ORDER BY period_start_year,period_end_year,created_at,id",[scope.profile_id])? {
        let overlap:Vec<_>=years.iter().filter(|&&y|n(&r,"period_start_year")<=y && y<=n(&r,"period_end_year")).collect();
        if overlap.is_empty() {continue;}
        impacts.push(dict([("filed_report_snapshot_id",value(&r,"id")),("report_kind",value(&r,"report_kind")),("report_state",value(&r,"report_state")),("affected_period_start_year",(**overlap.first().unwrap()).into()),("affected_period_end_year",(**overlap.last().unwrap()).into()),("before_classification_summary",object(field(&r,"classification_summary_json"))),("after_classification_summary",after.clone()),("before_gain_summary",object(field(&r,"gain_summary_json"))),("after_gain_summary",dict([("status",s("pending_journal_rebuild"))])),("amendment_warning",s(WARNING))]));
    }
    Ok(Value::List(impacts))
}

pub(super) fn persist_impacts(
    c: &Connection,
    scope: &Scope<'_>,
    plan: &Value,
    component: &Value,
    review: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    let now = sources.now(c)?;
    for preview in cs::list(plan, "filed_report_impacts") {
        let id = uuid(&format!(
            "kassiber:custody-filed-impact:{}:{}",
            text(preview, "filed_report_snapshot_id"),
            text(review, "id")
        ));
        let mut row = dict([
            ("id", s(id)),
            ("workspace_id", s(scope.workspace_id)),
            ("profile_id", s(scope.profile_id)),
            (
                "filed_report_snapshot_id",
                value(preview, "filed_report_snapshot_id"),
            ),
            ("component_id", value(component, "id")),
            ("review_id", value(review, "id")),
            ("gap_id", value(plan, "gap_id")),
            (
                "affected_period_start_year",
                value(preview, "affected_period_start_year"),
            ),
            (
                "affected_period_end_year",
                value(preview, "affected_period_end_year"),
            ),
        ]);
        for key in [
            "before_classification_summary",
            "after_classification_summary",
            "before_gain_summary",
            "after_gain_summary",
        ] {
            set(
                &mut row,
                &format!("{key}_json"),
                s(cs::json(field(preview, key))?),
            );
        }
        set(
            &mut row,
            "amendment_warning",
            value(preview, "amendment_warning"),
        );
        set(&mut row, "created_at", s(&now));
        cs::insert(c, "custody_filed_report_impacts", &row)?;
    }
    let mut out = Vec::new();
    for r in rows(c,"SELECT * FROM custody_filed_report_impacts WHERE profile_id=? AND review_id=? ORDER BY affected_period_start_year,created_at,id",[scope.profile_id,&text(review,"id")])? {
        let mut projected=project(&r,&["id","filed_report_snapshot_id","component_id","review_id","gap_id","affected_period_start_year","affected_period_end_year"]);
        for key in ["before_classification_summary","after_classification_summary","before_gain_summary","after_gain_summary"] {set(&mut projected,key,object(field(&r,&format!("{key}_json"))));}
        set(&mut projected,"amendment_warning",value(&r,"amendment_warning"));set(&mut projected,"resolution",Value::Null);set(&mut projected,"created_at",value(&r,"created_at"));out.push(projected);
    }
    Ok(Value::List(out))
}
