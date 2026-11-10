//! A database as comparable text: pragmas, schema objects, and every table's
//! rows, each value with its storage class.

use std::path::Path;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};

/// Whether `bytes` start like a plaintext SQLite database.
pub fn is_sqlite(bytes: &[u8]) -> bool {
    bytes.starts_with(b"SQLite format 3\0")
}

/// One line per pragma, schema object, and row; rows of each table in
/// rowid order (or primary-key order for a `WITHOUT ROWID` table). Returns
/// the per-table blocks so a caller can reorder rows it knows are unordered.
pub fn dump(path: &Path) -> Result<Vec<(String, Vec<String>)>, String> {
    let uri = format!("file:{}?immutable=1", path.display());
    let conn = Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    let mut blocks = Vec::new();
    let mut header = Vec::new();
    for pragma in [
        "user_version",
        "application_id",
        "page_size",
        "auto_vacuum",
        "encoding",
    ] {
        let value: String = conn
            .query_row(&format!("PRAGMA {pragma}"), [], |row| {
                Ok(match row.get_ref(0)? {
                    ValueRef::Integer(i) => i.to_string(),
                    ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
                    other => format!("{other:?}"),
                })
            })
            .map_err(|e| e.to_string())?;
        header.push(format!("pragma {pragma}={value}"));
    }
    let mut objects = conn
        .prepare(
            "SELECT type, name, tbl_name, COALESCE(sql, '') FROM sqlite_master ORDER BY type, name",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, String, String, String)> = objects
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    for (kind, name, table, sql) in &rows {
        header.push(format!("schema {kind} {name} on {table}: {sql}"));
    }
    blocks.push(("schema".to_owned(), header));
    for (kind, name, _, sql) in &rows {
        if kind != "table" || name.starts_with("sqlite_") {
            continue;
        }
        let without_rowid = sql.to_ascii_uppercase().contains("WITHOUT ROWID");
        let order = if without_rowid { "" } else { " ORDER BY rowid" };
        let quoted = name.replace('"', "\"\"");
        let mut statement = conn
            .prepare(&format!("SELECT * FROM \"{quoted}\"{order}"))
            .map_err(|e| e.to_string())?;
        let columns = statement.column_count();
        let names: Vec<String> = statement
            .column_names()
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut lines = Vec::new();
        let mut query = statement.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = query.next().map_err(|e| e.to_string())? {
            let mut values = Vec::with_capacity(columns);
            for (index, column) in names.iter().enumerate() {
                let value = match row.get_ref(index).map_err(|e| e.to_string())? {
                    ValueRef::Null => "null".to_owned(),
                    ValueRef::Integer(i) => format!("int {i}"),
                    ValueRef::Real(f) => format!("real {f:?}"),
                    ValueRef::Text(t) => format!(
                        "text {}",
                        serde_json::to_string(&String::from_utf8_lossy(t)).unwrap()
                    ),
                    ValueRef::Blob(b) => format!("blob {}", crate::hex(b)),
                };
                values.push(format!("{column}={value}"));
            }
            lines.push(values.join(" | "));
        }
        if without_rowid {
            lines.sort();
        }
        blocks.push((format!("table {name}"), lines));
    }
    Ok(blocks)
}
