use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::{rusqlite::Connection, Book};

fn dump(conn: &Connection) -> Vec<(String, Vec<String>)> {
    let names: Vec<String> = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap()
        .query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect();
    assert_eq!(names, kassiber_store::table_names());
    names
        .into_iter()
        .map(|name| {
            let mut statement = conn
                .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))
                .unwrap();
            let columns = statement.column_count();
            let mut records: Vec<_> = statement
                .query_map([], |row| {
                    Ok((0..columns)
                        .map(|column| format!("{:?}", row.get_ref(column).unwrap()))
                        .collect::<Vec<_>>())
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            records.sort();
            (
                name,
                records
                    .into_iter()
                    .map(|record| format!("{record:?}"))
                    .collect(),
            )
        })
        .collect()
}

fn outcome(result: Result<Value, AppError>) -> Value {
    match result {
        Ok(data) => dict([("ok", data)]),
        Err(error) => dict([(
            "error",
            dict([
                ("code", s(error.code)),
                ("message", s(error.message)),
                ("hint", error.hint.map(s).unwrap_or(Value::Null)),
                ("details", error.details.unwrap_or(Value::Null)),
                ("retryable", Value::Bool(error.retryable)),
            ]),
        )]),
    }
}

fn replay(witnesses: bool) {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/source_funds_review.json")).unwrap();
    assert!(cases.len() >= 100);
    for passphrase in [None, Some("synthetic-source-funds-review-passphrase")] {
        let root = tempfile::tempdir().unwrap();
        let book = Book::create(&root.path().join("data"), passphrase).unwrap();
        let conn = book.connection();
        for case in &cases {
            let name = case["name"].as_str().unwrap();
            if witnesses
                && ![
                    "standard-in",
                    "standard-excluded",
                    "full-out",
                    "attestation",
                    "missing-evidence",
                    "link-truncation",
                    "evidence-truncation",
                    "eligible-custody-current",
                    "eligible-structural-current",
                    "eligible-structural-with-manual",
                    "eligible-structural-current-strong",
                    "eligible-custody-current-chain",
                    "eligible-custody-current-from-allocation",
                    "eligible-custody-current-fractional-allocation",
                    "eligible-custody-current-fractional-from",
                    "eligible-beyond-inspection",
                    "request-invalid",
                    "request-no-missing-input",
                    "request-truncated-links",
                    "request-truncated-evidence",
                    "request-stale-evidence",
                ]
                .contains(&name)
                && !name.starts_with("fingerprint-")
                && !name.starts_with("explanation-")
                && !name.starts_with("args-")
            {
                continue;
            }

            conn.execute_batch("SAVEPOINT review_case").unwrap();
            conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
            let before = dump(conn);
            let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
            let actual = outcome(kassiber_reports::execute(
                conn,
                case["command"].as_str().unwrap(),
                &args,
            ));
            let actual = dumps(&actual, &Options::new().compact()).unwrap();
            let variants = case["expected_variants"].as_array().unwrap();
            if !variants.iter().any(|v| v.as_str() == Some(&actual)) {
                let expected = variants[0].as_str().unwrap();
                let index = actual
                    .bytes()
                    .zip(expected.bytes())
                    .position(|(a, b)| a != b)
                    .unwrap_or(actual.len().min(expected.len()));
                panic!(
                    "{} at byte {}: actual {:?}, expected {:?}",
                    case["name"],
                    index,
                    &actual[index.saturating_sub(80)..actual.len().min(index + 240)],
                    &expected[index.saturating_sub(80)..expected.len().min(index + 240)]
                );
            }
            assert_eq!(dump(conn), before, "{}", case["name"]);
            conn.execute_batch("ROLLBACK TO review_case;RELEASE review_case")
                .unwrap();
        }
    }
}

#[test]
fn source_funds_review_preserves_fingerprints_handoffs_eligibility_and_all_tables() {
    replay(false);
}

#[test]
fn source_funds_review_mutation_witnesses() {
    replay(true);
}

#[test]
fn source_funds_review_recipe_file_preserves_encodings_surrogates_and_limits() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/source_funds_review.json")).unwrap();
    let case = cases
        .iter()
        .find(|c| {
            c["args_json"].as_str()
                == Some(r#"{"target_transaction":"in","planned_note":"\ud800"}"#)
        })
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    let conn = book.connection();
    conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
    let before = dump(conn);
    let mut points =
        r#"{"data":{"recipe":{"planned_note":""#.chars().map(u32::from).collect::<Vec<_>>();
    points.push(0xd800);
    points.extend(r#""}}}"#.chars().map(u32::from));
    let mut encodings = vec![br#"{"data":{"recipe":{"planned_note":"\ud800"}}}"#.to_vec()];
    for little in [false, true] {
        for width in [2, 4] {
            let mut bytes = Vec::new();
            if width == 2 {
                bytes.extend(if little {
                    vec![0xff, 0xfe]
                } else {
                    vec![0xfe, 0xff]
                });
            } else {
                bytes.extend(if little {
                    vec![0xff, 0xfe, 0, 0]
                } else {
                    vec![0, 0, 0xfe, 0xff]
                });
            }
            for &point in &points {
                if width == 2 {
                    bytes.extend(if little {
                        (point as u16).to_le_bytes()
                    } else {
                        (point as u16).to_be_bytes()
                    });
                } else {
                    bytes.extend(if little {
                        point.to_le_bytes()
                    } else {
                        point.to_be_bytes()
                    });
                }
            }
            encodings.push(bytes);
        }
    }
    let path = root.path().join("recipe.json");
    let args = dict([
        ("target_transaction", s("in")),
        ("recipe_file", s(path.to_string_lossy().as_ref())),
    ]);
    for bytes in encodings {
        std::fs::write(&path, bytes).unwrap();
        let actual = dumps(
            &outcome(kassiber_reports::execute(
                conn,
                "source-funds.review-context",
                &args,
            )),
            &Options::new().compact(),
        )
        .unwrap();
        assert!(
            case["expected_variants"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_str() == Some(&actual)),
            "{actual}"
        );
        assert_eq!(dump(conn), before);
    }
    #[cfg(unix)]
    {
        let home = std::fs::canonicalize(std::env::var_os("HOME").unwrap()).unwrap();
        let parents = home
            .components()
            .filter(|c| matches!(c, std::path::Component::Normal(_)))
            .count();
        let selected = format!(
            "~/{}{}",
            "../".repeat(parents),
            path.strip_prefix("/").unwrap().display()
        );
        let args = dict([
            ("target_transaction", s("in")),
            ("recipe_file", s(selected)),
        ]);
        let actual = dumps(
            &outcome(kassiber_reports::execute(
                conn,
                "source-funds.review-context",
                &args,
            )),
            &Options::new().compact(),
        )
        .unwrap();
        assert!(
            case["expected_variants"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_str() == Some(&actual)),
            "{actual}"
        );
        assert_eq!(dump(conn), before);
    }
    std::fs::write(&path, vec![b' '; 1_000_001]).unwrap();
    let error = kassiber_reports::execute(conn, "source-funds.review-context", &args).unwrap_err();
    assert_eq!(error.code, "validation");
    assert_eq!(error.message, "Review input file is too large");
    assert_eq!(dump(conn), before);
}
