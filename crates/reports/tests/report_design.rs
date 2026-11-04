//! The shared report design's summary prototype: the data handed to the
//! renderers equals Python's summary model, and the PDF, XLSX and CSV bundle
//! are deterministic and machine-clean.
use kassiber_pyjson::Value;
use kassiber_reports::report_design;
use kassiber_store::{rusqlite::Connection, Book};

struct Fixed;
impl kassiber_setup::Sources for Fixed {
    fn now(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Ok("2026-01-02T03:04:05Z".into())
    }
    fn uuid(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        panic!("rendering minted an identity")
    }
}

fn encode(v: &Value) -> String {
    kassiber_pyjson::dumps(v, &Default::default()).unwrap()
}
fn get<'a>(v: &'a Value, key: &str) -> &'a Value {
    kassiber_envelope::get(v, key).unwrap_or(&Value::Null)
}

fn check_csv(files: &[(String, Vec<u8>)], name: &str) {
    let manifest: serde_json::Value =
        serde_json::from_slice(&files.last().unwrap().1).expect("manifest.json");
    assert_eq!(files.last().unwrap().0, "manifest.json");
    assert_eq!(manifest["schema"], "kassiber.report/1");
    let listed = manifest["files"].as_array().unwrap();
    assert_eq!(listed.len(), files.len() - 1, "{name}");
    for ((file, bytes), entry) in files.iter().zip(listed) {
        assert_eq!(entry["name"], file.as_str());
        let digest = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(bytes));
        assert_eq!(entry["sha256"], digest.as_str(), "{name}: {file}");
        let text = String::from_utf8(bytes.clone()).expect("UTF-8");
        let mut lines = text.lines();
        let header: Vec<&str> = lines.next().unwrap().split(',').collect();
        for column in &header {
            assert!(
                column
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{name}: {file}: header {column}"
            );
        }
        assert_eq!(
            entry["rows"].as_u64().unwrap() as usize,
            lines.clone().count()
        );
        for line in lines {
            for field in line.split(',') {
                // Plain decimals: no exponent, no grouping, ASCII minus.
                if field
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit() || c == '-')
                {
                    assert!(!field.contains(['e', 'E', ',']), "{name}: {file}: {field}");
                }
            }
        }
        for (i, column) in header.iter().enumerate() {
            if column.ends_with("_msat") {
                for line in text.lines().skip(1) {
                    let field = line.split(',').nth(i).unwrap_or("");
                    assert!(
                        field.is_empty() || field.parse::<i64>().is_ok(),
                        "{name}: {file}: {field}"
                    );
                }
            }
        }
    }
}

#[test]
fn summary_model_matches_python_and_renders_deterministically() {
    let cases = kassiber_pytext::loads(include_str!("data/summary_model.json")).unwrap();
    let Value::List(cases) = cases else { panic!() };
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    let conn = book.connection();
    let mut rendered = 0;
    for case in &cases {
        let name = kassiber_envelope::py_str(get(case, "name"));
        conn.execute_batch("SAVEPOINT oracle_fixture").unwrap();
        conn.execute_batch(include_str!("data/book.sql")).unwrap();
        conn.execute_batch(&kassiber_envelope::py_str(get(case, "sql")))
            .unwrap();
        let args =
            kassiber_pytext::loads(&kassiber_envelope::py_str(get(case, "args_json"))).unwrap();
        let expected = get(case, "expected");
        match report_design::summary_model(conn, &args, &Fixed) {
            Ok(model) => {
                let ok = get(expected, "ok");
                assert!(
                    !matches!(ok, Value::Null),
                    "{name}: Python failed with {}",
                    encode(expected)
                );
                assert_eq!(
                    encode(get(&model, "summary")),
                    encode(ok),
                    "{name}: summary model"
                );
                let document = report_design::summary(&model);
                let (pdf, pages) = report_design::pdf(&document).unwrap();
                assert!(pages >= 1 && pdf.starts_with(b"%PDF-"), "{name}");
                assert_eq!(
                    report_design::pdf(&document).unwrap().0,
                    pdf,
                    "{name}: PDF bytes"
                );
                let xlsx = report_design::xlsx(&document).unwrap();
                assert_eq!(
                    report_design::xlsx(&document).unwrap(),
                    xlsx,
                    "{name}: XLSX bytes"
                );
                let csv = report_design::csv(&document).unwrap();
                assert_eq!(
                    report_design::csv(&document).unwrap(),
                    csv,
                    "{name}: CSV bytes"
                );
                check_csv(&csv, &name);
                rendered += 1;
            }
            Err(e) => {
                let error = get(expected, "error");
                assert!(
                    !matches!(error, Value::Null),
                    "{name}: Rust failed with {e:?}"
                );
                assert_eq!(
                    e.code,
                    kassiber_envelope::py_str(get(error, "code")),
                    "{name}"
                );
                assert_eq!(
                    e.message,
                    kassiber_envelope::py_str(get(error, "message")),
                    "{name}"
                );
            }
        }
        conn.execute_batch("ROLLBACK TO oracle_fixture; RELEASE oracle_fixture")
            .unwrap();
    }
    assert!(rendered >= 15, "{rendered}");
}

#[test]
fn display_rounding_and_csv_text_come_from_one_exact_value() {
    use report_design::Cell;
    let value = |raw: &str| raw.parse::<kassiber_pydecimal::Decimal>().unwrap();
    assert_eq!(Cell::Fiat(value("-1234.565")).display(), "\u{2212}1,234.56");
    assert_eq!(Cell::Fiat(value("-1234.565")).csv(), "-1234.565");
    assert_eq!(Cell::Fiat(value("-0.001")).display(), "0.00");
    assert_eq!(Cell::Btc(value("1.6473E-7")).display(), "0.00000016");
    assert_eq!(Cell::Btc(value("1.6473E-7")).csv(), "0.00000016473");
    assert_eq!(Cell::Percent(value("59.25")).display(), "59.2%");
    assert_eq!(
        report_design::msat_of(&value("0.00000016473")),
        Cell::Msat(16473.into())
    );
    assert_eq!(
        report_design::msat_of(&value("0.000000000001")),
        Cell::Empty
    );
    assert_eq!(
        report_design::from_float(&Value::Float(1.6473e-07)),
        value("0.00000016473")
    );
}
