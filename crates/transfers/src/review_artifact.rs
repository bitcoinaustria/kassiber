//! Portable content construction and apply admission, without accounting authority.
use crate::{review_delta, review_operations as operations, value, Scope};
use kassiber_envelope::{dict, get, s, set, AppError};
use kassiber_pyjson::{Key, Value};

type Result<T> = std::result::Result<T, AppError>;
const FIELDS: [&str; 9] = [
    "schema_version",
    "workspace_id",
    "profile_id",
    "base_input_version",
    "operations",
    "before",
    "after",
    "delta",
    "digest",
];

fn invalid(message: &str) -> AppError {
    AppError::new("review_artifact_invalid", message)
}

/// `_tax_delta` over an artifact's own effects, refused as Python refuses it.
pub fn tax_delta(before: &Value, after: &Value) -> Result<Value> {
    review_delta::tax_delta(before, after).map_err(|fault| match fault {
        review_delta::Fault::Malformed => invalid("Review artifact is malformed"),
        review_delta::Fault::Internal(message) => AppError::new("internal_error", message),
    })
}
fn bounded(artifact: &Value) -> Result<()> {
    if operations::canonical(artifact)?.len() > operations::MAX_ARTIFACT_BYTES {
        return Err(AppError::new(
            "validation",
            "Review artifact exceeds the size limit",
        ));
    }
    Ok(())
}

/// Only the canonical planner may supply the before/after effects to this helper.
/// This function does not validate scope, freshness, operations or consent.
pub fn assemble(
    scope: &Scope<'_>,
    base_input_version: Value,
    normalized_operations: Value,
    before: Value,
    after: Value,
) -> Result<Value> {
    let delta = tax_delta(&before, &after)?;
    let mut artifact = dict([
        ("schema_version", 1.into()),
        ("workspace_id", s(scope.workspace_id)),
        ("profile_id", s(scope.profile_id)),
        ("base_input_version", base_input_version),
        ("operations", normalized_operations),
        ("before", before),
        ("after", after),
        ("delta", delta),
    ]);
    let digest = operations::digest(&artifact)?;
    set(&mut artifact, "digest", s(digest));
    bounded(&artifact)?;
    Ok(artifact)
}

fn key_value(key: &Key) -> Value {
    match key {
        Key::Null => Value::Null,
        Key::Bool(v) => Value::Bool(*v),
        Key::Int(v) => Value::Int(v.clone()),
        Key::Float(v) => Value::Float(*v),
        Key::Str(v) => Value::Str(v.clone()),
    }
}

/// Python JSON-value equality: mapping order is immaterial, sequence order and
/// exact integer values are retained. In particular True == 1 == 1.0.
pub fn equal(a: &Value, b: &Value) -> bool {
    let integer = |value: &Value| match value {
        Value::Bool(value) => Some(u8::from(*value).into()),
        Value::Int(value) => Some(value.clone()),
        Value::Float(value) if value.is_finite() && value.fract() == 0.0 => format!("{value:.0}")
            .parse::<kassiber_pyjson::BigInt>()
            .ok(),
        _ => None,
    };
    match (a, b) {
        (Value::Dict(a), Value::Dict(b)) => {
            a.len() == b.len()
                && a.iter().all(|(key, value)| {
                    b.iter().any(|(other_key, other)| {
                        equal(&key_value(key), &key_value(other_key)) && equal(value, other)
                    })
                })
        }
        (Value::List(a), Value::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b))
        }
        _ => match (integer(a), integer(b)) {
            (Some(a), Some(b)) => a == b,
            _ => a == b,
        },
    }
}

/// Check portable content in Python's refusal order, before reserving a writer.
/// Successful admission still requires historical retry/freshness checks and
/// canonical before/after recomputation inside the apply transaction.
pub fn admit(artifact: &Value, scope: &Scope<'_>, idempotency_key: &Value) -> Result<Value> {
    if !matches!(artifact, Value::Dict(fields) if fields.len() == FIELDS.len() && fields.iter().all(|(key, _)| {
        matches!(key, Key::Str(key) if FIELDS.iter().any(|field| key.is(field)))
    })) {
        return Err(invalid("Review artifact is malformed"));
    }
    bounded(artifact)?;
    let payload = dict(FIELDS[..8].iter().map(|key| (*key, value(artifact, key))));
    if !equal(&value(artifact, "schema_version"), &1.into())
        || value(artifact, "digest") != s(operations::digest(&payload)?)
    {
        return Err(invalid("Review artifact digest is invalid"));
    }
    if get(artifact, "profile_id") != Some(&s(scope.profile_id))
        || get(artifact, "workspace_id") != Some(&s(scope.workspace_id))
    {
        return Err(AppError::new(
            "review_scope_mismatch",
            "Review artifact belongs to another profile",
        ));
    }
    let normalized = operations::normalize(&value(artifact, "operations"))?;
    if !equal(&normalized, &value(artifact, "operations")) {
        return Err(invalid("Review operations are not canonical"));
    }
    let delta = tax_delta(&value(artifact, "before"), &value(artifact, "after"))?;
    if !equal(&delta, &value(artifact, "delta")) {
        return Err(invalid("Review tax delta does not match its effects"));
    }
    if !matches!(idempotency_key, Value::Str(key) if (1..=200).contains(&kassiber_pytext::strip_py(key).code_points().count()))
    {
        return Err(AppError::new(
            "validation",
            "Review requires an idempotency key of 1 to 200 characters",
        ));
    }
    Ok(normalized)
}
