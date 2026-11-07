//! Validation here only authors/loads a rule; it never matches a transaction.
use super::*;
use kassiber_envelope::{s, truthy};

pub const KINDS: &[&str] = &[
    "manual",
    "coinjoin",
    "whirlpool",
    "chain-swap",
    "peg-in",
    "peg-out",
    "reverse-submarine-swap",
    "submarine-swap",
    "swap-refund",
];
pub const POLICIES: &[&str] = &["carrying-value", "taxable"];
const STRINGS: &[&str] = &[
    "out_wallet_id",
    "in_wallet_id",
    "out_wallet_kind",
    "in_wallet_kind",
    "out_asset",
    "in_asset",
];

fn nonempty(value: Option<&Value>) -> Option<&Value> {
    value.filter(|v| !matches!(v, Value::Null) && **v != s(""))
}
fn validation(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}
fn predicate(input: &Value) -> Result<Value> {
    let Value::Dict(fields) = input else {
        return Err(validation("predicate must be an object"));
    };
    let mut unknown: Vec<_> = fields
        .iter()
        .map(|(k, _)| key_text(k))
        .filter(|k| {
            !STRINGS.contains(&k.as_str())
                && !["max_fee_pct", "min_confidence"].contains(&k.as_str())
        })
        .collect();
    unknown.sort();
    if !unknown.is_empty() {
        return Err(validation(format!(
            "predicate has unsupported field(s): {}",
            unknown.join(", ")
        )));
    }
    let mut normalized = Vec::new();
    for &name in STRINGS {
        if let Some(v) = nonempty(get(input, name)) {
            if !matches!(v, Value::Str(_)) {
                return Err(validation(format!(
                    "predicate field '{name}' must be a string"
                )));
            }
            normalized.push((name, v.clone()));
        }
    }
    if let Some(v) = nonempty(get(input, "max_fee_pct")) {
        let fail = || validation("predicate field 'max_fee_pct' must be a number");
        let n = match v {
            Value::Float(n) => *n,
            Value::Int(n) => {
                let n = n.to_string().parse::<f64>().map_err(|_| fail())?;
                if !n.is_finite() {
                    return Err(AppError::new(
                        "internal_error",
                        "int too large to convert to float",
                    ));
                }
                n
            }
            Value::Str(s) => {
                kassiber_pytext::parse_float(s.as_str().ok_or_else(fail)?).map_err(|_| fail())?
            }
            _ => return Err(fail()),
        };
        if !n.is_finite() || n < 0.0 {
            return Err(validation(
                "predicate field 'max_fee_pct' must be a finite non-negative number",
            ));
        }
        normalized.push(("max_fee_pct", Value::Float(n)));
    }
    if let Some(v) = nonempty(get(input, "min_confidence")) {
        if !matches!(v, Value::Str(_)) {
            return Err(validation(
                "predicate field 'min_confidence' must be a string",
            ));
        }
        if ![s("exact"), s("strong")].contains(v) {
            return Err(validation(
                "predicate field 'min_confidence' must be one of: exact, strong",
            ));
        }
        normalized.push(("min_confidence", v.clone()));
    }
    Ok(dict(normalized))
}
fn decode(raw: &Value) -> Result<Value> {
    if matches!(raw, Value::Null) || *raw == s("") {
        return Ok(dict::<&str>([]));
    }
    if let Value::Str(raw) = raw {
        let decoded = kassiber_pytext::loads(raw.as_str().unwrap_or_default())
            .map_err(|_| validation("predicate JSON is invalid"))?;
        predicate(&decoded)
    } else {
        predicate(raw)
    }
}
fn parse_bool(v: &Value) -> Result<bool> {
    if matches!(v, Value::Null) {
        return Ok(true);
    }
    match kassiber_pytext::lower(kassiber_pytext::strip(&py_str(v))).as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(AppError::new(
            "app_error",
            format!("Invalid boolean value: {}", py_str(v)),
        )),
    }
}
fn loaded(row: &Value) -> Result<Value> {
    let mut invalid = Vec::new();
    let predicate = match decode(&value(row, "predicate_json")) {
        Ok(p) => p,
        Err(e) => {
            invalid.push(e.message);
            dict::<&str>([])
        }
    };
    let mut kind = get(row, "kind")
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_else(|| "manual".into());
    if !KINDS.contains(&kind.as_str()) {
        invalid.push(format!("unsupported kind '{kind}'"));
        kind = "manual".into();
    }
    let mut policy = get(row, "policy")
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_else(|| "carrying-value".into());
    if !POLICIES.contains(&policy.as_str()) {
        invalid.push(format!("unsupported policy '{policy}'"));
        policy = "carrying-value".into();
    }
    let enabled = parse_bool(&value(row, "enabled"))? && invalid.is_empty();
    let mut out = project(row, &["id", "workspace_id", "profile_id", "name"]);
    if let Value::Dict(fields) = &mut out {
        fields.extend([
            ("predicate".into(), predicate),
            ("kind".into(), s(kind)),
            ("policy".into(), s(policy)),
            ("enabled".into(), Value::Bool(enabled)),
            ("created_at".into(), value(row, "created_at")),
            ("updated_at".into(), value(row, "updated_at")),
        ]);
        if !invalid.is_empty() {
            fields.push(("invalid_reason".into(), s(invalid.join("; "))));
        }
    }
    Ok(out)
}
pub(super) fn list(conn: &Connection, scope: &Scope<'_>) -> Result<Value> {
    rows(
        conn,
        "SELECT * FROM swap_matching_rules WHERE profile_id=? ORDER BY created_at DESC,id ASC",
        [scope.profile_id],
    )?
    .iter()
    .map(loaded)
    .collect::<Result<Vec<_>>>()
    .map(Value::List)
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

pub(super) fn recommended_policy(
    conn: &Connection,
    scope: &Scope<'_>,
    predicate: &Value,
) -> Result<&'static str> {
    let normalized = |name| {
        get(predicate, name)
            .filter(|v| truthy(v))
            .map(py_str)
            .map(|s| kassiber_pytext::upper(kassiber_pytext::strip(&s)))
            .unwrap_or_default()
    };
    let out = normalized("out_asset");
    let inbound = normalized("in_asset");
    if out == inbound {
        return Ok("carrying-value");
    }
    let profile = rows(
        conn,
        "SELECT tax_country,bitcoin_rail_carrying_value FROM profiles WHERE id=?",
        [scope.profile_id],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::new("not_found", "Review profile was not found"))?;
    let country = kassiber_pytext::lower(kassiber_pytext::strip(&text(&profile, "tax_country")));
    if country == "at" {
        return Ok("carrying-value");
    }
    if !["generic", ""].contains(&country.as_str()) {
        return Err(AppError::new(
            "internal_error",
            format!(
                "Unsupported tax country '{}'",
                text(&profile, "tax_country")
            ),
        ));
    }
    if ((out == "BTC" && inbound == "LBTC") || (out == "LBTC" && inbound == "BTC"))
        && get(&profile, "bitcoin_rail_carrying_value").is_some_and(truthy)
    {
        return Ok("carrying-value");
    }
    Ok("taxable")
}

pub(super) fn write(
    conn: &Connection,
    scope: &Scope<'_>,
    operation: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    use crate::sql::{atomic, bind, error};
    use kassiber_pyjson::{dumps, Options};
    use kassiber_store::rusqlite::params;
    atomic(conn, || {
        let id;
        if operation == "transfers.rules.create" {
            let kind = get(args, "kind").cloned().unwrap_or_else(|| s("manual"));
            if !KINDS.iter().any(|k| kind == s(*k)) {
                return Err(validation(format!(
                    "Unsupported pair kind '{}'. Supported: {}",
                    py_str(&kind),
                    KINDS.join(", ")
                )));
            }
            let raw = get(args, "predicate")
                .filter(|v| !matches!(v, Value::Null))
                .cloned()
                .unwrap_or_else(|| dict::<&str>([]));
            let predicate = predicate(&raw)?;
            let policy = match get(args, "policy").filter(|v| !matches!(v, Value::Null)) {
                Some(v) => v.clone(),
                None => s(recommended_policy(conn, scope, &predicate)?),
            };
            if !POLICIES.iter().any(|p| policy == s(*p)) {
                return Err(validation(format!(
                    "Unsupported pair policy '{}'. Supported: {}",
                    py_str(&policy),
                    POLICIES.join(", ")
                )));
            }
            id = sources.uuid(conn)?;
            let now = sources.now(conn)?;
            let payload = dumps(
                &predicate,
                &Options {
                    sort_keys: true,
                    ..Options::default()
                },
            )
            .map_err(|e| AppError::new("internal_error", e.to_string()))?;
            let enabled = get(args, "enabled").is_none_or(truthy);
            conn.execute("INSERT INTO swap_matching_rules(id,workspace_id,profile_id,name,predicate_json,kind,policy,enabled,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?)",params![id,scope.workspace_id,scope.profile_id,bind(&value(args,"name"),4)?,payload,bind(&kind,6)?,bind(&policy,7)?,i64::from(enabled),now,now]).map_err(error)?;
        } else {
            id = text(args, "rule_id");
            let row = rows(
                conn,
                "SELECT * FROM swap_matching_rules WHERE id=? AND profile_id=?",
                [id.as_str(), scope.profile_id],
            )?
            .into_iter()
            .next()
            .ok_or_else(|| AppError::new("not_found", format!("Rule '{id}' not found")))?;
            if operation == "transfers.rules.delete" {
                conn.execute("DELETE FROM swap_matching_rules WHERE id=?", [&id])
                    .map_err(error)?;
                // The removed rule, so it can be created again.
                return Ok(dict([("deleted", s(id)), ("rule", loaded(&row)?)]));
            }
            let enabled = operation == "transfers.rules.enable";
            if enabled {
                let row = loaded(&row)?;
                if let Some(reason) = get(&row, "invalid_reason") {
                    return Err(validation("Invalid auto-pair rule cannot be enabled")
                        .with_hint("Delete and recreate the rule with a valid predicate.")
                        .with_details(dict([("reason", reason.clone())])));
                }
            }
            conn.execute(
                "UPDATE swap_matching_rules SET enabled=?,updated_at=? WHERE id=?",
                params![i64::from(enabled), sources.now(conn)?, id],
            )
            .map_err(error)?;
        }
        let row = rows(conn, "SELECT * FROM swap_matching_rules WHERE id=?", [id])?
            .into_iter()
            .next()
            .ok_or_else(|| AppError::new("internal_error", "Rule row was not stored"))?;
        loaded(&row)
    })
}
