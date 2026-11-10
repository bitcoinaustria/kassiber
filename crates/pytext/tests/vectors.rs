//! Checks every case in `tests/data/vectors.txt` (CPython 3.11's answers,
//! from `scripts/gen_vectors.py`).

use kassiber_pyjson::{dumps, float_repr, Options, PyStr};
use kassiber_pytext::{loads, parse_float, parse_int, repr_py, strip_py};
use serde_json::Value;

fn text(points: &[Value]) -> PyStr {
    PyStr::from_code_points(points.iter().map(|p| p.as_u64().unwrap() as u32)).unwrap()
}

fn plain(points: &[Value]) -> Option<String> {
    text(points).as_str().map(str::to_owned)
}

#[test]
fn cpython_vectors() {
    let data = include_str!("data/vectors.txt");
    let mut counts = std::collections::BTreeMap::new();
    for (number, line) in data.lines().enumerate() {
        let case: Value = serde_json::from_str(line).unwrap();
        let op = case["op"].as_str().unwrap();
        let input = case["in"].as_array().unwrap();
        let expected = &case["out"];
        *counts.entry(op.to_owned()).or_insert(0usize) += 1;
        let context = || format!("line {}: {op} {input:?}", number + 1);
        match op {
            "repr" => assert_eq!(
                repr_py(&text(input)),
                expected.as_str().unwrap(),
                "{}",
                context()
            ),
            "strip" => assert_eq!(
                strip_py(&text(input)),
                text(expected.as_array().unwrap()),
                "{}",
                context()
            ),
            "int" => {
                // int()/float() take str arguments, which never hold surrogates
                // on the CLI; skip the generated ones that do.
                let Some(input) = plain(input) else { continue };
                let got = parse_int(&input).ok().map(|n| n.to_string());
                assert_eq!(got.as_deref(), expected.as_str(), "{}", context());
            }
            "float" => {
                let Some(input) = plain(input) else { continue };
                let got = parse_float(&input).ok().map(float_repr);
                assert_eq!(got.as_deref(), expected.as_str(), "{}", context());
            }
            "loads" => {
                let input = plain(input).expect("loads inputs are valid UTF-8");
                match (loads(&input), expected.get("ok")) {
                    (Ok(value), Some(ok)) => {
                        let got = dumps(&value, &Options::new().compact()).unwrap();
                        assert_eq!(got, ok.as_str().unwrap(), "{}", context());
                    }
                    (Err(error), None) => {
                        assert_eq!(
                            error.to_string(),
                            expected["message"].as_str().unwrap(),
                            "{}",
                            context()
                        );
                        assert_eq!(
                            error.is_decode_error(),
                            expected["error"] == "JSONDecodeError",
                            "{}",
                            context()
                        );
                    }
                    (got, _) => panic!("{}: got {got:?}, expected {expected}", context()),
                }
            }
            other => panic!("unknown op {other}"),
        }
    }
    for (op, minimum) in [
        ("repr", 7000),
        ("strip", 1000),
        ("int", 3000),
        ("float", 3000),
        ("loads", 5000),
    ] {
        assert!(
            counts.get(op).copied().unwrap_or(0) >= minimum,
            "too few {op} cases"
        );
    }
}
