//! Canonical, bounded review proposals. Digests bind content, never authority.
use crate::value;
use bitcoin_hashes::{sha256, Hash};
use kassiber_envelope::{dict, get, s, set, AppError};
use kassiber_pyjson::{dumps, Key, Options, PyStr, Value};
use std::collections::BTreeSet;

pub const MAX_OPERATIONS: usize = 50;
pub const MAX_ARTIFACT_BYTES: usize = 1_000_000;
/// Transactions one `accept_coarse_prices` operation may name.
pub const MAX_ACCEPTED_PRICES: usize = 5000;
pub const MAX_DISMISSAL_DAYS: i64 = 3650;
pub const CUSTODY_GAP_ACTIONS: [&str; 5] =
    ["create", "dismiss", "revise", "reopen", "classify_residual"];
/// Operations that change which transfer candidates are open.
pub const PAIRING_OPERATION_TYPES: [&str; 4] = ["pair", "unpair", "dismiss_pair", "undismiss_pair"];
type Result<T> = std::result::Result<T, AppError>;

/// Pair creation, candidate dismissal and custody-gap review: (required,
/// optional) fields, each mapped onto its existing core service.
fn term_fields(kind: &str) -> Option<(&'static [&'static str], &'static [&'static str])> {
    Some(match kind {
        "pair" => (
            &["in_transaction_id", "out_transaction_id", "reason"],
            &["out_amount_msat", "pair_kind", "policy"],
        ),
        "dismiss_pair" => (
            &["in_transaction_id", "out_transaction_id", "reason"],
            &["expires_in_days"],
        ),
        // The undo of a dismissal: the matcher may suggest that pair again.
        "undismiss_pair" => (&["in_transaction_id", "out_transaction_id", "reason"], &[]),
        "custody_gap" => (&["action", "gap_id", "reason"], &["classification"]),
        _ => return None,
    })
}

/// Whether an applied review artifact paired, unpaired or dismissed a move.
pub fn changes_transfer_candidates(artifact: &Value) -> bool {
    matches!(get(artifact, "operations"), Some(Value::List(operations)) if operations.iter().any(|operation| {
        matches!(get(operation, "type"), Some(Value::Str(kind)) if PAIRING_OPERATION_TYPES.iter().any(|known| kind.is(known)))
    }))
}

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
fn keys(operation: &Value) -> Vec<String> {
    match operation {
        Value::Dict(fields) => fields
            .iter()
            .filter_map(|(key, _)| match key {
                Key::Str(key) => Some(kassiber_envelope::py_str(&Value::Str(key.clone()))),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}
fn supported(message: &str, choices: &[&str]) -> AppError {
    error(message).with_details(dict([(
        "supported",
        Value::List(choices.iter().map(|choice| s(*choice)).collect()),
    )]))
}

fn term(operation: &mut Value, kind: &str) -> Result<()> {
    let (required, optional) = term_fields(kind).expect("a term operation");
    let fields: BTreeSet<String> = keys(operation)
        .into_iter()
        .filter(|key| key != "type")
        .collect();
    if !required.iter().all(|key| fields.contains(*key))
        || fields
            .iter()
            .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
    {
        let mut message = format!("{kind} review requires {}", required.join(", "));
        if !optional.is_empty() {
            message.push_str(&format!(" and accepts {}", optional.join(", ")));
        }
        return Err(error(message));
    }
    for key in &fields {
        if key == "out_amount_msat" || key == "expires_in_days" {
            continue;
        }
        trimmed(operation, key)?;
        if matches!(get(operation, key), Some(Value::Str(raw)) if raw.code_points().count() > 2000)
        {
            return Err(error(format!("Review {key} exceeds 2000 characters")));
        }
    }
    match kind {
        "pair" => {
            if get(operation, "out_transaction_id") == get(operation, "in_transaction_id") {
                return Err(error("Pair review requires two different transactions"));
            }
            let kinds = crate::pair_authoring::KINDS;
            if !get(operation, "pair_kind")
                .is_none_or(|v| kinds.iter().any(|known| v == &s(*known)))
            {
                return Err(supported("Pair review kind is unsupported", kinds));
            }
            let policies = crate::pair_authoring::POLICIES;
            if !get(operation, "policy")
                .is_none_or(|v| policies.iter().any(|known| v == &s(*known)))
            {
                return Err(supported("Pair review policy is unsupported", policies));
            }
            if get(operation, "out_amount_msat").is_some()
                && !matches!(get(operation, "out_amount_msat"), Some(Value::Int(n)) if n > &0.into())
            {
                return Err(error(
                    "Pair review out_amount_msat must be a positive integer",
                ));
            }
        }
        "dismiss_pair" => {
            // A reviewed batch never writes an irrevocable dismissal; a
            // permanent one stays an explicit `transfers dismiss --expires-in-days 0`.
            if get(operation, "expires_in_days").is_some()
                && !matches!(get(operation, "expires_in_days"), Some(Value::Int(n)) if n >= &1.into() && n <= &MAX_DISMISSAL_DAYS.into())
            {
                return Err(error(format!(
                    "Dismissal expires_in_days must be between 1 and {MAX_DISMISSAL_DAYS}"
                )));
            }
        }
        "custody_gap" => {
            if !CUSTODY_GAP_ACTIONS
                .iter()
                .any(|action| value(operation, "action") == s(*action))
            {
                return Err(supported(
                    "Custody-gap review action is unsupported",
                    &CUSTODY_GAP_ACTIONS,
                ));
            }
            if (value(operation, "action") == s("classify_residual"))
                != get(operation, "classification").is_some()
            {
                return Err(error(
                    "Custody-gap classification is required for classify_residual only",
                ));
            }
            if matches!(get(operation, "reason"), Some(Value::Str(reason)) if reason.code_points().count() > 500)
            {
                return Err(error("Custody-gap review reason exceeds 500 characters"));
            }
        }
        _ => (),
    }
    Ok(())
}

/// `transaction_ids` may be left out only when planning, for every case. A
/// plan fills in the cases it accepts, so the artifact names each one.
fn accept(operation: &mut Value) -> Result<()> {
    if !allowed_keys(operation, &["type", "reason", "transaction_ids"]) {
        return Err(error(
            "accept_coarse_prices takes reason and transaction_ids",
        ));
    }
    trimmed(operation, "reason")?;
    reason(operation)?;
    let Some(ids) = get(operation, "transaction_ids") else {
        return Ok(());
    };
    let ids = match ids {
        Value::List(ids)
            if (1..=MAX_ACCEPTED_PRICES).contains(&ids.len())
                && ids.iter().all(|id| {
                    matches!(id, Value::Str(id) if kassiber_pytext::strip_py(id).code_points().next().is_some())
                }) =>
        {
            ids.iter()
                .map(|id| match id {
                    Value::Str(id) => kassiber_pytext::strip_py(id),
                    _ => unreachable!("checked string ids"),
                })
                .collect::<Vec<_>>()
        }
        _ => {
            return Err(error(format!(
                "accept_coarse_prices takes 1 to {MAX_ACCEPTED_PRICES} transaction ids"
            )))
        }
    };
    let cleaned: BTreeSet<PyStr> = ids.iter().cloned().collect();
    if cleaned.len() != ids.len() {
        return Err(error("accept_coarse_prices names each transaction once"));
    }
    set(
        operation,
        "transaction_ids",
        Value::List(cleaned.into_iter().map(Value::Str).collect()),
    );
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
        let kind = match get(operation, "type") {
            Some(Value::Str(raw)) => raw.as_str().unwrap_or_default().to_owned(),
            _ => String::new(),
        };
        let kind = kind.as_str();
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
        if [
            "unpair",
            "repair_legacy_pair",
            "confirm_distinct_import_rows",
        ]
        .contains(&kind)
        {
            let target = if kind == "confirm_distinct_import_rows" {
                "transaction_id"
            } else {
                "pair_id"
            };
            if !exact(
                operation,
                &["type", target, "expected_fingerprint", "reason"],
            ) {
                return Err(error(format!(
                    "This review requires exactly {target}, expected_fingerprint and reason"
                )));
            }
            for key in [target, "expected_fingerprint", "reason"] {
                trimmed(operation, key)?;
            }
            reason(operation)?;
            continue;
        }
        if term_fields(kind).is_some() {
            term(operation, kind)?;
            continue;
        }
        if kind == "accept_coarse_prices" {
            accept(operation)?;
            continue;
        }
        let mut allowed = vec!["type", "transaction_id", "reason"];
        match kind {
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
