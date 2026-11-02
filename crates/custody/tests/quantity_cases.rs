//! Independent CPython outputs, including behavior pins and adversarial input.
use kassiber_custody::quantity::{allocation, balances, project};
use serde_json::Value;
fn vectors() -> &'static Value {
    static V: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/custody_quantity_primitives.json"
        ))
        .unwrap()
    })
}
#[test]
fn quantity_arbitration_vectors() {
    for case in vectors()["project"].as_array().unwrap() {
        let actual = project(
            case["observations"].as_array().unwrap(),
            case["claims"].as_array().unwrap(),
        );
        if let Some(error) = case["error"].as_str() {
            assert_eq!(actual.unwrap_err(), error, "{}", case["case"]);
        } else {
            assert_eq!(actual.unwrap(), case["expected"], "{}", case["case"]);
        }
    }
}
#[test]
fn quantity_fifo_vectors() {
    for case in vectors()["allocation"].as_array().unwrap() {
        let actual = allocation::allocate_msat_fifo(
            &case["sources"],
            &case["sinks"],
            case.get("amount_msat"),
        );
        if let Some(error) = case["error"].as_str() {
            let actual = actual.unwrap_err();
            assert_eq!(actual.code, error, "{}", case["case"]);
            assert_eq!(actual.message, case["message"]);
            assert_eq!(actual.details, case["details"]);
        } else {
            assert_eq!(actual.unwrap(), case["expected"], "{}", case["case"]);
        }
    }
}
#[test]
fn quantity_balance_vectors() {
    for case in vectors()["balances"].as_array().unwrap() {
        let actual = balances::compare_wallet_balances(
            &case["projection"],
            case["current"].as_array().unwrap(),
            &case["known"],
        );
        if let Some(error) = case["error"].as_str() {
            assert_eq!(actual.unwrap_err(), error);
        } else {
            assert_eq!(actual.unwrap(), case["expected"]);
        }
    }
}

#[test]
fn quantity_transfer_vectors() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_quantity_transfers.json"
    ))
    .unwrap();
    for case in cases.as_array().unwrap() {
        let actual = kassiber_custody::quantity::transfers::canonical_internal_transfer_rows(
            &case["projection"],
            case["eligible"].as_array().unwrap(),
            &case["refs"],
        )
        .unwrap();
        assert_eq!(actual, case["expected"], "{}", case["case"]);
        assert_eq!(
            actual.as_array().unwrap().len(),
            case["active"].as_u64().unwrap() as usize
        );
    }
}

fn composition_vectors(op: &str) {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_quantity_composition.json"
    ))
    .unwrap();
    for case in cases.as_array().unwrap().iter().filter(|c| c["op"] == op) {
        if op == "reviewed" {
            let actual = kassiber_custody::quantity::reviewed::outcome(
                &case["component"],
                &case["observations"],
            );
            let expected: serde_json::Map<String, Value> =
                ["expected", "error", "message", "details"]
                    .into_iter()
                    .filter_map(|k| case.get(k).map(|v| (k.to_owned(), v.clone())))
                    .collect();
            assert_eq!(actual, Value::Object(expected), "{}", case["case"]);
            continue;
        }
        let actual = kassiber_custody::quantity::native::compile(
            &case["canonical"],
            case["audit"].as_array().unwrap(),
            case["excluded"].as_array().unwrap(),
            &case["reserved"],
        );
        if let Some(error) = case["error"].as_str() {
            assert_eq!(actual.unwrap_err(), error, "{}", case["case"]);
        } else {
            assert_eq!(actual.unwrap(), case["expected"], "{}", case["case"]);
        }
    }
}
#[test]
fn quantity_reviewed_compilation_vectors() {
    composition_vectors("reviewed");
}
#[test]
fn quantity_native_augmentation_vectors() {
    composition_vectors("native");
}

#[test]
fn quantity_scoped_tax_barrier_vectors() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_quantity_runtime.json"
    ))
    .unwrap();
    for c in cases
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["op"] == "tax_eligibility")
    {
        let actual = kassiber_custody::quantity::runtime::tax_eligibility(
            &c["canonical"],
            &c["projection"],
            c["issues"].as_array().unwrap(),
            &c["rows"],
        );
        if let Some(error) = c["error"].as_str() {
            assert_eq!(actual.unwrap_err(), error, "{}", c["case"]);
        } else {
            assert_eq!(
                actual.unwrap_or_else(|e| panic!("{}: {e}", c["case"])),
                c["expected"],
                "{}",
                c["case"]
            );
        }
    }
}

#[test]
fn quantity_gap_hold_composition_vectors() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_quantity_runtime.json"
    ))
    .unwrap();
    for c in cases
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["op"] == "gap_holds")
    {
        let actual = kassiber_custody::quantity::holds::compose(
            &c["canonical"],
            c["ignored"].as_array().unwrap(),
            &c["dismissed"],
            &c["search"],
        );
        if let Some(error) = c["error"].as_str() {
            assert_eq!(actual.unwrap_err(), error, "{}", c["case"]);
        } else {
            assert_eq!(
                actual.unwrap_or_else(|e| panic!("{}: {e}", c["case"])),
                c["expected"],
                "{}",
                c["case"]
            );
        }
    }
}

#[test]
fn quantity_whole_state_vectors() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_quantity_runtime.json"
    ))
    .unwrap();
    for c in cases
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["op"] == "runtime")
    {
        let actual = kassiber_custody::quantity::runtime::build(c).unwrap();
        assert_eq!(actual, c["expected"], "{}", c["case"]);
    }
}

#[test]
fn quantity_runtime_wrapper_vectors() {
    use kassiber_custody::{
        codec, components, evidence, gaps, interpretation, quantity, snapshot::Snapshot,
    };
    use kassiber_pyjson::Value as Py;
    fn get<'a>(v: &'a Py, key: &str) -> &'a Py {
        if let Py::Dict(v) = v {
            if let Some((_, v)) = v
                .iter()
                .find(|(k, _)| *k == kassiber_pyjson::Key::Str(key.into()))
            {
                return v;
            }
        }
        &Py::Null
    }
    fn restore(v: &Py) -> Py {
        if let Py::Str(s) = get(v, "$float") {
            return Py::Float(match s.as_str().unwrap() {
                "nan" => f64::NAN,
                "inf" => f64::INFINITY,
                "-inf" => f64::NEG_INFINITY,
                _ => panic!("float tag"),
            });
        }
        match v {
            Py::List(v) => Py::List(v.iter().map(restore).collect()),
            Py::Dict(v) => Py::Dict(v.iter().map(|(k, v)| (k.clone(), restore(v))).collect()),
            _ => v.clone(),
        }
    }
    let Py::List(cases) = codec::loads(include_str!(
        "../../../tests/fixtures/custody_quantity_wrappers.json"
    ))
    .unwrap() else {
        panic!("vectors")
    };
    for c in &cases {
        let run = || -> Result<Py, String> {
            let s: Snapshot = serde_json::from_value(codec::to_json(get(c, "input"))).unwrap();
            s.validate()?;
            interpretation::admit_channels(&s)?;
            let comp = components::compute(&s).map_err(|e| e.code.to_owned())?;
            let migration = interpretation::migration_quarantines(&s)?;
            let ev = evidence::stage(&s)?;
            let i = interpretation::stage(&s, &comp, &ev, migration)?;
            let g = gaps::stage(&s, &comp, &ev, &i)?;
            let state = quantity::runtime::stage(&s, &comp, &ev, &i, &g)?;
            let py = state.py()?;
            // The typed stage's JSON is the JSON conversion of its Python value.
            let mut expected = journal_json(&py);
            assert_eq!(
                state.json_with(true, journal_json),
                expected,
                "{:?}",
                get(c, "case")
            );
            if let Ok(Value::Object(expected)) = &mut expected {
                expected.remove("canonical_input");
            }
            assert_eq!(
                state.json_with(false, journal_json),
                expected,
                "{:?}",
                get(c, "case")
            );
            Ok(py)
        };
        let actual = run();
        if let Py::Str(e) = get(c, "error") {
            assert_eq!(
                actual.unwrap_err(),
                e.as_str().unwrap(),
                "{:?}",
                get(c, "case")
            );
        } else {
            assert_eq!(
                codec::dumps(&actual.unwrap(), true).unwrap(),
                codec::dumps(&restore(get(c, "expected")), true).unwrap(),
                "{:?}",
                get(c, "case")
            );
        }
    }
}

/// The journal render input's conversion of a stage's Python value.
fn journal_json(value: &kassiber_pyjson::Value) -> Result<Value, String> {
    use kassiber_pyjson::Key;
    use kassiber_pyjson::Value as Py;
    Ok(match value {
        Py::Null => Value::Null,
        Py::Bool(v) => serde_json::json!(v),
        Py::Int(v) => serde_json::from_str(&v.to_string()).map_err(|e| e.to_string())?,
        Py::Float(v) if v.is_infinite() => serde_json::from_str(if v.is_sign_negative() {
            "-1e999"
        } else {
            "1e999"
        })
        .unwrap(),
        Py::Float(v) if v.is_nan() => return Err("NaN".into()),
        Py::Float(v) => serde_json::json!(v),
        Py::Decimal(v) => serde_json::json!({"$decimal":v.to_string()}),
        Py::Bytes(_) => kassiber_custody::codec::to_json(value),
        Py::Str(v) => serde_json::json!(v.as_str().ok_or("surrogate")?),
        Py::List(v) => Value::Array(v.iter().map(journal_json).collect::<Result<_, _>>()?),
        Py::Dict(v) => Value::Object(
            v.iter()
                .map(|(key, value)| {
                    let Key::Str(key) = key else {
                        return Err("key".to_owned());
                    };
                    Ok((
                        key.as_str().ok_or("surrogate")?.to_owned(),
                        journal_json(value)?,
                    ))
                })
                .collect::<Result<_, String>>()?,
        ),
    })
}

#[test]
fn quantity_component_runtime_vectors() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_quantity_runtime.json"
    ))
    .unwrap();
    for c in cases
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["op"] == "component_runtime")
    {
        let actual = kassiber_custody::quantity::runtime::component_claims(
            &c["canonical"],
            c["components"].as_array().unwrap(),
            &c["evidence"],
        );
        if let Some(e) = c["error"].as_str() {
            assert_eq!(actual.unwrap_err(), e, "{}", c["case"]);
        } else {
            assert_eq!(actual.unwrap(), c["expected"], "{}", c["case"]);
        }
    }
}

fn review_case(name: &str) {
    use kassiber_custody::quantity::{holds, reviewed, runtime};
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_quantity_review.json"
    ))
    .unwrap();
    let c = cases
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["case"] == name)
        .unwrap();
    let actual = match c["op"].as_str().unwrap() {
        "runtime" => runtime::build(c),
        "tax_eligibility" => runtime::tax_eligibility(
            &c["canonical"],
            &c["projection"],
            c["issues"].as_array().unwrap(),
            &c["rows"],
        ),
        "component_runtime" => runtime::component_claims(
            &c["canonical"],
            c["components"].as_array().unwrap(),
            &c["evidence"],
        ),
        "project" => project(
            c["observations"].as_array().unwrap(),
            c["claims"].as_array().unwrap(),
        ),
        "gap_holds" => holds::compose(
            &c["canonical"],
            c["ignored"].as_array().unwrap(),
            &c["dismissed"],
            &c["search"],
        ),
        "reviewed" => {
            let actual = reviewed::outcome(&c["component"], &c["observations"]);
            let expected = if c.get("error").is_some() {
                {
                    let mut expected = serde_json::Map::new();
                    for key in ["error", "message", "details"] {
                        if let Some(value) = c.get(key) {
                            expected.insert(key.into(), value.clone());
                        }
                    }
                    Value::Object(expected)
                }
            } else {
                serde_json::json!({"expected":c["expected"]})
            };
            assert_eq!(actual, expected, "{name}");
            return;
        }
        op => panic!("unknown review operation {op}"),
    };
    if let Some(error) = c["error"].as_str() {
        assert_eq!(actual.unwrap_err(), error, "{name}");
    } else {
        assert_eq!(actual.unwrap(), c["expected"], "{name}");
    }
}

#[test]
fn quantity_rejected_rows_earliest() {
    review_case("rejected_rows_earliest");
}
#[test]
fn quantity_zero_blocker_unquantified() {
    review_case("zero_blocker_unquantified");
}
#[test]
fn quantity_bundle_anchor_identity() {
    review_case("bundle_anchor_identity");
}
#[test]
fn quantity_unknown_state_ineligible() {
    review_case("unknown_state_ineligible");
}
#[test]
fn quantity_empty_asset_identity_refusal() {
    review_case("empty_asset_identity_refusal");
}
#[test]
fn quantity_empty_observation_hash_refusal() {
    review_case("empty_observation_hash_refusal");
}

fn review_wrapper(name: &str) {
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_quantity_review_wrappers.json"
    ))
    .unwrap();
    let c = cases.iter().find(|c| c["case"] == name).unwrap();
    let actual: Value =
        serde_json::from_str(&kassiber_custody::custody(&c["input"].to_string())).unwrap();
    assert_eq!(
        actual["stages"]["quantity"], c["expected"],
        "{name}: {actual}"
    );
}
#[test]
fn quantity_native_reconciled_filter() {
    review_wrapper("native_reconciled_filter");
}
#[test]
fn quantity_evidence_subject_identity() {
    review_wrapper("evidence_subject_identity");
}

#[test]
fn quantity_operator_audit_vectors() {
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_quantity_review.json"
    ))
    .unwrap();
    for case in cases {
        review_case(case["case"].as_str().unwrap());
    }
    quantity_arbitration_vectors();
    quantity_fifo_vectors();
    quantity_balance_vectors();
    quantity_transfer_vectors();
    quantity_reviewed_compilation_vectors();
    quantity_native_augmentation_vectors();
    quantity_scoped_tax_barrier_vectors();
    quantity_gap_hold_composition_vectors();
    quantity_whole_state_vectors();
    quantity_component_runtime_vectors();
    quantity_runtime_wrapper_vectors();
    quantity_rejected_rows_earliest();
    quantity_zero_blocker_unquantified();
    quantity_bundle_anchor_identity();
    quantity_unknown_state_ineligible();
    quantity_empty_asset_identity_refusal();
    quantity_empty_observation_hash_refusal();
    quantity_native_reconciled_filter();
    quantity_evidence_subject_identity();
}

#[test]
fn quantity_inclusive_gap_principal() {
    review_case("inclusive_gap_principal");
}

#[test]
fn quantity_invalid_inbound_inclusive_order() {
    review_case("invalid_inbound_inclusive_order");
}

#[test]
fn quantity_invalid_included_fee_order() {
    review_case("invalid_included_fee_order");
}

#[test]
fn quantity_zero_gap_fallback() {
    review_case("zero_gap_fallback");
}

#[test]
fn quantity_zero_component_fallback() {
    review_case("zero_component_fallback");
}

#[test]
fn quantity_zero_unobserved_allocations() {
    review_case("zero_unobserved_allocations");
}

#[test]
fn quantity_mixed_bundle_priority() {
    review_case("mixed_bundle_priority");
}

#[test]
fn quantity_equal_quantity_direct_payout() {
    review_case("equal_quantity_direct_payout");
}

#[test]
fn quantity_empty_target_hash_message() {
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_quantity_review.json"
    ))
    .unwrap();
    let c = cases
        .iter()
        .find(|c| c["case"] == "empty_target_observation_hash")
        .unwrap();
    let error = kassiber_custody::quantity::reviewed::compile(&c["component"], &c["observations"])
        .unwrap_err();
    assert_eq!(error.code, "ValueError");
    assert_eq!(error.message, "quantity slices require an observation hash");
}

#[test]
fn quantity_bundle_earliest_timestamp() {
    review_case("bundle_earliest_timestamp");
}

#[test]
fn quantity_fifo_exact_capacity_terminates() {
    let result = allocation::allocate_msat_fifo(
        &serde_json::json!([["source", 1]]),
        &serde_json::json!([["sink", 1]]),
        None,
    )
    .unwrap();
    assert_eq!(result["allocated_msat"], 1);
}
