use super::*;
use kassiber_store::rusqlite::{types::ValueRef, Connection};

fn canonical(v: &Value) -> String {
    let py = kassiber_pytext::loads(&v.to_string()).unwrap();
    kassiber_pyjson::dumps(
        &py,
        &kassiber_pyjson::Options {
            sort_keys: true,
            separators: Some((",".into(), ":".into())),
            ..Default::default()
        },
    )
    .unwrap()
}

fn tables(conn: &Connection) -> Vec<(String, Vec<Vec<String>>)> {
    let names=conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r| r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    assert_eq!(
        names.len(),
        crate::analysis_index::fixture::seed_table_count()
    );
    names
        .into_iter()
        .map(|name| {
            let mut stmt = conn.prepare(&format!("SELECT * FROM \"{name}\"")).unwrap();
            let count = stmt.column_count();
            let mut rows = stmt
                .query_map([], |r| {
                    Ok((0..count)
                        .map(|i| match r.get_ref(i).unwrap() {
                            ValueRef::Null => "null".into(),
                            ValueRef::Integer(n) => format!("i:{n}"),
                            ValueRef::Real(n) => format!("f:{n:?}"),
                            ValueRef::Text(v) => format!("s:{}", std::str::from_utf8(v).unwrap()),
                            ValueRef::Blob(v) => format!("b:{v:?}"),
                        })
                        .collect::<Vec<_>>())
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows.sort();
            (name, rows)
        })
        .collect()
}

fn run(group: &str) {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!(
        "../../../reports/tests/data/privacy_hygiene_seed.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
        .unwrap();
    let before = tables(&conn);
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../tests/data/analysis_features.json")).unwrap();
    let mut count = 0;
    for c in cases.iter().filter(|c| c["group"] == group) {
        count += 1;
        let source = c["source"].as_str().unwrap_or("stored_transaction");
        let subject = c["subject"].as_str();
        let actual = match c["operation"].as_str().unwrap() {
            "extract" => extract(
                &c["raw"],
                source,
                subject,
                c["signatures"].as_array().map_or(&[], Vec::as_slice),
            ),
            "normalize" => normalize(&c["snapshot"], source, subject).unwrap_or(Value::Null),
            "evaluate" => json!(evaluate(
                &c["snapshot"],
                c.get("collaboration").filter(|v| !v.is_null())
            )),
            "script" => json!(script_type(&c["value"])),
            "signature" => signature_encoding(&bytes(&c["value"]).unwrap(), c["schnorr"] == true)
                .unwrap_or(Value::Null),
            _ => panic!("unknown operation"),
        };
        assert_eq!(
            canonical(&actual),
            canonical(&c["expected"]),
            "{}",
            c["name"]
        );
    }
    assert!(count > 20, "empty fixture selection: {group}");
    assert_eq!(
        tables(&conn),
        before,
        "public-feature reads must preserve every seeded table"
    );
}

#[test]
fn public_feature_script_templates_match_python_without_payloads() {
    run("script");
}
#[test]
fn public_feature_signature_encodings_match_python_without_verification() {
    run("signature");
}
#[test]
fn public_feature_extraction_matches_python_and_all_112_tables() {
    run("extract");
}
#[test]
fn public_feature_cache_rebinding_matches_python_and_rejects_private_fields() {
    run("normalize");
}
#[test]
fn public_feature_rules_match_python_with_collaboration_and_missing_prerequisites() {
    run("rules");
}
