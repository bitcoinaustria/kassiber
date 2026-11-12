//! Compare observable SQLite structure, independent of ALTER column order and
//! insignificant SQL whitespace. Literal contents and constraints remain exact.
use crate::{map_sqlite_error, Error, Result};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const RETIRED_SYNC: &[&str] = &[
    "sync_event_id",
    "sync_replica_id",
    "sync_replica_seq",
    "sync_hlc",
    "sync_author_member_id",
    "sync_signature",
    "sync_context_json",
];
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Tokenize without folding quoted string contents or removing whitespace
/// inside literals. Quotes around identifiers do not change their structure.
fn tokens(sql: &str) -> Vec<String> {
    let mut chars = sql.chars().peekable();
    let mut out = Vec::new();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if c == '-' && chars.peek() == Some(&'-') {
            chars.next();
            for ch in chars.by_ref() {
                if ch == '\n' {
                    break;
                }
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev = '\0';
            for ch in chars.by_ref() {
                if prev == '*' && ch == '/' {
                    break;
                }
                prev = ch;
            }
            continue;
        }
        if ['\'', '"', '`', '['].contains(&c) {
            let close = if c == '[' { ']' } else { c };
            let mut value = String::new();
            while let Some(ch) = chars.next() {
                if ch == close {
                    if chars.peek() == Some(&close) && c != '[' {
                        chars.next();
                        value.push(close);
                    } else {
                        break;
                    }
                } else {
                    value.push(ch);
                }
            }
            if c == '\'' {
                out.push(format!("'{}'", value.replace('\'', "''")));
            } else {
                out.push(value);
            }
        } else if c.is_alphanumeric() || c == '_' || c == '$' {
            let mut value = c.to_lowercase().to_string();
            while chars
                .peek()
                .is_some_and(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '$')
            {
                value.extend(chars.next().expect("peeked char").to_lowercase());
            }
            out.push(value);
        } else {
            out.push(c.to_string());
        }
    }
    if out.last().is_some_and(|s| s == ";") {
        out.pop();
    }
    out
}
fn normalized_table(sql: &str, table: &str) -> Value {
    let words = tokens(sql);
    let Some(start) = words.iter().position(|v| v == "(") else {
        return json!(words);
    };
    let mut clauses = Vec::new();
    let mut clause = Vec::new();
    let mut depth = 0;
    for word in words.iter().skip(start + 1) {
        if word == ")" && depth == 0 {
            if !clause.is_empty() {
                clauses.push(clause);
            }
            break;
        }
        if word == "," && depth == 0 {
            clauses.push(std::mem::take(&mut clause));
            continue;
        }
        if word == "(" {
            depth += 1;
        } else if word == ")" {
            depth -= 1;
        }
        clause.push(word.clone());
    }
    if table == "transaction_edit_events" {
        clauses.retain(|clause| {
            !(clause.len() == 2
                && clause
                    .first()
                    .is_some_and(|v| RETIRED_SYNC.contains(&v.as_str()))
                && clause[1]
                    == if clause[0] == "sync_replica_seq" {
                        "integer"
                    } else {
                        "text"
                    })
        });
    }
    if table == "custody_components" {
        // Python intentionally ALTER-adds these two nullable commitment counts
        // without the CHECK present in new schemas. Recognize only that known
        // history; preserve every other table/column constraint.
        for clause in &mut clauses {
            if let Some(name) = clause.first().cloned().filter(|n| {
                ["expected_economic_term_count", "expected_evidence_count"].contains(&n.as_str())
            }) {
                if *clause == tokens(&format!("{name} INTEGER CHECK ({name} >= 0)")) {
                    *clause = tokens(&format!("{name} INTEGER"));
                }
            }
        }
    }
    clauses.sort();
    let suffix = words
        .iter()
        .rposition(|v| v == ")")
        .map(|i| words[i + 1..].to_vec())
        .unwrap_or_default();
    json!([clauses, suffix])
}
fn rows(conn: &Connection, sql: &str, indexes: &[usize]) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(sql).map_err(map_sqlite_error)?;
    let mut query = stmt.query([]).map_err(map_sqlite_error)?;
    let mut out = Vec::new();
    while let Some(row) = query.next().map_err(map_sqlite_error)? {
        let mut values = Vec::new();
        for index in indexes {
            let value = match row.get_ref(*index).map_err(map_sqlite_error)? {
                rusqlite::types::ValueRef::Null => Value::Null,
                rusqlite::types::ValueRef::Integer(v) => json!(v),
                rusqlite::types::ValueRef::Text(v) => {
                    json!(std::str::from_utf8(v).map_err(|_| Error::migration("schema_encoding"))?)
                }
                _ => return Err(Error::migration("schema_value")),
            };
            values.push(value);
        }
        out.push(Value::Array(values));
    }
    Ok(out)
}
/// The `gl_*` triggers Kassiber shipped with the removed general ledger, as
/// `(name, table, normalized SQL)`, generated from every ledger revision by
/// `crates/store/scripts/gen_legacy_ledger_triggers.py`.
fn shipped_ledger_triggers() -> &'static std::collections::HashSet<(String, String, Vec<String>)> {
    static SHIPPED: std::sync::OnceLock<std::collections::HashSet<(String, String, Vec<String>)>> =
        std::sync::OnceLock::new();
    SHIPPED.get_or_init(|| {
        let fixture: Value =
            serde_json::from_str(include_str!("../fixtures/legacy-ledger-triggers.json"))
                .expect("checked-in legacy ledger triggers");
        fixture["triggers"]
            .as_array()
            .expect("trigger list")
            .iter()
            .map(|trigger| {
                let text = |key: &str| trigger[key].as_str().expect("trigger field").to_owned();
                (text("name"), text("table"), tokens(&text("sql")))
            })
            .collect()
    })
}

/// Archived general-ledger storage an earlier version left (Python's
/// `legacy_ledger`, `name LIKE 'gl\_%'`): tables that still hold rows stay,
/// with their indexes and retention triggers, until
/// `maintenance purge-legacy-ledger --confirm`. Tables and their indexes are
/// accepted by name; a trigger only when its name, table and normalized SQL
/// are one Kassiber shipped (all `BEFORE` guards on ledger tables or on
/// `external_documents`). Any other `gl_` trigger is drift.
fn legacy_ledger(kind: &str, name: &str, table: &str, sql: Option<&str>) -> bool {
    let ledger = |text: &str| text.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("gl_"));
    match kind {
        "table" => ledger(name),
        "index" => ledger(table),
        "trigger" => sql.is_some_and(|sql| {
            shipped_ledger_triggers().contains(&(name.to_owned(), table.to_owned(), tokens(sql)))
        }),
        _ => false,
    }
}
fn snapshot(conn: &Connection) -> Result<Value> {
    let mut stmt=conn.prepare("SELECT type,name,tbl_name,sql FROM sqlite_master WHERE name NOT GLOB 'sqlite_*' ORDER BY type,name").map_err(map_sqlite_error)?;
    let objects: Vec<(String, String, String, Option<String>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<_>>()
        .map_err(map_sqlite_error)?;
    let mut out = Vec::new();
    for (kind, name, table, sql) in objects {
        if legacy_ledger(&kind, &name, &table, sql.as_deref()) {
            continue;
        }
        let sql = sql.unwrap_or_default();
        let mut object = json!({"type":kind,"name":name,"table":table,"sql":if kind=="table" {normalized_table(&sql,&name)} else {json!(tokens(&sql))}});
        if kind == "table" {
            let mut columns = rows(
                conn,
                &format!("PRAGMA table_xinfo({})", quote(&name)),
                &[1, 2, 3, 4, 5, 6],
            )?;
            if name == "transaction_edit_events" {
                columns.retain(|c| {
                    !(RETIRED_SYNC.contains(&c[0].as_str().unwrap_or(""))
                        && c[1]
                            == if c[0] == "sync_replica_seq" {
                                "INTEGER"
                            } else {
                                "TEXT"
                            }
                        && c[2] == 0
                        && c[3].is_null()
                        && c[4] == 0
                        && c[5] == 0)
                });
            }
            columns.sort_by_key(Value::to_string);
            object["columns"] = json!(columns);
            let fks = rows(
                conn,
                &format!("PRAGMA foreign_key_list({})", quote(&name)),
                &[0, 1, 2, 3, 4, 5, 6, 7],
            )?;
            let mut grouped = BTreeMap::<i64, Vec<Value>>::new();
            for fk in fks {
                let mut parts = fk.as_array().expect("FK row").clone();
                let id = parts.remove(0).as_i64().expect("FK id");
                grouped.entry(id).or_default().push(json!(parts));
            }
            let mut fks: Vec<_> = grouped.into_values().collect();
            fks.sort_by_key(|v| json!(v).to_string());
            object["foreign_keys"] = json!(fks);
            let mut indexes = rows(
                conn,
                &format!("PRAGMA index_list({})", quote(&name)),
                &[1, 2, 3, 4],
            )?;
            for index in &mut indexes {
                let index_name = index[0].as_str().expect("index name");
                let mut info = rows(
                    conn,
                    &format!("PRAGMA index_xinfo({})", quote(index_name)),
                    &[0, 1, 2, 3, 4, 5],
                )?;
                for field in &mut info {
                    if field[1].as_i64().is_some_and(|v| v >= 0) {
                        field[1] = json!(0);
                    }
                }
                let values = index.as_array_mut().expect("index row");
                values.push(json!(info));
                // Autoindex numbering follows declaration order; its constraints
                // and full ordered key structure carry its meaning.
                if values[2] != "c" {
                    values[0] = Value::Null;
                }
            }
            indexes.sort_by_key(Value::to_string);
            object["indexes"] = json!(indexes);
        }
        out.push(object);
    }
    Ok(json!(out))
}
pub(crate) fn verify(conn: &Connection, manifest: &Value) -> Result<()> {
    let expected = Connection::open_in_memory().map_err(map_sqlite_error)?;
    for kind in ["table", "index", "view", "trigger"] {
        for object in manifest["objects"]
            .as_array()
            .expect("schema objects")
            .iter()
            .filter(|o| o[0] == kind)
        {
            if let Some(sql) = object[3].as_str() {
                expected.execute_batch(sql).map_err(map_sqlite_error)?;
            }
        }
    }
    let actual = snapshot(conn)?;
    let reference = snapshot(&expected)?;
    if actual != reference {
        if ephemeral_gap(&actual, &reference) {
            return Err(Error::new(
                crate::ErrorKind::DatabaseBusy,
                "schema objects may be temporarily absent during a Python open",
            )
            .retryable(true)
            .details(json!({"reason":"schema_install_in_progress"})));
        }
        let mismatch = actual
            .as_array()
            .expect("objects")
            .iter()
            .zip(reference.as_array().expect("objects"))
            .find(|(a, b)| a != b);
        if let Some((a, b)) = mismatch {
            let aspects: Vec<_> = [
                "type",
                "name",
                "table",
                "sql",
                "columns",
                "indexes",
                "foreign_keys",
            ]
            .into_iter()
            .filter(|key| a[key] != b[key])
            .collect();
            return Err(Error::migration("schema_structure").details(
                json!({"reason":"schema_structure","object":b["name"],"aspects":aspects}),
            ));
        }
        return Err(Error::migration("schema_structure"));
    }
    Ok(())
}
fn ephemeral_gap(actual: &Value, reference: &Value) -> bool {
    let actual = actual.as_array().expect("schema objects");
    let reference = reference.as_array().expect("schema objects");
    if actual.iter().any(|a| {
        !reference
            .iter()
            .any(|b| a["type"] == b["type"] && a["name"] == b["name"])
    }) {
        return false;
    }
    let mut missing = false;
    for expected in reference {
        match actual
            .iter()
            .find(|a| a["type"] == expected["type"] && a["name"] == expected["name"])
        {
            Some(found) => {
                let mut found = found.clone();
                let mut expected = expected.clone();
                if found["type"] == "table" {
                    found.as_object_mut().expect("table").remove("indexes");
                    expected.as_object_mut().expect("table").remove("indexes");
                }
                if found != expected {
                    return false;
                }
            }
            None if ["index", "trigger", "view"]
                .contains(&expected["type"].as_str().unwrap_or("")) =>
            {
                missing = true
            }
            None => return false,
        }
    }
    missing
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Ledger archive tables (accepted by name) and an index on one.
    const LEGACY_TABLES: &str = "
        CREATE TABLE gl_entries(id TEXT PRIMARY KEY, profile_id TEXT NOT NULL REFERENCES profiles(id)
            ON DELETE RESTRICT, description TEXT NOT NULL, UNIQUE(profile_id, id));
        CREATE INDEX gl_entries_by_profile ON gl_entries(profile_id);
        CREATE TABLE gl_evidence(id TEXT PRIMARY KEY, profile_id TEXT NOT NULL,
            source_document_id TEXT REFERENCES external_documents(id) ON DELETE RESTRICT);";

    /// The shipped SQL of every `gl_` trigger on `gl_evidence` and
    /// `external_documents`, as the fixture records it.
    fn shipped_evidence_triggers() -> Vec<String> {
        let fixture: Value =
            serde_json::from_str(include_str!("../fixtures/legacy-ledger-triggers.json")).unwrap();
        let triggers: Vec<String> = fixture["triggers"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| {
                ["gl_evidence", "external_documents"].contains(&t["table"].as_str().unwrap())
            })
            .map(|t| t["sql"].as_str().unwrap().to_owned())
            .collect();
        assert!(triggers.len() >= 5, "{triggers:?}");
        triggers
    }

    // A book that keeps populated ledger archives with the guards Kassiber
    // shipped opens; any other `gl_` trigger is drift and fails closed.
    #[test]
    fn populated_legacy_ledger_archives_pass_the_structural_check() {
        let root = tempfile::tempdir().unwrap();
        let book = crate::Book::create(root.path(), None).unwrap();
        let conn = book.connection();
        let manifest: Value =
            serde_json::from_str(crate::schema::MANIFEST).expect("checked-in schema manifest");
        conn.execute_batch(LEGACY_TABLES).unwrap();
        for sql in shipped_evidence_triggers() {
            conn.execute_batch(&sql).unwrap();
        }
        verify(conn, &manifest).unwrap();
        let refused = |sql: &str| {
            conn.execute_batch(sql).unwrap();
            let error = verify(conn, &manifest).unwrap_err();
            assert_eq!(
                error.kind,
                crate::ErrorKind::RequiresPythonMigration,
                "{sql}"
            );
        };
        // An AFTER trigger on external_documents that writes a core table.
        refused(
            "CREATE TRIGGER gl_evidence_document_mirror AFTER INSERT ON external_documents
                BEGIN UPDATE profiles SET label = NEW.label; END;",
        );
        conn.execute_batch("DROP TRIGGER gl_evidence_document_mirror;")
            .unwrap();
        // A shipped name and table with a different body.
        conn.execute_batch("DROP TRIGGER gl_evidence_document_retained;")
            .unwrap();
        refused(
            "CREATE TRIGGER gl_evidence_document_retained BEFORE DELETE ON external_documents
                BEGIN DELETE FROM transactions; END;",
        );
        conn.execute_batch("DROP TRIGGER gl_evidence_document_retained;")
            .unwrap();
        // A shipped body under a name Kassiber never used.
        refused(
            "CREATE TRIGGER gl_evidence_document_guard BEFORE DELETE ON external_documents WHEN EXISTS
                (SELECT 1 FROM gl_evidence WHERE source_document_id=OLD.id)
                BEGIN SELECT RAISE(ABORT,'accounting_evidence_retained'); END;",
        );
        conn.execute_batch("DROP TRIGGER gl_evidence_document_guard;")
            .unwrap();
        // A `gl_` trigger on a core table.
        refused("CREATE TRIGGER gl_core_guard BEFORE DELETE ON transactions BEGIN SELECT 1; END;");
        conn.execute_batch("DROP TRIGGER gl_core_guard;").unwrap();
        // Tables outside the prefix are drift too.
        refused("CREATE TABLE ledger_like(id TEXT);");
    }

    #[test]
    fn normalization_preserves_literals_and_table_constraints() {
        assert_eq!(
            tokens(" SELECT 'a b', \"field\" -- comment\n FROM t;"),
            tokens("select 'a b',field from t")
        );
        assert_ne!(tokens("SELECT 'a b'"), tokens("SELECT 'ab'"));
        assert_eq!(
            normalized_table("CREATE TABLE t(a TEXT, b INT CHECK(b>0))", "t"),
            normalized_table("CREATE TABLE \"t\" (b INT CHECK(b>0), a TEXT)", "t")
        );
        assert_ne!(
            normalized_table("CREATE TABLE t(a INT CHECK(a>0))", "t"),
            normalized_table("CREATE TABLE t(a INT CHECK(a>=0))", "t")
        );
    }
}
