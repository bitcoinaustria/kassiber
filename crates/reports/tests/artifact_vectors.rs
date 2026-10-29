use kassiber_envelope::{dict, s};
use kassiber_pyjson::Value;
use kassiber_store::{rusqlite::Connection, Book};

struct Fixed;
impl kassiber_setup::Sources for Fixed {
    fn now(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Ok("2026-01-02T03:04:05Z".into())
    }
    fn uuid(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        panic!("CSV preparation requested an identity")
    }
}
fn check(command: &str) {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/artifact_csv.json")).unwrap();
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    let conn = book.connection();
    for case in cases.iter().filter(|c| c["command"] == command) {
        conn.execute_batch("SAVEPOINT oracle_fixture").unwrap();
        conn.execute_batch(include_str!("data/book.sql")).unwrap();
        conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
        let before = conn.total_changes();
        let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
        let result = kassiber_reports::artifact_csv::prepare(conn, command, &args, &Fixed);
        let actual = match result {
            Ok(p) => dict([(
                "ok",
                dict([
                    ("metadata", p.metadata),
                    (
                        "files",
                        Value::List(
                            p.files
                                .into_iter()
                                .map(|f| {
                                    dict([
                                        ("name", s(f.name)),
                                        ("sheet", s(f.sheet)),
                                        ("rows", Value::Int(f.rows.into())),
                                        ("content", s(String::from_utf8(f.bytes).unwrap())),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ]),
            )]),
            Err(e) => dict([(
                "error",
                dict([
                    ("code", s(e.code)),
                    ("message", s(e.message)),
                    ("hint", e.hint.map(s).unwrap_or(Value::Null)),
                    ("details", e.details.unwrap_or(Value::Null)),
                    ("retryable", Value::Bool(e.retryable)),
                ]),
            )]),
        };
        let actual: serde_json::Value =
            serde_json::from_str(&kassiber_pyjson::dumps(&actual, &Default::default()).unwrap())
                .unwrap();
        let expected = &case["expected"];
        if actual.get("ok").is_some() && expected.get("ok").is_some() {
            assert_eq!(
                actual["ok"]["metadata"], expected["ok"]["metadata"],
                "{} metadata",
                case["name"]
            );
            for (a, e) in actual["ok"]["files"]
                .as_array()
                .unwrap()
                .iter()
                .zip(expected["ok"]["files"].as_array().unwrap())
            {
                let a_content = a["content"].as_str().unwrap();
                let e_content = e["content"].as_str().unwrap();
                let index = a_content
                    .bytes()
                    .zip(e_content.bytes())
                    .position(|(a, e)| a != e)
                    .unwrap_or(a_content.len().min(e_content.len()));
                let sample = |v: &str| {
                    v.get(index.saturating_sub(120)..(index + 200).min(v.len()))
                        .unwrap_or("non-ascii boundary")
                        .to_owned()
                };
                assert!(
                    a_content == e_content,
                    "{} {}: byte {} actual {:?} expected {:?}",
                    case["name"],
                    a["name"],
                    index,
                    sample(a_content),
                    sample(e_content)
                );
                assert_eq!(a, e, "{}", case["name"]);
            }
            assert_eq!(actual, *expected, "{}", case["name"]);
        } else {
            assert_eq!(actual, *expected, "{}", case["name"]);
        }
        assert_eq!(
            before,
            conn.total_changes(),
            "preparation wrote accounting data"
        );
        conn.execute_batch("ROLLBACK TO oracle_fixture; RELEASE oracle_fixture")
            .unwrap();
    }
}
#[test]
fn full_report_csv_artifact_vectors() {
    check("reports.export-csv");
}
#[test]
fn austrian_bundle_csv_artifact_vectors() {
    check("reports.export-austrian-e1kv-csv");
}
