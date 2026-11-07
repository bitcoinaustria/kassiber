//! Version-bound component plan/apply: create, revise, undo, activate, supersede.
//! A plan is a pure read; apply rebuilds it inside one savepoint.
use super::component_rows::{self as rows_, fail, int, internal, s};
use super::component_store as store;
use super::*;
use bitcoin_hashes::{sha1, Hash};
use kassiber_custody::components::review;
use kassiber_envelope::remediation::{build_remediation, Remediation};
use kassiber_envelope::{set, truthy};
use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_store::rusqlite::params;
use sql::{atomic, error};
use std::collections::BTreeSet;

const MAX_COMPONENTS: usize = 50;
const MAX_LEGS_PER_COMPONENT: usize = 256;
const MAX_ALLOCATIONS_PER_COMPONENT: usize = 4096;
const MAX_TOTAL_LEGS: usize = 2000;
const MAX_TOTAL_ALLOCATIONS: usize = 10_000;
const ACTIONS: &[&str] = &["create", "revise", "undo", "activate", "supersede"];
const LIGHTNING_KINDS: &[&str] = &["cln", "coreln", "lightning", "lnd", "nwc", "phoenix"];
const COMPONENT_FIELDS: &[&str] = &[
    "component_type",
    "conservation_mode",
    "evidence_kind",
    "evidence_grade",
    "evidence",
    "conversion_policy",
    "conversion_reviewed",
    "conversion_metadata",
    "notes",
    "change_reason",
    "component_id",
    "lineage_id",
    "created_at",
];
const REVISION_FIELDS: &[&str] = &[
    "allocations",
    "change_reason",
    "component_type",
    "conservation_mode",
    "conversion_metadata",
    "conversion_policy",
    "conversion_reviewed",
    "evidence",
    "evidence_grade",
    "evidence_kind",
    "legs",
    "notes",
];

/// The planner's `_error`: details are omitted when empty.
fn refuse(message: impl Into<String>, code: &str, details: Vec<(&str, Value)>) -> AppError {
    let error = AppError::new(code, message);
    if details.is_empty() {
        error
    } else {
        error.with_details(dict(details))
    }
}
const COMPONENT_PLAN_TOOL: &str = "ui_transfers_components_plan";
/// `**_issue_remediation(issues)`: append the issues' remediation, if any.
fn with_remediation(mut details: Vec<(&str, Value)>) -> Vec<(&str, Value)> {
    let found = details
        .iter()
        .find(|(key, _)| *key == "issues")
        .and_then(|(_, issues)| rows_::issues_remediation(issues));
    if let Some(remediation) = found {
        details.push(("remediation", remediation));
    }
    details
}
/// A lineage holds one draft at a time: revise that draft, or supersede it.
fn existing_draft_remediation(draft: &str) -> Value {
    build_remediation(
        "revise_existing_draft",
        Remediation {
            target: Some(dict([("component_ids", Value::List(vec![s(draft)]))])),
            tool: Some(COMPONENT_PLAN_TOOL),
            arguments_hint: Some(dict([("action", s("revise")), ("component_id", s(draft))])),
            message: Some("A lineage holds one draft at a time. Revise or activate that draft, or supersede it before revising this revision."),
            alternatives: vec![dict([
                ("action", s("supersede")),
                ("tool", s(COMPONENT_PLAN_TOOL)),
                (
                    "arguments_hint",
                    dict([("action", s("supersede")), ("component_id", s(draft))]),
                ),
            ])],
            ..Remediation::default()
        },
    )
}
fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    get(v, key).unwrap_or(&Value::Null)
}
fn present(v: &Value, key: &str) -> bool {
    kassiber_envelope::contains_key(v, key)
}
pub(super) fn uuid5(name: &str) -> String {
    let mut bytes = vec![
        0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30,
        0xc8,
    ];
    bytes.extend(name.as_bytes());
    let mut h = sha1::Hash::hash(&bytes).to_byte_array();
    h[6] = (h[6] & 15) | 80;
    h[8] = (h[8] & 63) | 128;
    let hex: String = h[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}
fn utf8(v: &str, value: &Value) -> Result<String> {
    if let Value::Str(text) = value {
        if text.has_surrogates() {
            return Err(internal(
                "'utf-8' codec can't encode character: surrogates not allowed",
            ));
        }
    }
    Ok(v.to_owned())
}
fn deterministic(component: &str, kind: &str, ordinal: usize) -> String {
    uuid5(&format!(
        "kassiber:custody-component:{component}:{kind}:{ordinal}"
    ))
}
fn canonical(spec: &Value) -> Result<String> {
    kassiber_pyjson::dumps(
        spec,
        &kassiber_pyjson::Options {
            sort_keys: true,
            ..kassiber_pyjson::Options::default().compact()
        },
    )
    .map_err(|e| internal(e.to_string()))
}
/// `str(value or "")`.
fn or_text(v: &Value) -> String {
    if truthy(v) {
        py_str(v)
    } else {
        String::new()
    }
}
/// `str(reference or "").strip()`.
fn reference(v: &Value) -> String {
    kassiber_pytext::strip(&or_text(v)).to_owned()
}
fn pop(leg: &mut Value, key: &str) -> Value {
    let Value::Dict(pairs) = leg else {
        return Value::Null;
    };
    match pairs.iter().rposition(
        |(k, _)| matches!(k, kassiber_pyjson::Key::Str(name) if name.as_str() == Some(key)),
    ) {
        Some(at) => pairs.remove(at).1,
        None => Value::Null,
    }
}
fn profile_row(c: &Connection, scope: &Scope<'_>) -> Result<Value> {
    rows(
        c,
        "SELECT * FROM profiles WHERE id=? AND workspace_id=?",
        [scope.profile_id, scope.workspace_id],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| {
        refuse(
            "Custody component profile was not found",
            "not_found",
            vec![],
        )
    })
}
fn resolve_transaction(c: &Connection, scope: &Scope<'_>, raw: &Value) -> Result<Value> {
    kassiber_transactions::lookup::find_transaction(c, scope.profile_id, raw, None)?.ok_or_else(
        || {
            refuse(
                format!("Transaction '{}' was not found", reference(raw)),
                "not_found",
                vec![],
            )
        },
    )
}
fn resolve_wallet(c: &Connection, scope: &Scope<'_>, raw: &Value) -> Result<Value> {
    let text = reference(raw);
    if let Some(row) = rows(
        c,
        "SELECT * FROM wallets WHERE profile_id=? AND id=?",
        [scope.profile_id, &text],
    )?
    .into_iter()
    .next()
    {
        return Ok(row);
    }
    let found = rows(
        c,
        "SELECT * FROM wallets WHERE profile_id=? AND lower(label)=lower(?) ORDER BY label,id LIMIT 2",
        [scope.profile_id, &text],
    )?;
    if found.len() > 1 {
        return Err(refuse(
            format!("Wallet label '{text}' is ambiguous"),
            "validation",
            vec![],
        ));
    }
    found.into_iter().next().ok_or_else(|| {
        refuse(
            format!("Wallet '{text}' was not found"),
            "not_found",
            vec![],
        )
    })
}
/// `btc_to_msat(Decimal(str(value)))` with CPython's decimal signals.
fn btc_to_msat(v: &Value) -> Result<Value> {
    let raw = py_str(v);
    let text = kassiber_pytext::strip(&raw);
    let lower = text.to_ascii_lowercase();
    let unsigned = lower.trim_start_matches(['+', '-']);
    if unsigned.starts_with("snan") && unsigned[4..].bytes().all(|b| b.is_ascii_digit()) {
        return Err(internal("[<class 'decimal.InvalidOperation'>]"));
    }
    if unsigned.starts_with("nan") && unsigned[3..].bytes().all(|b| b.is_ascii_digit()) {
        return Err(internal("cannot convert NaN to integer"));
    }
    if matches!(unsigned, "inf" | "infinity") {
        return Err(internal("cannot convert Infinity to integer"));
    }
    let decimal = text
        .parse::<Decimal>()
        .map_err(|_| internal("[<class 'decimal.ConversionSyntax'>]"))?;
    let scaled = decimal
        .mul(
            &"100000000000".parse().unwrap(),
            &Context::new(28, Rounding::HalfEven),
        )
        .map_err(|_| internal("[<class 'decimal.Overflow'>]"))?;
    let rounded = scaled
        .to_fixed_places(0, Rounding::HalfUp)
        .map_err(|_| internal("decimal conversion failed"))?;
    Ok(Value::Int(
        rounded
            .parse()
            .map_err(|_| internal("decimal conversion failed"))?,
    ))
}

type Planned = Vec<(String, Value)>;
fn planned_ids(planned: &Planned) -> BTreeSet<String> {
    planned.iter().map(|(id, _)| id.clone()).collect()
}

/// `_prepare_legs`: resolve references and fill rail/exposure/unit defaults.
fn prepare_legs(
    c: &Connection,
    scope: &Scope<'_>,
    component: &str,
    raw: &Value,
    planned: &mut Planned,
) -> Result<Vec<Value>> {
    let Value::List(raw) = raw else {
        return Err(refuse(
            "Custody component legs must be a JSON array",
            "validation",
            vec![],
        ));
    };
    let mut prepared = Vec::new();
    for (ordinal, raw) in raw.iter().enumerate() {
        if !matches!(raw, Value::Dict(_)) {
            return Err(refuse(
                format!("Custody component leg {ordinal} must be an object"),
                "validation",
                vec![],
            ));
        }
        let mut leg = raw.clone();
        if !present(&leg, "id") {
            set(&mut leg, "id", s(deterministic(component, "leg", ordinal)));
        }
        let first = pop(&mut leg, "transaction");
        let transaction = if truthy(&first) {
            first
        } else {
            pop(&mut leg, "transaction_ref")
        };
        let untracked = pop(&mut leg, "untracked_wallet");
        let first = pop(&mut leg, "wallet");
        let wallet_ref = if truthy(&first) {
            first
        } else {
            pop(&mut leg, "wallet_ref")
        };
        let has_tx = !matches!(transaction, Value::Null);
        let has_untracked = !matches!(untracked, Value::Null);
        if has_tx && has_untracked {
            return Err(refuse(
                format!("Custody component leg {ordinal} cannot combine transaction with untracked_wallet"),
                "validation",
                vec![],
            ));
        }
        if has_untracked && (!matches!(wallet_ref, Value::Null) || truthy(field(&leg, "wallet_id")))
        {
            return Err(refuse(
                format!(
                    "Custody component leg {ordinal} cannot combine untracked_wallet with wallet"
                ),
                "validation",
                vec![],
            ));
        }
        let mut wallet: Option<Value> = None;
        if has_tx {
            let tx = resolve_transaction(c, scope, &transaction)?;
            set(&mut leg, "transaction_id", value(&tx, "id"));
            for (key, source) in [
                ("wallet_id", "wallet_id"),
                ("asset", "asset"),
                ("occurred_at", "occurred_at"),
            ] {
                if !present(&leg, key) {
                    set(&mut leg, key, value(&tx, source));
                }
            }
        }
        if has_untracked {
            if let Value::Str(label) = &untracked {
                super::component_reads::reference_bind(
                    &Value::Str(kassiber_pytext::strip_py(label)),
                    1,
                )?;
            }
            let label = match &untracked {
                Value::Str(_) if !kassiber_pytext::strip(&py_str(&untracked)).is_empty() => {
                    kassiber_pytext::strip(&py_str(&untracked)).to_owned()
                }
                _ => {
                    return Err(refuse(
                        format!("Custody component leg {ordinal} untracked_wallet must be a label"),
                        "validation",
                        vec![],
                    ))
                }
            };
            match resolve_wallet(c, scope, &s(&label)) {
                Err(e) if e.code == "not_found" => {
                    let id = uuid5(&utf8(
                        &format!(
                            "kassiber:planned-untracked-wallet:{}:{}",
                            scope.profile_id,
                            kassiber_pytext::casefold(&label)
                        ),
                        &untracked,
                    )?);
                    if !planned.iter().any(|(known, _)| *known == id) {
                        planned.push((
                            id.clone(),
                            dict([
                                ("id", s(&id)),
                                ("label", s(&label)),
                                ("kind", s("untracked")),
                            ]),
                        ));
                    }
                    set(&mut leg, "wallet_id", s(id));
                }
                Err(e) => return Err(e),
                Ok(found) => {
                    if kassiber_pytext::lower(&or_text(&value(&found, "kind"))) != "untracked" {
                        return Err(refuse(
                            format!("Wallet '{label}' exists but is not an untracked placeholder"),
                            "conflict",
                            vec![],
                        ));
                    }
                    set(&mut leg, "wallet_id", value(&found, "id"));
                    wallet = Some(found);
                }
            }
        }
        if !matches!(wallet_ref, Value::Null) {
            let found = resolve_wallet(c, scope, &wallet_ref)?;
            set(&mut leg, "wallet_id", value(&found, "id"));
            wallet = Some(found);
        }
        if present(&leg, "amount_btc") {
            if present(&leg, "amount_msat") {
                return Err(refuse(
                    format!("Custody component leg {ordinal} cannot set both amount_btc and amount_msat"),
                    "validation",
                    vec![],
                ));
            }
            let amount = pop(&mut leg, "amount_btc");
            set(&mut leg, "amount_msat", btc_to_msat(&amount)?);
        }
        let asset_input = field(&leg, "asset");
        let asset = if truthy(asset_input) {
            super::pair_spec::asset(asset_input)
        } else {
            "BTC".to_owned()
        };
        set(&mut leg, "asset", s(&asset));
        let wallet_id = field(&leg, "wallet_id").clone();
        if wallet.is_none()
            && !matches!(&wallet_id, Value::Null)
            && wallet_id != s("")
            && !planned.iter().any(|(id, _)| *id == py_str(&wallet_id))
        {
            wallet = Some(resolve_wallet(c, scope, &s(py_str(&wallet_id)))?);
        }
        let is_planned = planned.iter().any(|(id, _)| *id == or_text(&wallet_id));
        let kind = if is_planned {
            "untracked".to_owned()
        } else {
            kassiber_pytext::lower(&match &wallet {
                Some(w) => py_str(&value(w, "kind")),
                None => String::new(),
            })
        };
        if !truthy(field(&leg, "rail")) {
            let rail = if kind == "untracked" {
                "untracked"
            } else if LIGHTNING_KINDS.contains(&kind.as_str()) {
                "lightning"
            } else if asset == "LBTC" {
                "liquid"
            } else {
                "bitcoin"
            };
            set(&mut leg, "rail", s(rail));
        }
        let native = asset == "BTC" || asset == "LBTC";
        if !truthy(field(&leg, "exposure")) {
            let exposure = if native {
                "bitcoin".to_owned()
            } else {
                kassiber_pytext::lower(&asset)
            };
            set(&mut leg, "exposure", s(exposure));
        }
        if !truthy(field(&leg, "conservation_unit")) {
            set(
                &mut leg,
                "conservation_unit",
                s(if native { "msat" } else { "asset-quantum" }),
            );
        }
        prepared.push(leg);
    }
    Ok(prepared)
}
/// `_prepare_allocations`.
fn prepare_allocations(component: &str, raw: &Value) -> Result<Vec<Value>> {
    let items = match raw {
        Value::Null => return Ok(Vec::new()),
        Value::List(items) => items,
        _ => {
            return Err(refuse(
                "Custody component allocations must be a JSON array",
                "validation",
                vec![],
            ))
        }
    };
    let mut prepared = Vec::new();
    for (ordinal, raw) in items.iter().enumerate() {
        if !matches!(raw, Value::Dict(_)) {
            return Err(refuse(
                format!("Custody component allocation {ordinal} must be an object"),
                "validation",
                vec![],
            ));
        }
        let mut allocation = raw.clone();
        if !truthy(field(raw, "id")) {
            set(
                &mut allocation,
                "id",
                s(deterministic(component, "allocation", ordinal)),
            );
        }
        prepared.push(allocation);
    }
    Ok(prepared)
}

struct Arguments {
    action: String,
    components: Value,
    component_id: String,
    spec: Value,
    activate: Option<bool>,
    reason: Value,
}
/// `_component_review_arguments`.
fn arguments(args: &Value) -> Result<Arguments> {
    let action = field(args, "action");
    if !matches!(action, Value::Str(_)) {
        return Err(refuse(
            "Custody component review action is unsupported",
            "validation",
            vec![],
        ));
    }
    let name = match action {
        Value::Str(text) => text.as_str().filter(|a| ACTIONS.contains(a)),
        _ => None,
    };
    let Some(name) = name else {
        return Err(refuse(
            "Custody component review action is unsupported",
            "validation",
            vec![("action", action.clone())],
        ));
    };
    let reason = field(args, "reason").clone();
    if !matches!(reason, Value::Null | Value::Str(_)) {
        return Err(refuse(
            "Custody component review reason must be text",
            "validation",
            vec![],
        ));
    }
    let activate = match field(args, "activate") {
        Value::Null => None,
        Value::Bool(b) => Some(*b),
        _ => {
            return Err(refuse(
                "Custody component review activate must be a boolean",
                "validation",
                vec![],
            ))
        }
    };
    let components = field(args, "components").clone();
    let component_id = field(args, "component_id").clone();
    let spec = field(args, "spec").clone();
    if name == "create" {
        let fields: Vec<_> = [
            ("component_id", &component_id),
            ("spec", &spec),
            ("reason", &reason),
        ]
        .into_iter()
        .filter(|(_, v)| !matches!(v, Value::Null))
        .map(|(k, _)| s(k))
        .collect();
        if !fields.is_empty() {
            return Err(refuse(
                "Create accepts components and activate only",
                "validation",
                vec![("fields", Value::List(fields))],
            ));
        }
        return Ok(Arguments {
            action: name.into(),
            components,
            component_id: String::new(),
            spec,
            activate: Some(activate.unwrap_or(true)),
            reason,
        });
    }
    if !matches!(components, Value::Null) {
        return Err(refuse(
            format!("{name} does not accept components"),
            "validation",
            vec![],
        ));
    }
    let id = match &component_id {
        Value::Str(text) if !kassiber_pytext::strip(&py_str(&component_id)).is_empty() => {
            let _ = text;
            kassiber_pytext::strip(&py_str(&component_id)).to_owned()
        }
        _ => {
            return Err(refuse(
                format!("{name} requires component_id"),
                "validation",
                vec![],
            ))
        }
    };
    let mut result = Arguments {
        action: name.into(),
        components,
        component_id: id,
        spec: spec.clone(),
        activate,
        reason: reason.clone(),
    };
    match name {
        "revise" => {
            if !matches!(spec, Value::Dict(_)) {
                return Err(refuse(
                    "Revise requires a component spec",
                    "validation",
                    vec![],
                ));
            }
            result.activate = Some(activate.unwrap_or(false));
        }
        "undo" => {
            if !matches!(spec, Value::Null) || activate == Some(true) {
                return Err(refuse(
                    "Undo does not accept spec or activation",
                    "validation",
                    vec![],
                ));
            }
            result.activate = Some(false);
            result.reason = if truthy(&reason) { reason } else { s("undo") };
        }
        _ => {
            if !matches!(spec, Value::Null) || activate.is_some() {
                return Err(refuse(
                    format!("{name} does not accept spec or activate"),
                    "validation",
                    vec![],
                ));
            }
            if name == "activate" && !matches!(reason, Value::Null) {
                return Err(refuse(
                    "Activate does not accept reason",
                    "validation",
                    vec![],
                ));
            }
        }
    }
    Ok(result)
}

fn summary(components: &[Value]) -> Value {
    let active = components
        .iter()
        .filter(|c| field(c, "effective_state") == &s("active"))
        .count();
    dict([
        ("count", int(components.len())),
        ("active", int(active)),
        ("draft", int(components.len() - active)),
    ])
}
fn issues(validation: &Value) -> Value {
    field(validation, "issues").clone()
}
fn activatable(validation: &Value) -> bool {
    truthy(field(validation, "activatable"))
}
fn without(v: &Value, drop: &[&str]) -> Value {
    match v {
        Value::Dict(pairs) => Value::Dict(
            pairs
                .iter()
                .filter(|(k, _)| {
                    !matches!(k, kassiber_pyjson::Key::Str(name) if name.as_str().is_some_and(|n| drop.contains(&n)))
                })
                .cloned()
                .collect(),
        ),
        other => other.clone(),
    }
}

struct Batch {
    input_version: i64,
    planned: Planned,
    prepared: Vec<(bool, Value, Value)>,
    activate: bool,
}
fn plan_batch(
    c: &Connection,
    scope: &Scope<'_>,
    specs: &Value,
    activate: bool,
    source: &str,
    sources: &dyn Sources,
) -> Result<Batch> {
    let specs = match specs {
        Value::List(items) if !items.is_empty() => items,
        _ => {
            return Err(refuse(
                "Bulk custody resolution requires a non-empty components array",
                "validation",
                vec![],
            ))
        }
    };
    if specs.len() > MAX_COMPONENTS {
        return Err(refuse(
            format!("Bulk custody resolution accepts at most {MAX_COMPONENTS} components"),
            "validation",
            vec![
                ("count", int(specs.len())),
                ("max_components", int(MAX_COMPONENTS)),
            ],
        ));
    }
    let profile = profile_row(c, scope)?;
    let snapshot = store::snapshot(c, scope)?;
    let mut planned: Planned = Vec::new();
    let mut prepared = Vec::new();
    let (mut total_legs, mut total_allocations) = (0, 0);
    for (index, spec) in specs.iter().enumerate() {
        if !matches!(spec, Value::Dict(_)) {
            return Err(refuse(
                "Custody component spec must be a JSON object",
                "validation",
                vec![],
            ));
        }
        let canonical = canonical(spec).map_err(|_| {
            refuse(
                "Custody component spec is not JSON-safe",
                "validation",
                vec![],
            )
        })?;
        let requested = reference(field(spec, "component_id"));
        let id = if requested.is_empty() {
            uuid5(&utf8(
                &format!(
                    "kassiber:custody-bulk:{}:{source}:{}:{index}:{canonical}",
                    scope.profile_id,
                    u8::from(activate)
                ),
                &Value::Null,
            )?)
        } else {
            requested.clone()
        };
        if !rows(
            c,
            "SELECT 1 FROM custody_components WHERE id=? AND profile_id=?",
            [&id, scope.profile_id],
        )?
        .is_empty()
        {
            if !requested.is_empty() {
                return Err(refuse(
                    "Custody component id already exists",
                    "conflict",
                    vec![("component_id", s(&id))],
                ));
            }
            let component = rows_::get_component(c, scope, &id, true)?;
            prepared.push((true, Value::Null, component));
            continue;
        }
        let legs = prepare_legs(c, scope, &id, field(spec, "legs"), &mut planned)?;
        let allocations = prepare_allocations(&id, field(spec, "allocations"))?;
        total_legs += legs.len();
        total_allocations += allocations.len();
        if legs.len() > MAX_LEGS_PER_COMPONENT {
            return Err(refuse(
                format!("Custody component accepts at most {MAX_LEGS_PER_COMPONENT} legs"),
                "validation",
                vec![
                    ("count", int(legs.len())),
                    ("max_legs", int(MAX_LEGS_PER_COMPONENT)),
                ],
            ));
        }
        if allocations.len() > MAX_ALLOCATIONS_PER_COMPONENT {
            return Err(refuse(
                "Custody component contains too many allocations",
                "validation",
                vec![
                    ("count", int(allocations.len())),
                    ("max_allocations", int(MAX_ALLOCATIONS_PER_COMPONENT)),
                ],
            ));
        }
        let mut input = Vec::new();
        for key in COMPONENT_FIELDS {
            if matches!(
                *key,
                "component_type" | "conservation_mode" | "component_id"
            ) {
                continue;
            }
            if let Some(v) = get(spec, key) {
                input.push((*key, v.clone()));
            }
        }
        let or = |key: &str, default: &str| {
            let v = field(spec, key);
            if truthy(v) {
                v.clone()
            } else {
                s(default)
            }
        };
        input.push(("component_type", or("component_type", "manual_bridge")));
        input.push(("conservation_mode", or("conservation_mode", "quantity")));
        input.push(("component_id", s(&id)));
        input.push(("authored_source", s(source)));
        let head = rows_::header(&dict(input))?;
        let check = rows_::validate_plan(
            c,
            scope,
            &snapshot,
            &head,
            &legs,
            &allocations,
            None,
            &planned_ids(&planned),
            sources,
        )?;
        if activate && !activatable(&check.validation) {
            return Err(refuse(
                "Custody component plan cannot activate",
                "custody_component_not_activatable",
                with_remediation(vec![
                    ("component_id", s(&id)),
                    ("issues", issues(&check.validation)),
                ]),
            ));
        }
        let mut spec_out = without(&head, &["authored_source"]);
        set(&mut spec_out, "legs", Value::List(check.legs.clone()));
        set(
            &mut spec_out,
            "allocations",
            Value::List(check.allocations.clone()),
        );
        let state = s(if activate { "active" } else { "draft" });
        let component = dict([
            ("id", s(&id)),
            ("lineage_id", field(&head, "lineage_id").clone()),
            ("workspace_id", s(scope.workspace_id)),
            ("profile_id", s(scope.profile_id)),
            ("revision", int(1)),
            ("component_type", field(&head, "component_type").clone()),
            (
                "conservation_mode",
                field(&head, "conservation_mode").clone(),
            ),
            ("state", state.clone()),
            ("effective_state", state),
            ("evidence_kind", field(&head, "evidence_kind").clone()),
            ("evidence_grade", field(&head, "evidence_grade").clone()),
            (
                "conversion_policy",
                field(&head, "conversion_policy").clone(),
            ),
            (
                "conversion_reviewed",
                field(&head, "conversion_reviewed").clone(),
            ),
            ("expected_leg_count", int(check.legs.len())),
            ("expected_allocation_count", int(check.allocations.len())),
            ("authored_source", s(source)),
            ("notes", field(&head, "notes").clone()),
            ("change_reason", field(&head, "change_reason").clone()),
            ("activated_at", Value::Null),
            ("superseded_at", Value::Null),
            ("created_at", field(&head, "created_at").clone()),
            ("legs", Value::List(check.legs)),
            ("allocations", Value::List(check.allocations)),
            ("validation", check.validation),
        ]);
        prepared.push((false, spec_out, component));
    }
    if total_legs > MAX_TOTAL_LEGS {
        return Err(refuse(
            "Bulk custody resolution contains too many legs",
            "validation",
            vec![
                ("count", int(total_legs)),
                ("max_legs", int(MAX_TOTAL_LEGS)),
            ],
        ));
    }
    if total_allocations > MAX_TOTAL_ALLOCATIONS {
        return Err(refuse(
            "Bulk custody resolution contains too many allocations",
            "validation",
            vec![
                ("count", int(total_allocations)),
                ("max_allocations", int(MAX_TOTAL_ALLOCATIONS)),
            ],
        ));
    }
    if activate {
        let candidates: Vec<&Value> = prepared
            .iter()
            .filter(|(existing, _, _)| !existing)
            .map(|(_, _, component)| component)
            .collect();
        let batch = batch_issues(c, scope, snapshot, &candidates, &planned)?;
        if !batch.is_empty() {
            return Err(refuse(
                "Custody component batch cannot activate",
                "custody_component_not_activatable",
                with_remediation(vec![("issues", Value::List(batch))]),
            ));
        }
    }
    Ok(Batch {
        input_version: rows_::input_version(&profile),
        planned,
        prepared,
        activate,
    })
}
/// `validate_planned_active_batch`.
fn batch_issues(
    c: &Connection,
    scope: &Scope<'_>,
    mut snapshot: kassiber_custody::snapshot::Snapshot,
    components: &[&Value],
    planned: &Planned,
) -> Result<Vec<Value>> {
    if components.is_empty() {
        return Ok(Vec::new());
    }
    let mut issues = Vec::new();
    let mut by_transaction: std::collections::BTreeMap<String, BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for component in components {
        let id = py_str(field(component, "id"));
        for leg in rows_::list(component, "legs") {
            let anchor = field(leg, "anchor_transaction_id");
            let tx = if truthy(anchor) {
                anchor
            } else {
                field(leg, "transaction_id")
            };
            if !matches!(tx, Value::Null) && *tx != s("") {
                by_transaction
                    .entry(py_str(tx))
                    .or_default()
                    .insert(id.clone());
            }
        }
    }
    for (tx, ids) in by_transaction {
        if ids.len() > 1 {
            issues.push(dict([
                ("code", s("active_transaction_membership_conflict")),
                (
                    "message",
                    s("a transaction belongs to multiple planned active components"),
                ),
                ("transaction_id", s(&tx)),
                ("component_ids", Value::List(ids.iter().map(s).collect())),
                // These are ids of the same unsaved batch, so the fix is in
                // the submitted document, not a stored component.
                (
                    "remediation",
                    build_remediation(
                        "merge_planned_components",
                        Remediation {
                            target: Some(dict([
                                ("transaction_ids", Value::List(vec![s(&tx)])),
                                (
                                    "planned_component_ids",
                                    Value::List(ids.iter().map(s).collect()),
                                ),
                            ])),
                            tool: Some(COMPONENT_PLAN_TOOL),
                            message: Some("One transaction is anchored by several components of this batch. Put all of its legs in one component and plan the batch again."),
                            ..Remediation::default()
                        },
                    ),
                ),
            ]));
        }
    }
    let quantity: Vec<&&Value> = components
        .iter()
        .filter(|c| field(c, "conservation_mode") == &s("quantity"))
        .collect();
    if quantity.is_empty() {
        return Ok(issues);
    }
    let candidates: BTreeSet<String> = quantity.iter().map(|c| py_str(field(c, "id"))).collect();
    let to_json = |v: &Value| rows_::to_json(v);
    let mut legs: Vec<serde_json::Value> = rows(
        c,
        "SELECT l.* FROM custody_component_legs l JOIN custody_components c ON c.id=l.component_id WHERE c.profile_id=? AND c.state='active' AND c.conservation_mode='quantity' ORDER BY c.id,l.ordinal,l.id",
        [scope.profile_id],
    )?
    .iter()
    .map(to_json)
    .collect();
    let mut allocations: Vec<serde_json::Value> = rows(
        c,
        "SELECT a.* FROM custody_component_allocations a JOIN custody_components c ON c.id=a.component_id WHERE c.profile_id=? AND c.state='active' AND c.conservation_mode='quantity' ORDER BY c.id,a.ordinal,a.id",
        [scope.profile_id],
    )?
    .iter()
    .map(to_json)
    .collect();
    for component in &quantity {
        let id = field(component, "id").clone();
        for (key, out) in [("legs", &mut legs), ("allocations", &mut allocations)] {
            for item in rows_::list(component, key) {
                let mut item = item.clone();
                set(&mut item, "component_id", id.clone());
                out.push(to_json(&item));
            }
        }
    }
    for (id, wallet) in planned {
        if !snapshot
            .anchor_wallets
            .iter()
            .any(|w| w.get("id").and_then(|v| v.as_str()) == Some(id))
        {
            let _ = wallet;
            let serde_json::Value::Object(record) = serde_json::json!({
                "id": id, "kind": "untracked", "wallet_kind": "untracked", "config_json": "{}"
            }) else {
                unreachable!()
            };
            snapshot.anchor_wallets.push(record.into());
        }
    }
    let routes = review::planned_batch_routes(&snapshot, &legs, &allocations, &candidates)
        .map_err(|e| internal(e.message))?;
    for issue in routes {
        issues.push(rows_::project_validation(&issue)?);
    }
    Ok(issues)
}

fn create_wallets(
    c: &Connection,
    scope: &Scope<'_>,
    planned: &Planned,
    sources: &dyn Sources,
) -> Result<()> {
    for (id, wallet) in planned {
        let account = resolve_account(c, scope)?;
        let label = field(wallet, "label").clone();
        let now = sources.now(c)?;
        let inserted = c.execute(
            "INSERT INTO wallets(id,workspace_id,profile_id,account_id,label,kind,config_json,created_at) VALUES(?,?,?,?,?,?,?,?)",
            params![id, scope.workspace_id, scope.profile_id, account, py_str(&label), "untracked", "{}", now],
        );
        if let Err(e) = inserted {
            if matches!(&e, kassiber_store::rusqlite::Error::SqliteFailure(f, _) if f.code == kassiber_store::rusqlite::ErrorCode::ConstraintViolation)
            {
                let profile = rows(
                    c,
                    "SELECT label FROM profiles WHERE id=?",
                    [scope.profile_id],
                )?;
                return Err(AppError::new(
                    "conflict",
                    format!(
                        "Wallet '{}' already exists in profile '{}'",
                        py_str(&label),
                        profile
                            .first()
                            .map(|p| text(p, "label"))
                            .unwrap_or_default()
                    ),
                )
                .with_hint("Choose a different wallet label or update the existing wallet."));
            }
            return Err(error(e));
        }
        store::invalidate(c, scope.profile_id)?;
    }
    Ok(())
}
/// `resolve_account(conn, profile_id, "treasury")`.
fn resolve_account(c: &Connection, scope: &Scope<'_>) -> Result<String> {
    for sql in [
        "SELECT * FROM accounts WHERE profile_id=? AND id=? LIMIT 1",
        "SELECT * FROM accounts WHERE profile_id=? AND lower(code)=lower(?) LIMIT 1",
    ] {
        if let Some(row) = rows(c, sql, [scope.profile_id, "treasury"])?.first() {
            return Ok(text(row, "id"));
        }
    }
    let found = rows(
        c,
        "SELECT * FROM accounts WHERE profile_id=? AND lower(label)=lower(?) ORDER BY code ASC",
        [scope.profile_id, "treasury"],
    )?;
    match found.len() {
        1 => Ok(text(&found[0], "id")),
        0 => Err(AppError::new("not_found", "Account 'treasury' not found")),
        _ => Err(
            AppError::new("validation", "Account label 'treasury' is ambiguous")
                .with_hint("Use the account code or id instead of the non-unique label.")
                .with_details(dict([(
                    "matches",
                    Value::List(
                        found
                            .iter()
                            .map(|r| {
                                dict([
                                    ("id", value(r, "id")),
                                    ("code", value(r, "code")),
                                    ("label", value(r, "label")),
                                ])
                            })
                            .collect(),
                    ),
                )])),
        ),
    }
}

fn public_batch(batch: &Batch, scope: &Scope<'_>, source: &str) -> Value {
    let components: Vec<Value> = batch.prepared.iter().map(|(_, _, c)| c.clone()).collect();
    dict([
        ("dry_run", Value::Bool(true)),
        ("requires_explicit_confirmation", Value::Bool(true)),
        ("schema_version", int(1)),
        ("workspace_id", s(scope.workspace_id)),
        ("profile_id", s(scope.profile_id)),
        ("input_version", int(batch.input_version)),
        ("activate", Value::Bool(batch.activate)),
        ("authored_source", s(source)),
        ("summary", summary(&components)),
        ("components", Value::List(components)),
    ])
    .reorder(&[
        "dry_run",
        "requires_explicit_confirmation",
        "schema_version",
        "workspace_id",
        "profile_id",
        "input_version",
        "activate",
        "authored_source",
        "components",
        "summary",
    ])
}
trait Reorder {
    fn reorder(self, keys: &[&str]) -> Value;
}
impl Reorder for Value {
    fn reorder(self, keys: &[&str]) -> Value {
        dict(keys.iter().map(|k| (*k, field(&self, k).clone())))
    }
}
fn apply_batch(
    c: &Connection,
    scope: &Scope<'_>,
    batch: Batch,
    source: &str,
    sources: &dyn Sources,
) -> Result<Value> {
    let persisted: Vec<Value> = batch
        .prepared
        .iter()
        .filter(|(existing, _, _)| *existing)
        .map(|(_, _, c)| field(c, "id").clone())
        .collect();
    if !persisted.is_empty() {
        return Err(refuse(
            "Custody component plan was already applied",
            "custody_review_plan_stale",
            vec![("component_ids", Value::List(persisted))],
        ));
    }
    create_wallets(c, scope, &batch.planned, sources)?;
    let mut created = Vec::new();
    for (_, spec, _) in &batch.prepared {
        let id = rows_::create(c, scope, spec, source, sources)?;
        if batch.activate {
            rows_::activate(c, scope, &id, sources)?;
        }
        created.push(rows_::get_component(c, scope, &id, false)?);
    }
    Ok(dict([
        ("input_version", int(batch.input_version)),
        ("summary", summary(&created)),
        ("components", Value::List(created)),
    ])
    .reorder(&["input_version", "components", "summary"]))
}

struct Revision {
    input_version: i64,
    planned: Planned,
    prepared: Value,
    component: Value,
    commitment: Value,
}
fn old_children(c: &Connection, id: &str, table: &str) -> Result<Vec<Value>> {
    let ints: &[&str] = match table {
        "custody_component_legs" => &["ordinal", "amount_msat"],
        "custody_component_allocations" => &["ordinal", "source_amount_msat", "sink_amount_msat"],
        _ => &[
            "reviewed_source_amount_msat",
            "swap_fee_msat",
            "payout_amount_msat",
        ],
    };
    let mut result = rows(
        c,
        &format!("SELECT * FROM {table} WHERE component_id=? ORDER BY ordinal,id"),
        [id],
    )?;
    for row in &mut result {
        for key in ints {
            if let Some(Value::Str(t)) = get(row, key).cloned() {
                let raw = t.as_str().unwrap_or_default();
                let n = kassiber_pytext::parse_int(raw).map_err(|e| rows_::int_error(e, raw))?;
                set(row, key, Value::Int(n));
            }
        }
        if table == "custody_component_legs" {
            if let Some(Value::Str(t)) = get(row, "valuation_amount").cloned() {
                let raw = t.as_str().unwrap_or_default();
                let n = kassiber_pytext::parse_int(raw).map_err(|e| rows_::int_error(e, raw))?;
                set(row, "valuation_amount", Value::Int(n));
            }
        }
    }
    Ok(result)
}
fn old_component(c: &Connection, scope: &Scope<'_>, id: &str) -> Result<Value> {
    let found = rows(c, "SELECT * FROM custody_components WHERE id=?", [id])?;
    let Some(header) = found.into_iter().next() else {
        return Err(fail(
            "not_found",
            "custody component was not found",
            vec![("component_id", s(id))],
        ));
    };
    if text(&header, "profile_id") != scope.profile_id {
        return Err(fail(
            "not_found",
            "custody component was not found in the profile",
            vec![("component_id", s(id)), ("profile_id", s(scope.profile_id))],
        ));
    }
    Ok(header)
}
#[allow(clippy::too_many_arguments)]
fn plan_revision(
    c: &Connection,
    scope: &Scope<'_>,
    action: &str,
    component_id: &str,
    spec: &Value,
    activate: bool,
    reason: &Value,
    source: &str,
    sources: &dyn Sources,
) -> Result<Revision> {
    let profile = profile_row(c, scope)?;
    let old = old_component(c, scope, component_id)?;
    // The reference materializes the old revision before planning.
    rows_::get_component(c, scope, component_id, true)?;
    if action == "undo" && text(&old, "state") != "superseded" {
        return Err(refuse(
            "Only a superseded revision can be restored",
            "custody_component_not_superseded",
            vec![
                ("component_id", s(component_id)),
                ("state", value(&old, "state")),
            ],
        ));
    }
    let empty = dict::<&str>([]);
    let spec = if matches!(spec, Value::Null) {
        &empty
    } else {
        spec
    };
    if !matches!(spec, Value::Dict(_)) {
        return Err(refuse(
            "Custody component revision must be a JSON object",
            "validation",
            vec![],
        ));
    }
    let mut unknown: Vec<String> = rows_::keys(spec)
        .into_iter()
        .filter(|k| !REVISION_FIELDS.contains(&k.as_str()))
        .collect();
    unknown.sort();
    unknown.dedup();
    if !unknown.is_empty() {
        return Err(refuse(
            "Custody component revision contains unsupported fields",
            "validation",
            vec![("fields", Value::List(unknown.iter().map(s).collect()))],
        ));
    }
    let canonical = canonical(spec).map_err(|_| {
        refuse(
            "Custody component revision is not JSON-safe",
            "validation",
            vec![],
        )
    })?;
    let lineage = text(&old, "lineage_id");
    if let Some(draft) = rows(
        c,
        "SELECT id FROM custody_components WHERE profile_id=? AND lineage_id=? AND state='draft' AND id!=?",
        params![scope.profile_id, lineage, component_id],
    )?
    .first()
    {
        return Err(refuse(
            "Component lineage already has a draft revision",
            "custody_component_draft_exists",
            vec![
                ("draft_component_id", value(draft, "id")),
                ("remediation", existing_draft_remediation(&text(draft, "id"))),
            ],
        ));
    }
    let next = rows(
        c,
        "SELECT COALESCE(MAX(revision),0)+1 AS revision FROM custody_components WHERE profile_id=? AND lineage_id=?",
        [scope.profile_id, &lineage],
    )?;
    let next = match value(&next[0], "revision") {
        Value::Int(n) => n.to_string(),
        _ => return Err(internal("revision is not an integer")),
    };
    let reason_text = if truthy(reason) {
        py_str(reason)
    } else {
        String::new()
    };
    let prefix = format!(
        "kassiber:custody-revision:{}:{component_id}:{action}:{next}:{source}:{}:",
        scope.profile_id,
        u8::from(activate)
    );
    let raw_reason = match reason {
        Value::Str(raw) => raw.clone(),
        _ => reason_text.as_str().into(),
    };
    let suffix = format!(":{canonical}");
    let preimage = kassiber_pyjson::PyStr::from_code_points(
        prefix
            .chars()
            .map(u32::from)
            .chain(raw_reason.code_points())
            .chain(suffix.chars().map(u32::from)),
    )
    .unwrap();
    let new_id = uuid5(
        preimage
            .as_str()
            .ok_or_else(|| internal(kassiber_pytext::utf8_error(&preimage).unwrap()))?,
    );
    if !rows(c, "SELECT 1 FROM custody_components WHERE id=?", [&new_id])?.is_empty() {
        return Err(refuse(
            "Planned custody component revision already exists",
            "conflict",
            vec![("component_id", s(&new_id))],
        ));
    }
    let mut planned: Planned = Vec::new();
    let old_legs = old_children(c, component_id, "custody_component_legs")?;
    let raw_legs = get(spec, "legs")
        .cloned()
        .unwrap_or_else(|| Value::List(old_legs.clone()));
    let Value::List(raw_legs) = raw_legs else {
        return Err(refuse(
            "Custody component legs must be a JSON array",
            "validation",
            vec![],
        ));
    };
    let hidden: Vec<(String, Value)> = old_legs
        .iter()
        .filter(|l| !matches!(field(l, "location_ref"), Value::Null))
        .map(|l| (py_str(field(l, "id")), field(l, "location_ref").clone()))
        .collect();
    let preserved: Vec<Value> = raw_legs
        .iter()
        .map(|raw| {
            if !matches!(raw, Value::Dict(_)) {
                return raw.clone();
            }
            let mut leg = raw.clone();
            let id = reference(field(&leg, "id"));
            if !present(&leg, "location_ref") {
                if let Some((_, location)) = hidden.iter().rev().find(|(k, _)| *k == id) {
                    set(&mut leg, "location_ref", location.clone());
                }
            }
            leg
        })
        .collect();
    let mut legs = prepare_legs(
        c,
        scope,
        &new_id,
        &Value::List(preserved.clone()),
        &mut planned,
    )?;
    let mut leg_map: Vec<(String, String)> = Vec::new();
    for (ordinal, (raw, leg)) in preserved.iter().zip(legs.iter_mut()).enumerate() {
        let target = deterministic(&new_id, "leg", ordinal);
        let raw_id = field(raw, "id");
        if matches!(raw, Value::Dict(_)) && !matches!(raw_id, Value::Null) && *raw_id != s("") {
            let key = py_str(raw_id);
            match leg_map.iter_mut().find(|(k, _)| *k == key) {
                Some(entry) => entry.1 = target.clone(),
                None => leg_map.push((key, target.clone())),
            }
        }
        set(leg, "id", s(target));
    }
    let mapped = |v: &Value| -> Option<String> {
        let key = reference(v);
        leg_map
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, t)| t.clone())
    };
    let raw_allocations = if present(spec, "allocations") {
        field(spec, "allocations").clone()
    } else if present(spec, "legs") {
        Value::List(Vec::new())
    } else {
        Value::List(old_children(
            c,
            component_id,
            "custody_component_allocations",
        )?)
    };
    let Value::List(raw_allocations) = raw_allocations else {
        return Err(refuse(
            "Custody component allocations must be a JSON array",
            "validation",
            vec![],
        ));
    };
    let remapped: Vec<Value> = raw_allocations
        .iter()
        .map(|raw| {
            if !matches!(raw, Value::Dict(_)) {
                return raw.clone();
            }
            let mut allocation = raw.clone();
            for key in ["source_leg_id", "sink_leg_id"] {
                if let Some(target) = mapped(field(&allocation, key)) {
                    set(&mut allocation, key, s(target));
                }
            }
            allocation
        })
        .collect();
    let mut allocations = prepare_allocations(&new_id, &Value::List(remapped))?;
    for (ordinal, allocation) in allocations.iter_mut().enumerate() {
        set(
            allocation,
            "id",
            s(deterministic(&new_id, "allocation", ordinal)),
        );
    }
    let mut terms = Vec::new();
    for (ordinal, term) in old_children(c, component_id, "custody_component_economic_terms")?
        .into_iter()
        .enumerate()
    {
        let source_id = mapped(field(&term, "source_leg_id"));
        let target_id = mapped(field(&term, "target_leg_id"));
        let (Some(source_id), Some(target_id)) = (source_id, target_id) else {
            return Err(refuse(
                "Revision must preserve legs bound by migrated economic terms",
                "custody_component_economic_term_leg_changed",
                vec![("term_id", value(&term, "id"))],
            ));
        };
        let mut term = term;
        set(
            &mut term,
            "id",
            s(deterministic(&new_id, "economic-term", ordinal)),
        );
        set(&mut term, "source_leg_id", s(source_id));
        set(&mut term, "target_leg_id", s(target_id));
        terms.push(term);
    }
    let mut header_values = vec![
        ("component_type", value(&old, "component_type")),
        ("conservation_mode", value(&old, "conservation_mode")),
        ("evidence_kind", value(&old, "evidence_kind")),
        ("evidence_grade", value(&old, "evidence_grade")),
        (
            "evidence",
            rows_::json_object(&value(&old, "evidence_json"), "evidence_json")?,
        ),
        ("conversion_policy", value(&old, "conversion_policy")),
        (
            "conversion_reviewed",
            Value::Bool(truthy(&value(&old, "conversion_reviewed"))),
        ),
        (
            "conversion_metadata",
            rows_::json_object(
                &value(&old, "conversion_metadata_json"),
                "conversion_metadata_json",
            )?,
        ),
        ("notes", value(&old, "notes")),
    ];
    for (key, v) in header_values.iter_mut() {
        if let Some(update) = get(spec, key) {
            *v = update.clone();
        }
    }
    header_values.push((
        "change_reason",
        if action == "undo" {
            reason.clone()
        } else {
            get(spec, "change_reason")
                .cloned()
                .unwrap_or_else(|| reason.clone())
        },
    ));
    header_values.push(("component_id", s(&new_id)));
    header_values.push(("lineage_id", s(&lineage)));
    header_values.push(("authored_source", s(source)));
    let head = rows_::header(&dict(header_values))?;
    let snapshot = store::snapshot(c, scope)?;
    let check = rows_::validate_plan(
        c,
        scope,
        &snapshot,
        &head,
        &legs,
        &allocations,
        Some(&lineage),
        &planned_ids(&planned),
        sources,
    )?;
    let terms = rows_::economic_terms(&terms, &check.legs, sources, c)?;
    if activate && !activatable(&check.validation) {
        return Err(refuse(
            "Custody component revision cannot activate",
            "custody_component_not_activatable",
            with_remediation(vec![
                ("component_id", s(&new_id)),
                ("issues", issues(&check.validation)),
            ]),
        ));
    }
    legs = check.legs;
    let mut prepared = without(&head, &["lineage_id"]);
    set(&mut prepared, "legs", Value::List(legs.clone()));
    set(
        &mut prepared,
        "allocations",
        Value::List(check.allocations.clone()),
    );
    set(&mut prepared, "economic_terms", Value::List(terms.clone()));
    let state = s(if activate { "active" } else { "draft" });
    let component = dict([
        ("id", s(&new_id)),
        ("lineage_id", s(&lineage)),
        ("workspace_id", s(scope.workspace_id)),
        ("profile_id", s(scope.profile_id)),
        (
            "revision",
            Value::Int(next.parse().map_err(|_| internal("revision"))?),
        ),
        ("component_type", field(&head, "component_type").clone()),
        (
            "conservation_mode",
            field(&head, "conservation_mode").clone(),
        ),
        ("state", state.clone()),
        ("effective_state", state),
        ("evidence_kind", field(&head, "evidence_kind").clone()),
        ("evidence_grade", field(&head, "evidence_grade").clone()),
        (
            "conversion_policy",
            field(&head, "conversion_policy").clone(),
        ),
        (
            "conversion_reviewed",
            field(&head, "conversion_reviewed").clone(),
        ),
        ("authored_source", s(source)),
        ("notes", field(&head, "notes").clone()),
        ("change_reason", field(&head, "change_reason").clone()),
        ("supersedes_component_id", s(component_id)),
        ("activated_at", Value::Null),
        ("superseded_at", Value::Null),
        ("created_at", field(&head, "created_at").clone()),
        (
            "legs",
            Value::List(legs.iter().map(|l| without(l, &["location_ref"])).collect()),
        ),
        ("allocations", Value::List(check.allocations)),
        ("economic_terms", Value::List(terms)),
        ("validation", check.validation),
    ]);
    let commitment = dict([
        ("schema_version", int(1)),
        ("workspace_id", s(scope.workspace_id)),
        ("profile_id", s(scope.profile_id)),
        ("input_version", int(rows_::input_version(&profile))),
        ("action", s(action)),
        ("source_component_id", s(component_id)),
        ("activate", Value::Bool(activate)),
        ("authored_source", s(source)),
        ("reason", s(reason_text)),
    ]);
    Ok(Revision {
        input_version: rows_::input_version(&profile),
        planned,
        prepared,
        component,
        commitment,
    })
}
fn public_revision(plan: &Revision) -> Value {
    let mut out = plan.commitment.clone();
    set(&mut out, "component", plan.component.clone());
    set(&mut out, "dry_run", Value::Bool(true));
    set(
        &mut out,
        "requires_explicit_confirmation",
        Value::Bool(true),
    );
    out
}

struct StateChange {
    input_version: i64,
    out: Value,
}
fn plan_state(
    c: &Connection,
    scope: &Scope<'_>,
    action: &str,
    component_id: &str,
    reason: &Value,
) -> Result<StateChange> {
    let profile = profile_row(c, scope)?;
    let mut component = rows_::get_component(c, scope, component_id, true)?;
    let current = field(&component, "state").clone();
    let resulting = if action == "activate" {
        component = rows_::validated(c, scope, component_id)?;
        "active"
    } else {
        "superseded"
    };
    let reason_text = if truthy(reason) {
        py_str(reason)
    } else {
        String::new()
    };
    let input_version = rows_::input_version(&profile);
    Ok(StateChange {
        input_version,
        out: dict([
            ("schema_version", int(1)),
            ("workspace_id", s(scope.workspace_id)),
            ("profile_id", s(scope.profile_id)),
            ("input_version", int(input_version)),
            ("action", s(action)),
            ("component_id", s(component_id)),
            ("current_state", current),
            ("resulting_state", s(resulting)),
            ("reason", s(reason_text)),
            ("component", component),
            ("dry_run", Value::Bool(true)),
            ("requires_explicit_confirmation", Value::Bool(true)),
        ]),
    })
}

pub(super) fn execute(
    c: &Connection,
    scope: &Scope<'_>,
    operation: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    let source = match get(args, "authored_source") {
        Some(Value::Str(text)) => text.code_points().filter_map(char::from_u32).collect(),
        _ => "user".to_owned(),
    };
    let applying = operation == "transfers.components.apply";
    if !applying {
        let a = arguments(args)?;
        return match a.action.as_str() {
            "create" => {
                let batch = plan_batch(
                    c,
                    scope,
                    &a.components,
                    a.activate.unwrap_or(true),
                    &source,
                    sources,
                )?;
                Ok(public_batch(&batch, scope, &source))
            }
            "revise" | "undo" => Ok(public_revision(&plan_revision(
                c,
                scope,
                &a.action,
                &a.component_id,
                &a.spec,
                a.activate.unwrap_or(false),
                &a.reason,
                &source,
                sources,
            )?)),
            _ => Ok(plan_state(c, scope, &a.action, &a.component_id, &a.reason)?.out),
        };
    }
    let a = arguments(args)?;
    atomic(c, || apply_review(c, scope, &a, args, &source, sources))
}

/// Every apply rebuilds its plan inside the caller-visible savepoint.
fn apply_review(
    c: &Connection,
    scope: &Scope<'_>,
    a: &Arguments,
    args: &Value,
    source: &str,
    sources: &dyn Sources,
) -> Result<Value> {
    rows_::require_input_version(c, scope, field(args, "expected_input_version"))?;
    match a.action.as_str() {
        "create" => {
            let batch = plan_batch(
                c,
                scope,
                &a.components,
                a.activate.unwrap_or(true),
                source,
                sources,
            )?;
            apply_batch(c, scope, batch, source, sources)
        }
        "revise" | "undo" => {
            let activate = a.activate.unwrap_or(false);
            let plan = plan_revision(
                c,
                scope,
                &a.action,
                &a.component_id,
                &a.spec,
                activate,
                &a.reason,
                source,
                sources,
            )?;
            create_wallets(c, scope, &plan.planned, sources)?;
            let new_id = py_str(field(&plan.component, "id"));
            let id = rows_::revise(c, &a.component_id, &new_id, &plan.prepared, sources)?;
            if activate {
                rows_::activate(c, scope, &id, sources)?;
            }
            Ok(dict([
                ("input_version", int(plan.input_version)),
                ("component", rows_::get_component(c, scope, &id, false)?),
            ]))
        }
        action => {
            let plan = plan_state(c, scope, action, &a.component_id, &a.reason)?;
            if action == "activate" {
                rows_::activate(c, scope, &a.component_id, sources)?;
            } else {
                rows_::supersede(c, scope, &a.component_id, &a.reason, sources)?;
            }
            Ok(dict([
                ("input_version", int(plan.input_version)),
                (
                    "component",
                    rows_::get_component(c, scope, &a.component_id, false)?,
                ),
            ]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The planner's `_resolve_transaction`: the shared lookup, with its own
    /// `not_found` wording (`gen_lookup_vectors.py`, mode `planner`).
    #[test]
    fn planner_transaction_references_match_python_vectors() {
        let vectors = kassiber_pytext::loads(include_str!(
            "../../transactions/tests/data/lookup-vectors.json"
        ))
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(tmp.path(), None).unwrap();
        let Some(Value::List(sql)) = get(&vectors, "sql") else {
            panic!()
        };
        for statement in sql {
            let Value::Str(sql) = statement else { panic!() };
            book.connection()
                .execute(sql.as_str().unwrap(), [])
                .unwrap();
        }
        let scope = Scope {
            workspace_id: "ws",
            profile_id: "profile",
        };
        let Some(Value::List(cases)) = get(&vectors, "cases") else {
            panic!()
        };
        for case in cases {
            let reference = field(case, "reference");
            let actual = match resolve_transaction(book.connection(), &scope, reference) {
                Ok(row) => dict([("id", value(&row, "id"))]),
                Err(error) => dict([(
                    "error",
                    dict([
                        ("code", s(error.code.as_str())),
                        ("message", s(error.message.as_str())),
                        ("hint", error.hint.as_deref().map_or(Value::Null, s)),
                        ("details", error.details.unwrap_or(Value::Null)),
                    ]),
                )]),
            };
            assert_eq!(
                &actual,
                field(case, "planner"),
                "{}",
                kassiber_envelope::py_repr(reference)
            );
        }
    }
}
