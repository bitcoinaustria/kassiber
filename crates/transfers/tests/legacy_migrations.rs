//! Legacy book migrations the Rust store runs through the custody layer
//! (`legacy_migration::install`): reviewed pair/payout consolidation, the
//! legacy pair fee review and componentless dismissal recovery.
//!
//! Each vector is a frozen input book and the complete typed dump Python's
//! `open_db` produced from it (`tests/rust_port/gen_legacy_migration_vectors.py`,
//! frozen while the Python oracle existed): every schema object, table and
//! row, each value with its storage class. The product's migrating open must
//! produce the identical dump, plaintext and SQLCipher, and a second open must
//! change nothing.
use kassiber_store::{rusqlite::Connection, Book, OpenOptions};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};

const IDENTITY: &str = "0123456789ab4def8123456789abcdef";
const TIMESTAMP: &str = "2026-10-27T12:34:56.789Z";
const PASSPHRASE: &str = "legacy migration '🔑";

fn vectors() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/legacy_migrations")
}

fn gunzip(path: &Path) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap())
        .read_to_end(&mut out)
        .unwrap();
    out
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// The generator's `dump`: objects by (type, name); rows by primary key,
/// then every column; values as [storage class, value], REAL as its bits.
fn dump(conn: &Connection) -> Value {
    let objects: Vec<Value> = conn
        .prepare("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name")
        .unwrap()
        .query_map([], |r| {
            Ok(json!([
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?
            ]))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut out = serde_json::Map::new();
    for table in tables {
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
        let mut statement = conn.prepare(&sql).unwrap();
        let rows: Vec<Value> = statement
            .query_map([], |r| {
                let mut row = Vec::new();
                for i in 0..width {
                    let kind: String = r.get(width + i)?;
                    use kassiber_store::rusqlite::types::ValueRef;
                    let value = match r.get_ref(i)? {
                        ValueRef::Null => Value::Null,
                        ValueRef::Integer(n) => json!(n),
                        ValueRef::Real(f) => json!(f
                            .to_be_bytes()
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>()),
                        ValueRef::Text(t) => json!(std::str::from_utf8(t).unwrap()),
                        ValueRef::Blob(b) => {
                            json!(b.iter().map(|b| format!("{b:02x}")).collect::<String>())
                        }
                    };
                    row.push(json!([kind, value]));
                }
                Ok(Value::Array(row))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        out.insert(
            table,
            json!({"columns": columns.iter().map(|c| c.0.clone()).collect::<Vec<_>>(), "rows": rows}),
        );
    }
    json!({"objects": objects, "tables": out})
}

/// The first difference, named well enough to find it.
fn difference(expected: &Value, actual: &Value) -> Option<String> {
    if expected["objects"] != actual["objects"] {
        let e = expected["objects"].as_array().unwrap();
        let a = actual["objects"].as_array().unwrap();
        for (i, (x, y)) in e.iter().zip(a).enumerate() {
            if x != y {
                return Some(format!("schema object {i}: expected {x}, got {y}"));
            }
        }
        return Some(format!("schema object count {} vs {}", e.len(), a.len()));
    }
    let (e, a) = (
        expected["tables"].as_object().unwrap(),
        actual["tables"].as_object().unwrap(),
    );
    let mut found = Vec::new();
    for (table, x) in e {
        let Some(y) = a.get(table) else {
            found.push(format!("table {table} missing"));
            continue;
        };
        if x == y {
            continue;
        }
        let (xr, yr) = (x["rows"].as_array().unwrap(), y["rows"].as_array().unwrap());
        if let Some((i, (p, q))) = xr.iter().zip(yr).enumerate().find(|(_, (p, q))| p != q) {
            found.push(format!(
                "{table} row {i} (of {} vs {}):\n expected {p}\n got      {q}\ncolumns {}",
                xr.len(),
                yr.len(),
                x["columns"]
            ));
        } else {
            found.push(format!(
                "{table}: expected {} rows, got {}\nexpected tail {}\ngot tail {}",
                xr.len(),
                yr.len(),
                xr.last().cloned().unwrap_or_default(),
                yr.last().cloned().unwrap_or_default()
            ));
        }
    }
    if a.len() != e.len() {
        found.push("extra tables".into());
    }
    (!found.is_empty()).then(|| found.join("\n"))
}

fn encrypt(plain: &Path, cipher: &Path) {
    let conn = Connection::open(plain).unwrap();
    conn.execute(
        &format!(
            "ATTACH DATABASE ?1 AS encrypted KEY '{}'",
            PASSPHRASE.replace('\'', "''")
        ),
        [cipher.to_str().unwrap()],
    )
    .unwrap();
    conn.execute_batch(
        "PRAGMA encrypted.cipher_compatibility=4; PRAGMA encrypted.kdf_iter=256000;
         PRAGMA encrypted.cipher_page_size=4096; SELECT sqlcipher_export('encrypted');
         DETACH DATABASE encrypted;",
    )
    .unwrap();
}

fn open_dump(database: &Path, passphrase: Option<&str>) -> Value {
    let conn = Connection::open(database).unwrap();
    if let Some(passphrase) = passphrase {
        conn.execute_batch(&format!(
            "PRAGMA key='{}'; PRAGMA cipher_compatibility=4; PRAGMA kdf_iter=256000; PRAGMA cipher_page_size=4096;",
            passphrase.replace('\'', "''")
        ))
        .unwrap();
    }
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
    dump(&conn)
}

fn migrate(root: &Path, passphrase: Option<&str>) -> Result<(), String> {
    let book = Book::open_migrating_with_sources(
        root,
        OpenOptions {
            passphrase,
            ..Default::default()
        },
        true,
        IDENTITY,
        TIMESTAMP,
    )
    .map_err(|e| format!("{e:?}"))?;
    book.close().map_err(|f| format!("{:?}", f.error))
}

fn names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(vectors())
        .unwrap_or_else(|e| {
            panic!(
                "{}: {e}; run scripts/fetch-test-data.sh",
                vectors().display()
            )
        })
        .filter_map(|entry| {
            let name = entry.unwrap().file_name().into_string().unwrap();
            name.strip_suffix(".input.sqlite3.gz").map(str::to_owned)
        })
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no legacy migration vectors");
    names
}

fn check(name: &str, encrypted: bool) -> Option<String> {
    kassiber_transfers::legacy_migration::install();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    std::fs::create_dir(&root).unwrap();
    let database = root.join("kassiber.sqlite3");
    let input = gunzip(&vectors().join(format!("{name}.input.sqlite3.gz")));
    let passphrase = encrypted.then_some(PASSPHRASE);
    if encrypted {
        let plain = dir.path().join("plain.sqlite3");
        std::fs::write(&plain, input).unwrap();
        encrypt(&plain, &database);
    } else {
        std::fs::write(&database, input).unwrap();
    }
    let expected: Value =
        serde_json::from_slice(&gunzip(&vectors().join(format!("{name}.expected.json.gz"))))
            .unwrap();
    if let Err(error) = migrate(&root, passphrase) {
        return Some(format!(
            "{name} (encrypted={encrypted}): open refused: {error}"
        ));
    }
    let first = open_dump(&database, passphrase);
    if let Some(diff) = difference(&expected, &first) {
        return Some(format!("{name} (encrypted={encrypted}): {diff}"));
    }
    // A migrated book opens again without any change.
    if let Err(error) = migrate(&root, passphrase) {
        return Some(format!(
            "{name} reopen (encrypted={encrypted}): refused: {error}"
        ));
    }
    let again = open_dump(&database, passphrase);
    difference(&first, &again).map(|diff| format!("{name} reopen (encrypted={encrypted}): {diff}"))
}

#[test]
fn rust_migration_matches_frozen_python_dumps_plaintext() {
    check_all(false);
}

#[test]
fn rust_migration_matches_frozen_python_dumps_sqlcipher() {
    check_all(true);
}

/// A copy of the vector's input, migrated until the pass reaches `point`,
/// where an injected failure interrupts it; returns the book's dump.
fn interrupt_at(name: &str, point: &str) -> (tempfile::TempDir, PathBuf, Value) {
    kassiber_transfers::legacy_migration::install();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("kassiber.sqlite3"),
        gunzip(&vectors().join(format!("{name}.input.sqlite3.gz"))),
    )
    .unwrap();
    let mut reached = false;
    let error = kassiber_store::test_hooks::migrate(&root, None, IDENTITY, TIMESTAMP, &mut |at| {
        if at == point {
            reached = true;
            return Err(kassiber_store::Error::new(
                kassiber_store::ErrorKind::Io,
                "injected interruption",
            ));
        }
        Ok(())
    })
    .unwrap_err();
    assert!(
        reached && error.message == "injected interruption",
        "{name}: {error:?}"
    );
    let dump = open_dump(&root.join("kassiber.sqlite3"), None);
    (dir, root, dump)
}

/// An interruption at the step's commit leaves the book exactly as an
/// interruption just before the step does (the earlier steps of the pass
/// stay committed, as in Python); the next open completes the identical
/// migration.
fn interrupted(name: &str, before: &str, commit: &str) {
    let (_dir, _, untouched) = interrupt_at(name, before);
    let (_dir, root, interrupted) = interrupt_at(name, commit);
    if let Some(diff) = difference(&untouched, &interrupted) {
        panic!("{name}: the interrupted step left writes: {diff}");
    }
    migrate(&root, None).unwrap();
    let expected: Value =
        serde_json::from_slice(&gunzip(&vectors().join(format!("{name}.expected.json.gz"))))
            .unwrap();
    if let Some(diff) = difference(&expected, &open_dump(&root.join("kassiber.sqlite3"), None)) {
        panic!("{name} after interruption: {diff}");
    }
}

#[test]
fn interrupted_consolidation_leaves_no_partial_component_and_resumes() {
    interrupted(
        "scenario-mixed",
        "legacy_authored_components.before",
        "legacy_authored_components.before_commit",
    );
}

#[test]
fn interrupted_pair_fee_review_keeps_journals_and_audit_together() {
    interrupted(
        "historical-pre_pair_fee_c0a015de",
        "custody-legacy-pair-fee-review-v1.before_audit",
        "custody-legacy-pair-fee-review-v1.before_commit",
    );
}

/// Without the custody layer the pass refuses a pending book before the
/// step writes anything (library opens that did not install it).
#[test]
fn every_product_open_installs_the_custody_migrations() {
    let main = include_str!("../../kassiber/src/main.rs");
    let install = main
        .find("kassiber_transfers::legacy_migration::install();")
        .expect("main installs the legacy custody migrations");
    let dispatch = main.find("let mut argv").expect("argument parsing");
    assert!(install < dispatch, "install before any command runs");
}

/// Every vector, four at a time (SQLCipher's key derivation dominates).
fn check_all(encrypted: bool) {
    let names = names();
    let failures: Vec<String> = std::thread::scope(|scope| {
        names
            .chunks(names.len().div_ceil(4))
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .filter_map(|n| check(n, encrypted))
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
