//! The year handoff workbook (EA / Jahresabschluss) in the shared report
//! design over the year handoff report; its checks stay live formulas.
use super::*;
use artifact_csv::{list, File, Prepared};
use kassiber_xlsx::Document;

pub(super) fn metadata() -> &'static Value {
    static META: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    META.get_or_init(|| kassiber_pytext::loads(include_str!("year_handoff_metadata.json")).unwrap())
}
/// The overview sheet rows, for the desktop export preview.
pub(super) fn overview(report: &Value) -> Vec<Value> {
    let profile = value(report, "profile");
    let mut rows = Vec::new();
    let mut add = |group: &str, field: &str, v: Value| {
        rows.push(dict([
            ("group", s(group)),
            ("field", s(field)),
            ("value", v),
        ]))
    };
    add(
        "Scope",
        "Workspace",
        value(value(report, "workspace"), "label").clone(),
    );
    add("Scope", "Book", value(profile, "label").clone());
    add("Scope", "Year", value(report, "year").clone());
    add("Scope", "Calendar", value(report, "calendar").clone());
    add(
        "Scope",
        "Period start (UTC)",
        value(report, "period_start").clone(),
    );
    add(
        "Scope",
        "Period end (UTC, exclusive)",
        value(report, "period_end").clone(),
    );
    for (field, key) in [
        ("Book currency", "fiat_currency"),
        ("Tax country", "tax_country"),
        ("Lot method", "gains_algorithm"),
    ] {
        add("Scope", field, value(profile, key).clone());
    }
    add(
        "Scope",
        "Wallets in book",
        Value::Int(list(value(report, "wallets")).len().into()),
    );
    let mut assets = std::collections::BTreeMap::new();
    for sheet in ["transactions", "balances", "journal"] {
        for row in list(value(report, sheet)) {
            assets
                .entry(text(row, "asset_id"))
                .or_insert_with(|| text(row, "asset"));
        }
    }
    for (id, label) in assets {
        add("Scope", "Asset", s(format!("{label} ({id})")));
    }
    add("Run", "Generated at", value(report, "generated_at").clone());
    add(
        "Run",
        "Journals processed at",
        value(profile, "last_processed_at").clone(),
    );
    add(
        "Run",
        "Kassiber version",
        value(report, "kassiber_version").clone(),
    );
    add(
        "Status",
        "Reconciliation",
        s(if truthy(value(report, "reconciled")) {
            "OK"
        } else {
            "DIFFERENCES"
        }),
    );
    add(
        "Status",
        "Quarantined transactions in or before the year",
        Value::Int(0.into()),
    );
    for warning in list(value(report, "warnings")) {
        add("Status", "Warning", warning.clone());
    }
    for (field, key) in [
        ("Transactions in the year", "transactions"),
        ("Excluded transactions in the year", "excluded"),
        ("Journal entries in the year", "journal"),
    ] {
        add(
            "Counts",
            field,
            Value::Int(list(value(report, key)).len().into()),
        );
    }
    let mut counts = std::collections::BTreeMap::<_, usize>::new();
    for row in list(value(report, "transactions")) {
        *counts.entry(text(row, "classification")).or_default() += 1;
    }
    for (classification, count) in counts {
        add("Classification", &classification, Value::Int(count.into()));
    }
    for note in list(value(metadata(), "notes")) {
        add("Notes", "", note.clone());
    }
    rows
}

pub(super) fn project(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<(Prepared, Document)> {
    let report = year_handoff::build(conn, args, sources)?;
    let year = int(&report, "year");
    let document = report_design::xlsx_document(&report_design::year_handoff(&report));
    let metadata = dict([
        ("year", Value::Int(year.into())),
        ("calendar", value(&report, "calendar").clone()),
        ("period_start", value(&report, "period_start").clone()),
        ("period_end", value(&report, "period_end").clone()),
        (
            "sheets",
            Value::List(
                document
                    .sheets
                    .iter()
                    .map(|sheet| s(sheet.name.as_str()))
                    .collect(),
            ),
        ),
        (
            "rows",
            dict(
                [
                    "wallets",
                    "transactions",
                    "journal",
                    "balances",
                    "reconciliation",
                    "excluded",
                ]
                .map(|k| (k, Value::Int(list(value(&report, k)).len().into()))),
            ),
        ),
        ("reconciled", value(&report, "reconciled").clone()),
        ("warnings", value(&report, "warnings").clone()),
    ]);
    Ok((
        Prepared {
            files: vec![],
            metadata,
            completion: Some(export_completion::Completion {
                kind: "year-handoff.xlsx".into(),
                year: Some(Value::Int(year.into())),
                end_year: None,
                scope: Value::Null,
            }),
        },
        document,
    ))
}

pub(super) fn prepare(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let (mut prepared, document) = project(conn, args, sources)?;
    let bytes = kassiber_xlsx::render(&document)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    prepared.files = vec![File {
        name: "report.xlsx".into(),
        sheet: String::new(),
        bytes,
        rows: 0,
    }];
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixed;
    impl kassiber_setup::Sources for Fixed {
        fn now(&self, _: &Connection) -> Result<String> {
            Ok("2026-01-02T03:04:05Z".into())
        }
        fn uuid(&self, _: &Connection) -> Result<String> {
            Ok("00000000-0000-4000-8000-000000000001".into())
        }
    }
    /// Equal on Python's fields, where one side keeps an exact decimal for a
    /// figure the other holds as a float: the decimal's nearest double is
    /// the float.
    fn same(actual: &serde_json::Value, expected: &serde_json::Value, at: &str) -> Option<String> {
        use serde_json::Value as J;
        match (actual, expected) {
            (J::String(text), J::Number(number)) | (J::Number(number), J::String(text)) => {
                let parsed = text.parse::<f64>().ok();
                (parsed != number.as_f64()).then(|| format!("{at}: {text} != {number}"))
            }
            // Every field Python's row has; Rust rows may carry fields no
            // sheet column reads.
            (J::Object(a), J::Object(b)) => b.iter().find_map(|(k, w)| match a.get(k) {
                Some(v) => same(v, w, &format!("{at}.{k}")),
                None => Some(format!("{at}: no {k}")),
            }),
            (J::Array(a), J::Array(b)) => {
                if a.len() != b.len() {
                    return Some(format!("{at}: {} rows != {}", a.len(), b.len()));
                }
                a.iter()
                    .zip(b)
                    .enumerate()
                    .find_map(|(i, (v, w))| same(v, w, &format!("{at}[{i}]")))
            }
            (J::Number(a), J::Number(b)) => {
                (a.as_f64() != b.as_f64()).then(|| format!("{at}: {a} != {b}"))
            }
            (a, b) => (a != b).then(|| format!("{at}: {a} != {b}")),
        }
    }
    /// Exact decimals as their text, as the oracle records them.
    fn decimals_as_text(v: &Value) -> Value {
        match v {
            Value::Decimal(d) => s(d.to_string()),
            Value::List(items) => Value::List(items.iter().map(decimals_as_text).collect()),
            Value::Dict(fields) => Value::Dict(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), decimals_as_text(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    fn normalize_numbers(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Number(number)
                if number.as_u64().is_none() && number.as_i64().is_none() =>
            {
                *number = serde_json::Number::from_f64(number.as_f64().unwrap()).unwrap();
            }
            serde_json::Value::Array(values) => values.iter_mut().for_each(normalize_numbers),
            serde_json::Value::Object(fields) => fields.values_mut().for_each(normalize_numbers),
            _ => (),
        }
    }
    #[test]
    fn year_handoff_report_metadata_and_checks_match_python() {
        let mut cases: serde_json::Value =
            serde_json::from_str(include_str!("../tests/data/year_handoff.json")).unwrap();
        normalize_numbers(&mut cases);
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        for case in cases.as_array().unwrap() {
            conn.execute_batch("SAVEPOINT xlsx_vector").unwrap();
            conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
            let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
            let result = project(conn, &args, &Fixed);
            if case["expected"].get("error").is_some() {
                let error = result.unwrap_err();
                let actual = dict([
                    ("code", s(error.code)),
                    ("message", s(error.message)),
                    ("hint", error.hint.map(s).unwrap_or(Value::Null)),
                    ("details", error.details.unwrap_or(Value::Null)),
                    ("retryable", Value::Bool(error.retryable)),
                ]);
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(
                        &kassiber_pyjson::dumps(&actual, &Default::default()).unwrap()
                    )
                    .unwrap(),
                    case["expected"]["error"],
                    "{} refusal",
                    case["name"]
                );
            } else {
                // Python's metadata but for the design's sheet names, and
                // Python's report rows exactly; the workbook keeps the checks
                // as live formulas.
                let (prepared, document) =
                    result.unwrap_or_else(|e| panic!("{}: {} {}", case["name"], e.code, e.message));
                let mut metadata: serde_json::Value = serde_json::from_str(
                    &kassiber_pyjson::dumps(&prepared.metadata, &Default::default()).unwrap(),
                )
                .unwrap();
                let mut expected = case["expected"]["metadata"].clone();
                metadata.as_object_mut().unwrap().remove("sheets");
                expected.as_object_mut().unwrap().remove("sheets");
                assert_eq!(metadata, expected, "{} metadata", case["name"]);
                let report = year_handoff::build(conn, &args, &Fixed).unwrap();
                let mut rows = dict::<&str>([]);
                for key in [
                    "wallets",
                    "transactions",
                    "journal",
                    "balances",
                    "roll_forward",
                    "reconciliation",
                    "excluded",
                    "reconciled",
                    "warnings",
                ] {
                    replace(&mut rows, key, decimals_as_text(value(&report, key)));
                }
                let rows: serde_json::Value = serde_json::from_str(
                    &kassiber_pyjson::dumps(&rows, &Default::default()).unwrap(),
                )
                .unwrap();
                let mut expected = case["expected"]["report"].clone();
                normalize_numbers(&mut expected);
                if let Some(difference) = same(&rows, &expected, "report") {
                    panic!("{}: {difference}", case["name"]);
                }
                let formulas = document
                    .sheets
                    .iter()
                    .flat_map(|s| &s.operations)
                    .filter(|op| {
                        matches!(
                            op,
                            kassiber_xlsx::Operation::Cell {
                                value: kassiber_xlsx::Cell::Formula { .. },
                                ..
                            }
                        )
                    })
                    .count();
                let expected_formulas = list(value(&report, "roll_forward")).len() * 2
                    + list(value(&report, "reconciliation")).len() * 4;
                assert_eq!(formulas, expected_formulas, "{} formulas", case["name"]);
            }
            conn.execute_batch("ROLLBACK TO xlsx_vector; RELEASE xlsx_vector")
                .unwrap();
        }
    }
    /// The workbook's satoshi columns hold Python's numbers, never the text
    /// `Decimal('50000000')`: whole satoshis as counts, fractions (msat)
    /// as exact decimals.
    #[test]
    fn year_handoff_satoshi_cells_are_pythons_numbers() {
        use crate::report_design::{Block, Cell};
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../tests/data/year_handoff.json")).unwrap();
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        let (mut whole, mut fractional) = (0, 0);
        for case in cases.as_array().unwrap() {
            let Some(expected) = case["expected"].get("report") else {
                continue;
            };
            conn.execute_batch("SAVEPOINT sats_vector").unwrap();
            conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
            let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
            let report = year_handoff::build(conn, &args, &Fixed).unwrap();
            let document = report_design::year_handoff(&report);
            let table = document
                .sections
                .iter()
                .flat_map(|s| &s.blocks)
                .find_map(|b| match b {
                    Block::Table(t) if t.id == "transactions" => Some(t),
                    _ => None,
                });
            let rows = expected["transactions"].as_array().unwrap();
            let Some(table) = table else {
                // No rows in the year: the sheet is a note.
                assert!(rows.is_empty(), "{}", case["name"]);
                conn.execute_batch("ROLLBACK TO sats_vector; RELEASE sats_vector")
                    .unwrap();
                continue;
            };
            assert_eq!(table.rows.len(), rows.len(), "{}", case["name"]);
            for key in ["amount_sats", "fee_sats", "net_change_sats"] {
                let col = table.columns.iter().position(|c| c.key == key).unwrap();
                for (cells, row) in table.rows.iter().zip(rows) {
                    let at = format!("{} {key}", case["name"]);
                    match (&cells[col], &row[key]) {
                        (Cell::Empty, serde_json::Value::Null) => {}
                        (Cell::Count(n), serde_json::Value::String(python)) => {
                            assert_eq!(n.to_string(), *python, "{at}");
                            whole += 1;
                        }
                        (Cell::Btc(d), serde_json::Value::String(python)) => {
                            assert!(python.contains('.'), "{at}: {python}");
                            assert_eq!(d.to_string(), *python, "{at}");
                            fractional += 1;
                        }
                        (cell, python) => panic!("{at}: {cell:?} for Python's {python}"),
                    }
                }
            }
            conn.execute_batch("ROLLBACK TO sats_vector; RELEASE sats_vector")
                .unwrap();
        }
        assert!(
            whole > 0 && fractional > 0,
            "{whole} whole, {fractional} fractional"
        );
    }
    #[test]
    fn year_handoff_managed_publication_saves_actual_hash_and_retains_freshness() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        let args = dict([("year", Value::Int(2024.into()))]);
        let reports = root.path().join("reports");
        std::fs::create_dir(&reports).unwrap();
        let reserved = reports.join("kassiber-year-handoff-2024-20260102-030405.xlsx");
        std::fs::write(&reserved, b"previous artifact").unwrap();
        let result = export_request::execute(
            conn,
            "ui.reports.export_year_handoff",
            &args,
            root.path(),
            &Fixed,
        )
        .unwrap();
        assert_eq!(text(&result, "format"), "xlsx");
        assert_eq!(text(&result, "scope"), "year_handoff");
        assert_eq!(
            text(&result, "filename"),
            "kassiber-year-handoff-2024-20260102-030405-2.xlsx"
        );
        assert_eq!(std::fs::read(&reserved).unwrap(), b"previous artifact");
        let target = std::path::PathBuf::from(text(&result, "file"));
        assert_eq!(
            int(&result, "bytes"),
            std::fs::metadata(&target).unwrap().len() as i64
        );
        let snapshot = value(&result, "report_snapshot");
        assert_eq!(text(snapshot, "report_state"), "saved");
        assert_eq!(text(snapshot, "report_kind"), "year-handoff.xlsx");
        assert_eq!(int(snapshot, "period_start_year"), 2024);
        assert_eq!(int(snapshot, "period_end_year"), 2024);
        assert_eq!(
            text(snapshot, "content_sha256"),
            export_completion::content_sha256(&[target]).unwrap()
        );
        assert!(rows(
            conn,
            "SELECT id FROM filed_report_snapshots WHERE report_state='filed'",
            []
        )
        .unwrap()
        .is_empty());
        for args in [
            dict::<&str>([]),
            dict([("year", Value::Bool(true))]),
            dict([("year", s("2024"))]),
            dict([("year", Value::Float(2024.0))]),
        ] {
            let error = export_request::execute(
                conn,
                "ui.reports.export_year_handoff",
                &args,
                root.path(),
                &Fixed,
            )
            .unwrap_err();
            assert_eq!(
                error.message,
                "ui.reports.export_year_handoff requires an integer year"
            );
        }
        let unknown = dict([("year", Value::Int(2024.into())), ("profile", s("p"))]);
        assert_eq!(
            export_request::execute(
                conn,
                "ui.reports.export_year_handoff",
                &unknown,
                root.path(),
                &Fixed
            )
            .unwrap_err()
            .message,
            "ui.reports.export_year_handoff received unsupported arguments"
        );
        conn.execute_batch("INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,blocking_reports,updated_at) VALUES('p','a','wallet','Cold','failed',1,'fixed'); UPDATE profiles SET last_processed_input_version=-1 WHERE id='p';").unwrap();
        for command in [
            "ui.reports.export_year_handoff",
            "reports.export-year-handoff",
        ] {
            assert_eq!(
                export_request::execute(conn, command, &args, root.path(), &Fixed)
                    .unwrap_err()
                    .code,
                "report_freshness_blocked"
            );
        }
    }
}
