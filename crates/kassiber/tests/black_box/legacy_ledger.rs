//! A book that still holds a populated general-ledger archive, written by the
//! last revision that shipped the ledger (its own `gl_*` tables and all 203
//! triggers it installed) and upgraded by this checkout's Python `open_db`.
//! `maintenance purge-legacy-ledger` in the binary must plan and remove it
//! exactly as Python did: the frozen plan and the complete typed dump of the
//! purged book (every schema object and row, each value with its storage
//! class) come from Python's purge of the same input.
//!
//! The vector is frozen (2026-11-11) and cannot be regenerated: the input
//! was written by revision `f36af60de^` (extracted with `git archive`),
//! whose own `open_db` and ledger API configured a book, opened a period,
//! posted two entries, left a draft and retained an evidence file linked to
//! an external document. The ledger required SQLCipher, so the book was
//! written encrypted, exported to plaintext, and upgraded by the Python
//! `open_db` of `archive/python-backend`'s line; Python's
//! `maintenance purge-legacy-ledger --confirm` then gave the expected plan
//! and dump. That revision draws random ids and reads the clock, so a rerun
//! would have produced a different, equally valid vector.
//!
//! A `gl_*` trigger no Kassiber version shipped is schema drift: the binary
//! refuses the book, names the trigger and leaves the file unchanged.
use super::support::*;
use kassiber_store::rusqlite::{types::ValueRef, Connection};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};

const PASSPHRASE: &str = "legacy ledger '🔑";

fn data() -> PathBuf {
    repo().join("crates/kassiber/tests/data/legacy_ledger")
}

fn gunzip(path: &Path) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap())
        .read_to_end(&mut out)
        .unwrap();
    out
}

fn expected() -> Value {
    serde_json::from_slice(&gunzip(&data().join("expected.json.gz"))).unwrap()
}

/// The frozen input in a fresh data root, plaintext or SQLCipher.
fn ledger_book(cipher: bool) -> Sandbox {
    let mut sandbox = Sandbox::empty();
    std::fs::create_dir_all(&sandbox.root).unwrap();
    let database = sandbox.root.join("kassiber.sqlite3");
    let input = gunzip(&data().join("input.sqlite3.gz"));
    if cipher {
        let plain = sandbox.path("plain.sqlite3");
        std::fs::write(&plain, input).unwrap();
        let conn = Connection::open(&plain).unwrap();
        conn.execute(
            &format!(
                "ATTACH DATABASE ?1 AS encrypted KEY '{}'",
                PASSPHRASE.replace('\'', "''")
            ),
            [database.to_str().unwrap()],
        )
        .unwrap();
        conn.execute_batch(
            "PRAGMA encrypted.cipher_compatibility=4; PRAGMA encrypted.kdf_iter=256000;
             PRAGMA encrypted.cipher_page_size=4096; SELECT sqlcipher_export('encrypted');
             DETACH DATABASE encrypted;",
        )
        .unwrap();
        drop(conn);
        std::fs::remove_file(plain).unwrap();
        sandbox.passphrase = Some(PASSPHRASE.to_owned());
    } else {
        std::fs::write(&database, input).unwrap();
    }
    sandbox
}

fn connect(sandbox: &Sandbox) -> Connection {
    let conn = Connection::open(sandbox.root.join("kassiber.sqlite3")).unwrap();
    if let Some(passphrase) = &sandbox.passphrase {
        conn.execute_batch(&format!(
            "PRAGMA key='{}'; PRAGMA cipher_compatibility=4; PRAGMA kdf_iter=256000; PRAGMA cipher_page_size=4096;",
            passphrase.replace('\'', "''")
        ))
        .unwrap();
    }
    conn
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `tests/rust_port/gen_legacy_migration_vectors.dump`: objects by (type,
/// name); rows by primary key, then every column; values as [storage class,
/// value], REAL as its bits and BLOB as hex.
fn dump(conn: &Connection) -> Value {
    let strings = |sql: &str| -> Vec<Vec<Option<String>>> {
        let mut statement = conn.prepare(sql).unwrap();
        let width = statement.column_count();
        statement
            .query_map([], |r| (0..width).map(|i| r.get(i)).collect())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let objects: Vec<Value> =
        strings("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name")
            .into_iter()
            .map(|row| json!(row))
            .collect();
    let mut tables = serde_json::Map::new();
    for row in strings("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name") {
        let table = row[0].clone().unwrap();
        let columns: Vec<(String, i64)> = conn
            .prepare(&format!("PRAGMA table_info({})", quote(&table)))
            .unwrap()
            .query_map([], |r| Ok((r.get(1)?, r.get(5)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let mut primary: Vec<_> = columns.iter().filter(|c| c.1 != 0).collect();
        primary.sort_by_key(|c| c.1);
        let mut order: Vec<&str> = Vec::new();
        for name in primary
            .iter()
            .map(|c| c.0.as_str())
            .chain(columns.iter().map(|c| c.0.as_str()))
        {
            if !order.contains(&name) {
                order.push(name);
            }
        }
        let kinds: Vec<String> = columns
            .iter()
            .map(|c| format!("typeof({})", quote(&c.0)))
            .collect();
        let order: Vec<String> = order.iter().map(|c| quote(c)).collect();
        let sql = format!(
            "SELECT *,{} FROM {} ORDER BY {}",
            kinds.join(","),
            quote(&table),
            order.join(",")
        );
        let width = columns.len();
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let rows: Vec<Value> = conn
            .prepare(&sql)
            .unwrap()
            .query_map([], |r| {
                (0..width)
                    .map(|i| {
                        let kind: String = r.get(width + i)?;
                        let value = match r.get_ref(i)? {
                            ValueRef::Null => Value::Null,
                            ValueRef::Integer(n) => json!(n),
                            ValueRef::Real(f) => json!(hex(&f.to_be_bytes())),
                            ValueRef::Text(t) => json!(std::str::from_utf8(t).unwrap()),
                            ValueRef::Blob(b) => json!(hex(b)),
                        };
                        Ok(json!([kind, value]))
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::Array)
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        tables.insert(
            table,
            json!({"columns": columns.iter().map(|c| c.0.clone()).collect::<Vec<_>>(), "rows": rows}),
        );
    }
    json!({"objects": objects, "tables": tables})
}

/// The first difference between two dumps, named well enough to find it.
fn difference(expected: &Value, actual: &Value) -> Option<String> {
    let (e, a) = (
        expected["objects"].as_array().unwrap(),
        actual["objects"].as_array().unwrap(),
    );
    if let Some((x, y)) = e.iter().zip(a).find(|(x, y)| x != y) {
        return Some(format!("schema object: expected {x}, got {y}"));
    }
    if e.len() != a.len() {
        return Some(format!("schema object count {} vs {}", e.len(), a.len()));
    }
    let (e, a) = (
        expected["tables"].as_object().unwrap(),
        actual["tables"].as_object().unwrap(),
    );
    for (table, x) in e {
        let Some(y) = a.get(table) else {
            return Some(format!("table {table} missing"));
        };
        if x != y {
            return Some(format!("{table}: expected {x}\ngot {y}"));
        }
    }
    (a.len() != e.len()).then(|| "extra tables".into())
}

fn purge_matches_python(cipher: bool) {
    let sandbox = ledger_book(cipher);
    let expected = expected();
    let before = std::fs::read(sandbox.root.join("kassiber.sqlite3")).unwrap();

    // Without --confirm: Python's plan, and nothing written.
    let plan = sandbox
        .machine(&["maintenance", "purge-legacy-ledger"])
        .json();
    assert_eq!(
        plan["kind"], "maintenance.purge-legacy-ledger.plan",
        "{plan}"
    );
    let mut python = expected["plan"].clone();
    python["applied"] = json!(false);
    python["hint"] = json!("Back up the project first (`kassiber backup export`), then re-run with --confirm. The archived ledger data cannot be recovered afterwards.");
    assert_eq!(plan["data"], python);
    assert_eq!(plan["data"]["trigger_count"], 203);
    assert!(std::fs::read(sandbox.root.join("kassiber.sqlite3")).unwrap() == before);

    let done = sandbox
        .machine(&["maintenance", "purge-legacy-ledger", "--confirm"])
        .json();
    assert_eq!(done["kind"], "maintenance.purge-legacy-ledger", "{done}");
    assert_eq!(done["data"], expected["applied"]);

    let conn = connect(&sandbox);
    assert_eq!(
        conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert!(!conn
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .exists([])
        .unwrap());
    let actual = dump(&conn);
    drop(conn);
    if let Some(found) = difference(&expected["dump"], &actual) {
        panic!("the purged book differs from Python's: {found}");
    }

    // Nothing is left to plan or remove.
    let again = sandbox
        .machine(&["maintenance", "purge-legacy-ledger", "--confirm"])
        .json();
    assert_eq!(
        again["data"],
        json!({"tables": [], "table_count": 0, "trigger_count": 0, "row_count": 0, "applied": true})
    );
}

#[test]
fn purge_of_a_shipped_ledger_archive_matches_python_plaintext() {
    purge_matches_python(false);
}

#[test]
fn purge_of_a_shipped_ledger_archive_matches_python_sqlcipher() {
    purge_matches_python(true);
}

/// The unshipped retention guard the Python test book carried until
/// 2026-11-11: no Kassiber version installed `gl_lines_no_delete`.
const UNSHIPPED: &str = "CREATE TRIGGER gl_lines_no_delete BEFORE DELETE ON gl_lines BEGIN
    SELECT RAISE(ABORT,'accounting_entry_immutable'); END";

#[test]
fn an_unshipped_ledger_trigger_is_refused_by_name_without_a_write() {
    for cipher in [false, true] {
        let sandbox = ledger_book(cipher);
        connect(&sandbox).execute_batch(UNSHIPPED).unwrap();
        let database = sandbox.root.join("kassiber.sqlite3");
        let before = std::fs::read(&database).unwrap();
        for argv in [
            &["maintenance", "purge-legacy-ledger", "--confirm"][..],
            &["maintenance", "purge-legacy-ledger"],
            &["status"],
        ] {
            let output = sandbox.machine(argv);
            assert_eq!(output.code, 1, "{argv:?}: {}", output.text());
            let error = &output.json()["error"];
            assert_eq!(error["code"], "requires_python_migration", "{error}");
            assert_eq!(
                error["message"],
                "this book's storage layout is not supported by this Kassiber build"
            );
            assert_eq!(
                error["details"],
                json!({"reason": "schema_structure", "object": "gl_lines_no_delete", "aspects": ["unexpected"]}),
                "{argv:?}"
            );
            assert!(
                std::fs::read(&database).unwrap() == before,
                "{argv:?} wrote the book"
            );
            assert!(!sandbox.root.join("kassiber.sqlite3-journal").exists());
        }
    }
}
