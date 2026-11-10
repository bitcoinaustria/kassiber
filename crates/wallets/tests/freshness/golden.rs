//! Recorded fixture calls and their replay.
//!
//! Each golden under `tests/data/freshness/` holds, per pytest case, what
//! the parity test handed a fixture binary and what came back while the
//! Rust fixture and the Python reference agreed: the input, the book before
//! the call (schema, rows with their rowids, pragmas), the printed line and
//! the book's canonical snapshot afterwards (every nonempty table, rows in
//! the parity tests' sort order). A replay restores the book at a fresh
//! path, runs the same fixture code in process, and requires the identical
//! line and snapshot. Book schemas live once in `schemas.json`, by digest.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use kassiber_envelope::{get, py_str};
use kassiber_pyjson::{Key, Options, Value};
use kassiber_store::rusqlite::{
    params_from_iter, types::Value as Sql, types::ValueRef, Connection,
};

/// The placeholder recordings use for the test's temporary directory.
const TMP: &str = "@TMP@";

pub struct Golden {
    root: Value,
    schemas: Value,
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap_or(&Value::Null)
}

fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        other => panic!("expected a list, not {other:?}"),
    }
}

fn entries(value: &Value) -> Vec<(String, &Value)> {
    match value {
        Value::Dict(items) => items
            .iter()
            .map(|(key, value)| match key {
                Key::Str(name) => (py_str(&Value::Str(name.clone())), value),
                other => panic!("golden keys are strings, not {other:?}"),
            })
            .collect(),
        Value::Null => vec![],
        other => panic!("expected an object, not {other:?}"),
    }
}

impl Golden {
    pub fn load(name: &str) -> Self {
        let read = |file: &str| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/data/freshness")
                .join(file);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            kassiber_pytext::loads(&text).expect("golden JSON")
        };
        Self {
            root: read(&format!("{name}.json")),
            schemas: read("schemas.json"),
        }
    }

    /// Every recorded case of one pytest test (all its parameters).
    pub fn cases(&self, test: &str) -> Vec<&Value> {
        let cases: Vec<&Value> = list(field(&self.root, "cases"))
            .iter()
            .filter(|case| py_str(field(case, "test")) == test)
            .collect();
        assert!(!cases.is_empty(), "no recorded case of {test}");
        cases
    }

    pub fn schema(&self, digest: &str) -> &[Value] {
        list(field(&self.schemas, digest))
    }
}

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A fresh directory for one replayed case, removed when dropped.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(label: &str) -> Self {
        let safe: String = label
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .take(60)
            .collect();
        let dir = std::env::temp_dir().join(format!(
            "kassiber-freshness-{}-{}-{safe}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub fn path(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }

    /// `text` with the recorded temporary directory placed here.
    pub fn place(&self, text: &str) -> String {
        text.replace(TMP, &self.path())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sql(value: &Value) -> Sql {
    match value {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Int(n) => Sql::Integer(i64::try_from(n.clone()).unwrap()),
        Value::Float(f) => Sql::Real(*f),
        Value::Str(_) => Sql::Text(py_str(value)),
        Value::Dict(_) => {
            let hex = py_str(field(value, "$blob"));
            Sql::Blob(
                (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                    .collect(),
            )
        }
        other => panic!("unsupported recorded value {other:?}"),
    }
}

/// Write the recorded book `before` at `path`: tables, then their rows at
/// their rowids, then indexes, views and triggers, so trigger-maintained
/// rows are restored rather than regenerated.
pub fn restore(golden: &Golden, before: &Value, path: &Path) {
    let conn = Connection::open(path).unwrap();
    let pragmas = field(before, "pragmas");
    conn.execute_batch(&format!(
        "PRAGMA page_size={}; PRAGMA journal_mode={}; PRAGMA user_version={}; PRAGMA application_id={};",
        py_str(field(pragmas, "page_size")),
        py_str(field(pragmas, "journal_mode")),
        py_str(field(pragmas, "user_version")),
        py_str(field(pragmas, "application_id")),
    ))
    .unwrap();
    let schema = golden.schema(&py_str(field(before, "schema")));
    let object = |row: &Value, at: usize| list(row)[at].clone();
    conn.execute_batch("PRAGMA foreign_keys=OFF; BEGIN")
        .unwrap();
    for row in schema {
        let name = py_str(&object(row, 1));
        if py_str(&object(row, 0)) == "table" && !name.starts_with("sqlite_") {
            conn.execute_batch(&py_str(&object(row, 3))).unwrap();
        }
    }
    for (table, data) in entries(field(before, "tables")) {
        let mut columns: Vec<String> = list(field(data, "columns"))
            .iter()
            .map(|c| format!("\"{}\"", py_str(c)))
            .collect();
        if kassiber_envelope::truthy(field(data, "rowid")) {
            columns.insert(0, "rowid".to_owned());
        }
        let statement = format!(
            "INSERT INTO \"{table}\"({}) VALUES({})",
            columns.join(","),
            vec!["?"; columns.len()].join(",")
        );
        for values in list(field(data, "rows")) {
            conn.execute(&statement, params_from_iter(list(values).iter().map(sql)))
                .unwrap();
        }
    }
    for values in list(field(before, "sequence")) {
        conn.execute(
            "INSERT INTO sqlite_sequence(name,seq) VALUES(?,?)",
            params_from_iter(list(values).iter().map(sql)),
        )
        .unwrap();
    }
    for kind in ["index", "view", "trigger"] {
        for row in schema {
            if py_str(&object(row, 0)) == kind && !matches!(object(row, 3), Value::Null) {
                conn.execute_batch(&py_str(&object(row, 3))).unwrap();
            }
        }
    }
    conn.execute_batch("COMMIT").unwrap();
}

fn column(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) => Value::Int(n.into()),
        ValueRef::Real(n) => Value::Float(n),
        ValueRef::Text(text) => Value::from(std::str::from_utf8(text).unwrap()),
        ValueRef::Blob(_) => panic!("freshness books hold no blobs"),
    }
}

fn text(value: &Value) -> String {
    kassiber_pyjson::dumps(value, &Options::new().sort_keys(true)).unwrap()
}

/// The parity tests' `dump`: every nonempty table's rows, sorted by their
/// `json.dumps(row, sort_keys=True)`.
pub fn snapshot(path: &Path) -> Value {
    let conn = Connection::open(path).unwrap();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut result = Vec::new();
    for table in tables {
        let mut statement = conn.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
        let count = statement.column_count();
        let rows: Vec<Value> = statement
            .query_map([], |row| {
                Ok(Value::List(
                    (0..count)
                        .map(|i| column(row.get_ref(i).unwrap()))
                        .collect(),
                ))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        if !rows.is_empty() {
            result.push((Key::from(table.as_str()), Value::List(rows)));
        }
    }
    canonical(&Value::Dict(result))
}

/// Tables by name, rows in their sorted order.
pub fn canonical(snapshot: &Value) -> Value {
    let mut tables: Vec<(String, Value)> = entries(snapshot)
        .into_iter()
        .map(|(name, rows)| {
            let mut rows = list(rows).to_vec();
            rows.sort_by_key(text);
            (name, Value::List(rows))
        })
        .collect();
    tables.sort_by(|a, b| a.0.cmp(&b.0));
    Value::Dict(
        tables
            .into_iter()
            .map(|(name, rows)| (Key::from(name.as_str()), rows))
            .collect(),
    )
}

/// The first differing table, for a readable failure.
pub fn assert_same_book(case: &str, actual: &Value, expected: &Value) {
    let actual = canonical(actual);
    let expected = canonical(expected);
    if text(&actual) == text(&expected) {
        return;
    }
    let names: std::collections::BTreeSet<String> = entries(&actual)
        .into_iter()
        .chain(entries(&expected))
        .map(|(name, _)| name)
        .collect();
    for name in names {
        let a = get(&actual, &name).map(text);
        let e = get(&expected, &name).map(text);
        assert_eq!(a, e, "{case}: table {name} differs from the recording");
    }
    panic!("{case}: book differs from the recording");
}

/// A short label for one case.
pub fn label(case: &Value) -> String {
    let param = py_str(field(case, "param"));
    let test = py_str(field(case, "test"));
    if param.is_empty() {
        test
    } else {
        format!("{test}[{param}]")
    }
}

pub fn calls(case: &Value) -> &[Value] {
    list(field(case, "calls"))
}

pub fn string(value: &Value, key: &str) -> String {
    py_str(field(value, key))
}

pub fn value<'a>(value: &'a Value, key: &str) -> &'a Value {
    field(value, key)
}
