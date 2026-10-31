//! Canonical, bounded review proposals. Digests bind content, never authority.
use crate::{text, value};
use bitcoin_hashes::{sha256, Hash};
use kassiber_envelope::{get, s, set, AppError};
use kassiber_pyjson::{dumps, Key, Options, Value};

pub const MAX_OPERATIONS: usize = 50;
pub const MAX_ARTIFACT_BYTES: usize = 1_000_000;
type Result<T> = std::result::Result<T, AppError>;

fn error(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}
pub fn canonical(value: &Value) -> Result<String> {
    dumps(
        value,
        &Options::new().sort_keys(true).compact().allow_nan(false),
    )
    .map_err(|_| error("Review data must contain finite JSON values"))
}
pub fn digest(value: &Value) -> Result<String> {
    Ok(sha256::Hash::hash(canonical(value)?.as_bytes()).to_string())
}
fn allowed_keys(value: &Value, expected: &[&str]) -> bool {
    match value {
        Value::Dict(fields) => fields.iter().all(|(key, _)| match key {
            Key::Str(key) => expected.iter().any(|expected| key.is(expected)),
            _ => false,
        }),
        _ => false,
    }
}
fn exact(value: &Value, expected: &[&str]) -> bool {
    matches!(value, Value::Dict(fields) if fields.len()==expected.len())
        && allowed_keys(value, expected)
}
fn trimmed(operation: &mut Value, key: &str) -> Result<()> {
    let Some(Value::Str(raw)) = get(operation, key) else {
        return Err(error(format!("Review operation requires {key}")));
    };
    let clean = kassiber_pytext::strip_py(raw);
    if clean.code_points().next().is_none() {
        return Err(error(format!("Review operation requires {key}")));
    }
    set(operation, key, Value::Str(clean));
    Ok(())
}
fn reason(operation: &Value) -> Result<()> {
    if matches!(get(operation, "reason"), Some(Value::Str(reason)) if reason.code_points().count()>2000)
    {
        return Err(error("Review reason exceeds 2000 characters"));
    }
    Ok(())
}

pub fn normalize(operations: &Value) -> Result<Value> {
    if !matches!(operations, Value::List(operations) if (1..=MAX_OPERATIONS).contains(&operations.len()))
    {
        return Err(error(format!(
            "Review requires 1 to {MAX_OPERATIONS} operations"
        )));
    }
    let encoded = canonical(operations)?;
    if encoded.len() > MAX_ARTIFACT_BYTES {
        return Err(error("Review operations exceed the size limit"));
    }
    let Value::List(mut operations) = kassiber_pytext::loads(&encoded)
        .map_err(|_| error("Review data must contain finite JSON values"))?
    else {
        unreachable!("encoded operation list")
    };
    for operation in &mut operations {
        if !matches!(operation, Value::Dict(_)) {
            return Err(error("Each review operation must be an object"));
        }
        let kind = text(operation, "type");
        if kind == "custody_component" {
            let request = value(operation, "request");
            if !exact(operation, &["type", "request"]) || !matches!(request, Value::Dict(_)) {
                return Err(error("Custody review requires a typed request"));
            }
            if !matches!(get(&request,"action"),Some(Value::Str(action)) if ["create","revise","undo","activate","supersede"].iter().any(|expected| action.is(expected)))
            {
                return Err(error("Custody review requires a supported action"));
            }
            if !allowed_keys(
                &request,
                &[
                    "action",
                    "components",
                    "component_id",
                    "spec",
                    "activate",
                    "reason",
                ],
            ) {
                return Err(error("Custody review request contains unsupported fields"));
            }
            continue;
        }
        if kind == "unpair" {
            if !exact(
                operation,
                &["type", "pair_id", "expected_fingerprint", "reason"],
            ) {
                return Err(error(
                    "Unpair review requires exactly pair_id, expected_fingerprint and reason",
                ));
            }
            for key in ["pair_id", "expected_fingerprint", "reason"] {
                trimmed(operation, key)?;
            }
            reason(operation)?;
            continue;
        }
        let mut allowed = vec!["type", "transaction_id", "reason"];
        match kind.as_str() {
            "price_override" => {
                allowed.extend(["fiat_rate", "fiat_value"]);
                let supplied: Vec<_> = ["fiat_rate", "fiat_value"]
                    .into_iter()
                    .filter(|key| get(operation, key).is_some())
                    .collect();
                if supplied.len() != 1 {
                    return Err(error(
                        "Price override requires exactly one fiat_rate or fiat_value",
                    ));
                }
                let key = supplied[0];
                let Some(Value::Str(raw)) = get(operation, key) else {
                    return Err(error(
                        "Review prices must be exact decimal strings of at most 512 characters",
                    ));
                };
                if raw.code_points().count() > 512 {
                    return Err(error(
                        "Review prices must be exact decimal strings of at most 512 characters",
                    ));
                }
                let number = kassiber_pydecimal::finite_price(raw.as_str().unwrap_or(""))
                    .map_err(|failure| error(failure.message(key)))?;
                if number < kassiber_pydecimal::Decimal::zero()
                    || (key == "fiat_rate" && number.is_zero())
                {
                    return Err(error("Review price is invalid"));
                }
                let fixed = number
                    .to_fixed_string()
                    .map_err(|_| error("Review price is invalid"))?;
                if fixed.len() > 512 {
                    return Err(error("Review price is invalid"));
                }
                set(operation, key, s(fixed));
            }
            "kind_override" => {
                allowed.extend(["kind", "valuation_mode"]);
                if !matches!(get(operation, "kind"), Some(Value::Null))
                    && !matches!(get(operation,"kind"),Some(Value::Str(kind)) if ["airdrop","buy","deposit","hardfork","income","interest","lending_interest","mining","staking","wages"].iter().any(|expected| kind.is(expected)))
                {
                    return Err(error("Acquisition review requires an inbound kind or null"));
                }
                if get(operation, "valuation_mode").is_some_and(|mode| mode != &s("market_value")) {
                    return Err(AppError::new(
                        "acquisition_valuation_unsupported",
                        "This acquisition valuation mode is unsupported",
                    ));
                }
            }
            "exclude" => (),
            _ => return Err(error("Review operation type is unsupported")),
        }
        if !allowed_keys(operation, &allowed) {
            return Err(error("Review operation contains unsupported fields"));
        }
        for key in ["transaction_id", "reason"] {
            trimmed(operation, key)?;
        }
        reason(operation)?;
    }
    Ok(Value::List(operations))
}
