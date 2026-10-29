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
            let ev = evidence::compute(&s)?;
            let i = interpretation::stage(&s, &comp, &ev, migration)?;
            let g = gaps::stage(&s, &comp, &ev, &i)?;
            quantity::runtime::stage(&s, &comp, &ev, &i, &g)
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
