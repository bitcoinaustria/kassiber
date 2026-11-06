//! Coarse-price acceptance (`pricing.reviewed_price`, `same_reviewed_price` and
//! `ledger_request._coarse_needs_review`): an accepted coarse price is not held
//! for review again while the price the journals read is unchanged.
use crate::codec;
use crate::finalized_projection::money::Money;
use serde_json::Value;

/// Python's exception family and message when a rule raises.
pub type Raised = (&'static str, String);

const INVALID_OPERATION: &str = "[<class 'decimal.InvalidOperation'>]";

/// `pricing.reviewed_price(row)`: `rate:<exact>` or `value:<exact>` for the
/// field that prices the row (the rate first, as journals read it), or `None`
/// when neither is positive. Raises as `dec` and a NaN comparison do.
pub fn reviewed_price(row: &Value) -> Result<Option<String>, Raised> {
    for (kind, exact, field) in [
        ("rate", "fiat_rate_exact", "fiat_rate"),
        ("value", "fiat_value_exact", "fiat_value"),
    ] {
        let Some(raw) = [&row[exact], &row[field]]
            .into_iter()
            .find(|v| !v.is_null() && *v != "")
        else {
            continue;
        };
        let money = Money::parse(raw).map_err(|_| {
            (
                "AppError",
                format!(
                    "Invalid decimal value: {}",
                    crate::interpretation::python_value_string(
                        &codec::from_json(raw).expect("JSON operand")
                    )
                ),
            )
        })?;
        let positive = money
            .positive()
            .map_err(|_| ("InvalidOperation", INVALID_OPERATION.to_owned()))?;
        if positive {
            let text = money
                .fixed()
                .map_err(|_| ("ValueError", "format too long".to_owned()))?;
            return Ok(Some(format!("{kind}:{text}")));
        }
    }
    Ok(None)
}

/// `pricing.same_reviewed_price(accepted, reviewed)`: one price, digit for
/// digit or by decimal value (`rate:3197.6` is `rate:3197.60`).
pub fn same_reviewed_price(accepted: &Value, reviewed: &Value) -> Result<bool, Raised> {
    if !codec::truth(accepted) || !codec::truth(reviewed) {
        return Ok(false);
    }
    if accepted == reviewed {
        return Ok(true);
    }
    let (Some(accepted), Some(reviewed)) = (accepted.as_str(), reviewed.as_str()) else {
        let other = if accepted.is_string() {
            reviewed
        } else {
            accepted
        };
        return Err((
            "AttributeError",
            format!("'{}' object has no attribute 'partition'", type_name(other)),
        ));
    };
    let (accepted_kind, accepted_amount) = accepted.split_once(':').unwrap_or((accepted, ""));
    let (reviewed_kind, reviewed_amount) = reviewed.split_once(':').unwrap_or((reviewed, ""));
    if accepted_kind != reviewed_kind {
        return Ok(false);
    }
    // Decimal equality: NaN equals nothing, and an sNaN or an unparseable
    // amount raises InvalidOperation, which the rule reads as no match.
    Ok(
        match (
            Money::parse(&Value::from(accepted_amount)),
            Money::parse(&Value::from(reviewed_amount)),
        ) {
            (Ok(Money::Finite(a)), Ok(Money::Finite(b))) => a == b,
            (Ok(Money::Special(a)), Ok(Money::Special(b))) => !a.contains("NaN") && a == b,
            _ => false,
        },
    )
}

/// `ledger_request._coarse_needs_review(row)`: a coarse price waits for review
/// unless the owner accepted this exact price. A projected row carries its
/// transaction's price as reviewed (`pricing_review_price`); another row is
/// read as it stands.
pub fn needs_review(row: &Value) -> Result<bool, Raised> {
    if row["pricing_quality"] != "coarse_fallback" {
        return Ok(false);
    }
    let accepted = &row["pricing_review_accepted_price"];
    if !codec::truth(accepted) {
        return Ok(true);
    }
    let reviewed = match row.get("pricing_review_price") {
        Some(reviewed) => reviewed.clone(),
        None => reviewed_price(row)?.map_or(Value::Null, Value::String),
    };
    Ok(!same_reviewed_price(accepted, &reviewed)?)
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_i64() || n.is_u64() => "int",
        Value::Number(_) => "float",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn coarse(extra: Value) -> Value {
        let mut row = json!({"pricing_quality":"coarse_fallback","fiat_rate_exact":"3197.6"});
        row.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        row
    }

    #[test]
    fn reviewed_price_prefers_a_positive_rate_and_keeps_its_digits() {
        for (row, expected) in [
            (
                json!({"fiat_rate_exact":"3197.60","fiat_rate":1.0}),
                Some("rate:3197.60"),
            ),
            (
                json!({"fiat_rate_exact":"2","fiat_value_exact":"5"}),
                Some("rate:2"),
            ),
            (
                json!({"fiat_rate_exact":"","fiat_rate":3197.6}),
                Some("rate:3197.6"),
            ),
            (json!({"fiat_rate_exact":"4E+4"}), Some("rate:40000")),
            (
                json!({"fiat_rate_exact":"0","fiat_value_exact":"12.50"}),
                Some("value:12.50"),
            ),
            (
                json!({"fiat_rate_exact":"-1","fiat_value":5}),
                Some("value:5"),
            ),
            (json!({"fiat_rate_exact":"Infinity"}), Some("rate:Infinity")),
            (
                json!({"fiat_rate_exact":"-Infinity","fiat_value_exact":"0"}),
                None,
            ),
            (json!({}), None),
        ] {
            assert_eq!(reviewed_price(&row).unwrap().as_deref(), expected, "{row}");
        }
        assert_eq!(
            reviewed_price(&json!({"fiat_rate_exact":"NaN"}))
                .unwrap_err()
                .0,
            "InvalidOperation"
        );
        assert_eq!(
            reviewed_price(&json!({"fiat_rate_exact":"junk"})).unwrap_err(),
            ("AppError", "Invalid decimal value: junk".to_owned())
        );
    }

    #[test]
    fn same_reviewed_price_compares_kind_then_decimal_value() {
        for (accepted, reviewed, expected) in [
            (json!("rate:3197.6"), json!("rate:3197.60"), true),
            (json!("rate:3197.6"), json!("rate:3197.6"), true),
            (json!("rate:3197.6"), json!("value:3197.6"), false),
            (json!("rate:3197.6"), json!("rate:3197.7"), false),
            (json!("rate:abc"), json!("rate:3197.6"), false),
            (json!("rate:NaN"), json!("rate:NaN"), true),
            (json!("rate:NaN"), json!("rate:nan"), false),
            (json!("rate:Infinity"), json!("rate:inf"), true),
            (json!("rate: 1_0 "), json!("rate:10"), true),
            (json!("rate"), json!("rate:"), false),
            (json!(""), json!("rate:1"), false),
            (json!("rate:1"), Value::Null, false),
        ] {
            assert_eq!(
                same_reviewed_price(&accepted, &reviewed).unwrap(),
                expected,
                "{accepted} {reviewed}"
            );
        }
        assert_eq!(
            same_reviewed_price(&json!(5), &json!("rate:5")).unwrap_err(),
            (
                "AttributeError",
                "'int' object has no attribute 'partition'".to_owned()
            )
        );
    }

    #[test]
    fn needs_review_holds_only_unaccepted_or_changed_coarse_prices() {
        assert!(!needs_review(&json!({"pricing_quality":"exact"})).unwrap());
        assert!(needs_review(&coarse(json!({}))).unwrap());
        assert!(needs_review(&coarse(json!({"pricing_review_accepted_price":""}))).unwrap());
        assert!(!needs_review(&coarse(
            json!({"pricing_review_accepted_price":"rate:3197.60"})
        ))
        .unwrap());
        assert!(needs_review(&coarse(
            json!({"pricing_review_accepted_price":"rate:3197.5"})
        ))
        .unwrap());
        // A projected row is judged by its transaction's price, not its own slice.
        let projected = coarse(json!({
            "fiat_rate_exact":"1",
            "pricing_review_price":"rate:3197.6",
            "pricing_review_accepted_price":"rate:3197.6",
        }));
        assert!(!needs_review(&projected).unwrap());
        let unpriced = coarse(json!({
            "pricing_review_price":null,
            "pricing_review_accepted_price":"rate:3197.6",
        }));
        assert!(needs_review(&unpriced).unwrap());
    }
}
