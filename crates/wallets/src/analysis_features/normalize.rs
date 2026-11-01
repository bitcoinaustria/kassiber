use super::{extract, integer, BigInt, Value};

type Field = (&'static str, fn(&Value) -> bool);

fn fields(v: &Value, tests: &[Field]) -> bool {
    v.as_object().is_some_and(|v| {
        v.len() == tests.len() && tests.iter().all(|(k, test)| v.get(*k).is_some_and(test))
    })
}
fn number(v: &Value) -> bool {
    integer(v, u32::MAX.into()).is_some()
}
fn nullable(v: &Value) -> bool {
    v.is_null() || number(v)
}
fn boolean(v: &Value) -> bool {
    v.is_null() || v.is_boolean()
}
fn items(v: &Value, limit: usize, test: impl Fn(&Value) -> bool) -> bool {
    v.as_array()
        .is_some_and(|rows| rows.len() <= limit && rows.iter().all(test))
}
fn bounded(v: &Value, limit: u64) -> bool {
    integer(v, limit).is_some()
}
fn script(v: &Value) -> bool {
    matches!(
        v.as_str(),
        Some(
            "p2pkh"
                | "p2sh"
                | "p2wpkh"
                | "p2wsh"
                | "p2tr"
                | "op_return"
                | "p2pk"
                | "witness_unknown"
                | "other"
                | "unknown"
        )
    )
}
fn amount(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        !s.is_empty()
            && s.len() <= 19
            && s.bytes().all(|c| c.is_ascii_digit())
            && s.parse::<u64>()
                .is_ok_and(|n| n <= 2_100_000_000_000_000_000 && n % 1000 == 0)
    })
}

fn valid(code: &str, v: &Value) -> bool {
    match code {
        "transaction_version" => {
            v.is_null()
                || v.as_i64()
                    .is_some_and(|n| (i32::MIN as i64..=i32::MAX as i64).contains(&n))
        }
        "cardinality" => fields(
            v,
            &[
                ("inputs", |v| bounded(v, 4096)),
                ("outputs", |v| bounded(v, 4096)),
            ],
        ),
        "sequences" => fields(
            v,
            &[
                ("values", |v| items(v, 4096, nullable)),
                ("signals_bip125", boolean),
                ("relative_lock_interpretation_available", Value::is_boolean),
                ("relative_locks", |v| {
                    items(v, 4096, |v| {
                        fields(
                            v,
                            &[
                                ("input_index", |v| bounded(v, 4095)),
                                ("unit", |v| matches!(v.as_str(), Some("seconds" | "blocks"))),
                                ("minimum", |v| bounded(v, 65535 * 512)),
                            ],
                        )
                    })
                }),
            ],
        ),
        "absolute_locktime" => fields(
            v,
            &[
                ("value", nullable),
                ("kind", |v| {
                    matches!(v.as_str(), Some("none" | "height" | "time" | "unknown"))
                }),
                ("enabled", boolean),
            ],
        ),
        "signature_encodings" => fields(
            v,
            &[
                ("count", |v| bounded(v, 16384)),
                ("observations", |v| {
                    items(v, 16384, |v| {
                        fields(
                            v,
                            &[
                                ("algorithm", |v| {
                                    matches!(v.as_str(), Some("ecdsa" | "schnorr"))
                                }),
                                ("sighash", |v| bounded(v, 255)),
                                ("low_r", boolean),
                                ("low_s", boolean),
                                ("input_index", |v| bounded(v, 4095)),
                                ("verified", |v| v.as_bool() == Some(false)),
                            ],
                        )
                    })
                }),
            ],
        ),
        "witness_shapes" => items(v, 4096, |v| v.is_null() || bounded(v, 100_000)),
        "output_value_pattern" => fields(
            v,
            &[
                ("equal_groups", |v| {
                    items(v, 4096, |v| {
                        fields(
                            v,
                            &[
                                ("amount_msat", amount),
                                ("count", |v| bounded(v, 4096) && v.as_u64() >= Some(2)),
                            ],
                        )
                    })
                }),
                ("round_1000_sat_count", |v| bounded(v, 4096)),
            ],
        ),
        "output_ordering" => fields(v, &[("bip69_value_script_order", boolean)]),
        "fee_rate" => fields(
            v,
            &[
                ("fee_sats", |v| {
                    v.is_null() || bounded(v, 2_100_000_000_000_000)
                }),
                ("vsize", nullable),
            ],
        ),
        "input_script_types" | "output_script_types" => fields(
            v,
            &[
                ("counts", |v| {
                    v.as_object().is_some_and(|m| {
                        m.len() <= 10
                            && m.iter()
                                .all(|(k, v)| script(&Value::String(k.clone())) && bounded(v, 4096))
                    })
                }),
                ("by_index", |v| items(v, 4096, script)),
            ],
        ),
        _ => false,
    }
}

fn version_one(v: &Value) -> bool {
    v.as_bool() == Some(true) || v.as_f64() == Some(1.0) || super::big(v) == Some(BigInt::from(1))
}

/// Rebind trusted categorical context and discard every imported metadata field.
/// A syntactically valid cache remains an observation, never authentication.
pub fn normalize(value: &Value, source: &str, subject: Option<&str>) -> Option<Value> {
    if !value.is_object()
        || !version_one(&value["schema_version"])
        || !version_one(&value["extractor_version"])
    {
        return None;
    }
    let rows = value["features"].as_array().filter(|r| r.len() <= 20)?;
    let mut template = extract(&serde_json::json!({}), source, subject, &[]);
    let known = template["features"].as_array().unwrap();
    let mut result = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for row in rows {
        let code = row["code"].as_str()?;
        let availability = row["availability"].as_str()?;
        if !matches!(availability, "observed" | "partial" | "unavailable")
            || !seen.insert(code)
            || !valid(code, &row["value"])
        {
            return None;
        }
        let mut trusted = known.iter().find(|r| r["code"] == code)?.clone();
        trusted["value"] = row["value"].clone();
        trusted["availability"] = availability.into();
        trusted["confidence"] = if availability == "unavailable" {
            "unknown"
        } else {
            "observed"
        }
        .into();
        result.push(trusted);
    }
    template["features"] = Value::Array(result);
    Some(template)
}
