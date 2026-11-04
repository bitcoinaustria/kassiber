use kassiber_envelope::{dict, get, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub const KINDS: &[&str] = &[
    "ui.transfers.suggest",
    "ui.transfers.review_context",
    "ui.transfers.list",
    "ui.transfers.payouts.list",
    "ui.transfers.payouts.create",
    "ui.transfers.payouts.delete",
    "ui.transfers.pair",
    "ui.transfers.unpair",
    "ui.transfers.update",
    "ui.transfers.bulk_pair",
    "ui.transfers.dismiss",
    "ui.transfers.components.list",
    "ui.transfers.components.get",
    "ui.transfers.components.plan",
    "ui.transfers.components.apply",
    "ui.custody.coverage.snapshot",
    "ui.custody.lineage.snapshot",
    "ui.custody.gaps.list",
    "ui.custody.gaps.review_context",
    "ui.custody.gaps.history",
    "ui.custody.review.plan",
    "ui.custody.review.apply",
    "ui.transfers.rules.list",
    "ui.transfers.rules.create",
    "ui.transfers.rules.delete",
    "ui.transfers.rules.set_enabled",
    "ui.transfers.rules.apply",
];

pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    execute_as(book, kind, args, "gui")
}

/// `_ui_swap_matching_payload_from_conn` and `_ui_custody_gap_payload_from_conn`
/// with their `authored_source`: `"gui"` for the desktop, `"ai_tool"` for
/// the assistant, which also gets the AI conversion-review guard.
pub fn execute_as(
    book: &mut dyn Book,
    kind: &str,
    args: Option<&Value>,
    source: &'static str,
) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = args.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    let Value::Dict(fields) = args else {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    };
    if kind == "ui.transfers.review_context" {
        return review_context(book, args);
    }
    if kind == "ui.custody.coverage.snapshot" {
        let mut unknown: Vec<_> = fields
            .iter()
            .filter(|(k, _)| !["workspace", "profile"].contains(&key_text(k).as_str()))
            .map(|(k, _)| s(key_text(k)))
            .collect();
        unknown.sort_by_key(kassiber_envelope::py_str);
        if !unknown.is_empty() {
            return Err(
                AppError::new("validation", format!("{kind} received unsupported fields"))
                    .with_details(dict([("unknown", Value::List(unknown))])),
            );
        }
        return book.transfers("custody.coverage", args);
    }
    if kind == "ui.custody.lineage.snapshot" {
        return book.transaction_desktop(
            kassiber_transactions::desktop::Projection::CustodyLineage,
            Some(args),
        );
    }
    if matches!(
        kind,
        "ui.custody.gaps.list" | "ui.custody.gaps.review_context"
    ) {
        return gap_reads(book, kind, args);
    }
    if matches!(
        kind,
        "ui.transfers.components.list" | "ui.transfers.components.get"
    ) {
        return component_reads(book, kind, args);
    }
    if matches!(
        kind,
        "ui.transfers.suggest" | "ui.transfers.bulk_pair" | "ui.transfers.rules.apply"
    ) {
        let payload = pairing_args(kind, args, source)?;
        return book.transfers(
            if kind == "ui.transfers.bulk_pair" {
                "transfers.bulk-pair"
            } else {
                kind.strip_prefix("ui.").unwrap()
            },
            &payload,
        );
    }
    if matches!(kind, "ui.custody.review.plan" | "ui.custody.review.apply") {
        return gaps(book, kind, args, source);
    }
    if matches!(
        kind,
        "ui.transfers.components.plan" | "ui.transfers.components.apply"
    ) {
        return components(book, kind, args, source);
    }
    let mut operation = if kind == "ui.custody.gaps.history" {
        let allowed = ["workspace", "profile", "gap_id", "limit"];
        let mut unknown: Vec<_> = fields
            .iter()
            .filter(|(k, _)| !allowed.contains(&key_text(k).as_str()))
            .map(|(k, _)| s(key_text(k)))
            .collect();
        unknown.sort_by_key(kassiber_envelope::py_str);
        if !unknown.is_empty() {
            return Err(
                AppError::new("validation", format!("{kind} received unsupported fields"))
                    .with_details(dict([("unknown", Value::List(unknown))])),
            );
        }
        let limit = get(args, "limit")
            .cloned()
            .unwrap_or(Value::Int(100.into()));
        if !matches!(&limit,Value::Int(n) if n>=&1.into() && n<=&200.into()) {
            return Err(AppError::new(
                "validation",
                format!("{kind} limit must be an integer between 1 and 200"),
            ));
        }
        if !matches!(
            get(args, "cursor"),
            None | Some(Value::Null) | Some(Value::Str(_))
        ) {
            return Err(AppError::new(
                "validation",
                format!("{kind} cursor is invalid"),
            ));
        }
        if !matches!(get(args,"gap_id"),Some(Value::Str(v)) if v.as_str().is_some_and(|s| !kassiber_pytext::strip(s).is_empty()))
        {
            return Err(AppError::new(
                "validation",
                format!("{kind} requires gap_id"),
            ));
        }
        "transfers.gaps.history"
    } else {
        kind.strip_prefix("ui.").expect("registered kind")
    };
    let mut args = args.clone();
    normalize_pair_args(kind, &mut args, source)?;
    normalize_payout_create_args(kind, &mut args, source);
    normalize_payout_delete_args(kind, &mut args, source)?;
    if kind == "ui.custody.gaps.history" {
        if let Value::Dict(fields) = &mut args {
            for (k, v) in fields {
                if *k == "gap_id".into() {
                    *v = s(kassiber_pytext::strip(&kassiber_envelope::py_str(v)));
                }
            }
        }
    }
    if kind == "ui.transfers.rules.create" {
        let predicate = get(&args, "predicate")
            .filter(|v| !matches!(v, Value::Null))
            .cloned()
            .unwrap_or_else(|| dict::<&str>([]));
        if !matches!(predicate, Value::Dict(_)) {
            return Err(AppError::new(
                "validation",
                "ui.transfers.rules.create predicate must be an object",
            ));
        }
        kassiber_envelope::set(&mut args, "predicate", predicate);
        let kind = get(&args, "kind")
            .filter(|v| kassiber_envelope::truthy(v))
            .map(kassiber_envelope::py_str)
            .unwrap_or_else(|| "manual".into());
        kassiber_envelope::set(&mut args, "kind", s(kind));
        if let Some(policy) = get(&args, "policy")
            .filter(|v| !matches!(v, Value::Null))
            .map(kassiber_envelope::py_str)
        {
            kassiber_envelope::set(&mut args, "policy", s(policy));
        }
    }
    if matches!(
        kind,
        "ui.transfers.rules.delete" | "ui.transfers.rules.set_enabled"
    ) {
        let id = get(&args, "rule_id")
            .filter(|v| kassiber_envelope::truthy(v))
            .ok_or_else(|| AppError::new("validation", format!("{kind} requires rule_id")))?;
        let id = kassiber_envelope::py_str(id);
        kassiber_envelope::set(&mut args, "rule_id", s(id));
        if kind == "ui.transfers.rules.set_enabled" {
            operation = if get(&args, "enabled").is_none_or(kassiber_envelope::truthy) {
                "transfers.rules.enable"
            } else {
                "transfers.rules.disable"
            };
        }
    }
    if kind == "ui.transfers.dismiss" {
        for (name, alias) in [("tx_out", "out_id"), ("tx_in", "in_id")] {
            let id = get(&args, name)
                .filter(|v| kassiber_envelope::truthy(v))
                .or_else(|| get(&args, alias))
                .cloned()
                .unwrap_or(Value::Null);
            kassiber_envelope::set(&mut args, name, id);
        }
        // Preserve Python's desktop `value or 90` independently of the CLI's
        // explicit zero (never expire).
        let raw = get(&args, "expires_in_days")
            .filter(|v| kassiber_envelope::truthy(v))
            .cloned()
            .unwrap_or(90.into());
        kassiber_envelope::set(&mut args, "expires_in_days", Value::Int(expiry_days(&raw)?));
    }
    let result = book.transfers(operation, &args)?;
    let result = if matches!(kind, "ui.transfers.list" | "ui.custody.gaps.history") {
        exact_integers(result)
    } else {
        result
    };
    match kind {
        "ui.transfers.list" => Ok(dict([("pairs", result)])),
        "ui.transfers.payouts.list" => Ok(dict([("payouts", result)])),
        "ui.transfers.rules.list" => Ok(dict([("rules", result)])),
        _ => Ok(result),
    }
}

pub(crate) fn pairing_args(
    kind: &str,
    args: &Value,
    source: &'static str,
) -> Result<Value, AppError> {
    use kassiber_envelope::{py_str, set, truthy};
    let mut payload = dict([("authored_source", s(source))]);
    for name in [
        "workspace",
        "profile",
        "confidence",
        "asset_pair",
        "route_pair",
        "method",
        "candidate_type",
    ] {
        if let Some(value) = get(args, name) {
            set(&mut payload, name, value.clone());
        }
    }
    if kind == "ui.transfers.bulk_pair" {
        let confidence = get(args, "confidence")
            .filter(|v| truthy(v))
            .map(py_str)
            .unwrap_or_else(|| "exact".into());
        set(&mut payload, "confidence", s(confidence));
    }
    let window = get(args, "time_window_seconds")
        .filter(|v| truthy(v))
        .cloned()
        .unwrap_or(86400.into());
    set(
        &mut payload,
        "time_window_seconds",
        Value::Int(expiry_days(&window)?),
    );
    let fee = get(args, "fee_pct_max")
        .filter(|v| truthy(v))
        .cloned()
        .unwrap_or(Value::Float(0.01));
    let fee = match &fee {
        Value::Float(n) => *n,
        Value::Bool(n) => f64::from(u8::from(*n)),
        Value::Int(n) => n
            .to_string()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .ok_or_else(|| AppError::new("internal_error", "int too large to convert to float"))?,
        Value::Str(t) => {
            kassiber_pytext::parse_float(t.as_str().unwrap_or_default()).map_err(|_| {
                AppError::new(
                    "internal_error",
                    format!(
                        "could not convert string to float: {}",
                        kassiber_envelope::py_repr(&fee)
                    ),
                )
            })?
        }
        v => {
            return Err(AppError::new(
                "internal_error",
                format!(
                    "float() argument must be a string or a real number, not '{}'",
                    type_name(v)
                ),
            ))
        }
    };
    set(&mut payload, "fee_pct_max", Value::Float(fee));
    let floor = get(args, "fee_sats_min")
        .filter(|v| truthy(v))
        .cloned()
        .unwrap_or(2500.into());
    set(
        &mut payload,
        "fee_sats_min",
        Value::Int(expiry_days(&floor)?),
    );
    Ok(payload)
}

fn component_read_limit(kind: &str, args: &Value) -> Result<usize, AppError> {
    if kind == "ui.transfers.components.get" {
        return Ok(0);
    }
    let limit = get(args, "limit").cloned().unwrap_or(200.into());
    if !matches!(&limit,Value::Int(n)if n>=&1.into()&&n<=&1000.into()) {
        return Err(AppError::new(
            "validation",
            format!("{kind} limit must be an integer between 1 and 1000"),
        ));
    }
    let Value::Int(n) = limit else { unreachable!() };
    Ok(usize::try_from(n).unwrap())
}

fn component_read_payload(
    kind: &str,
    args: &Value,
    limit: usize,
) -> Result<(&'static str, Value), AppError> {
    use kassiber_envelope::set;
    let mut payload = dict([("include_local_evidence", Value::Bool(false))]);
    for name in ["workspace", "profile"] {
        if let Some(v) = get(args, name) {
            set(&mut payload, name, v.clone());
        }
    }
    if kind == "ui.transfers.components.get" {
        let id = get(args, "component_id")
            .and_then(|v| {
                if let Value::Str(t) = v {
                    Some(kassiber_pytext::strip_py(t))
                } else {
                    None
                }
            })
            .filter(|s| s.code_points().next().is_some())
            .ok_or_else(|| AppError::new("validation", format!("{kind} requires component_id")))?;
        set(&mut payload, "component_id", Value::Str(id));
        Ok(("transfers.components.show", payload))
    } else {
        for name in ["state", "component_type", "transaction", "effective_only"] {
            if let Some(v) = get(args, name) {
                set(&mut payload, name, v.clone());
            }
        }
        set(&mut payload, "limit", (limit + 1).into());
        // The service checks this boolean after resolving the transaction,
        // matching the desktop's refusal order without duplicating resolution.
        set(&mut payload, "desktop_kind", s(kind));
        Ok(("transfers.components.list", payload))
    }
}

fn component_reads(book: &mut dyn Book, kind: &str, args: &Value) -> Result<Value, AppError> {
    let limit = component_read_limit(kind, args)?;
    book.resolve_scope(get(args, "workspace"), get(args, "profile"))?;
    let (operation, payload) = component_read_payload(kind, args, limit)?;
    let result = book.transfers(operation, &payload)?;
    if kind == "ui.transfers.components.get" {
        return Ok(exact_integers(result));
    }
    component_read_page(result, limit)
}

fn component_read_page(result: Value, limit: usize) -> Result<Value, AppError> {
    let Value::List(mut components) = result else {
        unreachable!("component list service returns a list")
    };
    let has_more = components.len() > limit;
    components.truncate(limit);
    Ok(exact_integers(dict([
        ("components", Value::List(components)),
        ("has_more", Value::Bool(has_more)),
        ("limit", limit.into()),
    ])))
}

fn normalize_payout_create_args(kind: &str, args: &mut Value, source: &str) {
    if kind != "ui.transfers.payouts.create" {
        return;
    }
    use kassiber_envelope::{py_str, set, truthy};
    for (name, alias) in [("tx_out", "out_id"), ("notes", "note")] {
        let v = get(args, name)
            .filter(|v| truthy(v))
            .or_else(|| get(args, alias))
            .cloned()
            .unwrap_or(Value::Null);
        set(args, name, v);
    }
    let kind = get(args, "kind")
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_else(|| "direct-swap-payout".into());
    set(args, "kind", s(kind));
    if let Some(v) = get(args, "policy")
        .filter(|v| !matches!(v, Value::Null))
        .map(py_str)
    {
        set(args, "policy", s(v));
    }
    set(args, "authored_source", s(source));
}

fn normalize_payout_delete_args(
    kind: &str,
    args: &mut Value,
    source: &str,
) -> Result<(), AppError> {
    if kind == "ui.transfers.payouts.delete" {
        let id = get(args, "payout_id")
            .filter(|v| kassiber_envelope::truthy(v))
            .ok_or_else(|| {
                AppError::new(
                    "validation",
                    "ui.transfers.payouts.delete requires payout_id",
                )
            })?;
        let id = kassiber_envelope::py_str(id);
        kassiber_envelope::set(args, "payout_id", s(id));
        kassiber_envelope::set(args, "authored_source", s(source));
    }

    Ok(())
}

// The desktop calls int() before dispatching to the scoped domain service.
// Preserve error precedence even when a transaction reference is missing.
fn normalize_pair_args(kind: &str, args: &mut Value, source: &str) -> Result<(), AppError> {
    if matches!(
        kind,
        "ui.transfers.pair" | "ui.transfers.unpair" | "ui.transfers.update"
    ) {
        kassiber_envelope::set(args, "authored_source", s(source));
        if kind == "ui.transfers.pair" {
            for (name, alias) in [("tx_out", "out_id"), ("tx_in", "in_id")] {
                let id = get(args, name)
                    .filter(|v| kassiber_envelope::truthy(v))
                    .or_else(|| get(args, alias))
                    .cloned()
                    .unwrap_or(Value::Null);
                kassiber_envelope::set(args, name, id);
            }
            for (name, default) in [("kind", "manual"), ("pair_source", "manual")] {
                let v = get(args, name)
                    .filter(|v| kassiber_envelope::truthy(v))
                    .map(kassiber_envelope::py_str)
                    .unwrap_or_else(|| default.into());
                kassiber_envelope::set(args, name, s(v));
            }
        } else {
            let id = get(args, "pair_id")
                .filter(|v| kassiber_envelope::truthy(v))
                .ok_or_else(|| AppError::new("validation", format!("{kind} requires pair_id")))?;
            let id = kassiber_envelope::py_str(id);
            kassiber_envelope::set(args, "pair_id", s(id));
            if let Some(v) = get(args, "kind")
                .filter(|v| !matches!(v, Value::Null))
                .map(kassiber_envelope::py_str)
            {
                kassiber_envelope::set(args, "kind", s(v));
            }
        }
        if let Some(v) = get(args, "policy")
            .filter(|v| !matches!(v, Value::Null))
            .map(kassiber_envelope::py_str)
        {
            kassiber_envelope::set(args, "policy", s(v));
        }
        if kind == "ui.transfers.pair"
            || get(args, "notes").is_some()
            || get(args, "note").is_some()
        {
            let notes = get(args, "notes")
                .filter(|v| kassiber_envelope::truthy(v))
                .or_else(|| get(args, "note"))
                .cloned()
                .unwrap_or(Value::Null);
            kassiber_envelope::set(args, "notes", notes);
        }
    }
    Ok(())
}

fn expiry_days(value: &Value) -> Result<kassiber_pyjson::BigInt, AppError> {
    match value {
        Value::Int(n) => Ok(n.clone()),
        Value::Bool(n) => Ok(u8::from(*n).into()),
        Value::Float(n) if n.is_finite() => Ok(format!("{:.0}", n.trunc())
            .parse()
            .expect("finite float has an integer truncation")),
        Value::Float(n) => Err(AppError::new(
            "internal_error",
            if n.is_nan() {
                "cannot convert float NaN to integer"
            } else {
                "cannot convert float infinity to integer"
            },
        )),
        Value::Str(t) => kassiber_pytext::parse_int(t.as_str().unwrap_or_default()).map_err(|_| {
            AppError::new(
                "internal_error",
                format!(
                    "invalid literal for int() with base 10: {}",
                    kassiber_envelope::py_repr(value)
                ),
            )
        }),
        _ => Err(AppError::new(
            "internal_error",
            format!(
                "int() argument must be a string, a bytes-like object or a real number, not '{}'",
                type_name(value)
            ),
        )),
    }
}

fn key_text(key: &kassiber_pyjson::Key) -> String {
    use kassiber_pyjson::Key;
    kassiber_envelope::py_str(&match key {
        Key::Str(v) => Value::Str(v.clone()),
        Key::Int(v) => Value::Int(v.clone()),
        Key::Bool(v) => Value::Bool(*v),
        Key::Float(v) => Value::Float(*v),
        Key::Null => Value::Null,
    })
}

/// Desktop component review: the bulk cap precedes scope; apply then requires
/// the plan's exact input version. Authorship is the desktop's, never the request's.
fn components(
    book: &mut dyn Book,
    kind: &str,
    args: &Value,
    source: &str,
) -> Result<Value, AppError> {
    component_cap(kind, args)?;
    if source == "ai_tool" && get(args, "action").is_none_or(|action| *action == s("create")) {
        if let Some(Value::List(components)) = get(args, "components") {
            let activate = match get(args, "activate") {
                None | Some(Value::Null) => true,
                Some(activate) => kassiber_envelope::truthy(activate),
            };
            ai_conversion_boundary(components, activate)?;
        }
    }
    book.resolve_scope(get(args, "workspace"), get(args, "profile"))?;
    let (operation, payload) = component_payload(kind, args, source)?;
    Ok(exact_integers(book.transfers(operation, &payload)?))
}

fn component_cap(kind: &str, args: &Value) -> Result<(), AppError> {
    if let Some(Value::List(items)) = get(args, "components") {
        if items.len() > 50 {
            return Err(AppError::new(
                "validation",
                format!("{kind} accepts at most 50 components"),
            )
            .with_details(dict([
                ("count", Value::Int(items.len().into())),
                ("max_components", Value::Int(50.into())),
            ])));
        }
    }
    Ok(())
}

/// `_validate_ai_custody_conversion_boundary`: an AI-authored conversion
/// component can neither attest its own review nor activate itself.
fn ai_conversion_boundary(components: &[Value], activate: bool) -> Result<(), AppError> {
    let conversions: Vec<usize> = components
        .iter()
        .enumerate()
        .filter(|(_, component)| {
            let mode = get(component, "conservation_mode")
                .filter(|mode| kassiber_envelope::truthy(mode))
                .map(kassiber_envelope::py_str)
                .unwrap_or_else(|| "quantity".to_owned());
            mode == "conversion"
        })
        .map(|(index, _)| index)
        .collect();
    if conversions.is_empty() {
        return Ok(());
    }
    let indexes = |indexes: Vec<usize>| Value::List(indexes.into_iter().map(Value::from).collect());
    let attested: Vec<usize> = conversions
        .iter()
        .copied()
        .filter(|&index| get(&components[index], "conversion_reviewed") == Some(&Value::Bool(true)))
        .collect();
    if !attested.is_empty() {
        return Err(AppError::new(
            "interaction_required",
            "AI-authored conversion components cannot attest their own review",
        )
        .with_hint(
            "Create the conversion as a draft, then review and activate it in Custody Review.",
        )
        .with_details(dict([("component_indexes", indexes(attested))])));
    }
    if activate {
        return Err(AppError::new(
            "interaction_required",
            "AI-authored conversion components require separate human review before activation",
        )
        .with_hint(
            "Retry with activate=false, then review and activate the draft in Custody Review.",
        )
        .with_details(dict([("component_indexes", indexes(conversions))])));
    }
    Ok(())
}

fn component_payload(
    kind: &str,
    args: &Value,
    source: &str,
) -> Result<(&'static str, Value), AppError> {
    let field = |key: &str| get(args, key).cloned().unwrap_or(Value::Null);
    let mut payload = dict([
        (
            "action",
            get(args, "action").cloned().unwrap_or_else(|| s("create")),
        ),
        ("components", field("components")),
        ("component_id", field("component_id")),
        (
            "spec",
            get(args, "spec")
                .cloned()
                .unwrap_or_else(|| field("component")),
        ),
        ("activate", field("activate")),
        ("reason", field("reason")),
        ("authored_source", s(source)),
    ]);
    for key in ["workspace", "profile"] {
        if let Some(v) = get(args, key) {
            kassiber_envelope::set(&mut payload, key, v.clone());
        }
    }
    if kind != "ui.transfers.components.apply" {
        return Ok(("transfers.components.plan", payload));
    }
    let version = field("expected_input_version");
    if !matches!(&version, Value::Int(n) if n >= &0.into()) {
        return Err(AppError::new(
            "validation",
            format!("{kind} requires the input version returned by plan"),
        ));
    }
    kassiber_envelope::set(&mut payload, "expected_input_version", version);
    Ok(("transfers.components.apply", payload))
}

fn exact_integers(value: Value) -> Value {
    match value {
        Value::Int(ref n)
            if n < &(-9007199254740991_i64).into() || n > &9007199254740991_i64.into() =>
        {
            s(n.to_string())
        }
        Value::List(items) => Value::List(items.into_iter().map(exact_integers).collect()),
        Value::Dict(fields) => Value::Dict(
            fields
                .into_iter()
                .map(|(k, v)| (k, exact_integers(v)))
                .collect(),
        ),
        v => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_gap_reads_bound_arguments_and_project_safe_exact_integers() {
        let kind = "ui.custody.gaps.list";
        for limit in [0.into(), 201.into(), Value::Bool(true)] {
            assert_eq!(
                gap_read_args(kind, &dict([("limit", limit)]))
                    .unwrap_err()
                    .code,
                "validation"
            );
        }
        let (operation, payload) = gap_read_args(kind, &dict::<&str>([])).unwrap();
        assert_eq!(operation, "transfers.gaps.list");
        assert_eq!(get(&payload, "limit"), Some(&100.into()));
        assert!(gap_read_args(kind, &dict([("cursor", 1.into())])).is_err());
        for key in ["authored_source", "include_local_evidence", "z"] {
            let error = gap_read_args(kind, &dict([(key, s("forged"))])).unwrap_err();
            assert_eq!(
                error.details,
                Some(dict([("unknown", Value::List(vec![s(key)]))]))
            );
        }
        let kind = "ui.custody.gaps.review_context";
        assert!(gap_read_args(kind, &dict([("gap_id", Value::Bool(true))])).is_err());
        let (operation, payload) = gap_read_args(kind, &dict([("gap_id", s("  gap  "))])).unwrap();
        assert_eq!(operation, "transfers.gaps.review");
        assert_eq!(get(&payload, "gap_id"), Some(&s("gap")));
        let lone = kassiber_pytext::loads("\"gap\\ud800\"").unwrap();
        let (_, payload) = gap_read_args(kind, &dict([("gap_id", lone.clone())])).unwrap();
        assert_eq!(get(&payload, "gap_id"), Some(&lone));
        let raw = dict([
            ("summary", dict([("total", 1.into())])),
            (
                "gaps",
                Value::List(vec![dict([
                    ("gap_id", s("gap")),
                    (
                        "source_total_msat",
                        Value::Int(9_007_199_254_740_993_i64.into()),
                    ),
                    ("correction", dict([("component_id", s("private"))])),
                    ("native_support_status", s("corroborated")),
                    ("retained_msat", 1.into()),
                ])]),
            ),
            ("next_cursor", s("opaque")),
        ]);
        let projected = gap_read_projection(raw);
        let Some(Value::List(gaps)) = get(&projected, "gaps") else {
            panic!()
        };
        assert_eq!(
            get(&gaps[0], "source_total_msat"),
            Some(&s("9007199254740993"))
        );
        for key in ["correction", "native_support_status", "retained_msat"] {
            assert!(get(&gaps[0], key).is_none());
        }
        assert_eq!(get(&gaps[0], "status_reason"), Some(&Value::Null));
        assert_eq!(get(&projected, "next_cursor"), Some(&s("opaque")));
    }

    #[test]
    fn desktop_component_reads_bound_pages_and_keep_local_evidence_private() {
        let lone = kassiber_pytext::loads("\"id\\ud800\"").unwrap();
        let (_, payload) = component_read_payload(
            "ui.transfers.components.get",
            &dict([("component_id", lone.clone())]),
            0,
        )
        .unwrap();
        assert_eq!(get(&payload, "component_id"), Some(&lone));
        let page = component_read_page(Value::List(vec![s("a"), s("b"), s("c")]), 2).unwrap();
        assert_eq!(
            page,
            dict([
                ("components", Value::List(vec![s("a"), s("b")])),
                ("has_more", Value::Bool(true)),
                ("limit", 2.into())
            ])
        );
        assert_eq!(
            get(
                &component_read_page(Value::List(vec![s("a")]), 1).unwrap(),
                "has_more"
            ),
            Some(&Value::Bool(false))
        );
        let kind = "ui.transfers.components.list";
        for n in [0, 1001] {
            assert_eq!(
                component_read_limit(kind, &dict([("limit", n.into())]))
                    .unwrap_err()
                    .code,
                "validation"
            );
        }
        assert!(component_read_limit(kind, &dict([("limit", Value::Bool(true))])).is_err());
        let args = dict([
            ("limit", 1.into()),
            ("effective_only", Value::Bool(true)),
            ("transaction", s("external")),
            ("include_local_evidence", Value::Bool(true)),
            ("desktop_kind", s("forged")),
        ]);
        let limit = component_read_limit(kind, &args).unwrap();
        let (operation, payload) = component_read_payload(kind, &args, limit).unwrap();
        assert_eq!(operation, "transfers.components.list");
        assert_eq!(get(&payload, "limit"), Some(&2.into()));
        assert_eq!(
            get(&payload, "include_local_evidence"),
            Some(&Value::Bool(false))
        );
        assert_eq!(get(&payload, "desktop_kind"), Some(&s(kind)));
        assert_eq!(get(&payload, "transaction"), Some(&s("external")));
        assert_eq!(get(&payload, "effective_only"), Some(&Value::Bool(true)));
        let kind = "ui.transfers.components.get";
        assert!(
            component_read_payload(kind, &dict([("component_id", Value::Bool(true))]), 0).is_err()
        );
        let (operation, payload) = component_read_payload(
            kind,
            &dict([
                ("component_id", s("  id  ")),
                ("include_local_evidence", Value::Bool(true)),
            ]),
            0,
        )
        .unwrap();
        assert_eq!(operation, "transfers.components.show");
        assert_eq!(get(&payload, "component_id"), Some(&s("id")));
        assert_eq!(
            get(&payload, "include_local_evidence"),
            Some(&Value::Bool(false))
        );
    }

    #[test]
    fn desktop_pairing_defaults_coercion_and_authority_match_python() {
        for kind in [
            "ui.transfers.suggest",
            "ui.transfers.bulk_pair",
            "ui.transfers.rules.apply",
        ] {
            assert!(KINDS.contains(&kind));
            let args = dict([
                ("time_window_seconds", 0.into()),
                ("fee_pct_max", Value::Float(0.0)),
                ("fee_sats_min", Value::Bool(false)),
                ("authored_source", s("untrusted")),
                ("dry_run", Value::Bool(true)),
                ("skip_user_unpaired", Value::Bool(true)),
            ]);
            let payload = pairing_args(kind, &args, "gui").unwrap();
            assert_eq!(get(&payload, "time_window_seconds"), Some(&86400.into()));
            assert_eq!(get(&payload, "fee_pct_max"), Some(&Value::Float(0.01)));
            assert_eq!(get(&payload, "fee_sats_min"), Some(&2500.into()));
            assert_eq!(get(&payload, "authored_source"), Some(&s("gui")));
            assert!(get(&payload, "dry_run").is_none());
            assert!(get(&payload, "skip_user_unpaired").is_none());
            if kind == "ui.transfers.bulk_pair" {
                assert_eq!(get(&payload, "confidence"), Some(&s("exact")));
            } else {
                assert!(get(&payload, "confidence").is_none());
            }
            let args = dict([
                ("time_window_seconds", s(" ８６_４００ ")),
                ("fee_pct_max", s("0.5")),
                ("fee_sats_min", Value::Float(2500.9)),
            ]);
            let payload = pairing_args(kind, &args, "gui").unwrap();
            assert_eq!(get(&payload, "time_window_seconds"), Some(&86400.into()));
            assert_eq!(get(&payload, "fee_pct_max"), Some(&Value::Float(0.5)));
            assert_eq!(get(&payload, "fee_sats_min"), Some(&2500.into()));
            let error = pairing_args(
                kind,
                &dict([
                    ("time_window_seconds", s("bad")),
                    ("fee_pct_max", s("also bad")),
                ]),
                "gui",
            )
            .unwrap_err();
            assert_eq!(
                error.message,
                "invalid literal for int() with base 10: 'bad'"
            );
        }
    }

    #[test]
    fn desktop_component_review_caps_aliases_versions_and_authority() {
        let plan = "ui.transfers.components.plan";
        let apply = "ui.transfers.components.apply";
        let many = dict([("components", Value::List(vec![dict::<&str>([]); 51]))]);
        let error = component_cap(plan, &many).unwrap_err();
        assert_eq!(
            error.message,
            format!("{plan} accepts at most 50 components")
        );
        assert_eq!(
            kassiber_pyjson::dumps(&error.details.unwrap(), &Default::default()).unwrap(),
            r#"{"count": 51, "max_components": 50}"#
        );
        let fifty = dict([("components", Value::List(vec![dict::<&str>([]); 50]))]);
        assert!(component_cap(apply, &fifty).is_ok());
        let args = dict([
            ("component", dict([("notes", s("alias"))])),
            ("authored_source", s("ai_tool")),
            ("action", Value::Null),
        ]);
        let (operation, payload) = component_payload(plan, &args, "gui").unwrap();
        assert_eq!(operation, "transfers.components.plan");
        assert_eq!(get(&payload, "action"), Some(&Value::Null));
        assert_eq!(get(&payload, "authored_source"), Some(&s("gui")));
        assert_eq!(get(&payload, "spec"), Some(&dict([("notes", s("alias"))])));
        let explicit = dict([("spec", Value::Null), ("component", dict::<&str>([]))]);
        let (_, payload) = component_payload(plan, &explicit, "gui").unwrap();
        assert_eq!(get(&payload, "spec"), Some(&Value::Null));
        assert_eq!(get(&payload, "action"), Some(&s("create")));
        for version in [
            Value::Bool(true),
            Value::Int((-1).into()),
            s("0"),
            Value::Null,
        ] {
            let args = dict([("expected_input_version", version)]);
            let error = component_payload(apply, &args, "gui").unwrap_err();
            assert_eq!(
                error.message,
                format!("{apply} requires the input version returned by plan")
            );
        }
        let args = dict([("expected_input_version", Value::Int(0.into()))]);
        let (operation, payload) = component_payload(apply, &args, "gui").unwrap();
        assert_eq!(operation, "transfers.components.apply");
        assert_eq!(
            get(&payload, "expected_input_version"),
            Some(&Value::Int(0.into()))
        );
    }

    #[test]
    fn desktop_payout_delete_requires_id_and_overrides_source() {
        let mut args = dict([
            ("payout_id", Value::Int(123.into())),
            ("authored_source", s("untrusted")),
        ]);
        normalize_payout_delete_args("ui.transfers.payouts.delete", &mut args, "gui").unwrap();
        assert_eq!(get(&args, "payout_id"), Some(&s("123")));
        assert_eq!(get(&args, "authored_source"), Some(&s("gui")));
        for id in [Value::Null, Value::Bool(false), s(""), Value::Int(0.into())] {
            let mut args = dict([("payout_id", id)]);
            let error =
                normalize_payout_delete_args("ui.transfers.payouts.delete", &mut args, "gui")
                    .unwrap_err();
            assert_eq!(error.code, "validation");
            assert_eq!(
                error.message,
                "ui.transfers.payouts.delete requires payout_id"
            );
        }
    }

    #[test]
    fn desktop_pair_aliases_defaults_and_absent_notes_match_python() {
        assert_eq!(
            &KINDS[..9],
            &[
                "ui.transfers.suggest",
                "ui.transfers.review_context",
                "ui.transfers.list",
                "ui.transfers.payouts.list",
                "ui.transfers.payouts.create",
                "ui.transfers.payouts.delete",
                "ui.transfers.pair",
                "ui.transfers.unpair",
                "ui.transfers.update"
            ]
        );
        let mut args = dict([
            ("tx_out", s("")),
            ("out_id", s("out")),
            ("tx_in", s("in")),
            ("in_id", s("ignored")),
            ("kind", Value::Bool(false)),
            ("pair_source", s("")),
            ("policy", 42.into()),
            ("notes", s("")),
            ("note", s("alias note")),
            ("authored_source", s("untrusted")),
        ]);
        normalize_pair_args("ui.transfers.pair", &mut args, "gui").unwrap();
        for (key, expected) in [
            ("tx_out", "out"),
            ("tx_in", "in"),
            ("kind", "manual"),
            ("pair_source", "manual"),
            ("policy", "42"),
            ("notes", "alias note"),
            ("authored_source", "gui"),
        ] {
            assert_eq!(get(&args, key), Some(&s(expected)), "{key}");
        }
        let mut args = dict([("pair_id", 17.into()), ("kind", Value::Bool(false))]);
        normalize_pair_args("ui.transfers.update", &mut args, "gui").unwrap();
        assert_eq!(get(&args, "pair_id"), Some(&s("17")));
        assert_eq!(get(&args, "kind"), Some(&s("False")));
        assert!(get(&args, "notes").is_none());
        kassiber_envelope::set(&mut args, "notes", s(""));
        normalize_pair_args("ui.transfers.update", &mut args, "gui").unwrap();
        assert_eq!(get(&args, "notes"), Some(&Value::Null));
        for kind in ["ui.transfers.update", "ui.transfers.unpair"] {
            for id in [Value::Null, s(""), Value::Bool(false)] {
                let mut args = dict([("pair_id", id)]);
                let error = normalize_pair_args(kind, &mut args, "gui").unwrap_err();
                assert_eq!(error.code, "validation");
                assert_eq!(error.message, format!("{kind} requires pair_id"));
            }
        }
    }

    #[test]
    fn desktop_dismissal_expiry_uses_python_integer_coercion() {
        for (input, expected) in [
            (s(" １_２ "), 12),
            (Value::Float(-1.9), -1),
            (Value::Bool(true), 1),
        ] {
            assert_eq!(expiry_days(&input).unwrap(), expected.into());
        }
        for (input, message) in [
            (s("bad"), "invalid literal for int() with base 10: 'bad'"),
            (
                Value::Float(f64::NAN),
                "cannot convert float NaN to integer",
            ),
            (
                Value::Float(f64::INFINITY),
                "cannot convert float infinity to integer",
            ),
            (
                Value::List(vec![]),
                "int() argument must be a string, a bytes-like object or a real number, not 'list'",
            ),
        ] {
            let error = expiry_days(&input).unwrap_err();
            assert_eq!(error.code, "internal_error");
            assert_eq!(error.message, message);
        }
    }
}

#[cfg(test)]
mod payout_create_tests {
    use super::*;
    #[test]
    fn desktop_payout_creation_normalizes_aliases_and_authority() {
        let mut args = dict([
            ("tx_out", s("")),
            ("out_id", s("out")),
            ("notes", s("")),
            ("note", s("note")),
            ("kind", Value::Bool(false)),
            ("policy", 123.into()),
            ("authored_source", s("untrusted")),
        ]);
        normalize_payout_create_args("ui.transfers.payouts.create", &mut args, "gui");
        assert_eq!(get(&args, "tx_out"), Some(&s("out")));
        assert_eq!(get(&args, "notes"), Some(&s("note")));
        assert_eq!(get(&args, "kind"), Some(&s("direct-swap-payout")));
        assert_eq!(get(&args, "policy"), Some(&s("123")));
        assert_eq!(get(&args, "authored_source"), Some(&s("gui")));
    }
}

/// Gap transport checks retain Python's order: shape, scope, action, candidate,
/// then apply's version. The service owns the candidate read inside its action.
fn gaps(book: &mut dyn Book, kind: &str, args: &Value, source: &str) -> Result<Value, AppError> {
    use kassiber_envelope::set;
    let applying = kind == "ui.custody.review.apply";
    let Value::Dict(fields) = args else {
        unreachable!()
    };
    let allowed = [
        "workspace",
        "profile",
        "action",
        "gap_id",
        "classification",
        "reason",
    ];
    let mut unknown: Vec<_> = fields
        .iter()
        .map(|(k, _)| key_text(k))
        .filter(|k| !allowed.contains(&k.as_str()) && !(applying && k == "expected_input_version"))
        .collect();
    unknown.sort();
    if !unknown.is_empty() {
        return Err(
            AppError::new("validation", format!("{kind} received unsupported fields"))
                .with_details(dict([(
                    "unknown",
                    Value::List(unknown.into_iter().map(s).collect()),
                )])),
        );
    }
    let gap = get(args, "gap_id");
    let Some(Value::Str(gap)) = gap else {
        return Err(AppError::new(
            "validation",
            format!("{kind} requires gap_id"),
        ));
    };
    let gap = kassiber_pytext::strip(gap.as_str().unwrap_or_default());
    if gap.is_empty() {
        return Err(AppError::new(
            "validation",
            format!("{kind} requires gap_id"),
        ));
    }
    if !matches!(get(args, "reason"), None | Some(Value::Null))
        && !matches!(get(args,"reason"),Some(Value::Str(t)) if t.code_points().count()<=500)
    {
        return Err(AppError::new(
            "validation",
            format!("{kind} reason must be text up to 500 characters"),
        ));
    }
    book.resolve_scope(get(args, "workspace"), get(args, "profile"))?;
    let actions = ["classify_residual", "create", "dismiss", "reopen", "revise"];
    if !matches!(get(args,"action"),Some(Value::Str(t)) if t.as_str().is_some_and(|s|actions.contains(&s)))
    {
        return Err(
            AppError::new("validation", format!("{kind} action is invalid")).with_details(dict([
                (
                    "supported_actions",
                    Value::List(actions.into_iter().map(s).collect()),
                ),
            ])),
        );
    }
    if get(args, "action") == Some(&s("classify_residual"))
        && !matches!(get(args, "classification"), Some(Value::Str(_)))
    {
        return Err(AppError::new(
            "validation",
            format!("{kind} requires classification"),
        ));
    }
    let mut payload = args.clone();
    set(&mut payload, "gap_id", s(gap));
    set(&mut payload, "authored_source", s(source));
    set(&mut payload, "desktop_kind", s(kind));
    Ok(exact_integers(book.transfers(
        if applying {
            "transfers.gaps.apply"
        } else {
            "transfers.gaps.plan"
        },
        &payload,
    )?))
}

#[cfg(test)]
mod gap_tests {
    use super::*;
    use kassiber_envelope::set;
    use kassiber_runtime::store::{BookContext, StatusFacts};

    struct Capture;
    impl Book for Capture {
        fn transfers(&mut self, operation: &str, args: &Value) -> Result<Value, AppError> {
            Ok(dict([("operation", s(operation)), ("args", args.clone())]))
        }
        fn resolve_scope(
            &self,
            _: Option<&Value>,
            _: Option<&Value>,
        ) -> Result<(String, String), AppError> {
            Ok(("w".into(), "p".into()))
        }
        fn is_initialized(&self) -> Result<bool, AppError> {
            unreachable!()
        }
        fn context(&self) -> Result<BookContext, AppError> {
            unreachable!()
        }
        fn status_facts(&self) -> Result<StatusFacts, AppError> {
            unreachable!()
        }
        fn rollback(&mut self) -> Result<(), AppError> {
            unreachable!()
        }
        fn close(&mut self) -> Result<(), AppError> {
            unreachable!()
        }
    }

    #[test]
    fn desktop_gap_checks_shape_authority_and_dispatch() {
        for kind in ["ui.custody.review.plan", "ui.custody.review.apply"] {
            assert!(KINDS.contains(&kind));
            let args = dict([
                ("action", s("create")),
                ("gap_id", s(" \u{2003}gap \n")),
                ("reason", s("🦀".repeat(500))),
            ]);
            let out = execute(&mut Capture, kind, Some(&args)).unwrap();
            assert_eq!(
                get(&out, "operation"),
                Some(&s(if kind.ends_with("apply") {
                    "transfers.gaps.apply"
                } else {
                    "transfers.gaps.plan"
                }))
            );
            let payload = get(&out, "args").unwrap();
            assert_eq!(get(payload, "gap_id"), Some(&s("gap")));
            assert_eq!(get(payload, "authored_source"), Some(&s("gui")));
            assert_eq!(get(payload, "desktop_kind"), Some(&s(kind)));
            // The domain checks apply's version after candidate resolution.
            assert!(get(payload, "expected_input_version").is_none());
            for (key, value, message) in [
                (
                    "authored_source",
                    s("ai_tool"),
                    "received unsupported fields",
                ),
                ("gap_id", s(" \u{2003}"), "requires gap_id"),
                (
                    "reason",
                    s("x".repeat(501)),
                    "reason must be text up to 500 characters",
                ),
                ("action", s("unknown"), "action is invalid"),
            ] {
                let mut invalid = args.clone();
                set(&mut invalid, key, value);
                let error = execute(&mut Capture, kind, Some(&invalid)).unwrap_err();
                assert_eq!(error.code, "validation");
                assert_eq!(error.message, format!("{kind} {message}"));
            }
            let mut invalid = args.clone();
            set(&mut invalid, "action", s("classify_residual"));
            assert_eq!(
                execute(&mut Capture, kind, Some(&invalid))
                    .unwrap_err()
                    .message,
                format!("{kind} requires classification")
            );
        }
    }
}

fn gap_read_args(kind: &str, args: &Value) -> Result<(&'static str, Value), AppError> {
    use kassiber_envelope::set;
    let listing = kind == "ui.custody.gaps.list";
    let allowed = if listing {
        vec!["workspace", "profile", "limit", "cursor"]
    } else {
        vec!["workspace", "profile", "gap_id"]
    };
    let Value::Dict(fields) = args else {
        unreachable!()
    };
    let mut unknown: Vec<_> = fields
        .iter()
        .map(|(k, _)| key_text(k))
        .filter(|k| !allowed.contains(&k.as_str()))
        .collect();
    unknown.sort();
    if !unknown.is_empty() {
        let error = AppError::new("validation", format!("{kind} received unsupported fields"))
            .with_details(dict([(
                "unknown",
                Value::List(unknown.into_iter().map(s).collect()),
            )]));
        return Err(error);
    }
    let mut payload = args.clone();
    if listing {
        let limit = get(args, "limit").cloned().unwrap_or(100.into());
        if !matches!(&limit,Value::Int(n)if n>=&1.into()&&n<=&200.into()) {
            return Err(AppError::new(
                "validation",
                format!("{kind} limit must be an integer between 1 and 200"),
            ));
        }
        if !matches!(
            get(args, "cursor"),
            None | Some(Value::Null) | Some(Value::Str(_))
        ) {
            return Err(AppError::new(
                "validation",
                format!("{kind} cursor is invalid"),
            ));
        }
        set(&mut payload, "limit", limit);
        Ok(("transfers.gaps.list", payload))
    } else {
        let gap = get(args, "gap_id")
            .and_then(|v| {
                if let Value::Str(t) = v {
                    Some(kassiber_pytext::strip_py(t))
                } else {
                    None
                }
            })
            .filter(|s| s.code_points().next().is_some())
            .ok_or_else(|| AppError::new("validation", format!("{kind} requires gap_id")))?;
        set(&mut payload, "gap_id", Value::Str(gap));
        Ok(("transfers.gaps.review", payload))
    }
}

fn gap_read_projection(raw: Value) -> Value {
    let field = |v: &Value, key: &str| get(v, key).cloned().unwrap_or(Value::Null);
    let raw_summary = field(&raw, "summary");
    let summary = dict(
        [
            "total",
            "needs_review",
            "conflicting",
            "resolved",
            "dismissed",
            "unresolved_msat",
            "candidate_residual_msat",
            "candidate_residual_by_asset",
            "canonical_unresolved_msat",
            "canonical_issue_count",
            "canonical_unresolved_by_asset",
            "canonical_unquantified_issue_count",
            "canonical_status",
            "canonical_status_text",
            "derived_state_current",
            "qualification",
            "search_complete",
            "search_status",
            "search_limit_kind",
            "search_candidate_count",
        ]
        .into_iter()
        .map(|key| (key, get(&raw_summary, key).cloned().unwrap_or(0.into()))),
    );
    let gaps = if let Some(Value::List(gaps)) = get(&raw, "gaps") {
        gaps.as_slice()
    } else {
        &[]
    };
    let gaps = gaps
        .iter()
        .filter(|v| matches!(v, Value::Dict(_)))
        .map(|gap| {
            let mut projected = dict(
                [
                    "gap_id",
                    "candidate_fingerprint",
                    "status",
                    "status_reason",
                    "asset",
                    "source_wallet_label",
                    "destination_wallet_labels",
                    "source_total_msat",
                    "source_fee_msat",
                    "source_debit_msat",
                    "return_total_msat",
                    "residual_msat",
                    "started_at",
                    "ended_at",
                    "confidence",
                    "promotion_eligible",
                    "competitor_score_margin",
                    "reason_codes",
                ]
                .into_iter()
                .map(|key| (key, field(gap, key))),
            );
            let downstream = field(gap, "downstream");
            kassiber_envelope::set(
                &mut projected,
                "downstream",
                dict(
                    ["affected_disposals", "affected_years"]
                        .into_iter()
                        .map(|key| (key, field(&downstream, key))),
                ),
            );
            projected
        })
        .collect();
    exact_integers(dict([
        ("summary", summary),
        ("gaps", Value::List(gaps)),
        ("next_cursor", field(&raw, "next_cursor")),
    ]))
}

fn gap_reads(book: &mut dyn Book, kind: &str, args: &Value) -> Result<Value, AppError> {
    let (operation, payload) = gap_read_args(kind, args)?;
    book.transfers(operation, &payload).map(gap_read_projection)
}

/// `build_swap_review_context_payload`: validate in Python's order, then hand
/// the shared suggestion and saved-view reads to the transfer service.
fn review_context(book: &mut dyn Book, args: &Value) -> Result<Value, AppError> {
    let kind = "ui.transfers.review_context";
    let Value::Dict(fields) = args else {
        unreachable!()
    };
    let allowed = [
        "workspace",
        "profile",
        "limit",
        "confidence",
        "method",
        "asset_pair",
        "route_pair",
        "candidate_type",
        "time_window_seconds",
        "fee_pct_max",
        "fee_sats_min",
    ];
    let mut unknown: Vec<_> = fields
        .iter()
        .filter(|(k, _)| !allowed.contains(&key_text(k).as_str()))
        .map(|(k, _)| s(key_text(k)))
        .collect();
    unknown.sort_by_key(kassiber_envelope::py_str);
    if !unknown.is_empty() {
        return Err(AppError::new(
            "validation",
            format!("{kind} received unsupported arguments"),
        )
        .with_details(dict([("unknown", Value::List(unknown))])));
    }
    book.resolve_scope(get(args, "workspace"), get(args, "profile"))?;
    let invalid = || AppError::new("validation", format!("{kind} limit must be an integer"));
    let limit = match get(args, "limit").cloned().unwrap_or(Value::Int(8.into())) {
        Value::Int(n) => i64::try_from(n).unwrap_or(i64::MAX),
        Value::Bool(b) => i64::from(b),
        Value::Float(n) if n.is_finite() => n.trunc() as i64,
        Value::Float(n) if n.is_nan() => return Err(invalid()),
        Value::Float(_) => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        Value::Str(t) => kassiber_pytext::parse_int(t.as_str().unwrap_or_default())
            .ok()
            .map(|n| i64::try_from(n).unwrap_or(i64::MAX))
            .ok_or_else(invalid)?,
        _ => return Err(invalid()),
    };
    if limit < 1 {
        return Err(AppError::new(
            "validation",
            format!("{kind} limit must be positive"),
        ));
    }
    let suggest = pairing_args("ui.transfers.suggest", args, "gui")?;
    let suggestion = book.transfers("transfers.suggest", &suggest)?;
    let surface = match get(args, "candidate_type").and_then(|v| match v {
        Value::Str(t) => t.as_str(),
        _ => None,
    }) {
        Some("transfer") => Value::Str("transfer_candidates".into()),
        Some("swap") => Value::Str("swap_candidates".into()),
        _ => Value::Null,
    };
    let mut scope = dict([("surface", surface)]);
    for name in ["workspace", "profile"] {
        if let Some(value) = get(args, name) {
            kassiber_envelope::set(&mut scope, name, value.clone());
        }
    }
    let views = book.service("ui.saved_views.list", &scope)?;
    let mut payload = dict([
        ("args", args.clone()),
        ("suggestion", suggestion),
        ("limit", Value::Int(limit.min(50).into())),
        (
            "saved_views",
            get(&views, "views").cloned().unwrap_or(Value::List(vec![])),
        ),
    ]);
    for name in ["workspace", "profile", "fee_pct_max", "fee_sats_min"] {
        if let Some(value) = get(&suggest, name) {
            kassiber_envelope::set(&mut payload, name, value.clone());
        }
    }
    book.transfers("transfers.review-context", &payload)
}
