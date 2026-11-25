//! A source that does not reach the report period's end marks an export
//! provisional in every rendering (PDF, XLSX, CSV bundle) while the export
//! still proceeds and no figure changes
//! (docs/reference/desktop.md#sources-live-sync-and-file-snapshots).
use kassiber_pyjson::Value;
use kassiber_reports::report_design::{self, Document};
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

/// Cold becomes a file snapshot whose last import ends on 31 Dec 2024,
/// resting on the rows that run created: behind for any later period end.
const LAGGING: &str = "
    UPDATE wallets SET config_json = '{\"source_file\": \"cold.csv\"}' WHERE id = 'cold';
    INSERT INTO import_batches(id,workspace_id,profile_id,wallet_id,source_format,imported_at,
        rows_inserted,first_row_at,last_row_at,rows_read)
    VALUES('run','ws','p','cold','csv','2025-01-05T00:00:00Z',2,
        '2024-01-01T00:00:00Z','2024-12-31T23:30:00Z',2);
    INSERT INTO import_batch_transactions(batch_id,transaction_id) VALUES('run','in'),('run','out');";

const ENGLISH: [&str; 3] = [
    "Provisional: 1 source(s) do not cover the period through",
    "Behind",
    "Import a newer export",
];

fn encode(v: &Value) -> String {
    kassiber_pyjson::dumps(v, &Default::default()).unwrap()
}

fn get<'a>(v: &'a Value, key: &str) -> &'a Value {
    kassiber_envelope::get(v, key).unwrap_or(&Value::Null)
}

fn without(model: &Value, key: &str) -> Value {
    let mut model = model.clone();
    if let Value::Dict(fields) = &mut model {
        fields.retain(|(k, _)| *k != key.into());
    }
    model
}

/// The case's (args, sql) from a frozen model fixture.
fn case(cases: &str, name: &str) -> (Value, String) {
    let Value::List(cases) = kassiber_pytext::loads(cases).unwrap() else {
        panic!()
    };
    let case = cases
        .iter()
        .find(|case| kassiber_envelope::py_str(get(case, "name")) == name)
        .unwrap_or_else(|| panic!("no case {name}"));
    (
        kassiber_pytext::loads(&kassiber_envelope::py_str(get(case, "args_json"))).unwrap(),
        kassiber_envelope::py_str(get(case, "sql")),
    )
}

/// Every rendering of `document` states the provisional note and lists Cold.
/// `words`: the note's opening, the state and the action in the report's language.
fn check_renderings(document: &Document, name: &str, words: [&str; 3]) {
    let notice = document
        .coverage_notice()
        .unwrap_or_else(|| panic!("{name}: no coverage notice"))
        .to_owned();
    assert!(notice.starts_with(words[0]), "{name}: {notice}");
    let rendered = report_design::rendered_pdf(document).unwrap();
    assert!(rendered.bytes.starts_with(b"%PDF-"), "{name}");
    assert_eq!(rendered.off_page, Vec::<String>::new(), "{name}");
    let xlsx = serde_json::to_string(&report_design::xlsx_document(document)).unwrap();
    assert!(
        xlsx.contains(notice.as_str()),
        "{name}: XLSX lacks the note"
    );
    assert!(xlsx.contains(words[2]), "{name}: XLSX lacks the sources");
    let csv = report_design::csv(document).unwrap();
    let file = |wanted: &str| {
        let (_, bytes) = csv
            .iter()
            .find(|(file, _)| file == wanted)
            .unwrap_or_else(|| panic!("{name}: no {wanted}"));
        String::from_utf8(bytes.clone()).unwrap()
    };
    assert_eq!(
        file("source_coverage.csv"),
        format!(
            "wallet,state,covered_through,action\nCold,{},2024-12-31,{}\n",
            words[1], words[2]
        ),
        "{name}"
    );
    assert!(
        file("summary.csv").contains("source_coverage,notice,"),
        "{name}: CSV summary lacks the note"
    );
    let manifest: serde_json::Value = serde_json::from_str(&file("manifest.json")).unwrap();
    assert_eq!(
        manifest["source_coverage_notice"],
        notice.as_str(),
        "{name}"
    );
}

/// A covered book's rendering carries none of it.
fn check_quiet(document: &Document, name: &str) {
    assert_eq!(document.coverage_notice(), None, "{name}");
    let csv = report_design::csv(document).unwrap();
    assert!(
        csv.iter().all(|(file, _)| file != "source_coverage.csv"),
        "{name}"
    );
}

/// Runs `build` on the case's book as Python froze it and again with Cold
/// lagging; returns both models.
fn both(conn: &Connection, sql: &str, build: impl Fn(&Connection) -> Value) -> (Value, Value) {
    conn.execute_batch("SAVEPOINT coverage_case").unwrap();
    conn.execute_batch(include_str!("data/book.sql")).unwrap();
    conn.execute_batch(sql).unwrap();
    let covered = build(conn);
    conn.execute_batch(LAGGING).unwrap();
    let lagging = build(conn);
    conn.execute_batch("ROLLBACK TO coverage_case; RELEASE coverage_case")
        .unwrap();
    (covered, lagging)
}

#[test]
fn a_lagging_source_marks_every_export_provisional_without_changing_a_figure() {
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    let conn = book.connection();

    // Summary: the period ends at its timeframe end.
    let (args, sql) = case(include_str!("data/summary_model.json"), "default-period");
    let (covered, lagging) = both(conn, &sql, |conn| {
        report_design::summary_model(conn, &args, &Fixed).unwrap()
    });
    let (covered_model, lagging_model) = (covered, lagging);
    let (covered, lagging) = (
        get(&covered_model, "summary"),
        get(&lagging_model, "summary"),
    );
    assert_eq!(get(covered, "source_coverage"), &Value::Null);
    assert_eq!(
        encode(get(lagging, "source_coverage")),
        r#"{"period_end": "2026-01-02", "sources": [{"wallet": "Cold", "source_mode": "file", "freshness_state": "behind", "covered_through": "2024-12-31", "action": "import_newer_file"}]}"#
    );
    assert_eq!(
        encode(&without(lagging, "source_coverage")),
        encode(covered)
    );
    check_quiet(&report_design::summary(&covered_model), "summary");
    check_renderings(&report_design::summary(&lagging_model), "summary", ENGLISH);

    // Full report: the period ends when it is generated.
    let (args, sql) = case(include_str!("data/full_pdf.json"), "balance");
    let (covered, lagging) = both(conn, &sql, |conn| {
        let (workspace, profile) = kassiber_setup::references::scope(conn, &args).unwrap();
        kassiber_reports::full_pdf::model(conn, &workspace, &profile, &args, &Fixed).unwrap()
    });
    assert_eq!(get(&covered, "source_coverage"), &Value::Null);
    assert_eq!(
        encode(&without(&lagging, "source_coverage")),
        encode(&covered)
    );
    check_quiet(&report_design::full(&covered), "full");
    check_renderings(&report_design::full(&lagging), "full", ENGLISH);

    // Austrian report: the period ends with its tax year.
    let (args, sql) = case(include_str!("data/austrian_pdf.json"), "at-base");
    let (covered, lagging) = both(conn, &sql, |conn| {
        let (workspace, profile) = kassiber_setup::references::scope(conn, &args).unwrap();
        kassiber_reports::austrian_pdf::model(conn, &workspace, &profile, &args, &Fixed).unwrap()
    });
    assert_eq!(get(&covered, "source_coverage"), &Value::Null);
    assert_eq!(
        encode(&without(&lagging, "source_coverage")),
        encode(&covered)
    );
    check_quiet(&report_design::austrian(&covered), "austrian");
    check_renderings(
        &report_design::austrian(&lagging),
        "austrian",
        [
            "Vorläufig: 1 Quelle(n) decken den Zeitraum nicht bis 31.12.2025 ab.",
            "Im Rückstand",
            "Neueren Export importieren",
        ],
    );
}
