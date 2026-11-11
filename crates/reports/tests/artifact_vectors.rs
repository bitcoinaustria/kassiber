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
/// The new transaction CSV against Python's: the same rows; renamed unit
/// columns (`amount_btc` for `amount`) and exact decimals for Python's
/// floats compare by value.
fn same_rows(actual: &str, expected: &str, name: &str) {
    // Records of RFC 4180 text: quoted cells may hold line breaks.
    let records = |text: &str| -> Vec<Vec<String>> {
        let mut records = Vec::new();
        let mut record = Vec::new();
        let mut cell = String::new();
        let mut quoted = false;
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match (c, quoted) {
                ('"', true) if chars.peek() == Some(&'"') => {
                    cell.push('"');
                    chars.next();
                }
                ('"', _) => quoted = !quoted,
                (',', false) => record.push(std::mem::take(&mut cell)),
                ('\r', false) => (),
                ('\n', false) => {
                    record.push(std::mem::take(&mut cell));
                    records.push(std::mem::take(&mut record));
                }
                _ => cell.push(c),
            }
        }
        records
    };
    // Python's file heads its columns with labels (`Occurred At`); they
    // name the same keys.
    let old: Vec<Vec<String>> = records(expected)
        .into_iter()
        .map(|r| {
            if r.first().is_some_and(|c| c == "Occurred At") {
                r.iter()
                    .map(|c| c.to_lowercase().replace(' ', "_"))
                    .collect()
            } else {
                r
            }
        })
        .collect();
    let start = old
        .iter()
        .position(|r| r.first().map(String::as_str) == Some("occurred_at"))
        .unwrap();
    let old_header = old[start].clone();
    let old_rows: Vec<Vec<String>> = old[start + 1..]
        .iter()
        .take_while(|r| !(r.len() == 1 && r[0].is_empty()))
        // Python writes a placeholder row where the new file has none.
        .filter(|r| !(r.len() == 1 && r[0] == "No rows in scope."))
        .cloned()
        .collect();
    let mut new = records(actual).into_iter();
    let new_header = new.next().unwrap();
    let new_rows: Vec<Vec<String>> = new.collect();
    assert_eq!(new_rows.len(), old_rows.len(), "{name}: rows");
    // The design's own column: whether the file is a draft list, which
    // Python said only in its section title.
    assert_eq!(new_header.last().map(String::as_str), Some("status"));
    let draft = old.iter().any(|r| {
        r.first()
            .is_some_and(|c| c.starts_with("Transactions (draft"))
    });
    for (new, old) in new_rows.iter().zip(&old_rows) {
        let status = if draft { "draft" } else { "final" };
        assert_eq!(new.last().map(String::as_str), Some(status), "{name}");
        for (column, value) in new_header.iter().zip(&new[..new.len() - 1]) {
            let source = column.trim_end_matches("_btc");
            let index = old_header.iter().position(|h| h == source).unwrap();
            let before = &old[index];
            let equal = value == before
                || matches!(
                    (value.parse::<f64>(), before.parse::<f64>()),
                    (Ok(a), Ok(b)) if a == b
                );
            assert!(equal, "{name}: {column} {value:?} != {before:?}");
        }
    }
}

/// The records of the design's CSV text, each ending in a line feed outside
/// quotes; quoted cells may hold line breaks.
fn csv_records(text: &str) -> usize {
    let mut quoted = false;
    let mut count = 0;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            '\n' if !quoted => count += 1,
            _ => (),
        }
    }
    count
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
            Ok(p) => {
                let directory = root.path().join("artifacts");
                // The full report's bundle is binary (a ZIP).
                let text =
                    |bytes: Vec<u8>| String::from_utf8(bytes).unwrap_or_else(|_| "<binary>".into());
                let published = if command != "reports.export-austrian-e1kv-csv" {
                    vec![kassiber_reports::artifact_publication::file(
                        &p,
                        &directory.join(&p.files[0].name),
                    )
                    .unwrap()]
                } else {
                    kassiber_reports::artifact_publication::bundle(&p, &directory).unwrap()
                };
                assert_eq!(published.len(), p.files.len());
                for (actual, file) in published.iter().zip(&p.files) {
                    assert_eq!(
                        actual.path,
                        directory.join(&file.name).canonicalize().unwrap()
                    );
                    assert_eq!(actual.bytes, file.bytes.len() as u64);
                    assert_eq!(actual.rows, file.rows);
                    assert_eq!(actual.sheet, file.sheet);
                    assert_eq!(std::fs::read(&actual.path).unwrap(), file.bytes);
                }
                dict([(
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
                                            ("content", s(text(f.bytes))),
                                        ])
                                    })
                                    .collect(),
                            ),
                        ),
                    ]),
                )])
            }
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
            // The CSV is the shared design's machine format: Python's
            // metadata but for the keys the layout names, Python's values.
            let mut metadata = actual["ok"]["metadata"].clone();
            let mut wanted = expected["ok"]["metadata"].clone();
            for key in ["sections", "sheets", "rows"] {
                if command != "reports.export-transactions-csv" || key != "rows" {
                    metadata.as_object_mut().unwrap().remove(key);
                    wanted.as_object_mut().unwrap().remove(key);
                }
            }
            assert_eq!(metadata, wanted, "{} metadata", case["name"]);
            let files = actual["ok"]["files"].as_array().unwrap();
            match command {
                "reports.export-transactions-csv" => same_rows(
                    files[0]["content"].as_str().unwrap(),
                    expected["ok"]["files"][0]["content"].as_str().unwrap(),
                    case["name"].as_str().unwrap(),
                ),
                "reports.export-csv" => {
                    let names: Vec<&str> =
                        files.iter().map(|f| f["name"].as_str().unwrap()).collect();
                    assert_eq!(names, ["report.zip"], "{}", case["name"]);
                }
                _ => {
                    let names: Vec<&str> =
                        files.iter().map(|f| f["name"].as_str().unwrap()).collect();
                    assert_eq!(names[0], "summary.csv", "{}", case["name"]);
                    assert_eq!(names.last(), Some(&"manifest.json"), "{}", case["name"]);
                    // Each table file's row count, in its metadata and in
                    // the manifest, is the number of records under its header.
                    let manifest: serde_json::Value =
                        serde_json::from_str(files.last().unwrap()["content"].as_str().unwrap())
                            .unwrap();
                    for (file, listed) in files[..files.len() - 1]
                        .iter()
                        .zip(manifest["files"].as_array().unwrap())
                    {
                        let records = csv_records(file["content"].as_str().unwrap()) - 1;
                        assert_eq!(listed["name"], file["name"], "{}", case["name"]);
                        assert_eq!(listed["rows"], records, "{} {}", case["name"], file["name"]);
                        if file["name"] != "summary.csv" {
                            assert_eq!(file["rows"], records, "{} {}", case["name"], file["name"]);
                        }
                    }
                }
            }
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

#[test]
fn standalone_transactions_csv_artifact_vectors() {
    check("reports.export-transactions-csv");
}
