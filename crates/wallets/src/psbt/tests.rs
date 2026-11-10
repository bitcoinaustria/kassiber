use super::*;
use kassiber_store::rusqlite::{types::ValueRef, Connection};

fn canonical(v: &Value) -> String {
    kassiber_pyjson::dumps(
        &kassiber_pytext::loads(&v.to_string()).unwrap(),
        &kassiber_pyjson::Options {
            sort_keys: true,
            separators: Some((",".into(), ":".into())),
            ..Default::default()
        },
    )
    .unwrap()
}
fn tables(conn: &Connection) -> Vec<(String, Vec<Vec<String>>)> {
    let names = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r| r.get::<_,String>(0)).unwrap().collect::<std::result::Result<Vec<_>,_>>().unwrap();
    assert_eq!(
        names.len(),
        crate::analysis_index::fixture::seed_table_count()
    );
    names
        .into_iter()
        .map(|name| {
            let mut statement = conn.prepare(&format!("SELECT * FROM \"{name}\"")).unwrap();
            let count = statement.column_count();
            let mut rows = statement
                .query_map([], |r| {
                    Ok((0..count)
                        .map(|i| match r.get_ref(i).unwrap() {
                            ValueRef::Null => "null".into(),
                            ValueRef::Integer(v) => format!("i:{v}"),
                            ValueRef::Real(v) => format!("f:{v:?}"),
                            ValueRef::Text(v) => format!("s:{}", std::str::from_utf8(v).unwrap()),
                            ValueRef::Blob(v) => format!("b:{v:?}"),
                        })
                        .collect::<Vec<_>>())
                })
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap();
            rows.sort();
            (name, rows)
        })
        .collect()
}
fn result(case: &Value) -> Value {
    use bitcoin::hex::FromHex;
    let data = case["binary_hex"]
        .as_str()
        .map(|v| Vec::<u8>::from_hex(v).unwrap());
    let network = case["network"].as_str().unwrap_or("main");
    let subject = case["subject"].as_str().unwrap_or("psbt:transaction");
    let result = match (case["operation"].as_str().unwrap(), data) {
        ("structure", Some(v)) => decode_structure(&v),
        ("structure", None) => decode_text_structure(case["text"].as_str().unwrap()),
        (_, Some(v)) => analyze(&v, network, subject),
        (_, None) => analyze_text(case["text"].as_str().unwrap(), network, subject),
    };
    match result {
        Ok(v) => json!({"ok":v}),
        Err(e) => {
            json!({"error":{"code":e.code,"message":e.message,"hint":e.hint,"details":e.details.map(|v| serde_json::from_str::<Value>(&kassiber_pyjson::dumps(&v,&Default::default()).unwrap()).unwrap()),"retryable":e.retryable}})
        }
    }
}
fn run(groups: &[&str]) {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!(
        "../../../reports/tests/data/privacy_hygiene_seed.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
        .unwrap();
    let before = tables(&conn);
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../tests/data/psbt.json")).unwrap();
    let mut count = 0;
    for case in cases
        .iter()
        .filter(|c| groups.contains(&c["group"].as_str().unwrap()))
    {
        count += 1;
        assert_eq!(
            canonical(&result(case)),
            canonical(&case["expected"]),
            "{}",
            case["name"]
        );
    }
    assert!(count > 20);
    assert_eq!(
        tables(&conn),
        before,
        "PSBT primitives preserve every seeded table"
    );
}
#[test]
fn psbt_bip174_bip370_vectors_match_python_and_all_112_tables() {
    run(&["bip"]);
}
#[test]
fn psbt_framing_and_transport_errors_match_python_without_payloads() {
    run(&["framing"]);
}
#[test]
fn psbt_v2_locktime_sequences_and_fields_match_python() {
    run(&["v2"]);
}
#[test]
fn psbt_utxo_hashes_amounts_and_script_bindings_match_python() {
    run(&["utxo"]);
}
#[test]
fn psbt_redacted_analysis_signatures_and_final_sizes_match_python() {
    run(&["analysis"]);
}

#[test]
fn psbt_size_count_and_map_limits_refuse_before_unbounded_parsing() {
    fn reason<T: std::fmt::Debug>(v: Result<T>) -> String {
        let error = v.unwrap_err();
        kassiber_envelope::get(error.details.as_ref().unwrap(), "reason")
            .and_then(|v| match v {
                kassiber_pyjson::Value::Str(s) => s.as_str(),
                _ => None,
            })
            .unwrap()
            .to_owned()
    }
    let mut payload = b"psbt\xff".to_vec();
    payload.resize(MAX_PSBT_BYTES + 1, 0);
    assert_eq!(reason(decode_structure(&payload)), "payload_limit");
    let encoded = hex(&payload);
    assert_eq!(reason(decode_text_structure(&encoded)), "payload_limit");
    assert_eq!(
        reason(decode_structure(encoded.as_bytes())),
        "payload_limit"
    );
    assert_eq!(
        reason(decode_structure(&vec![0xff; MAX_PSBT_BYTES * 3 + 1])),
        "payload_limit"
    );
    assert_eq!(
        reason(decode_text_structure(&"a".repeat(MAX_PSBT_BYTES * 3 + 1))),
        "payload_limit"
    );
    assert_eq!(
        reason(decode_structure(b"\xff")),
        "invalid_binary_or_text_encoding"
    );
    let mut payload = b"psbt\xff".to_vec();
    for i in 0..10001u32 {
        let mut key = vec![0xfc];
        key.extend_from_slice(&i.to_le_bytes());
        variable(&key, &mut payload);
        variable(b"", &mut payload);
    }
    payload.push(0);
    assert_eq!(reason(decode_structure(&payload)), "map_entry_limit");
    let mut tx = 2u32.to_le_bytes().to_vec();
    compact(MAX_LEGS + 1, &mut tx);
    let mut payload = b"psbt\xff\x01\x00".to_vec();
    variable(&tx, &mut payload);
    payload.push(0);
    assert_eq!(
        reason(decode_structure(&payload)),
        "invalid_or_excessive_leg_count"
    );
}
