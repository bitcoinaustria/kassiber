//! Test-only synthetic books replayed from Python oracle dumps.
use kassiber_envelope::get;
use kassiber_pyjson::{Key, Value};
use kassiber_store::rusqlite::{
    params_from_iter, types::Value as Sql, types::ValueRef, Connection,
};

fn manifest() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../store/fixtures/current-schema.json")).unwrap()
}

fn sql(value: &Value) -> Sql {
    match value {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Int(n) => Sql::Integer(i64::try_from(n.clone()).unwrap()),
        Value::Float(f) => Sql::Real(*f),
        Value::Str(_) => Sql::Text(kassiber_envelope::py_str(value)),
        other => panic!("unsupported dump value {other:?}"),
    }
}

fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        _ => panic!("expected list"),
    }
}

/// Tables first, then the dumped rows, then indexes/views/triggers, so rows
/// maintained by Python's triggers are restored rather than regenerated.
pub(crate) fn load(dump: &Value) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    let manifest = manifest();
    let objects = manifest["objects"].as_array().unwrap();
    for object in objects.iter().filter(|o| o[0] == "table") {
        conn.execute_batch(object[3].as_str().unwrap()).unwrap();
    }
    conn.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
    if let Value::Dict(tables) = dump {
        for (key, data) in tables {
            let Key::Str(table) = key else {
                panic!("table names are strings")
            };
            let table = kassiber_envelope::py_str(&Value::Str(table.clone()));
            let columns: Vec<String> = list(get(data, "columns").unwrap())
                .iter()
                .map(kassiber_envelope::py_str)
                .collect();
            let placeholders = vec!["?"; columns.len()].join(",");
            let statement = format!(
                "INSERT INTO \"{table}\"({}) VALUES({placeholders})",
                columns
                    .iter()
                    .map(|c| format!("\"{c}\""))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            for row in list(get(data, "rows").unwrap()) {
                conn.execute(&statement, params_from_iter(list(row).iter().map(sql)))
                    .unwrap();
            }
        }
    }
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    for kind in ["index", "view", "trigger"] {
        for object in objects.iter().filter(|o| o[0] == kind) {
            if let Some(text) = object[3].as_str() {
                conn.execute_batch(text).unwrap();
            }
        }
    }
    conn
}

fn column(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) => Value::Int(n.into()),
        ValueRef::Real(n) => Value::Float(n),
        ValueRef::Text(text) => Value::from(std::str::from_utf8(text).unwrap()),
        ValueRef::Blob(_) => panic!("synthetic books contain no blobs"),
    }
}

/// `balance_checkpoints` is newer than these frozen dumps
/// (docs/reference/balance-checkpoints.md): left out only while it holds no
/// row, so a step that writes one still shows.
fn newer_and_empty(conn: &Connection, table: &str) -> bool {
    table == "balance_checkpoints"
        && conn
            .query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM balance_checkpoints)",
                [],
                |r| r.get(0),
            )
            .unwrap_or(true)
}

/// Every table's rows, sorted like Python's `json.dumps(row, sort_keys=True)`.
pub(crate) fn snapshot(conn: &Connection) -> Value {
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut result = Vec::new();
    for table in tables.into_iter().filter(|t| !newer_and_empty(conn, t)) {
        let mut statement = conn.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
        let count = statement.column_count();
        let mut rows: Vec<Value> = statement
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
        rows.sort_by_key(text);
        result.push((Key::from(table.as_str()), Value::List(rows)));
    }
    Value::Dict(result)
}

/// Python's `json.dumps(value, sort_keys=True)`.
pub(crate) fn text(value: &Value) -> String {
    kassiber_pyjson::dumps(value, &kassiber_pyjson::Options::new().sort_keys(true)).unwrap()
}

/// Trace every row write, including repeated writes and foreign-key/graph
/// trigger effects. TEMP instrumentation never enters the logical book dump.
pub(crate) fn watch_writes(conn: &Connection, fail_at: usize) {
    conn.execute_batch(
        "CREATE TEMP TABLE publication_write_trace(name TEXT, operation TEXT);
         CREATE TEMP TABLE publication_write_failure(position INTEGER);",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO publication_write_failure VALUES(?)",
        [fail_at as i64],
    )
    .unwrap();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for table in tables {
        for operation in ["INSERT", "UPDATE", "DELETE"] {
            conn.execute_batch(&format!(
                "CREATE TEMP TRIGGER probe_{table}_{operation} BEFORE {operation} ON main.\"{table}\"
                 BEGIN
                   INSERT INTO publication_write_trace VALUES('{table}', '{operation}');
                   SELECT RAISE(ABORT, 'injected publication write')
                     WHERE (SELECT COUNT(*) FROM publication_write_trace) =
                           (SELECT position FROM publication_write_failure);
                 END;"
            ))
            .unwrap();
        }
    }
}

pub(crate) fn write_trace(conn: &Connection) -> Vec<(String, String)> {
    conn.prepare("SELECT name,operation FROM publication_write_trace ORDER BY rowid")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Deterministic `uuid4()` / `now_iso()` sequences matching `publication_books.py`.
pub(crate) struct Clock {
    ids: u32,
    now: u32,
    prefix: &'static str,
}

impl Clock {
    pub(crate) fn new(prefix: &'static str) -> Self {
        Self {
            ids: 0,
            now: 0,
            prefix,
        }
    }
    pub(crate) fn resume(prefix: &'static str, ids: u32, now: u32) -> Self {
        Self { ids, now, prefix }
    }
    pub(crate) fn id(&mut self) -> String {
        self.ids += 1;
        format!("{}-{:04}", self.prefix, self.ids)
    }
    pub(crate) fn now(&mut self) -> String {
        self.now += 1;
        kassiber_chain::format_unix_time(1_893_456_000 + i64::from(self.now)).unwrap()
    }
}

#[cfg(test)]
mod newer_table_tests {
    use super::*;

    /// The newer checkpoint table drops out of a snapshot only while empty.
    #[test]
    fn a_written_checkpoint_stays_in_the_snapshot() {
        let conn = load(&Value::Dict(vec![]));
        let has = |conn: &Connection| {
            matches!(&snapshot(conn), Value::Dict(tables)
                if tables.iter().any(|(k, _)| *k == Key::from("balance_checkpoints")))
        };
        assert!(!has(&conn));
        conn.execute_batch(
            "PRAGMA foreign_keys=OFF;
             INSERT INTO balance_checkpoints(id,workspace_id,profile_id,wallet_id,asset,as_of,amount_msat,origin,created_at)
             VALUES('c','ws','p','w','BTC','2025-01-01T00:00:00Z',1,'manual','x')",
        )
        .unwrap();
        assert!(has(&conn));
    }
}
