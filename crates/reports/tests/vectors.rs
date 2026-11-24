use kassiber_envelope::{dict, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::Book;
use serde_json::Value as Json;

/// `KASSIBER_GOLDEN_WRITE=1` rewrites each mismatching case's `expected` in
/// `data/vectors.json` from the current output instead of failing, and prints
/// every changed field (docs/reference/testing.md#changing-a-frozen-golden).
/// Refused under `CI`. Only the `expected` string of a changed case is
/// replaced; every other byte of the file stays as it was.
fn golden_write() -> bool {
    let write = std::env::var_os("KASSIBER_GOLDEN_WRITE").is_some_and(|v| v == "1");
    assert!(
        !(write && std::env::var_os("CI").is_some()),
        "KASSIBER_GOLDEN_WRITE is refused in CI"
    );
    write
}

/// Every field that differs between two JSON documents, as `path: old -> new`.
fn structural_diff(path: &str, old: &Json, new: &Json, out: &mut Vec<String>) {
    match (old, new) {
        (Json::Object(a), Json::Object(b)) => {
            let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for key in keys {
                let at = format!("{path}.{key}");
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => structural_diff(&at, x, y, out),
                    (Some(x), None) => out.push(format!("{at}: {x} -> (absent)")),
                    (None, Some(y)) => out.push(format!("{at}: (absent) -> {y}")),
                    (None, None) => {}
                }
            }
        }
        (Json::Array(a), Json::Array(b)) => {
            for i in 0..a.len().max(b.len()) {
                let at = format!("{path}[{i}]");
                match (a.get(i), b.get(i)) {
                    (Some(x), Some(y)) => structural_diff(&at, x, y, out),
                    (Some(x), None) => out.push(format!("{at}: {x} -> (absent)")),
                    (None, Some(y)) => out.push(format!("{at}: (absent) -> {y}")),
                    (None, None) => {}
                }
            }
        }
        (a, b) if a != b => out.push(format!("{path}: {a} -> {b}")),
        _ => {}
    }
}

/// Replace one case's `expected` string in the vectors file, found by the
/// case's unique name, leaving every other byte unchanged.
fn rewrite_expected(name: &str, old: &str, new: &str) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/vectors.json");
    let text = std::fs::read_to_string(&path).unwrap();
    let marker = format!("\"name\": {}", serde_json::to_string(name).unwrap());
    assert_eq!(
        text.matches(&marker).count(),
        1,
        "case name not unique: {name}"
    );
    let start = text.find(&marker).unwrap();
    let field = "\"expected\": ";
    let at = start + text[start..].find(field).unwrap() + field.len();
    let encoded_old = serde_json::to_string(old).unwrap();
    assert!(
        text[at..].starts_with(&encoded_old),
        "{name}: expected is not encoded as this writer would encode it"
    );
    let rewritten = format!(
        "{}{}{}",
        &text[..at],
        serde_json::to_string(new).unwrap(),
        &text[at + encoded_old.len()..]
    );
    std::fs::write(&path, rewritten).unwrap();
}

fn check(group: &str) {
    let write = golden_write();
    let cases: Vec<Json> = serde_json::from_str(include_str!("data/vectors.json")).unwrap();
    assert!(
        cases.iter().any(|v| v["group"] == group),
        "empty oracle group: {group}"
    );
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    for case in cases.iter().filter(|v| v["group"] == group) {
        book.connection()
            .execute_batch("SAVEPOINT oracle_fixture")
            .unwrap();
        if case["base"].as_bool().unwrap_or(true) {
            book.connection()
                .execute_batch(include_str!("data/book.sql"))
                .unwrap();
        }
        book.connection()
            .execute_batch(case["sql"].as_str().unwrap())
            .unwrap();
        let args = kassiber_pytext::loads(&case["args"].to_string()).unwrap();
        let before = book.connection().total_changes();
        let result =
            kassiber_reports::execute(book.connection(), case["command"].as_str().unwrap(), &args);
        let value = match result {
            Ok(v) => dict([("ok", v)]),
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
        let actual = dumps(
            &value,
            &Options {
                ensure_ascii: false,
                separators: Some((",".into(), ":".into())),
                ..Default::default()
            },
        )
        .unwrap();
        let expected = case["expected"].as_str().unwrap();
        if write && actual != expected {
            let name = case["name"].as_str().unwrap();
            let mut changed = Vec::new();
            structural_diff(
                "$",
                &serde_json::from_str(expected).unwrap(),
                &serde_json::from_str(&actual).unwrap(),
                &mut changed,
            );
            eprintln!("golden {group}/{name}: {} field(s) changed", changed.len());
            for line in &changed {
                eprintln!("  {line}");
            }
            rewrite_expected(name, expected, &actual);
        } else {
            assert_eq!(actual, expected, "{}", case["name"]);
        }
        assert_eq!(
            before,
            book.connection().total_changes(),
            "a report wrote to the book"
        );
        book.connection()
            .execute_batch("ROLLBACK TO oracle_fixture; RELEASE oracle_fixture")
            .unwrap();
    }
}
#[test]
fn capital_gains_vectors() {
    check("gains");
}
#[test]
fn holdings_vectors() {
    check("holdings");
}
#[test]
fn history_period_vectors() {
    check("history");
}
#[test]
fn history_quantity_vectors() {
    check("history-quantity");
}
#[test]
fn history_quote_vectors() {
    check("history-quotes");
}
#[test]
fn history_scope_vectors() {
    check("history-scope");
}
#[test]
fn history_readiness_vectors() {
    check("history-readiness");
}
#[test]
fn desktop_history_vectors() {
    check("desktop-history");
}
#[test]
fn summary_vectors() {
    check("summary");
}
#[test]
fn summary_text_vectors() {
    check("text");
}
#[test]
fn desktop_vectors() {
    check("desktop");
}
#[test]
fn readiness_vectors() {
    check("readiness");
}
#[test]
fn snapshots_vectors() {
    check("snapshots");
}

#[test]
fn components_vectors() {
    check("components");
}

#[test]
fn tax_summary_vectors() {
    check("tax");
}
#[test]
fn austrian_vectors() {
    check("austrian");
}
#[test]
fn austrian_text_vectors() {
    check("austrian-text");
}
#[test]
fn desktop_tax_vectors() {
    check("desktop-tax");
}

#[test]
fn journal_entry_vectors() {
    check("entries");
}

#[test]
fn report_csv_projection_vectors() {
    check("csv");
}
