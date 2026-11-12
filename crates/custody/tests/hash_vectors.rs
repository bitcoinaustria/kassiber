//! Reuse the immutable CPython custody fixtures from R0, not regenerated outputs.
use kassiber_custody::{codec, evidence};
use kassiber_pydecimal::Decimal;
use kassiber_pyjson::{BigInt, Key, Value as Py};
use serde_json::{json, Value};
use std::str::FromStr;

fn fixtures(name: &str) -> Vec<Value> {
    // A reused Cargo target must not retain a removed mutation scratch path.
    let text = match name {
        "quantity_issue_id" => {
            include_str!("../../../tests/fixtures/custody_hash_vectors/quantity_issue_id.jsonl")
        }
        "quantity_store_ids" => {
            include_str!("../../../tests/fixtures/custody_hash_vectors/quantity_store_ids.jsonl")
        }
        "observed_quantity_hash" => include_str!(
            "../../../tests/fixtures/custody_hash_vectors/observed_quantity_hash.jsonl"
        ),
        "graph_hash" => {
            include_str!("../../../tests/fixtures/custody_hash_vectors/graph_hash.jsonl")
        }
        "evidence_snapshot" => {
            include_str!("../../../tests/fixtures/custody_hash_vectors/evidence_snapshot.jsonl")
        }
        "reconciled_observation" => include_str!(
            "../../../tests/fixtures/custody_hash_vectors/reconciled_observation.jsonl"
        ),
        _ => panic!("unknown hash vector family"),
    };
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn bytes(v: &Value) -> Vec<u8> {
    let s = v["$bytes"].as_str().unwrap();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn bytes_repr(v: &[u8]) -> String {
    let quote = if v.contains(&b'\'') && !v.contains(&b'"') {
        b'"'
    } else {
        b'\''
    };
    let mut s = format!("b{}", char::from(quote));
    for &b in v {
        match b {
            b'\\' => s.push_str("\\\\"),
            b'\t' => s.push_str("\\t"),
            b'\n' => s.push_str("\\n"),
            b'\r' => s.push_str("\\r"),
            _ if b == quote => {
                s.push('\\');
                s.push(char::from(b));
            }
            32..=126 => s.push(char::from(b)),
            _ => s.push_str(&format!("\\x{b:02x}")),
        }
    }
    s.push(char::from(quote));
    s
}
fn decode(v: &Value) -> Py {
    if let Some(fields) = v.get("$event_key").and_then(Value::as_array) {
        let event = json!({"chain":fields[0],"network":fields[1],"native_namespace":fields[2],"native_event_id":fields[3]});
        // The dataclass string is prepared by the production hash helper below.
        return codec::from_json(&event).unwrap();
    }
    if let Some(s) = v.get("$float").and_then(Value::as_str) {
        return Py::Float(match s {
            "nan" => f64::NAN,
            "inf" => f64::INFINITY,
            "-inf" => f64::NEG_INFINITY,
            _ => s.parse().unwrap(),
        });
    }
    if let Some(s) = v.get("$int").and_then(Value::as_str) {
        return Py::Int(BigInt::from_str(s).unwrap());
    }
    if let Some(s) = v.get("$decimal").and_then(Value::as_str) {
        return Py::Decimal(Decimal::from_str(s).unwrap());
    }
    if v.get("$bytes").is_some() {
        return Py::Str(bytes_repr(&bytes(v)).into());
    }
    if let Some(a) = v.get("$tuple").and_then(Value::as_array) {
        return Py::List(a.iter().map(decode).collect());
    }
    if let Some(a) = v.as_array() {
        return Py::List(a.iter().map(decode).collect());
    }
    if let Some(o) = v.as_object() {
        return Py::Dict(
            o.iter()
                .map(|(k, v)| (Key::Str(k.as_str().into()), decode(v)))
                .collect(),
        );
    }
    codec::from_json(v).unwrap()
}

#[test]
fn quantity_issue_ids_match_all_r0_vectors() {
    use kassiber_custody::quantity::hashes;
    for v in fixtures("quantity_issue_id") {
        let parts = v["input"]["parts"]["$tuple"].as_array().unwrap();
        let actual = if parts.get(1).is_some_and(|p| p.get("$event_key").is_some()) {
            let event = codec::to_json(&decode(&parts[1]));
            hashes::rejected_event_issue_id(&event, parts[2].as_str().unwrap()).unwrap()
        } else {
            hashes::issue_id(&parts.iter().map(decode).collect::<Vec<_>>()).unwrap()
        };
        assert_eq!(actual.0, v["output"]["issue_id"]);
        assert_eq!(actual.1, v["output"]["issue_preimage"]);
    }
}

#[test]
fn quantity_store_ids_match_all_r0_vectors() {
    use kassiber_custody::quantity::hashes;
    for v in fixtures("quantity_store_ids") {
        let kind = v["input"]["kind"].as_str().unwrap();
        if kind == "commitment_id" {
            continue;
        } // Already ported with components.
        let fields = v["input"]["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(decode)
            .collect::<Vec<_>>();
        let actual = if kind == "decision_id" {
            hashes::custody_decision_id(&fields)
        } else {
            hashes::economic_relation_id(&fields)
        }
        .unwrap();
        assert_eq!(actual.0, v["output"]["id"]);
        assert_eq!(actual.1, v["output"]["id_preimage"]);
    }
}
fn row(v: &Value) -> Value {
    let mut r = codec::to_json(&decode(v));
    if v["raw_json"].get("$bytes").is_some() {
        r["raw_json"] = json!(String::from_utf8_lossy(&bytes(&v["raw_json"])).to_string());
    }
    r
}

#[test]
fn observed_quantity_commitments_match_all_r0_vectors() {
    let vectors = fixtures("observed_quantity_hash");
    assert_eq!(vectors.len(), 43);
    for v in vectors {
        let result = evidence::observed_hash(&row(&v["input"]));
        if v["output"].get("error").is_some() {
            assert!(result.is_err(), "{}", v["case"]);
        } else {
            let (hash, text) = result.unwrap();
            assert_eq!(json!(hash), v["output"]["quantity_hash"], "{}", v["case"]);
            assert_eq!(
                json!(text),
                v["output"]["quantity_preimage"],
                "{}",
                v["case"]
            );
        }
    }
}
#[test]
fn graph_commitments_match_all_r0_vectors() {
    let vectors = fixtures("graph_hash");
    assert_eq!(vectors.len(), 94);
    for v in vectors {
        let raw = &v["input"]["raw_json"];
        let result = if raw.is_string() {
            evidence::graph_hash(raw)
        } else {
            codec::hash_py(&decode(raw), false).map_err(Into::into)
        };
        if v["output"].get("error").is_some() {
            assert!(result.is_err(), "{}", v["case"]);
        } else {
            let (hash, text) = result.unwrap();
            assert_eq!(json!(hash), v["output"]["graph_hash"], "{}", v["case"]);
            assert_eq!(json!(text), v["output"]["graph_preimage"], "{}", v["case"]);
        }
    }
}
#[test]
fn event_and_evidence_commitments_match_all_r0_vectors() {
    let vectors = fixtures("evidence_snapshot");
    assert_eq!(vectors.len(), 150);
    for v in vectors {
        let r = row(&v["input"]);
        let key = evidence::event_key(&r);
        if v["output"].get("event_key_error").is_some() {
            assert!(key.is_err(), "{}", v["case"]);
            continue;
        }
        let key = key.unwrap_or_else(|e| panic!("{}: {e}", v["case"]));
        assert_eq!(
            json!([
                key.chain,
                key.network,
                key.native_namespace,
                key.native_event_id
            ]),
            v["output"]["event_key"],
            "{}",
            v["case"]
        );
        let raw = &v["input"]["raw_json"];
        let raw = if raw.is_null() {
            Py::Dict(vec![])
        } else if raw.is_string() || raw.get("$bytes").is_some() {
            evidence::raw_py(&r["raw_json"], "unparsed_text").unwrap()
        } else {
            decode(raw)
        };
        let result = evidence::evidence_snapshot(&r, &key, Some(raw));
        if v["output"].get("snapshot_error").is_some() {
            assert!(result.is_err(), "{}", v["case"]);
            continue;
        }
        let output = result.unwrap_or_else(|e| panic!("{}: {e}", v["case"]));
        for key in ["quantity_hash", "detail_hash"] {
            assert_eq!(output[key], v["output"][key], "{} {key}", v["case"]);
        }
        assert_eq!(
            output["payload_json"], v["output"]["detail_preimage"],
            "{}",
            v["case"]
        );
        let mut payload = codec::loads(output["payload_json"].as_str().unwrap()).unwrap();
        if let Py::Dict(ref mut values) = payload {
            values.retain(|(k,_)|!matches!(k,Key::Str(s) if ["fingerprint","occurred_at","confirmed_at","raw_json"].contains(&s.as_str().unwrap())));
        }
        assert_eq!(
            json!(codec::hash_py(&payload, false).unwrap().0),
            v["output"]["commitment_detail_hash"],
            "{}",
            v["case"]
        );
    }
}
#[test]
fn reconciled_observation_hashes_match_all_r0_vectors() {
    let vectors = fixtures("reconciled_observation");
    assert_eq!(vectors.len(), 25);
    for v in vectors {
        let r = row(&v["input"]["row"]);
        let proof = &v["input"]["proof"];
        let mut reconciliation = json!({});
        reconciliation[codec::text(&r["id"])] = proof.clone();
        let actual = evidence::canonical_input(&[r], &reconciliation, &Default::default()).unwrap();
        let observations = actual["events"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|e| e["legs"].as_array().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            observations.len(),
            v["output"]["observations"].as_array().unwrap().len()
        );
        for (o, expected) in observations
            .iter()
            .zip(v["output"]["observations"].as_array().unwrap())
        {
            assert_eq!(
                o["quantity_hash"], expected["quantity_hash"],
                "{}",
                v["case"]
            );
            assert_eq!(
                o["evidence_detail_hash"], expected["detail_hash"],
                "{}",
                v["case"]
            );
            assert_eq!(
                o["evidence_payload_json"], expected["detail_preimage"],
                "{}",
                v["case"]
            );
        }
    }
}

#[test]
fn borrowed_hash_preimages_match_python_values() {
    for text in [
        r#"{"é":{"🐍":"钱\n\t\u007f"},"a":[null,true,false,-0.0,1.0,1e-9,1e22]}"#,
        "[0,-1,9223372036854775807,-9223372036854775808,18446744073709551615]",
        "[18446744073709551616,-18446744073709551617,1e999,-1e999]",
        "[1e999,-1e999]",
        "[[],{},[{},[]]]",
    ] {
        let value: Value = serde_json::from_str(text).unwrap();
        for ascii in [false, true] {
            assert_eq!(
                codec::hash(&value, ascii),
                codec::hash_py(&codec::from_json(&value).unwrap(), ascii),
                "{text}:{ascii}"
            );
        }
    }
}
