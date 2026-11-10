//! Python's insertion order for redacted PSBT analysis envelopes.
//! The analysis is built in sorted serde maps; product wire bytes follow the
//! reference dict construction order instead.
use kassiber_envelope::get;
use kassiber_pyjson::{Key, Value};

pub(super) fn py(value: &serde_json::Value) -> Value {
    kassiber_pytext::loads(&value.to_string()).expect("serde JSON is valid Python JSON")
}

fn pick(value: &Value, keys: &[&str], nested: impl Fn(&str, &Value) -> Value) -> Value {
    let Value::Dict(items) = value else {
        return value.clone();
    };
    let named = |key: &Key, name: &str| matches!(key, Key::Str(k) if k.as_str() == Some(name));
    let mut out = Vec::with_capacity(items.len());
    for name in keys {
        if let Some((key, item)) = items.iter().find(|(key, _)| named(key, name)) {
            out.push((key.clone(), nested(name, item)));
        }
    }
    for (key, item) in items {
        if !keys.iter().any(|name| named(key, name)) {
            out.push((key.clone(), item.clone()));
        }
    }
    Value::Dict(out)
}
fn plain(value: &Value, keys: &[&str]) -> Value {
    pick(value, keys, |_, v| v.clone())
}
fn rows(value: &Value, each: impl Fn(usize, &Value) -> Value) -> Value {
    match value {
        Value::List(items) => {
            Value::List(items.iter().enumerate().map(|(i, v)| each(i, v)).collect())
        }
        other => other.clone(),
    }
}
fn evidence(value: &Value) -> Value {
    rows(value, |_, row| {
        plain(row, &["source", "subject_id", "field"])
    })
}

/// One feature value. `adapter` counts trailing PSBT-adapter signature rows,
/// which the reference re-keys in a different order from raw-script rows.
pub(super) fn feature_value(code: &str, value: &Value, adapter: usize) -> Value {
    match code {
        "cardinality" => plain(value, &["inputs", "outputs"]),
        "sequences" => pick(
            value,
            &[
                "values",
                "signals_bip125",
                "relative_locks",
                "relative_lock_interpretation_available",
            ],
            |key, v| {
                if key == "relative_locks" {
                    rows(v, |_, row| plain(row, &["input_index", "unit", "minimum"]))
                } else {
                    v.clone()
                }
            },
        ),
        "absolute_locktime" => plain(value, &["value", "kind", "enabled"]),
        "input_script_types" | "output_script_types" => plain(value, &["counts", "by_index"]),
        "signature_encodings" => pick(value, &["observations", "count"], |key, v| {
            if key != "observations" {
                return v.clone();
            }
            let total = match v {
                Value::List(items) => items.len(),
                _ => 0,
            };
            rows(v, |i, row| {
                let tail = if i + adapter >= total {
                    ["input_index", "verified"]
                } else {
                    ["verified", "input_index"]
                };
                plain(
                    row,
                    &["algorithm", "sighash", "low_r", "low_s", tail[0], tail[1]],
                )
            })
        }),
        "output_value_pattern" => pick(
            value,
            &["equal_groups", "round_1000_sat_count"],
            |key, v| {
                if key == "equal_groups" {
                    rows(v, |_, row| plain(row, &["amount_msat", "count"]))
                } else {
                    v.clone()
                }
            },
        ),
        "output_ordering" => plain(value, &["bip69_value_script_order"]),
        "fee_rate" => plain(value, &["fee_sats", "vsize"]),
        _ => value.clone(),
    }
}

fn features(value: &Value, adapter: usize) -> Value {
    pick(
        value,
        &[
            "schema_version",
            "extractor_version",
            "source",
            "subject_id",
            "features",
        ],
        |key, v| {
            if key != "features" {
                return v.clone();
            }
            rows(v, |_, row| {
                let code = match get(row, "code") {
                    Some(Value::Str(code)) => code.as_str().unwrap_or("").to_owned(),
                    _ => String::new(),
                };
                pick(
                    row,
                    &[
                        "code",
                        "value",
                        "availability",
                        "source",
                        "confidence",
                        "evidence",
                        "assumptions",
                    ],
                    |key, v| match key {
                        "value" => feature_value(&code, v, adapter),
                        "evidence" => evidence(v),
                        _ => v.clone(),
                    },
                )
            })
        },
    )
}

pub(super) fn findings(value: &Value) -> Value {
    rows(value, |_, row| {
        pick(
            row,
            &[
                "code",
                "rule_version",
                "severity",
                "authority",
                "message",
                "feature_codes",
                "evidence",
                "assumptions",
                "contradictions",
            ],
            |key, v| {
                if key == "evidence" {
                    evidence(v)
                } else {
                    v.clone()
                }
            },
        )
    })
}

/// Partial signatures on unfinalized inputs reach features through the adapter.
pub(super) fn adapter_signatures(analysis: &serde_json::Value) -> usize {
    analysis["validation"]["input_metadata"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["finalized"] == serde_json::Value::Bool(false))
        .filter_map(|row| row["partial_signature_count"].as_u64())
        .sum::<u64>() as usize
}

pub(super) fn analysis(analysis: &serde_json::Value) -> Value {
    let adapter = adapter_signatures(analysis);
    pick(
        &py(analysis),
        &[
            "schema_version",
            "network",
            "chain",
            "psbt_version",
            "subject_id",
            "content_sha256",
            "transaction_facts",
            "features",
            "findings",
            "totals",
            "coverage",
            "validation",
        ],
        |key, v| match key {
            "transaction_facts" => pick(
                v,
                &[
                    "inputs",
                    "outputs",
                    "fee_msat",
                    "complete",
                    "collaboration",
                    "amount_provenance",
                    "amounts_chain_verified",
                ],
                |key, v| match key {
                    "inputs" => rows(v, |_, row| {
                        plain(
                            row,
                            &[
                                "output_id",
                                "amount_msat",
                                "input_index",
                                "sequence",
                                "utxo_evidence",
                            ],
                        )
                    }),
                    "outputs" => rows(v, |_, row| {
                        plain(
                            row,
                            &["output_id", "output_index", "amount_msat", "script_type"],
                        )
                    }),
                    _ => v.clone(),
                },
            ),
            "features" => features(v, adapter),
            "findings" => findings(v),
            "totals" => plain(
                v,
                &[
                    "input_msat",
                    "output_msat",
                    "fee_msat",
                    "final_vsize",
                    "final_fee_rate_sat_vb",
                ],
            ),
            "coverage" => plain(
                v,
                &[
                    "known_input_amounts",
                    "input_count",
                    "previous_transaction_hashes_verified",
                    "missing_input_indices",
                ],
            ),
            "validation" => pick(
                v,
                &[
                    "status",
                    "network_source",
                    "network_verified",
                    "chain_membership_verified",
                    "unspentness_verified",
                    "signatures_verified",
                    "input_metadata",
                    "output_derivation_metadata_present",
                    "global_xpubs_present",
                    "transaction_modifiable_flags",
                    "validated_fields",
                    "discarded_metadata_is_not_authenticated",
                    "limitations",
                ],
                |key, v| {
                    if key == "input_metadata" {
                        rows(v, |_, row| {
                            plain(
                                row,
                                &[
                                    "input_index",
                                    "finalized",
                                    "partial_signature_count",
                                    "derivation_metadata_present",
                                    "sighash_declared",
                                ],
                            )
                        })
                    } else {
                        v.clone()
                    }
                },
            ),
            _ => v.clone(),
        },
    )
}
