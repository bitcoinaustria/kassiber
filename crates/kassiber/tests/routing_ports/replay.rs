//! Replays a frozen golden: the `kassiber` processes one pytest parity test
//! started, with the state each found, the network tape its fake servers
//! answered, and what each printed and changed.
//!
//! The goldens were recorded where Rust and the Python reference agreed
//! (docs/plan/22-python-removal.md, decision D1 option 3). Each session
//! lists:
//! - `setup`: what turns the state roots from the previous session's end into
//!   this one's start (books as row patches, other files whole);
//! - `args`, `env`, `cwd`, `stdin` and `tape`: the process inputs;
//! - `expect`: exit code, stdout, stderr, and every book row and file the
//!   process changed under its roots.
//!
//! Paths are templated: `{tmp}` is the test's directory (the recording's
//! `/mnt/tmp` lives at `{tmp}/systmp`), `{repo}` the checkout. Rows and
//! `elapsed_ms` values are compared as recorded, with wall-clock
//! `elapsed_ms` fixed at 0; a value larger than 16 KiB compares by length
//! and FNV-1a 64 hash.
//!
//! `KASSIBER_GOLDEN_WRITE=1` rewrites each golden's `expect` from the
//! current binary instead of asserting; it refuses to run under `CI`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use kassiber_store::rusqlite::{self, types::ValueRef, Connection};
use serde_json::{json, Map, Value};

const BIG: usize = 16384;
/// A table change with more rows compares by its count and digest.
const MANY: usize = 2000;
const TIMEOUT: Duration = Duration::from_secs(180);
/// How long before its first recorded process a test may have written state.
const LOOKBACK_US: i64 = 600_000_000;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/routing_ports/goldens")
        .join(format!("{name}.json"))
}

#[derive(Clone, Default)]
struct Paths {
    tmp: String,
    repo: String,
    /// A served session's loopback address as recorded and as bound for
    /// this replay (`127.0.0.1:<port>` both), so its fake takes any free
    /// port: inputs carry the bound address, outputs the recorded one.
    address: Option<(String, String)>,
}

thread_local! {
    /// The running replay's paths, for book cells (one replay per thread).
    static PATHS: std::cell::RefCell<Paths> = std::cell::RefCell::new(Paths::default());
}

impl Paths {
    fn forward(&self, text: &str) -> String {
        if self.tmp.is_empty() {
            return text.to_owned();
        }
        let text = text
            .replace("{tmp}", &self.tmp)
            .replace("{repo}", &self.repo);
        match &self.address {
            Some((recorded, bound)) => replace_address(&text, recorded, bound),
            None => text,
        }
    }

    fn back(&self, text: &str) -> String {
        if self.tmp.is_empty() {
            return text.to_owned();
        }
        let text = text
            .replace(&self.tmp, "{tmp}")
            .replace(&self.repo, "{repo}");
        match &self.address {
            Some((recorded, bound)) => replace_address(&text, bound, recorded),
            None => text,
        }
    }
}

/// `text` with each `from` address (`host:port`) that no further digit
/// extends replaced by `to`.
fn replace_address(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(from) {
        let end = at + from.len();
        out.push_str(&rest[..at]);
        if rest[end..].starts_with(|c: char| c.is_ascii_digit()) {
            out.push_str(from);
        } else {
            out.push_str(to);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn fnv64(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn big(bytes: &[u8]) -> Value {
    json!({"fnv64": fnv64(bytes), "len": bytes.len()})
}

/// Wall-clock `elapsed_ms` values become 0, as the parity tests fixed them.
pub fn fixed_elapsed(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("\"elapsed_ms\":") {
        let after = at + "\"elapsed_ms\":".len();
        out.push_str(&rest[..after]);
        rest = &rest[after..];
        if let Some(stripped) = rest.strip_prefix(' ') {
            out.push(' ');
            rest = stripped;
        }
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            out.push('0');
            rest = &rest[digits..];
        }
    }
    out.push_str(rest);
    out
}

fn ignored(relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    relative.split('/').any(|part| part == "owners")
        || name.contains(".lock")
        || name.ends_with("-wal")
        || name.ends_with("-shm")
        || name.ends_with("-journal")
        || name.starts_with(".operator-owner")
        || name == "served.jsonl"
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard padded base64 (the tape's encoding); no crate for a test.
pub fn b64encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn b64decode(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for byte in text.bytes().filter(|b| *b != b'=') {
        let value = ALPHABET
            .iter()
            .position(|a| *a == byte)
            .unwrap_or_else(|| panic!("base64 byte {byte}")) as u32;
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

// ---------------------------------------------------------------- books

fn cell(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(r) => json!({"real": format!("{:016x}", r.to_bits())}),
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => Value::String(fixed_elapsed(&PATHS.with(|p| p.borrow().back(text)))),
            Err(_) => json!({"text_hex": hex(bytes)}),
        },
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).expect("hex"))
        .collect()
}

/// A book's logical state: schema rows, pragmas, and rows by rowid.
#[derive(Clone, PartialEq)]
struct Book {
    schema: Vec<Value>,
    pragmas: BTreeMap<String, i64>,
    tables: BTreeMap<String, Table>,
}

#[derive(Clone, PartialEq)]
enum Table {
    Rows(BTreeMap<i64, Vec<Value>>),
    WithoutRowid(Vec<Vec<Value>>),
}

fn dump(path: &Path) -> Book {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let schema = conn
        .prepare(
            "SELECT type,name,tbl_name,sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY rowid",
        )
        .unwrap()
        .query_map([], |row| {
            Ok(Value::Array(
                (0..4).map(|i| cell(row.get_ref(i).unwrap())).collect(),
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut pragmas = BTreeMap::new();
    for pragma in ["user_version", "application_id"] {
        let value: i64 = conn
            .query_row(&format!("PRAGMA {pragma}"), [], |r| r.get(0))
            .unwrap();
        pragmas.insert(pragma.to_owned(), value);
    }
    let names: Vec<(String, String)> = conn
        .prepare(
            "SELECT name, coalesce(sql,'') FROM sqlite_master WHERE type='table' ORDER BY name",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut tables = BTreeMap::new();
    for (name, sql) in names {
        let without_rowid = sql.to_uppercase().contains("WITHOUT ROWID");
        let query = if without_rowid {
            format!("SELECT * FROM \"{name}\"")
        } else {
            format!("SELECT rowid,* FROM \"{name}\" ORDER BY rowid")
        };
        let mut statement = conn.prepare(&query).unwrap();
        let width = statement.column_count();
        let rows: Vec<Vec<Value>> = statement
            .query_map([], |row| {
                Ok((0..width).map(|i| cell(row.get_ref(i).unwrap())).collect())
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let table = if without_rowid {
            let mut rows = rows;
            rows.sort_by_key(|r| serde_json::to_string(r).unwrap());
            Table::WithoutRowid(rows)
        } else {
            Table::Rows(
                rows.into_iter()
                    .map(|mut r| (r.remove(0).as_i64().expect("rowid"), r))
                    .collect(),
            )
        };
        tables.insert(name, table);
    }
    Book {
        schema,
        pragmas,
        tables,
    }
}

fn empty_book() -> Book {
    Book {
        schema: vec![],
        pragmas: BTreeMap::new(),
        tables: BTreeMap::new(),
    }
}

fn expect_cell(value: &Value) -> Value {
    match value {
        Value::String(text) if text.len() > BIG => big(text.as_bytes()),
        Value::Object(map) => match map.get("text_hex").and_then(Value::as_str) {
            Some(h) if h.len() > 2 * BIG => big(&unhex(h)),
            _ => value.clone(),
        },
        _ => value.clone(),
    }
}

/// The patch that turns `before` into `after` (the generator's format).
fn book_patch(before: &Book, after: &Book, for_expect: bool) -> Option<Value> {
    let mut patch = Map::new();
    if before.schema != after.schema {
        patch.insert("schema".into(), Value::Array(after.schema.clone()));
    }
    if before.pragmas != after.pragmas {
        patch.insert("pragmas".into(), json!(after.pragmas));
    }
    let cellf = |v: &Value| {
        if for_expect {
            expect_cell(v)
        } else {
            v.clone()
        }
    };
    let mut tables = Map::new();
    let names: BTreeSet<&String> = before.tables.keys().chain(after.tables.keys()).collect();
    let none = Table::Rows(BTreeMap::new());
    for name in names {
        let a = before.tables.get(name).unwrap_or(&none);
        let b = after.tables.get(name).unwrap_or(&none);
        match (a, b) {
            (Table::Rows(a), Table::Rows(b)) => {
                let delete: Vec<i64> = a.keys().filter(|k| !b.contains_key(k)).copied().collect();
                let upsert: Vec<Value> = b
                    .iter()
                    .filter(|(k, v)| a.get(k) != Some(v))
                    .map(|(k, v)| {
                        let mut row = vec![json!(k)];
                        row.extend(v.iter().map(cellf));
                        Value::Array(row)
                    })
                    .collect();
                let mut change = Map::new();
                if !delete.is_empty() {
                    change.insert("delete".into(), json!(delete));
                }
                if !upsert.is_empty() {
                    change.insert("upsert".into(), Value::Array(upsert));
                }
                if !change.is_empty() {
                    tables.insert(name.clone(), Value::Object(change));
                }
            }
            _ => {
                if a != b {
                    let rows = match b {
                        Table::WithoutRowid(rows) => rows
                            .iter()
                            .map(|r| Value::Array(r.iter().map(cellf).collect()))
                            .collect(),
                        Table::Rows(_) => vec![],
                    };
                    tables.insert(name.clone(), json!({"replace": rows}));
                }
            }
        }
    }
    if for_expect {
        for change in tables.values_mut() {
            let Some(change) = change.as_object_mut() else {
                continue;
            };
            for key in ["upsert", "replace", "delete"] {
                if change
                    .get(key)
                    .and_then(Value::as_array)
                    .is_some_and(|rows| rows.len() > MANY)
                {
                    let rows = change.remove(key).unwrap();
                    let mut digest = big(serde_json::to_string(&rows).unwrap().as_bytes());
                    digest["rows"] = json!(rows.as_array().unwrap().len());
                    change.insert(format!("{key}_digest"), digest);
                }
            }
        }
    }
    if !tables.is_empty() {
        patch.insert("tables".into(), Value::Object(tables));
    }
    (!patch.is_empty()).then_some(Value::Object(patch))
}

fn sql_value(value: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as V;
    match value {
        Value::Null => V::Null,
        Value::Number(n) => V::Integer(n.as_i64().expect("integer cell")),
        Value::String(text) => V::Text(PATHS.with(|p| p.borrow().forward(text))),
        Value::Object(map) => {
            if let Some(bits) = map.get("real").and_then(Value::as_str) {
                V::Real(f64::from_bits(u64::from_str_radix(bits, 16).unwrap()))
            } else if let Some(h) = map.get("text_hex").and_then(Value::as_str) {
                V::Blob(unhex(h))
            } else {
                panic!("cell {value}")
            }
        }
        other => panic!("cell {other}"),
    }
}

fn columns(conn: &Connection, table: &str) -> Vec<String> {
    conn.prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn quoted(names: &[String]) -> String {
    names
        .iter()
        .map(|n| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(",")
}

fn apply_tables(conn: &Connection, tables: &Map<String, Value>) {
    for (table, change) in tables {
        let cols = columns(conn, table);
        if let Some(rows) = change.get("replace").and_then(Value::as_array) {
            conn.execute(&format!("DELETE FROM \"{table}\""), [])
                .unwrap();
            for row in rows {
                insert(conn, table, &cols, false, row.as_array().unwrap());
            }
            continue;
        }
        let empty = vec![];
        let delete = change
            .get("delete")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        let upsert = change
            .get("upsert")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        for rowid in delete
            .iter()
            .chain(upsert.iter().map(|r| &r.as_array().unwrap()[0]))
        {
            conn.execute(
                &format!("DELETE FROM \"{table}\" WHERE rowid=?1"),
                [rowid.as_i64().unwrap()],
            )
            .unwrap();
        }
        for row in upsert {
            insert(conn, table, &cols, true, row.as_array().unwrap());
        }
    }
}

fn insert(conn: &Connection, table: &str, cols: &[String], rowid: bool, row: &[Value]) {
    let mut names = Vec::new();
    if rowid {
        names.push("rowid".to_owned());
    }
    names.extend(cols.iter().cloned());
    assert_eq!(names.len(), row.len(), "{table}: row width");
    let marks = (1..=row.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let values: Vec<rusqlite::types::Value> = row.iter().map(sql_value).collect();
    conn.execute(
        &format!(
            "INSERT INTO \"{table}\"({}) VALUES({marks})",
            quoted(&names)
        ),
        rusqlite::params_from_iter(values),
    )
    .unwrap_or_else(|e| panic!("{table}: {e}"));
}

/// Schema rows `[type, name, tbl_name, sql]`, tables and views first.
fn create_schema(conn: &Connection, schema: &[Value], early: bool) {
    for object in schema {
        let kind = object[0].as_str().unwrap();
        let name = object[1].as_str().unwrap();
        if name.starts_with("sqlite_") {
            continue;
        }
        if matches!(kind, "table" | "view") == early {
            conn.execute_batch(object[3].as_str().unwrap()).unwrap();
        }
    }
}

fn current_schema() -> Vec<Value> {
    let manifest: Value =
        serde_json::from_str(include_str!("../../../store/fixtures/current-schema.json")).unwrap();
    let mut out = Vec::new();
    for kind in ["table", "index", "view", "trigger"] {
        for object in manifest["objects"].as_array().unwrap() {
            if object[0] == kind && object[3].is_string() {
                out.push(object.clone());
            }
        }
    }
    out
}

fn set_pragmas(conn: &Connection, pragmas: &Value) {
    if let Some(map) = pragmas.as_object() {
        for (name, value) in map {
            conn.execute_batch(&format!("PRAGMA {name}={}", value.as_i64().unwrap()))
                .unwrap();
        }
    }
}

fn create_book(path: &Path, spec: &Value) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{suffix}", path.display()));
    }
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    let schema = match &spec["schema"] {
        Value::String(s) if s == "current" => current_schema(),
        Value::Array(list) => list.clone(),
        other => panic!("schema {other}"),
    };
    conn.execute_batch("BEGIN").unwrap();
    create_schema(&conn, &schema, true);
    if let Some(tables) = spec["tables"].as_object() {
        apply_tables(&conn, tables);
    }
    create_schema(&conn, &schema, false);
    conn.execute_batch("COMMIT").unwrap();
    set_pragmas(&conn, &spec["pragmas"]);
}

fn patch_book(path: &Path, patch: &Value) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    let triggers: Vec<(String, String)> = conn
        .prepare("SELECT name, sql FROM sqlite_master WHERE type='trigger'")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    conn.execute_batch("BEGIN").unwrap();
    for (name, _) in &triggers {
        conn.execute_batch(&format!("DROP TRIGGER \"{name}\""))
            .unwrap();
    }
    let mut keep_triggers = triggers.clone();
    if let Some(schema) = patch.get("schema").and_then(Value::as_array) {
        let old = dump(path).schema;
        let gone: Vec<&Value> = old.iter().filter(|o| !schema.contains(o)).collect();
        let new: Vec<Value> = schema
            .iter()
            .filter(|o| !old.contains(o))
            .cloned()
            .collect();
        for object in gone {
            let (kind, name) = (object[0].as_str().unwrap(), object[1].as_str().unwrap());
            if kind == "trigger" {
                keep_triggers.retain(|(n, _)| n != name);
            } else {
                conn.execute_batch(&format!("DROP {} \"{name}\"", kind.to_uppercase()))
                    .unwrap();
            }
        }
        create_schema(&conn, &new, true);
        let later: Vec<Value> = new.into_iter().filter(|o| o[0] != "trigger").collect();
        create_schema(&conn, &later, false);
        for object in schema.iter().filter(|o| o[0] == "trigger") {
            let name = object[1].as_str().unwrap().to_owned();
            if !keep_triggers.iter().any(|(n, _)| *n == name) {
                keep_triggers.push((name, object[3].as_str().unwrap().to_owned()));
            }
        }
    }
    if let Some(tables) = patch.get("tables").and_then(Value::as_object) {
        apply_tables(&conn, tables);
    }
    for (_, sql) in &keep_triggers {
        conn.execute_batch(sql).unwrap();
    }
    conn.execute_batch("COMMIT").unwrap();
    if let Some(pragmas) = patch.get("pragmas") {
        set_pragmas(&conn, pragmas);
    }
}

// ---------------------------------------------------------------- files

#[derive(Clone, PartialEq)]
enum Entry {
    Dir(u32),
    File(u32, Vec<u8>),
    Symlink(String),
    Book(Book),
}

fn is_sqlite(path: &Path) -> bool {
    let mut head = [0u8; 16];
    fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && &head == b"SQLite format 3\0"
}

fn snapshot(root: &Path, books: &[PathBuf], out: &mut BTreeMap<PathBuf, Entry>) {
    let Ok(meta) = fs::symlink_metadata(root) else {
        return;
    };
    let mut pending = vec![(root.to_path_buf(), meta)];
    while let Some((path, meta)) = pending.pop() {
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if !relative.is_empty() && ignored(&relative) {
            continue;
        }
        let mode = meta.permissions().mode() & 0o7777;
        if meta.file_type().is_symlink() {
            let target = fs::read_link(&path).unwrap();
            out.insert(path, Entry::Symlink(target.to_string_lossy().into_owned()));
        } else if meta.is_dir() {
            if !relative.is_empty() {
                out.insert(path.clone(), Entry::Dir(mode));
            }
            for child in fs::read_dir(&path).unwrap().flatten() {
                if let Ok(meta) = fs::symlink_metadata(child.path()) {
                    pending.push((child.path(), meta));
                }
            }
        } else if meta.is_file() {
            if is_sqlite(&path) {
                if books.iter().any(|b| path.starts_with(b)) {
                    out.insert(path.clone(), Entry::Book(dump(&path)));
                }
                continue;
            }
            out.insert(path.clone(), Entry::File(mode, fs::read(&path).unwrap()));
        }
    }
}

fn file_value(bytes: &[u8], mode: u32, paths: &Paths, for_expect: bool) -> Value {
    let mut out = Map::new();
    out.insert("mode".into(), json!(mode));
    if bytes.len() <= 65536 {
        if let Ok(text) = std::str::from_utf8(bytes) {
            out.insert("text".into(), Value::String(paths.back(text)));
            return Value::Object(out);
        }
    } else if for_expect {
        out.insert("sha256".into(), Value::String("(see base64)".into()));
    }
    out.insert("base64".into(), Value::String(b64encode(bytes)));
    Value::Object(out)
}

/// `entry` with in-window wall-clock readings as `<now>` (strings in book
/// cells, text files), so a rewrite that only moves a clock is no change.
fn timeless(entry: &Entry, paths: &Paths, windows: &[(i64, i64)]) -> Entry {
    let times = |text: &str| times_only(text, windows);
    match entry {
        Entry::Book(book) => {
            let cell = |v: &Value| match v {
                Value::String(s) => Value::String(times(s)),
                other => other.clone(),
            };
            let tables = book
                .tables
                .iter()
                .map(|(name, table)| {
                    let table = match table {
                        Table::Rows(rows) => Table::Rows(
                            rows.iter()
                                .map(|(k, r)| (*k, r.iter().map(cell).collect()))
                                .collect(),
                        ),
                        Table::WithoutRowid(rows) => {
                            let mut rows: Vec<Vec<Value>> =
                                rows.iter().map(|r| r.iter().map(cell).collect()).collect();
                            rows.sort_by_key(|r| serde_json::to_string(r).unwrap());
                            Table::WithoutRowid(rows)
                        }
                    };
                    (name.clone(), table)
                })
                .collect();
            Entry::Book(Book {
                schema: book.schema.clone(),
                pragmas: book.pragmas.clone(),
                tables,
            })
        }
        Entry::File(mode, bytes) if bytes.len() <= 65536 => match std::str::from_utf8(bytes) {
            Ok(text) => Entry::File(*mode, times(&paths.back(text)).into_bytes()),
            Err(_) => entry.clone(),
        },
        other => other.clone(),
    }
}

fn changes(
    before: &BTreeMap<PathBuf, Entry>,
    after: &BTreeMap<PathBuf, Entry>,
    paths: &Paths,
    windows: &[(i64, i64)],
) -> Value {
    let before: BTreeMap<PathBuf, Entry> = before
        .iter()
        .map(|(p, e)| (p.clone(), timeless(e, paths, windows)))
        .collect();
    let after: BTreeMap<PathBuf, Entry> = after
        .iter()
        .map(|(p, e)| (p.clone(), timeless(e, paths, windows)))
        .collect();
    let (before, after) = (&before, &after);
    let mut out = Map::new();
    let key = |p: &Path| paths.back(&p.to_string_lossy());
    for path in before.keys().filter(|p| !after.contains_key(*p)) {
        out.insert(key(path), json!({"deleted": true}));
    }
    for (path, entry) in after {
        let old = before.get(path);
        let value = match entry {
            Entry::Book(book) => {
                let base = match old {
                    Some(Entry::Book(b)) => b.clone(),
                    _ => empty_book(),
                };
                match book_patch(&base, book, true) {
                    Some(patch) => json!({"book": patch}),
                    None if old.is_none() => json!({"book": {}}),
                    None => continue,
                }
            }
            _ if old == Some(entry) => continue,
            Entry::Dir(_) => json!({"dir": true}),
            Entry::Symlink(target) => json!({"symlink": paths.back(target)}),
            Entry::File(mode, bytes) => file_value(bytes, *mode, paths, true),
        };
        out.insert(key(path), value);
    }
    Value::Object(out)
}

/// Large file contents compare by hash: the generator stored `sha256` for
/// them, and this harness only compares the base64 body.
fn comparable(changes: &Value) -> Value {
    let mut changes = changes.clone();
    if let Some(map) = changes.as_object_mut() {
        for value in map.values_mut() {
            if let Some(file) = value.as_object_mut() {
                file.remove("sha256");
            }
        }
    }
    changes
}

// ---------------------------------------------------------------- setup

fn under(tmp: &Path, path: &Path) {
    assert!(
        path.starts_with(tmp),
        "setup path {} is outside the test directory",
        path.display()
    );
}

fn remove(path: &Path) {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path).unwrap(),
        Ok(_) => fs::remove_file(path).unwrap(),
        Err(_) => {}
    }
}

fn apply_setup(op: &Value, paths: &Paths, tmp: &Path) {
    let target = |key: &str| PathBuf::from(paths.forward(op[key].as_str().unwrap()));
    let mode = || op["mode"].as_u64().map(|m| m as u32);
    if op.get("delete").is_some() {
        let path = target("delete");
        under(tmp, &path);
        remove(&path);
    } else if op.get("mkdir").is_some() {
        let path = target("mkdir");
        under(tmp, &path);
        fs::create_dir_all(&path).unwrap();
        if let Some(mode) = mode() {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        }
    } else if op.get("symlink").is_some() {
        let path = target("symlink");
        under(tmp, &path);
        remove(&path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(paths.forward(op["target"].as_str().unwrap()), &path).unwrap();
    } else if op.get("write").is_some() {
        let path = target("write");
        under(tmp, &path);
        remove(&path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = match (op.get("text"), op.get("base64")) {
            (Some(text), _) => paths.forward(text.as_str().unwrap()).into_bytes(),
            (None, Some(b)) => b64decode(b.as_str().unwrap()),
            _ => panic!("write without content: {op}"),
        };
        fs::write(&path, bytes).unwrap();
        if let Some(mode) = mode() {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        }
    } else if op.get("book").is_some() {
        let path = target("book");
        under(tmp, &path);
        if let Some(spec) = op.get("create") {
            create_book(&path, spec);
        } else {
            patch_book(&path, &op["patch"]);
        }
    } else {
        panic!("unknown setup {op}");
    }
}

// ---------------------------------------------------------------- tape

/// kassiber-net's tape line format from the golden's compressed form.
fn tape_lines(tape: &Value) -> String {
    let b64 = b64encode;
    let templates: Vec<&str> = tape["templates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    let payload = |value: &Value| -> Vec<u8> {
        match value {
            Value::String(text) => text.clone().into_bytes(),
            Value::Array(parts) => {
                let template = templates[parts[0].as_u64().unwrap() as usize];
                let mut args = parts[1..].iter().map(|a| a.as_str().unwrap());
                let mut out = String::new();
                for (i, piece) in template.split('\u{1}').enumerate() {
                    if i > 0 {
                        out.push_str(args.next().expect("template argument"));
                    }
                    out.push_str(piece);
                }
                out.into_bytes()
            }
            Value::Object(map) => b64decode(map["base64"].as_str().unwrap()),
            other => panic!("payload {other}"),
        }
    };
    let mut out = String::new();
    for conn in tape["connections"].as_array().unwrap() {
        let id = conn[0].as_u64().unwrap();
        out.push_str(&format!(
            "open\t{id}\t{}\t{}\t{}\n",
            u8::from(conn[1].as_bool().unwrap()),
            b64(conn[2].as_str().unwrap().as_bytes()),
            conn[3]
        ));
        if let Some(n) = conn[4].as_u64() {
            out.push_str(&format!("attempts\t{id}\t{n}\n"));
        }
        if let Some(fail) = conn[5].as_str() {
            out.push_str(&format!("fail\t{id}\t{fail}\n"));
        }
        for event in conn[6].as_array().unwrap() {
            match event[0].as_str().unwrap() {
                "w" => {
                    let count = event[1].as_u64().unwrap();
                    match event.get(2) {
                        Some(p) => {
                            out.push_str(&format!("w\t{id}\t{count}\t{}\n", b64(&payload(p))))
                        }
                        None => out.push_str(&format!("w\t{id}\t{count}\t\n")),
                    }
                }
                "r" => out.push_str(&format!("r\t{id}\t{}\n", b64(&payload(&event[1])))),
                "re" => out.push_str(&format!("re\t{id}\t{}\n", event[1].as_str().unwrap())),
                "eof" => out.push_str(&format!("eof\t{id}\n")),
                other => panic!("tape event {other}"),
            }
        }
    }
    out
}

/// Newline-delimited JSON-RPC answers by request (method and params), from a
/// tape that kept every write, served by `kassiber-net`'s loopback fake on
/// a free port. Returns the fake and the address it stands for, the
/// recorded `127.0.0.1:<port>` the session's inputs and outputs name; a
/// fixed port could already be taken on the machine running the test.
fn jsonrpc_server(tape: &Value) -> (kassiber_net::test_support::FakeJsonRpc, String) {
    let text = tape_lines(tape);
    let mut writes: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut reads: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut recorded: Option<String> = None;
    for line in text.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        match parts[0] {
            "open" => {
                let host = String::from_utf8(b64decode(parts[3])).unwrap();
                let address = format!("{host}:{}", parts[4]);
                assert!(
                    recorded.as_ref().is_none_or(|a| *a == address),
                    "a served tape names one address: {recorded:?} and {address}"
                );
                recorded = Some(address);
            }
            "w" => writes
                .entry(parts[1].to_owned())
                .or_default()
                .extend(b64decode(parts.get(3).copied().unwrap_or(""))),
            "r" => reads
                .entry(parts[1].to_owned())
                .or_default()
                .extend(b64decode(parts[2])),
            _ => {}
        }
    }
    let parse = |bytes: &[u8]| -> Vec<Value> {
        String::from_utf8_lossy(bytes)
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .collect()
    };
    let mut answers: BTreeMap<String, Value> = BTreeMap::new();
    for (id, written) in &writes {
        let replies: BTreeMap<String, Value> =
            parse(reads.get(id).map(Vec::as_slice).unwrap_or(&[]))
                .into_iter()
                .filter_map(|r| Some((r.get("id")?.to_string(), r)))
                .collect();
        for request in parse(written) {
            let (Some(rid), Some(method)) = (request.get("id"), request.get("method")) else {
                continue;
            };
            if let Some(reply) = replies.get(&rid.to_string()) {
                answers
                    .entry(format!("{method}{}", request["params"]))
                    .or_insert_with(|| reply.clone());
            }
        }
    }
    let recorded = recorded.expect("a served tape opens a connection");
    assert!(
        recorded.starts_with("127.0.0.1:"),
        "a served tape is loopback: {recorded}"
    );
    let fake = kassiber_net::test_support::FakeJsonRpc::start(0, move |line| {
        let request: Value = serde_json::from_str(line).unwrap_or(Value::Null);
        let key = format!("{}{}", request["method"], request["params"]);
        let mut reply = answers.get(&key).cloned().unwrap_or_else(
            || json!({"jsonrpc": "2.0", "error": {"code": -32601, "message": "not recorded"}}),
        );
        reply["id"] = request["id"].clone();
        reply.to_string()
    })
    .expect("bind a loopback port");
    (fake, recorded)
}

// ---------------------------------------------------------------- process

/// How many stdout lines to wait for before each stdin chunk. The recorder
/// counted lines when the chunk arrived, which can trail the reply the test
/// waited for; so a chunk also waits for the final (non-progress) reply to
/// every request the previous chunk sent, where the recorded stdout has it.
fn stdin_thresholds(chunks: &[Value], stdout: Option<&str>) -> Vec<u64> {
    let lines: Vec<Value> = stdout
        .unwrap_or("")
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or(Value::Null))
        .collect();
    let mut out = Vec::new();
    let mut previous: Vec<String> = Vec::new();
    for chunk in chunks {
        let mut after = chunk["after_lines"].as_u64().unwrap();
        for id in &previous {
            let last = lines.iter().rposition(|line| {
                line.get("request_id").and_then(Value::as_str) == Some(id.as_str())
                    && !line
                        .get("kind")
                        .and_then(Value::as_str)
                        .is_some_and(|k| k.ends_with(".progress"))
            });
            if let Some(last) = last {
                after = after.max(last as u64 + 1);
            }
        }
        previous = chunk
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|v| {
                v.get("request_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        out.push(after);
    }
    out
}

struct Lines {
    /// Lines printed, end of output, and the request ids a final
    /// (non-progress) reply answered.
    count: Mutex<(u64, bool, BTreeSet<String>)>,
    changed: Condvar,
}

/// The request id a final (non-progress) reply line answers.
fn final_reply(line: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(line).ok()?;
    let kind = value.get("kind").and_then(Value::as_str).unwrap_or("");
    if kind.ends_with(".progress") {
        return None;
    }
    value
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn request_ids(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| {
            v.get("request_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

fn run_process(
    session: &Value,
    paths: &Paths,
    tape: Option<&Path>,
    fallback: &Path,
) -> (Option<i32>, Vec<u8>, Vec<u8>) {
    let args: Vec<String> = session["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| paths.forward(a.as_str().unwrap()))
        .collect();
    let mut command = Command::new(env!("CARGO_BIN_EXE_kassiber"));
    command
        .args(&args)
        .current_dir(paths.forward(session["cwd"].as_str().unwrap()))
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in ["PATH", "HOME", "LANG"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    // Never the account's app-data root, whatever the recording held.
    command.env("XDG_DATA_HOME", fallback.join("xdg-data"));
    command.env(
        "KASSIBER_OFFLINE_PREFERENCE_FILE",
        fallback.join("offline-preference.json"),
    );
    for (key, value) in session["env"].as_object().unwrap() {
        command.env(key, paths.forward(value.as_str().unwrap()));
    }
    if let Some(tape) = tape {
        command.env(
            "KASSIBER_TEST_NET_TAPE",
            format!("replay-strict:{}", tape.display()),
        );
    }
    let mut child = command.spawn().expect("the binary starts");
    let lines = Arc::new(Lines {
        count: Mutex::new((0, false, BTreeSet::new())),
        changed: Condvar::new(),
    });
    let mut stdout = child.stdout.take().unwrap();
    let reader_lines = Arc::clone(&lines);
    let reader = std::thread::spawn(move || {
        let mut all = Vec::new();
        let mut buffer = [0u8; 65536];
        loop {
            let n = stdout.read(&mut buffer).unwrap_or(0);
            let mut state = reader_lines.count.lock().unwrap();
            if n == 0 {
                state.1 = true;
                reader_lines.changed.notify_all();
                break;
            }
            let start = all.len();
            all.extend_from_slice(&buffer[..n]);
            state.0 += buffer[..n].iter().filter(|b| **b == b'\n').count() as u64;
            // The lines this chunk completed: from the start of the line it
            // continued up to its last newline.
            let from = all[..start]
                .iter()
                .rposition(|b| *b == b'\n')
                .map_or(0, |i| i + 1);
            let to = all.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
            if to > from {
                for line in all[from..to - 1].split(|b| *b == b'\n') {
                    if let Some(id) = final_reply(line) {
                        state.2.insert(id);
                    }
                }
            }
            reader_lines.changed.notify_all();
        }
        all
    });
    let mut stderr = child.stderr.take().unwrap();
    let errors = std::thread::spawn(move || {
        let mut all = Vec::new();
        let _ = stderr.read_to_end(&mut all);
        all
    });
    let mut stdin = child.stdin.take();
    let chunks: Vec<Value> = session
        .get("stdin")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| vec![json!({"after_lines": 0, "close": true})]);
    let writer_lines = Arc::clone(&lines);
    let thresholds = stdin_thresholds(&chunks, session["expect"]["stdout"].as_str());
    let mut previous: Vec<String> = Vec::new();
    let forward: Vec<(u64, Vec<String>, Option<String>)> = chunks
        .iter()
        .zip(thresholds)
        .map(|(c, after)| {
            let text = c.get("text").map(|t| paths.forward(t.as_str().unwrap()));
            let waits =
                std::mem::replace(&mut previous, request_ids(text.as_deref().unwrap_or("")));
            (after, waits, text)
        })
        .collect();
    let writer = std::thread::spawn(move || {
        for (after, waits, text) in forward {
            // Wait for as much output as the recording had seen; a process
            // that already gave its final reply to every earlier request
            // (a shorter, diverging answer) gets the chunk at once.
            let deadline = Instant::now() + TIMEOUT;
            let mut state = writer_lines.count.lock().unwrap();
            while state.0 < after
                && !state.1
                && Instant::now() < deadline
                && (waits.is_empty() || !waits.iter().all(|id| state.2.contains(id)))
            {
                state = writer_lines
                    .changed
                    .wait_timeout(state, Duration::from_millis(200))
                    .unwrap()
                    .0;
            }
            drop(state);
            match text {
                Some(text) => {
                    if let Some(pipe) = stdin.as_mut() {
                        let _ = pipe.write_all(text.as_bytes());
                        let _ = pipe.flush();
                    }
                }
                None => {
                    stdin.take();
                }
            }
        }
        drop(stdin);
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let out = reader.join().unwrap();
    let err = errors.join().unwrap();
    writer.join().unwrap();
    let code = status.and_then(|s| s.code());
    (code, out, err)
}

// ---------------------------------------------------------------- canon

/// Generated identities and wall-clock readings differ between the
/// recording and a replay. Both outcomes map each uuid to its order of first
/// appearance, and an ISO timestamp inside the run's window to `<now>`;
/// fixed fixture timestamps stay as they are.
pub struct Canon {
    uuids: BTreeMap<String, usize>,
    next: usize,
    windows: Vec<(i64, i64)>,
}

impl Canon {
    fn new() -> Self {
        Self {
            uuids: BTreeMap::new(),
            next: 0,
            windows: Vec::new(),
        }
    }

    fn text(&mut self, text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        let mut copied = 0;
        while i < bytes.len() {
            if let Some(len) = token_at(bytes, i) {
                out.push_str(&text[copied..i]);
                let next = self.next;
                let n = *self
                    .uuids
                    .entry(text[i..i + len].to_owned())
                    .or_insert_with(|| next);
                if n == next {
                    self.next += 1;
                }
                out.push_str(&format!("<id-{n}>"));
                i += len;
                copied = i;
                continue;
            }
            if let Some((len, us)) = timestamp_at(bytes, i) {
                if self.windows.iter().any(|(a, b)| *a <= us && us <= *b) {
                    out.push_str(&text[copied..i]);
                    out.push_str("<now>");
                    i += len;
                    copied = i;
                    continue;
                }
                i += len;
                continue;
            }
            i += 1;
        }
        out.push_str(&text[copied..]);
        out
    }

    fn value(&mut self, value: &mut Value) {
        match value {
            Value::String(text) => *text = self.text(text),
            Value::Array(items) => items.iter_mut().for_each(|v| self.value(v)),
            Value::Object(map) => {
                let entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
                for (key, mut item) in entries {
                    self.value(&mut item);
                    map.insert(self.text(&key), item);
                }
            }
            _ => {}
        }
    }
}

/// Only the clock part of `Canon`: in-window ISO timestamps become `<now>`.
pub fn times_only(text: &str, windows: &[(i64, i64)]) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut copied) = (0, 0);
    while i < bytes.len() {
        if let Some((len, us)) = timestamp_at(bytes, i) {
            if windows.iter().any(|(a, b)| *a <= us && us <= *b) {
                out.push_str(&text[copied..i]);
                out.push_str("<now>");
                copied = i + len;
            }
            i += len;
            continue;
        }
        i += 1;
    }
    out.push_str(&text[copied..]);
    out
}

/// Replay identity to recorded identity, by the number both canons gave it.
fn id_map(recorded: &Canon, replayed: &Canon) -> BTreeMap<String, String> {
    let by_number: BTreeMap<usize, &String> = recorded.uuids.iter().map(|(k, n)| (*n, k)).collect();
    replayed
        .uuids
        .iter()
        .filter_map(|(k, n)| by_number.get(n).map(|v| (k.clone(), (*v).clone())))
        .filter(|(k, v)| k != v)
        .collect()
}

fn replace_ids(text: &str, map: &BTreeMap<String, String>) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut copied) = (0, 0);
    while i < bytes.len() {
        if let Some(len) = token_at(bytes, i) {
            if let Some(to) = map.get(&text[i..i + len]) {
                out.push_str(&text[copied..i]);
                out.push_str(to);
                copied = i + len;
            }
            i += len;
            continue;
        }
        i += 1;
    }
    out.push_str(&text[copied..]);
    out
}

/// Give the identities this replay generated back their recorded values in
/// every book and text file under `roots`, so later sessions (and orders
/// by id) see the recording's state. Triggers stay quiet meanwhile.
fn restore_ids(roots: &[PathBuf], books: &[PathBuf], map: &BTreeMap<String, String>) {
    if map.is_empty() {
        return;
    }
    let mut entries = BTreeMap::new();
    for root in roots {
        snapshot(root, books, &mut entries);
    }
    for (path, entry) in entries {
        match entry {
            Entry::File(mode, bytes) => {
                if let Ok(text) = std::str::from_utf8(&bytes) {
                    let replaced = replace_ids(text, map);
                    if replaced != text {
                        fs::write(&path, replaced).unwrap();
                        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
                    }
                }
            }
            Entry::Book(book) => {
                let conn = Connection::open(&path).unwrap();
                conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
                let triggers: Vec<(String, String)> = conn
                    .prepare("SELECT name, sql FROM sqlite_master WHERE type='trigger'")
                    .unwrap()
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                conn.execute_batch("BEGIN").unwrap();
                for (name, _) in &triggers {
                    conn.execute_batch(&format!("DROP TRIGGER \"{name}\""))
                        .unwrap();
                }
                for (table, kind) in &book.tables {
                    if !matches!(kind, Table::Rows(_)) {
                        continue;
                    }
                    let cols = columns(&conn, table);
                    let mut updates = Vec::new();
                    {
                        let mut statement = conn
                            .prepare(&format!("SELECT rowid,* FROM \"{table}\""))
                            .unwrap();
                        let mut rows = statement.query([]).unwrap();
                        while let Some(row) = rows.next().unwrap() {
                            let rowid: i64 = row.get(0).unwrap();
                            for (i, column) in cols.iter().enumerate() {
                                if let ValueRef::Text(bytes) = row.get_ref(i + 1).unwrap() {
                                    if let Ok(text) = std::str::from_utf8(bytes) {
                                        let replaced = replace_ids(text, map);
                                        if replaced != text {
                                            updates.push((column.clone(), replaced, rowid));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    for (column, text, rowid) in updates {
                        conn.execute(
                            &format!("UPDATE \"{table}\" SET \"{column}\"=?1 WHERE rowid=?2"),
                            rusqlite::params![text, rowid],
                        )
                        .unwrap_or_else(|e| panic!("{table}.{column}: {e}"));
                    }
                }
                for (_, sql) in &triggers {
                    conn.execute_batch(sql).unwrap();
                }
                conn.execute_batch("COMMIT").unwrap();
            }
            _ => {}
        }
    }
}

/// A uuid, or a bare 32-digit lowercase hex identity, at `i`.
fn token_at(bytes: &[u8], i: usize) -> Option<usize> {
    uuid_at(bytes, i).or_else(|| hex32_at(bytes, i))
}

fn hex32_at(bytes: &[u8], i: usize) -> Option<usize> {
    if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
        return None;
    }
    if !hex_run(bytes, i, 32) || bytes.get(i + 32).is_some_and(u8::is_ascii_alphanumeric) {
        return None;
    }
    // Hex only: a run of digits alone is a number, not an identity.
    bytes[i..i + 32]
        .iter()
        .any(|b| b.is_ascii_alphabetic())
        .then_some(32)
}

fn hex_run(bytes: &[u8], at: usize, n: usize) -> bool {
    bytes.len() >= at + n
        && bytes[at..at + n]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}

fn uuid_at(bytes: &[u8], i: usize) -> Option<usize> {
    if i > 0 && (bytes[i - 1].is_ascii_hexdigit()) {
        return None;
    }
    let mut at = i;
    for (index, n) in [8, 4, 4, 4, 12].into_iter().enumerate() {
        if !hex_run(bytes, at, n) {
            return None;
        }
        at += n;
        if index < 4 {
            if bytes.get(at) != Some(&b'-') {
                return None;
            }
            at += 1;
        }
    }
    if bytes.get(at).is_some_and(u8::is_ascii_hexdigit) {
        return None;
    }
    Some(at - i)
}

fn digits(bytes: &[u8], at: usize, n: usize) -> Option<i64> {
    if bytes.len() < at + n || !bytes[at..at + n].iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(&bytes[at..at + n]).ok()?.parse().ok()
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `YYYY-MM-DD[T ]HH:MM:SS[.f][Z|±HH:MM]` at `i`: its length and epoch µs.
fn timestamp_at(bytes: &[u8], i: usize) -> Option<(usize, i64)> {
    if i > 0 && bytes[i - 1].is_ascii_digit() {
        return None;
    }
    let year = digits(bytes, i, 4)?;
    if bytes.get(i + 4) != Some(&b'-') || bytes.get(i + 7) != Some(&b'-') {
        return None;
    }
    let month = digits(bytes, i + 5, 2)?;
    let day = digits(bytes, i + 8, 2)?;
    if !matches!(bytes.get(i + 10), Some(b'T') | Some(b' ')) {
        return None;
    }
    let hour = digits(bytes, i + 11, 2)?;
    let minute = digits(bytes, i + 14, 2)?;
    let second = digits(bytes, i + 17, 2)?;
    if bytes.get(i + 13) != Some(&b':') || bytes.get(i + 16) != Some(&b':') {
        return None;
    }
    let mut at = i + 19;
    let mut micros = 0i64;
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        let fraction = std::str::from_utf8(&bytes[start..at]).ok()?;
        let padded = format!("{:0<6}", &fraction[..fraction.len().min(6)]);
        micros = padded.parse().ok()?;
    }
    let mut offset = 0i64;
    match bytes.get(at) {
        Some(b'Z') => at += 1,
        Some(sign @ (b'+' | b'-')) if bytes.get(at + 3) == Some(&b':') => {
            let h = digits(bytes, at + 1, 2)?;
            let m = digits(bytes, at + 4, 2)?;
            offset = (h * 60 + m) * 60 * if *sign == b'+' { 1 } else { -1 };
            at += 6;
        }
        _ => {}
    }
    let seconds =
        days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    Some((at - i, seconds * 1_000_000 + micros))
}

fn now_us() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64
}

// ---------------------------------------------------------------- driver

fn write_mode() -> bool {
    match std::env::var("KASSIBER_GOLDEN_WRITE").as_deref() {
        Ok("1") => {
            assert!(
                std::env::var_os("CI").is_none(),
                "KASSIBER_GOLDEN_WRITE refuses to run under CI"
            );
            true
        }
        _ => false,
    }
}

fn stdout_value(text: &str) -> Value {
    if text.len() > 4 * BIG {
        big(text.as_bytes())
    } else {
        Value::String(text.to_owned())
    }
}

/// Replay golden `name` (`<module>/<test>`) and check every session.
pub fn replay(name: &str) {
    replay_with(name, |_, _| {})
}

thread_local! {
    static SERVED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// As `replay`, but each tape is served by a loopback Electrum JSON-RPC
/// server answering every request with its recorded reply (for clients whose
/// request order varies between runs, which a byte tape cannot follow).
pub fn replay_served(name: &str) {
    SERVED.with(|s| s.set(true));
    replay(name);
    SERVED.with(|s| s.set(false));
}

/// As `replay`, with `normalize(index, &mut outcome-json)` applied to both
/// the recorded and the replayed outcome of each session before comparing.
pub fn replay_with(name: &str, normalize: impl Fn(usize, &mut Value)) {
    let path = golden_path(name);
    let mut golden: Value = serde_json::from_str(
        &fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .unwrap();
    let base = tempfile::Builder::new()
        .prefix("routing-port-")
        .tempdir()
        .unwrap();
    let base_path = base.path().canonicalize().unwrap();
    let tmp = base_path.join("t");
    fs::create_dir_all(tmp.join("systmp")).unwrap();
    let fallback = base_path.join("fallback");
    fs::create_dir_all(&fallback).unwrap();
    let mut paths = Paths {
        tmp: tmp.to_string_lossy().into_owned(),
        repo: repo().to_string_lossy().into_owned(),
        address: None,
    };
    PATHS.with(|p| *p.borrow_mut() = paths.clone());
    let sessions = golden["sessions"].as_array().unwrap().clone();
    // Every data root a session names holds books; other SQLite files (the
    // Python reference's book) were never part of the Rust sessions.
    let mut books: Vec<PathBuf> = Vec::new();
    for session in &sessions {
        let args: Vec<&str> = session["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        for (i, arg) in args.iter().enumerate() {
            if *arg == "--data-root" {
                books.push(PathBuf::from(paths.forward(args[i + 1])));
            } else if let Some(root) = arg.strip_prefix("--data-root=") {
                books.push(PathBuf::from(paths.forward(root)));
            }
        }
    }
    let writing = write_mode();
    let replay_started = now_us();
    let (mut expected_canon, mut actual_canon) = (Canon::new(), Canon::new());
    let mut failures = Vec::new();
    for (index, session) in sessions.iter().enumerate() {
        let served = SERVED.with(|s| s.get());
        let server = session.get("tape").filter(|_| served).map(jsonrpc_server);
        paths.address = server.as_ref().map(|(fake, recorded)| {
            let bound = fake.url().strip_prefix("tcp://").unwrap().to_owned();
            (recorded.clone(), bound)
        });
        PATHS.with(|p| *p.borrow_mut() = paths.clone());
        for op in session
            .get("setup")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            apply_setup(&op, &paths, &tmp);
        }
        let roots: Vec<PathBuf> = session["roots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| PathBuf::from(paths.forward(r.as_str().unwrap())))
            .collect();
        let mut before = BTreeMap::new();
        for root in &roots {
            snapshot(root, &books, &mut before);
        }
        let tape = session.get("tape").filter(|_| !served).map(|tape| {
            let file = base_path.join(format!("tape-{index}"));
            fs::write(&file, tape_lines(tape)).unwrap();
            file
        });
        let started = now_us();
        let (exit, out, err) = run_process(session, &paths, tape.as_deref(), &fallback);
        let ended = now_us();
        let _ = fs::write(base_path.join(format!("session-{index}.out")), &out);
        let _ = fs::write(base_path.join(format!("session-{index}.err")), &err);
        let mut after = BTreeMap::new();
        for root in &roots {
            snapshot(root, &books, &mut after);
        }
        let recorded = &session["recorded_us"];
        let recorded_window = (
            recorded[0].as_i64().unwrap() - LOOKBACK_US,
            recorded[2].as_i64().unwrap() + 2_000_000,
        );
        let replay_window = (replay_started - LOOKBACK_US, ended + 2_000_000);
        let windows = if writing {
            vec![replay_window]
        } else {
            vec![recorded_window, replay_window]
        };
        // A strict tape notes what the replay asked that the recording never
        // answered; asking more is a difference even when its failure is
        // swallowed.
        if let Some(tape) = &tape {
            let misses = PathBuf::from(format!("{}.misses", tape.display()));
            if let Ok(text) = fs::read_to_string(&misses) {
                failures.push(format!(
                    "session {index} {}: requests the recording never answered:\n{}",
                    session["args"],
                    clip(&Value::String(text))
                ));
            }
        }
        let stdout = fixed_elapsed(&paths.back(&String::from_utf8_lossy(&out)));
        let stderr = paths.back(&String::from_utf8_lossy(&err));
        let changed = changes(&before, &after, &paths, &windows);
        let mut actual = json!({
            "exit": exit,
            "stdout": stdout_value(&stdout),
            "stderr": stderr,
            "changes": comparable(&changed),
        });
        if writing {
            // Number identities as a comparison would, for later translation.
            let mut expected = session["expect"].clone();
            expected_canon.windows = vec![recorded_window];
            actual_canon.windows = windows.clone();
            expected_canon.value(&mut expected);
            actual_canon.value(&mut actual);
            golden["sessions"][index]["recorded_us"] = json!([replay_started, started, ended]);
            golden["sessions"][index]["expect"] = json!({
                "exit": exit,
                "stdout": stdout_value(&stdout),
                "stderr": stderr,
                "changes": changed,
            });
        } else {
            let mut expected = session["expect"].clone();
            expected["changes"] = comparable(&expected["changes"]);
            expected_canon.windows = vec![recorded_window];
            actual_canon.windows = windows.clone();
            expected_canon.value(&mut expected);
            actual_canon.value(&mut actual);
            normalize(index, &mut expected);
            normalize(index, &mut actual);
            if expected != actual {
                failures.push(describe(index, &session["args"], &expected, &actual));
            }
        }
        restore_ids(&roots, &books, &id_map(&expected_canon, &actual_canon));
        // The book now holds the recorded identities: number them as the
        // recording's canon did.
        for (id, n) in &expected_canon.uuids {
            actual_canon.uuids.entry(id.clone()).or_insert(*n);
        }
    }
    if writing {
        fs::write(&path, serde_json::to_string_pretty(&golden).unwrap() + "\n").unwrap();
    }
    if !failures.is_empty() && std::env::var_os("KASSIBER_PORT_KEEP").is_some() {
        let kept = base.keep();
        panic!("{name} (kept {}):\n{}", kept.display(), failures.join("\n"));
    }
    assert!(failures.is_empty(), "{name}:\n{}", failures.join("\n"));
}

fn describe(index: usize, args: &Value, expected: &Value, actual: &Value) -> String {
    let mut out = format!("session {index} {args}:");
    for key in ["exit", "stdout", "stderr"] {
        if expected[key] != actual[key] {
            match (expected[key].as_str(), actual[key].as_str()) {
                (Some(x), Some(y)) => {
                    out.push_str(&format!("\n  {key}: {}", first_difference(x, y)))
                }
                _ => out.push_str(&format!(
                    "\n  {key}: expected {}\n  {key}:   actual {}",
                    clip(&expected[key]),
                    clip(&actual[key])
                )),
            }
        }
    }
    let (empty, none) = (Map::new(), Value::Null);
    let e = expected["changes"].as_object().unwrap_or(&empty);
    let a = actual["changes"].as_object().unwrap_or(&empty);
    let keys: BTreeSet<&String> = e.keys().chain(a.keys()).collect();
    for key in keys {
        let (x, y) = (e.get(key).unwrap_or(&none), a.get(key).unwrap_or(&none));
        if x == y {
            continue;
        }
        match (x.get("book"), y.get("book")) {
            (Some(xb), Some(yb)) => {
                for part in ["schema", "pragmas"] {
                    if xb.get(part) != yb.get(part) {
                        out.push_str(&format!("\n  {key}: {part} differs"));
                    }
                }
                let (xt, yt) = (
                    xb.get("tables")
                        .and_then(Value::as_object)
                        .unwrap_or(&empty),
                    yb.get("tables")
                        .and_then(Value::as_object)
                        .unwrap_or(&empty),
                );
                let tables: BTreeSet<&String> = xt.keys().chain(yt.keys()).collect();
                for table in tables {
                    if xt.get(table) != yt.get(table) {
                        let (x, y) = (
                            xt.get(table).unwrap_or(&none).to_string(),
                            yt.get(table).unwrap_or(&none).to_string(),
                        );
                        out.push_str(&format!("\n  {key} {table}: {}", first_difference(&x, &y)));
                    }
                }
            }
            _ => out.push_str(&format!(
                "\n  {key}: expected {}\n  {key}:   actual {}",
                clip(x),
                clip(y)
            )),
        }
    }
    out
}

fn first_difference(x: &str, y: &str) -> String {
    let at = x
        .char_indices()
        .zip(y.chars())
        .find(|((_, a), b)| a != b)
        .map(|((i, _), _)| i)
        .unwrap_or(x.len().min(y.len()));
    let window = |s: &str| {
        let mut start = at.saturating_sub(300);
        while !s.is_char_boundary(start) {
            start -= 1;
        }
        let mut end = (at + 300).min(s.len());
        while !s.is_char_boundary(end) {
            end += 1;
        }
        s.get(start..end).unwrap_or("").to_owned()
    };
    format!(
        "differ at byte {at} (lengths {} and {})\n    expected …{}…\n    actual   …{}…",
        x.len(),
        y.len(),
        window(x),
        window(y)
    )
}

fn clip(value: &Value) -> String {
    let text = value.to_string();
    if text.len() > 1500 {
        let mut end = 1500;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &text[..end])
    } else {
        text
    }
}

/// A normalizer that masks the clock-derived numbers under `keys`
/// (`"age_seconds": 27108034`) in stdout and changed text, as the pytest
/// tests settled them.
pub fn clock_numbers(keys: &'static [&'static str]) -> impl Fn(usize, &mut Value) {
    move |_, outcome| {
        fn walk(value: &mut Value, keys: &[&str]) {
            match value {
                Value::String(text) => {
                    for key in keys {
                        *text = mask_number(text, key);
                    }
                }
                Value::Array(items) => items.iter_mut().for_each(|v| walk(v, keys)),
                Value::Object(map) => map.values_mut().for_each(|v| walk(v, keys)),
                _ => {}
            }
        }
        walk(outcome, keys);
    }
}

fn mask_number(text: &str, key: &str) -> String {
    let needle = format!("\"{key}\":");
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(&needle) {
        let after = at + needle.len();
        out.push_str(&rest[..after]);
        rest = &rest[after..];
        let spaces = rest.len() - rest.trim_start_matches(' ').len();
        out.push_str(&rest[..spaces]);
        rest = &rest[spaces..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            out.push_str("\"<clock>\"");
            rest = &rest[digits..];
        }
    }
    out.push_str(rest);
    out
}

/// A normalizer for Core Lightning syncs: a bookkeeper balance snapshot is
/// keyed by the day the sync ran (the product's wall clock, with no test
/// hook), so its day and the hashes derived from it are masked.
pub fn balance_days(_: usize, outcome: &mut Value) {
    let Some(changes) = outcome.get_mut("changes").and_then(Value::as_object_mut) else {
        return;
    };
    for change in changes.values_mut() {
        let Some(rows) = change
            .pointer_mut("/book/tables/lightning_node_records/upsert")
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        for row in rows {
            let Some(cells) = row.as_array_mut() else {
                continue;
            };
            if !cells.iter().any(|c| c == "balance_snapshot") {
                continue;
            }
            for cell in cells {
                let Some(text) = cell.as_str() else { continue };
                let bytes = text.as_bytes();
                if text.len() == 64 && hex_run(bytes, 0, 64) {
                    *cell = json!("<day-hash>");
                } else if text.len() == 20 && text.ends_with("T00:00:00Z") {
                    *cell = json!("<day>");
                }
            }
        }
    }
}
