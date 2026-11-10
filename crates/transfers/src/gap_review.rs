//! Version-bound, atomic guided gap authoring. Discovery never grants authority.
use super::component_rows::s;
use super::*;
use super::{component_rows as cr, component_store as cs, gap_review_store as gs};
use gs::{fail, fallback, field, n, uuid};
use kassiber_custody::{codec, components::review, gaps};
use kassiber_envelope::{set, truthy};
use kassiber_pyjson::BigInt;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn execute(
    c: &Connection,
    scope: &Scope<'_>,
    operation: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    // Python's CLI/daemon resolves this before entering the action savepoint.
    let candidate = if ["create", "revise", "dismiss"].contains(&text(args, "action").as_str()) {
        super::gap_candidates::find(c, scope, &text(args, "gap_id"))?
    } else {
        Value::Null
    };
    if operation == "transfers.gaps.plan" {
        return Ok(public(&plan(c, scope, args, candidate, sources)?));
    }
    if let Some(Value::Str(kind)) = get(args, "desktop_kind") {
        if !matches!(get(args,"expected_input_version"),Some(Value::Int(v)) if v>=&0.into()) {
            return Err(AppError::new(
                "validation",
                format!(
                    "{} requires a non-negative expected_input_version",
                    kind.as_str().unwrap_or_default()
                ),
            ));
        }
    }
    sql::atomic(c, || {
        cr::require_input_version(c, scope, field(args, "expected_input_version"))?;
        let p = plan(c, scope, args, candidate, sources)?;
        apply(c, scope, &p, sources)
    })
}
fn warnings(candidate: &Value) -> Value {
    let mut out = vec![s("manual_review_required")];
    if !truthy(field(candidate, "promotion_eligible")) {
        out.push(s("weak_advisory_evidence"));
    }
    if n(candidate, "conflict_size") > 1 {
        out.push(s("competing_candidates"));
    }
    if truthy(field(candidate, "residual_msat")) {
        out.push(s("unresolved_residual"));
    }
    if truthy(field(candidate, "excess_msat")) {
        out.push(s("excess_return_unclassified"));
    }
    Value::List(out)
}
fn fingerprint(candidate: &Value, authored: bool) -> Result<String> {
    let raw = codec::to_json(candidate);
    (if authored {
        gaps::authored_claim_fingerprint(&raw)
    } else {
        gaps::candidate_fingerprint(&raw)
    })
    .map_err(|e| fail("internal_error", &e))
}
fn snapshot(c: &Connection, scope: &Scope<'_>, candidate: &Value) -> Result<Value> {
    let affected=rows(c,"SELECT occurred_at FROM transactions WHERE profile_id=? AND direction='outbound' AND asset=? AND occurred_at>?",[scope.profile_id,&text(candidate,"asset"),&text(candidate,"ended_at")])?;
    let years: BTreeSet<_> = affected
        .iter()
        .filter_map(|r| gaps::year_prefix(&text(r, "occurred_at")))
        .map(|year| integer(&s(year)))
        .collect::<Result<_>>()?;
    let mut out = dict([
        ("gap_id", value(candidate, "gap_id")),
        ("candidate_fingerprint", s(fingerprint(candidate, false)?)),
        (
            "authored_claim_fingerprint",
            s(fingerprint(candidate, true)?),
        ),
        ("status", s("needs_review")),
        ("asset", value(candidate, "asset")),
        (
            "source_wallet_label",
            s(cs::list(candidate, "source_wallet_labels")
                .iter()
                .map(py_str)
                .collect::<Vec<_>>()
                .join(" + ")),
        ),
    ]);
    for key in [
        "destination_wallet_labels",
        "source_total_msat",
        "source_fee_msat",
        "source_debit_msat",
        "return_total_msat",
        "retained_msat",
        "residual_msat",
        "excess_msat",
        "started_at",
        "ended_at",
        "confidence",
        "promotion_eligible",
        "competitor_score_margin",
        "reason_codes",
    ] {
        set(&mut out, key, value(candidate, key));
    }
    set(&mut out, "review_warnings", warnings(candidate));
    set(
        &mut out,
        "downstream",
        dict([
            ("affected_disposals", Value::Int(affected.len().into())),
            (
                "affected_years",
                Value::List(years.into_iter().map(Value::Int).collect()),
            ),
        ]),
    );
    Ok(out)
}
fn amount(v: &Value, key: &str) -> Result<BigInt> {
    integer(
        get(v, key)
            .filter(|v| truthy(v))
            .unwrap_or(&Value::Int(0.into())),
    )
}
fn allocation(source: &str, sink: &str, amount: &BigInt) -> Value {
    dict([
        ("source_leg_id", s(source)),
        ("sink_leg_id", s(sink)),
        ("source_amount_msat", Value::Int(amount.clone())),
        ("sink_amount_msat", Value::Int(amount.clone())),
    ])
}
fn anchored(id: &str, role: &str, amount: BigInt, row: &Value) -> Value {
    let (rail, chain, network) = review::protocol_scope(&codec::to_json(row));
    dict([
        ("id", s(id)),
        ("role", s(role)),
        ("rail", s(rail)),
        ("chain", chain.map(s).unwrap_or(Value::Null)),
        ("network", network.map(s).unwrap_or(Value::Null)),
        ("asset", value(row, "asset")),
        ("exposure", s("bitcoin")),
        ("conservation_unit", s("msat")),
        ("amount_msat", Value::Int(amount)),
        ("transaction_id", value(row, "id")),
        ("wallet_id", value(row, "wallet_id")),
        ("occurred_at", value(row, "occurred_at")),
    ])
}
fn guided(
    c: &Connection,
    scope: &Scope<'_>,
    candidate: &Value,
    fp: &str,
    source: &Value,
) -> Result<Value> {
    let mut by_id = BTreeMap::new();
    for id in cs::list(candidate, "source_ids")
        .iter()
        .chain(cs::list(candidate, "return_ids"))
    {
        let found=rows(c,"SELECT t.id,t.wallet_id,t.direction,t.asset,t.amount,t.fee,t.amount_includes_fee,t.occurred_at,t.external_id,t.raw_json,w.kind AS wallet_kind,w.config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.id=?",[scope.profile_id,&py_str(id)])?;
        let Some(row) = found.first() else {
            return Err(fail("custody_gap_stale", "Custody-gap evidence changed"));
        };
        by_id.insert(py_str(id), row.clone());
    }
    for row in by_id.values() {
        let (rail, _, network) = review::protocol_scope(&codec::to_json(row));
        if rail != text(candidate, "protocol_chain")
            || network.as_deref() != Some(text(candidate, "network").as_str())
        {
            return Err(fail(
                "custody_gap_stale",
                "Custody-gap protocol scope changed",
            ));
        }
    }
    let mut legs = Vec::new();
    let mut allocations = Vec::new();
    let mut source_remaining = Vec::new();
    for (i, id) in cs::list(candidate, "source_ids").iter().enumerate() {
        let row = &by_id[&py_str(id)];
        let boundary = codec::from_json(
            &review::boundary_amounts(&codec::to_json(row))
                .map_err(|e| fail(e.code, &e.message))?,
        )
        .map_err(|e| fail("internal_error", &e))?;
        let source_id = format!("source-{i}");
        legs.push(anchored(
            &source_id,
            "source",
            amount(&boundary, "wallet_movement_msat")?,
            row,
        ));
        source_remaining.push((
            source_id,
            amount(&boundary, "principal_msat")?,
            row,
            amount(&boundary, "fee_msat")?,
        ));
    }
    let mut retained = amount(candidate, "retained_msat")?;
    let mut destinations = Vec::new();
    for (i, id) in cs::list(candidate, "return_ids").iter().enumerate() {
        let row = &by_id[&py_str(id)];
        let boundary = codec::from_json(
            &review::boundary_amounts(&codec::to_json(row))
                .map_err(|e| fail(e.code, &e.message))?,
        )
        .map_err(|e| fail("internal_error", &e))?;
        let a = amount(&boundary, "principal_msat")?.min(retained.clone());
        if a <= BigInt::from(0) {
            continue;
        }
        let id = format!("destination-{i}");
        legs.push(anchored(&id, "destination", a.clone(), row));
        destinations.push((id, a.clone()));
        retained -= a;
    }
    if retained != BigInt::from(0) {
        return Err(fail(
            "custody_gap_stale",
            "Custody-gap retained quantity is no longer available",
        ));
    }
    // FIFO order is candidate.source_ids / return_ids, both canonical sorted ids.
    let mut index = 0;
    for (dest, mut remaining) in destinations {
        while remaining > BigInt::from(0) {
            while index < source_remaining.len() && source_remaining[index].1 == BigInt::from(0) {
                index += 1;
            }
            let Some((source, available, _, _)) = source_remaining.get_mut(index) else {
                return Err(fail(
                    "custody_gap_stale",
                    "Custody-gap retained quantity is no longer available",
                ));
            };
            let cell = available.clone().min(remaining.clone());
            allocations.push(allocation(source, &dest, &cell));
            *available -= &cell;
            remaining -= cell;
        }
    }
    for (i, (source, remaining, row, fee)) in source_remaining.into_iter().enumerate() {
        if remaining != BigInt::from(0) {
            let id = format!("suspense-{i}");
            let (_, _, network) = review::protocol_scope(&codec::to_json(row));
            legs.push(dict([
                ("id", s(&id)),
                ("role", s("suspense")),
                ("rail", s("untracked")),
                ("chain", Value::Null),
                ("network", network.map(s).unwrap_or(Value::Null)),
                ("asset", value(row, "asset")),
                ("exposure", s("bitcoin")),
                ("conservation_unit", s("msat")),
                ("amount_msat", Value::Int(remaining.clone())),
                ("occurred_at", value(row, "occurred_at")),
            ]));
            allocations.push(allocation(&source, &id, &remaining));
        }
        if fee != BigInt::from(0) {
            let id = format!("fee-{i}");
            legs.push(anchored(&id, "fee", fee.clone(), row));
            allocations.push(allocation(&source, &id, &fee));
        }
    }
    Ok(dict([
        (
            "component_id",
            s(uuid(&format!(
                "kassiber:custody-gap-bridge:{}:{}:{fp}",
                scope.profile_id,
                text(candidate, "gap_id")
            ))),
        ),
        ("component_type", s("manual_bridge")),
        ("conservation_mode", s("quantity")),
        ("evidence_kind", s("custody_gap_review")),
        ("evidence_grade", s("reviewed")),
        (
            "evidence",
            dict([
                ("gap_id", value(candidate, "gap_id")),
                ("authored_claim_fingerprint", s(fp)),
                ("review_warnings", warnings(candidate)),
            ]),
        ),
        (
            "notes",
            s("Guided bridge from explicitly reviewed custody-gap evidence"),
        ),
        ("authored_source", source.clone()),
        ("legs", Value::List(legs)),
        ("allocations", Value::List(allocations)),
    ]))
}
fn component_plan(
    c: &Connection,
    scope: &Scope<'_>,
    spec: Value,
    id: &str,
    replacing: Option<&str>,
    sources: &dyn Sources,
) -> Result<Value> {
    let mut spec = spec;
    let mut map = BTreeMap::new();
    let mut legs = cs::list(&spec, "legs").to_vec();
    for (i, leg) in legs.iter_mut().enumerate() {
        let old = py_str(&fallback(field(leg, "id"), &format!("leg:{i}")));
        let new = uuid(&format!("kassiber:custody-plan:{id}:leg:{i}"));
        map.insert(old, new.clone());
        set(leg, "id", s(new));
    }
    let mut allocations = cs::list(&spec, "allocations").to_vec();
    for (i, allocation) in allocations.iter_mut().enumerate() {
        set(
            allocation,
            "id",
            s(uuid(&format!("kassiber:custody-plan:{id}:allocation:{i}"))),
        );
        for endpoint in ["source", "sink"] {
            let key = format!("{endpoint}_leg_id");
            let old = text(allocation, &key);
            let new = map.get(&old).ok_or_else(|| {
                fail(
                    "custody_component_validation",
                    "Custody review plan references an unknown leg",
                )
                .with_details(dict([
                    ("allocation_ordinal", Value::Int(i.into())),
                    ("endpoint", s(endpoint)),
                ]))
            })?;
            set(allocation, &key, s(new));
        }
    }
    let snapshot = cs::snapshot(c, scope)?;
    let checked = cr::validate_plan(
        c,
        scope,
        &snapshot,
        &spec,
        &legs,
        &allocations,
        replacing,
        &BTreeSet::new(),
        sources,
    )?;
    set(&mut spec, "component_id", s(id));
    set(&mut spec, "legs", Value::List(checked.legs));
    set(&mut spec, "allocations", Value::List(checked.allocations));
    set(&mut spec, "validation", checked.validation);
    Ok(spec)
}
const CLASSIFICATIONS: &[&str] = &[
    "external_disposal",
    "external_gift",
    "external_loss",
    "external_payment",
    "retained_custody",
    "suspense_continuation",
];
fn custody(classification: &str) -> &'static str {
    if classification.starts_with("external_") {
        "external_confirmed"
    } else if classification == "retained_custody" {
        "internal_reviewed"
    } else {
        "custody_suspense"
    }
}
fn residual(context: &Value, classification: &str, source: &Value) -> Result<(Value, BigInt)> {
    let old = field(context, "component");
    let mut legs = Vec::new();
    let mut residual = BigInt::from(0);
    for raw in cs::list(old, "legs") {
        let mut leg = project(
            raw,
            &[
                "id",
                "role",
                "rail",
                "chain",
                "network",
                "asset",
                "exposure",
                "conservation_unit",
                "amount_msat",
                "valuation_unit",
                "valuation_amount",
                "occurred_at",
                "transaction_id",
                "anchor_transaction_id",
                "wallet_id",
                "location_ref",
                "notes",
            ],
        );
        if text(&leg, "role") == "suspense" && amount(&leg, "amount_msat")? > BigInt::from(0) {
            residual += amount(&leg, "amount_msat")?;
            set(
                &mut leg,
                "notes",
                s(format!("reviewed_residual:{classification}")),
            );
            if classification.starts_with("external_") {
                set(&mut leg, "role", s("external"));
            }
            if classification == "retained_custody" {
                set(&mut leg, "role", s("retained"));
                set(
                    &mut leg,
                    "location_ref",
                    s(format!(
                        "reviewed-retained-custody:{}",
                        text(field(context, "review"), "gap_id")
                    )),
                );
            }
        }
        legs.push(leg);
    }
    if residual <= BigInt::from(0) {
        return Err(fail(
            "custody_gap_residual_missing",
            "Reviewed bridge has no suspense residual to classify",
        ));
    }
    let allocations = cs::list(old, "allocations")
        .iter()
        .map(|r| {
            project(
                r,
                &[
                    "source_leg_id",
                    "sink_leg_id",
                    "source_amount_msat",
                    "sink_amount_msat",
                ],
            )
        })
        .collect();
    let mut evidence = value(old, "evidence");
    set(
        &mut evidence,
        "residual_classification",
        dict([
            ("schema_version", 1.into()),
            ("classification", s(classification)),
            ("custody_state", s(custody(classification))),
            ("country_tax_meaning", s("not_assigned")),
            ("amount_msat", Value::Int(residual.clone())),
        ]),
    );
    let mut spec = project(
        old,
        &[
            "component_type",
            "conservation_mode",
            "evidence_kind",
            "evidence_grade",
            "conversion_policy",
            "notes",
        ],
    );
    set(&mut spec, "evidence", evidence);
    set(
        &mut spec,
        "conversion_reviewed",
        Value::Bool(truthy(field(old, "conversion_reviewed"))),
    );
    set(
        &mut spec,
        "conversion_metadata",
        value(old, "conversion_metadata"),
    );
    set(&mut spec, "authored_source", source.clone());
    set(&mut spec, "legs", Value::List(legs));
    set(&mut spec, "allocations", Value::List(allocations));
    Ok((spec, residual))
}
fn plan(
    c: &Connection,
    scope: &Scope<'_>,
    args: &Value,
    mut candidate: Value,
    sources: &dyn Sources,
) -> Result<Value> {
    let action = text(args, "action");
    let reason = get(args, "reason").cloned().unwrap_or(Value::Null);
    let source = get(args, "authored_source")
        .cloned()
        .unwrap_or_else(|| s("user"));
    let profile = rows(
        c,
        "SELECT journal_input_version FROM profiles WHERE id=?",
        [scope.profile_id],
    )?;
    let version = profile.first().map(cr::input_version).unwrap_or(0);
    let mut gap = value(args, "gap_id");
    let mut classification = value(args, "classification");
    let mut ctx = Value::Null;
    let mut component = Value::Null;
    let mut residual_msat = BigInt::from(0);
    let mut activatable = true;
    let mut revision = Value::Null;
    let mut authored = Value::Null;
    let mut warn = Value::List(vec![]);
    let base;
    let snap;
    match action.as_str() {
        "create" | "revise" | "dismiss" => {
            let current = super::gap_candidates::find(c, scope, &text(&candidate, "gap_id"))
                .map_err(|_| {
                    fail(
                        "custody_gap_stale",
                        "Custody-gap evidence changed after it was loaded",
                    )
                    .with_hint("Reload the review queue before confirming.")
                })?;
            if fingerprint(&current, false)? != fingerprint(&candidate, false)? {
                return Err(fail(
                    "custody_gap_stale",
                    "Custody-gap evidence changed after it was loaded",
                )
                .with_hint("Reload the review queue before confirming."));
            }
            candidate = current;
            gap = value(&candidate, "gap_id");
            let fp = fingerprint(&candidate, true)?;
            authored = s(&fp);
            warn = warnings(&candidate);
            snap = snapshot(c, scope, &candidate)?;
            if action == "dismiss" {
                base = fp;
            } else if action == "create" {
                base = fp.clone();
                revision = 1.into();
                if truthy(field(&candidate, "excess_msat")) {
                    activatable = false;
                } else {
                    let spec = guided(c, scope, &candidate, &fp, &source)?;
                    let id = text(&spec, "component_id");
                    component = component_plan(c, scope, spec, &id, None, sources)?;
                }
            } else {
                ctx = gs::context(c, scope, &py_str(&gap), false)?;
                base = gs::correction(
                    c,
                    &ctx,
                    "bridge_revised",
                    dict([
                        ("authored_claim_fingerprint", s(&fp)),
                        ("reason", fallback(&reason, "")),
                    ]),
                )?;
                if truthy(field(&candidate, "excess_msat")) {
                    return Err(fail("custody_gap_bridge_excess_return","The return exceeds the source principal").with_hint("Classify the excess origin separately, then preview this exact bridge again.").with_details(dict([("excess_msat",value(&candidate,"excess_msat"))])));
                }
                let old = field(&ctx, "component");
                let lineage = text(old, "lineage_id");
                revision=value(&rows(c,"SELECT COALESCE(MAX(revision),0)+1 AS revision FROM custody_components WHERE profile_id=? AND lineage_id=?",[scope.profile_id,&lineage])?[0],"revision");
                let spec = guided(c, scope, &candidate, &fp, &source)?;
                component = component_plan(
                    c,
                    scope,
                    spec,
                    &uuid(&format!("kassiber:custody-gap-revision:{base}")),
                    Some(&lineage),
                    sources,
                )?;
            }
        }
        "reopen" | "classify_residual" => {
            if !truthy(&gap) {
                return Err(fail("invalid_argument", "Custody-gap id is required"));
            }
            if action == "classify_residual" {
                let normalized = kassiber_pytext::lower(kassiber_pytext::strip(&py_str(
                    &fallback(&classification, ""),
                )))
                .replace('-', "_");
                if !CLASSIFICATIONS.contains(&normalized.as_str()) {
                    return Err(fail(
                        "custody_gap_residual_classification",
                        "Residual classification is unsupported",
                    )
                    .with_details(dict([(
                        "supported",
                        Value::List(CLASSIFICATIONS.iter().map(s).collect()),
                    )])));
                }
                classification = s(normalized);
            }
            ctx = gs::context(c, scope, &py_str(&gap), true)?;
            snap = value(&ctx, "snapshot");
            let mut details = dict([("reason", fallback(&reason, ""))]);
            if action == "classify_residual" {
                set(&mut details, "classification", classification.clone());
            }
            base = gs::correction(
                c,
                &ctx,
                if action == "reopen" {
                    "bridge_reopened"
                } else {
                    "residual_classified"
                },
                details,
            )?;
            if action == "classify_residual" {
                let (spec, r) = residual(&ctx, &py_str(&classification), &source)?;
                residual_msat = r;
                let lineage = text(field(&ctx, "component"), "lineage_id");
                revision=value(&rows(c,"SELECT COALESCE(MAX(revision),0)+1 AS revision FROM custody_components WHERE profile_id=? AND lineage_id=?",[scope.profile_id,&lineage])?[0],"revision");
                component = component_plan(
                    c,
                    scope,
                    spec,
                    &uuid(&format!("kassiber:custody-gap-residual:{base}")),
                    Some(&lineage),
                    sources,
                )?;
            }
        }
        _ => {
            return Err(
                fail("invalid_argument", "Custody review action is unsupported")
                    .with_details(dict([("action", value(args, "action"))])),
            )
        }
    }
    if !matches!(component, Value::Null) {
        activatable = truthy(field(field(&component, "validation"), "activatable"));
    }
    let after = gs::summary(&snap, &action, &py_str(&classification), &residual_msat)?;
    let impacts = gs::preview(c, scope, &snap, &after)?;
    Ok(dict([
        ("schema_version", 1.into()),
        ("action", s(action)),
        ("workspace_id", s(scope.workspace_id)),
        ("profile_id", s(scope.profile_id)),
        ("gap_id", gap),
        ("input_version", version.into()),
        ("base_fingerprint", s(base)),
        ("classification", classification),
        ("reason", fallback(&reason, "")),
        ("review_reason", reason),
        ("authored_source", source),
        ("new_component_revision", revision),
        ("component_plan", component),
        ("filed_report_impacts", impacts),
        ("candidate", candidate),
        ("context", ctx),
        ("authored_claim_fingerprint", authored),
        ("activatable", Value::Bool(activatable)),
        ("warnings", warn),
        ("residual_msat", Value::Int(residual_msat)),
    ]))
}
fn public(plan: &Value) -> Value {
    let component = field(plan, "component_plan");
    let candidate = field(plan, "candidate");
    let action = text(plan, "action");
    let classification = text(plan, "classification");
    let public_component = if matches!(component, Value::Null) {
        Value::Null
    } else {
        let validation = field(component, "validation");
        let mut checked = dict([(
            "activatable",
            Value::Bool(truthy(field(validation, "activatable"))),
        )]);
        for key in ["issues", "warnings"] {
            set(
                &mut checked,
                key,
                Value::List(
                    cs::list(validation, key)
                        .iter()
                        .map(|v| {
                            let mut out = dict::<&str>([]);
                            for k in ["code", "message"] {
                                if !matches!(field(v, k), Value::Null) {
                                    set(&mut out, k, value(v, k));
                                }
                            }
                            out
                        })
                        .collect(),
                ),
            );
        }
        dict([
            ("id", value(component, "component_id")),
            ("component_type", value(component, "component_type")),
            ("conservation_mode", value(component, "conservation_mode")),
            (
                "legs",
                Value::List(
                    cs::list(component, "legs")
                        .iter()
                        .map(|v| {
                            project(
                                v,
                                &[
                                    "id",
                                    "role",
                                    "rail",
                                    "asset",
                                    "exposure",
                                    "conservation_unit",
                                    "amount_msat",
                                    "valuation_unit",
                                    "valuation_amount",
                                ],
                            )
                        })
                        .collect(),
                ),
            ),
            (
                "allocations",
                Value::List(
                    cs::list(component, "allocations")
                        .iter()
                        .map(|v| {
                            project(
                                v,
                                &[
                                    "id",
                                    "source_leg_id",
                                    "sink_leg_id",
                                    "source_amount_msat",
                                    "sink_amount_msat",
                                ],
                            )
                        })
                        .collect(),
                ),
            ),
            ("validation", checked),
        ])
    };
    let has_candidate = !matches!(candidate, Value::Null);
    dict([
        ("action", value(plan, "action")),
        ("gap_id", value(plan, "gap_id")),
        ("input_version", value(plan, "input_version")),
        ("dry_run", Value::Bool(true)),
        ("activatable", value(plan, "activatable")),
        ("requires_explicit_confirmation", Value::Bool(true)),
        ("classification", value(plan, "classification")),
        (
            "country_tax_meaning",
            if action == "classify_residual" {
                s("not_assigned")
            } else {
                Value::Null
            },
        ),
        (
            "custody_state",
            if action == "classify_residual" {
                s(custody(&classification))
            } else {
                Value::Null
            },
        ),
        (
            "new_component_revision",
            value(plan, "new_component_revision"),
        ),
        (
            "current_component_revision",
            value(field(field(plan, "context"), "component"), "revision"),
        ),
        (
            "review_mode",
            if has_candidate {
                s(
                    if truthy(field(candidate, "promotion_eligible"))
                        && n(candidate, "conflict_size") == 1
                    {
                        "structured_candidate"
                    } else {
                        "manual_weak_hint"
                    },
                )
            } else {
                Value::Null
            },
        ),
        (
            "retained_msat",
            if has_candidate {
                value(candidate, "retained_msat")
            } else {
                0.into()
            },
        ),
        (
            "residual_msat",
            if has_candidate {
                value(candidate, "residual_msat")
            } else {
                value(plan, "residual_msat")
            },
        ),
        (
            "fee_msat",
            if has_candidate {
                value(candidate, "source_fee_msat")
            } else {
                0.into()
            },
        ),
        (
            "source_count",
            Value::Int(cs::list(candidate, "source_ids").len().into()),
        ),
        (
            "destination_count",
            Value::Int(cs::list(candidate, "return_ids").len().into()),
        ),
        ("warnings", value(plan, "warnings")),
        ("filed_report_impacts", value(plan, "filed_report_impacts")),
        ("component_plan", public_component),
    ])
}
fn apply(c: &Connection, scope: &Scope<'_>, plan: &Value, sources: &dyn Sources) -> Result<Value> {
    let action = text(plan, "action");
    let candidate = field(plan, "candidate");
    let reason = field(plan, "review_reason");
    let source = text(plan, "authored_source");
    let ctx = field(plan, "context");
    if action == "dismiss" {
        let snap = snapshot(c, scope, candidate)?;
        let r = gs::append(
            c,
            scope,
            plan,
            &Value::Null,
            &snap,
            &fingerprint(candidate, false)?,
            "review_decision",
            "dismissed",
            reason.clone(),
            sources,
        )?;
        cs::invalidate(c, scope.profile_id)?;
        let mut out = project(
            &r,
            &[
                "gap_id",
                "revision",
                "action",
                "event_kind",
                "authored_source",
                "reason",
                "created_at",
            ],
        );
        set(
            &mut out,
            "filed_report_impacts",
            value(plan, "filed_report_impacts"),
        );
        set(
            &mut out,
            "input_version",
            value(
                &rows(
                    c,
                    "SELECT journal_input_version FROM profiles WHERE id=?",
                    [scope.profile_id],
                )?[0],
                "journal_input_version",
            ),
        );
        return Ok(out);
    }
    let old = field(ctx, "component");
    let component = if action == "reopen" {
        let id = text(old, "id");
        cr::supersede(
            c,
            scope,
            &id,
            &fallback(reason, "guided_bridge_reopened"),
            sources,
        )?;
        cr::get_component(c, scope, &id, false)?
    } else {
        if action == "create" && truthy(field(candidate, "excess_msat")) {
            return Err(fail(
                "custody_gap_bridge_excess_return",
                "The return exceeds the source principal",
            )
            .with_hint("Classify the excess origin separately, then preview again.")
            .with_details(dict([("excess_msat", value(candidate, "excess_msat"))])));
        }
        let spec = field(plan, "component_plan");
        let validation = field(spec, "validation");
        if !truthy(field(validation, "activatable")) {
            return Err(fail(
                "custody_component_incomplete",
                "Custody review plan is not activatable",
            )
            .with_details(dict([("validation", validation.clone())])));
        }
        let id = text(spec, "component_id");
        if action == "create" {
            cr::create(c, scope, spec, &source, sources)?;
        } else {
            let mut spec = spec.clone();
            set(&mut spec, "change_reason", fallback(reason, &action));
            if get(&spec, "conversion_reviewed").is_none() {
                set(&mut spec, "conversion_reviewed", Value::Bool(false));
            }
            cr::revise(c, &text(old, "id"), &id, &spec, sources)?;
        }
        cr::activate(c, scope, &id, sources)?;
        cr::get_component(c, scope, &id, false)?
    };
    let (snap, fp, event, status, review_reason) = match action.as_str() {
        "create" | "revise" => (
            snapshot(c, scope, candidate)?,
            text(plan, "authored_claim_fingerprint"),
            if action == "create" {
                "bridge_created"
            } else {
                "bridge_revised"
            },
            "resolved",
            if action == "create" {
                s("guided_custody_bridge")
            } else {
                fallback(reason, "guided_bridge_revised")
            },
        ),
        "reopen" => {
            let mut snap = value(ctx, "snapshot");
            set(&mut snap, "status", s("needs_review"));
            set(
                &mut snap,
                "correction",
                dict([
                    ("strategy", s("create_revision_then_activate")),
                    ("event_kind", s("bridge_reopened")),
                    ("plan_input_version", value(plan, "input_version")),
                    ("correction_fingerprint", value(plan, "base_fingerprint")),
                ]),
            );
            (
                snap,
                text(plan, "base_fingerprint"),
                "bridge_reopened",
                "needs_review",
                fallback(reason, "guided_bridge_reopened"),
            )
        }
        _ => {
            let mut snap = value(ctx, "snapshot");
            set(
                &mut snap,
                "plan_input_version",
                value(plan, "input_version"),
            );
            set(
                &mut snap,
                "correction_fingerprint",
                value(plan, "base_fingerprint"),
            );
            set(
                &mut snap,
                "residual_classification",
                dict([
                    ("classification", value(plan, "classification")),
                    ("custody_state", s(custody(&text(plan, "classification")))),
                    ("country_tax_meaning", s("not_assigned")),
                    ("amount_msat", value(plan, "residual_msat")),
                ]),
            );
            (
                snap,
                text(field(ctx, "review"), "candidate_fingerprint"),
                "residual_classified",
                "resolved",
                fallback(
                    reason,
                    &format!("reviewed_{}", text(plan, "classification")),
                ),
            )
        }
    };
    let r = gs::append(
        c,
        scope,
        plan,
        &component,
        &snap,
        &fp,
        event,
        if action == "reopen" {
            "dismissed"
        } else {
            "resolved"
        },
        review_reason,
        sources,
    )?;
    let impacts = gs::persist_impacts(c, scope, plan, &component, &r, sources)?;
    let mut out = dict([("gap_id", value(plan, "gap_id")), ("status", s(status))]);
    if action == "classify_residual" {
        set(&mut out, "classification", value(plan, "classification"));
        set(
            &mut out,
            "custody_state",
            s(custody(&text(plan, "classification"))),
        );
        set(&mut out, "country_tax_meaning", s("not_assigned"));
        set(&mut out, "residual_msat", value(plan, "residual_msat"));
    }
    for (key, v) in [
        ("component_id", value(&component, "id")),
        ("component_revision", value(&component, "revision")),
        ("review_id", value(&r, "id")),
        ("review_revision", value(&r, "revision")),
    ] {
        set(&mut out, key, v);
    }
    if action == "create" || action == "revise" {
        set(&mut out, "retained_msat", value(candidate, "retained_msat"));
        set(&mut out, "residual_msat", value(candidate, "residual_msat"));
    }
    set(&mut out, "filed_report_impacts", impacts);
    if action == "create" {
        set(&mut out, "fee_msat", value(candidate, "source_fee_msat"));
    }
    set(
        &mut out,
        "input_version",
        value(
            &rows(
                c,
                "SELECT journal_input_version FROM profiles WHERE id=?",
                [scope.profile_id],
            )?[0],
            "journal_input_version",
        ),
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gap_desktop_version_validation_follows_candidate_resolution() {
        let fixture = kassiber_pytext::loads(include_str!("../tests/gaps.json")).unwrap();
        let scenario = &cs::list(&fixture, "scenarios")[0];
        let dir = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(dir.path(), None).unwrap();
        let c = book.connection();
        for sql in cs::list(scenario, "sql") {
            c.execute_batch(&py_str(sql)).unwrap();
        }
        let scope = Scope {
            workspace_id: "w",
            profile_id: "p",
        };
        for version in [Value::Null, Value::Bool(true), (-1).into(), s("0")] {
            let mut args = value(&cs::list(scenario, "cases")[0], "args");
            set(&mut args, "desktop_kind", s("ui.custody.review.apply"));
            set(&mut args, "expected_input_version", version);
            let error =
                execute(c, &scope, "transfers.gaps.apply", &args, &SystemSources).unwrap_err();
            assert_eq!(error.code, "validation");
            assert_eq!(
                error.message,
                "ui.custody.review.apply requires a non-negative expected_input_version"
            );
            set(&mut args, "gap_id", s("absent"));
            assert_eq!(
                execute(c, &scope, "transfers.gaps.apply", &args, &SystemSources)
                    .unwrap_err()
                    .code,
                "custody_gap_not_found"
            );
        }
    }

    #[test]
    fn gap_plan_rechecks_loaded_candidate_before_authoring() {
        let fixture = kassiber_pytext::loads(include_str!("../tests/gaps.json")).unwrap();
        let scenario = &cs::list(&fixture, "scenarios")[0];
        let dir = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(dir.path(), None).unwrap();
        let c = book.connection();
        for sql in cs::list(scenario, "sql") {
            c.execute_batch(&py_str(sql)).unwrap();
        }
        let scope = Scope {
            workspace_id: "w",
            profile_id: "p",
        };
        let args = field(&cs::list(scenario, "cases")[0], "args");
        let candidate =
            super::super::gap_candidates::find(c, &scope, &text(args, "gap_id")).unwrap();
        c.execute("UPDATE transactions SET amount=1001 WHERE id='out'", [])
            .unwrap();
        let error = plan(c, &scope, args, candidate, &SystemSources).unwrap_err();
        assert_eq!(error.code, "custody_gap_stale");
        assert_eq!(
            error.message,
            "Custody-gap evidence changed after it was loaded"
        );
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM custody_gap_reviews", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
