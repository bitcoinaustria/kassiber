//! Withholds a book's own credentials from error text it stored before
//! servers' echoes were scrubbed: `_migrate_stored_error_credentials` in
//! `kassiber/db.py`, with the same rows, bytes and audit.
use crate::{map_sqlite_error, Error, ErrorKind, Result};
use kassiber_credentials::{
    backend_credentials, credential_forms, url_credentials, withhold, withhold_py, Forms,
};
use kassiber_pyjson::{dumps, Key, Options, Value as PyValue};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, TransactionBehavior};
use std::collections::{BTreeMap, BTreeSet};

pub const MIGRATION: &str = "stored-error-credentials-v1";
/// `STORED_ERROR_COLUMNS`: (table, row key, column); `*_json` holds JSON.
const COLUMNS: [(&str, &str, &str); 5] = [
    ("freshness_jobs", "id", "error_json"),
    ("freshness_source_states", "rowid", "last_error_message"),
    ("btcpay_store_sync_states", "rowid", "last_error_message"),
    ("lightning_node_syncs", "id", "error_json"),
    ("ai_provider_checks", "rowid", "error_json"),
];

/// Python's `text(key)` in `backend_credentials` for a SQLite value.
fn sql_text(value: &SqlValue) -> String {
    match value {
        SqlValue::Text(text) => kassiber_pytext::strip(text).to_owned(),
        SqlValue::Integer(number) => number.to_string(),
        _ => String::new(),
    }
}

/// The same for a value decoded from `config_json`.
fn json_text(value: &PyValue) -> String {
    match value {
        PyValue::Str(text) => text
            .as_str()
            .map(|text| kassiber_pytext::strip(text).to_owned())
            .unwrap_or_default(),
        PyValue::Int(number) => number.to_string(),
        _ => String::new(),
    }
}

fn tables(conn: &Connection) -> Result<BTreeSet<String>> {
    let mut statement = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .map_err(map_sqlite_error)?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<_>>()
        .map_err(map_sqlite_error)?;
    Ok(names)
}

/// `_stored_error_credentials`: every backend's and AI provider's credentials.
fn credentials(conn: &Connection, tables: &BTreeSet<String>) -> Result<Vec<String>> {
    let mut found = Vec::new();
    if tables.contains("backends") {
        let mut statement = conn
            .prepare("SELECT url, auth_header, token, tor_proxy, config_json FROM backends ORDER BY name")
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map([], |row| {
                (0..5)
                    .map(|i| row.get::<_, SqlValue>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(map_sqlite_error)?;
        for row in rows {
            let row = row.map_err(map_sqlite_error)?;
            let mut backend: BTreeMap<String, String> = BTreeMap::new();
            let config = match &row[4] {
                SqlValue::Text(text) if !text.is_empty() => kassiber_pytext::loads(text).ok(),
                _ => kassiber_pytext::loads("{}").ok(),
            };
            if let Some(PyValue::Dict(pairs)) = config {
                for (key, value) in pairs {
                    if let Key::Str(key) = key {
                        if let Some(key) = key.as_str() {
                            backend.insert(key.to_owned(), json_text(&value));
                        }
                    }
                }
            }
            for (index, key) in ["url", "auth_header", "token", "tor_proxy"]
                .iter()
                .enumerate()
            {
                backend.insert((*key).to_owned(), sql_text(&row[index]));
            }
            found.extend(backend_credentials(|key| {
                backend.get(key).cloned().unwrap_or_default()
            }));
        }
    }
    if tables.contains("ai_providers") {
        let mut statement = conn
            .prepare("SELECT api_key, base_url FROM ai_providers ORDER BY name")
            .map_err(map_sqlite_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, SqlValue>(0)?, row.get::<_, SqlValue>(1)?))
            })
            .map_err(map_sqlite_error)?;
        for row in rows {
            let (api_key, base_url) = row.map_err(map_sqlite_error)?;
            if let SqlValue::Text(key) = api_key {
                if !key.is_empty() {
                    found.push(key);
                }
            }
            if let SqlValue::Text(url) = base_url {
                found.extend(url_credentials(&url, None));
            }
        }
    }
    Ok(found)
}

/// `_withhold_stored_json`: strings and string keys; a key that becomes
/// equal to an earlier one keeps that key's place and takes the later value.
fn withhold_json(value: PyValue, forms: &Forms) -> PyValue {
    match value {
        PyValue::Dict(pairs) => {
            let mut out: Vec<(Key, PyValue)> = Vec::with_capacity(pairs.len());
            for (key, item) in pairs {
                let key = match key {
                    Key::Str(text) => Key::Str(withhold_py(&text, forms)),
                    other => other,
                };
                let item = withhold_json(item, forms);
                match out.iter_mut().find(|(existing, _)| *existing == key) {
                    Some(slot) => slot.1 = item,
                    None => out.push((key, item)),
                }
            }
            PyValue::Dict(out)
        }
        PyValue::List(items) => PyValue::List(
            items
                .into_iter()
                .map(|item| withhold_json(item, forms))
                .collect(),
        ),
        PyValue::Str(text) => PyValue::Str(withhold_py(&text, forms)),
        other => other,
    }
}

pub(crate) fn migrate(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_sqlite_error)?;
    if tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migration_audits WHERE migration_name=?)",
            [MIGRATION],
            |r| r.get::<_, bool>(0),
        )
        .map_err(map_sqlite_error)?
    {
        return Ok(());
    }
    let tables = tables(&tx)?;
    let forms = credential_forms(credentials(&tx, &tables)?);
    let mut scrubbed = 0usize;
    for (table, key, column) in COLUMNS {
        if forms.is_empty() || !tables.contains(table) {
            continue;
        }
        let rows: Vec<(SqlValue, SqlValue)> = {
            let mut statement = tx
                .prepare(&format!(
                    "SELECT {key}, \"{column}\" FROM \"{table}\" WHERE \"{column}\" IS NOT NULL ORDER BY {key}"
                ))
                .map_err(map_sqlite_error)?;
            let rows = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(map_sqlite_error)?
                .collect::<rusqlite::Result<_>>()
                .map_err(map_sqlite_error)?;
            rows
        };
        for (row_key, text) in rows {
            let SqlValue::Text(text) = text else { continue };
            let mut updated = withhold(&text, &forms);
            if updated == text {
                continue;
            }
            if column.ends_with("_json") {
                // Decoded strings are scrubbed so a replacement cannot break
                // the JSON; text that does not decode stays scrubbed as text.
                if let Ok(value) = kassiber_pytext::loads(&text) {
                    if let Ok(encoded) = dumps(
                        &withhold_json(value, &forms),
                        &Options::new().sort_keys(true).separators(",", ":"),
                    ) {
                        updated = encoded;
                    }
                }
            }
            tx.execute(
                &format!("UPDATE \"{table}\" SET \"{column}\" = ?1 WHERE {key} = ?2"),
                rusqlite::params![updated, row_key],
            )
            .map_err(map_sqlite_error)?;
            scrubbed += 1;
        }
    }
    let impact = crate::layout::python_value(&serde_json::json!({
        "schema_version": 1, "migration": MIGRATION, "scrubbed_value_count": scrubbed,
    }));
    let impact = dumps(&impact, &Options::default().sort_keys(true))
        .map_err(|_| Error::new(ErrorKind::Validation, "invalid migration audit JSON"))?;
    observer(&format!("{MIGRATION}.before_audit"))?;
    tx.execute(
        "INSERT OR IGNORE INTO schema_migration_audits(id,migration_name,schema_version,impact_json,created_at) VALUES(?1,?1,1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        [MIGRATION, &impact],
    )
    .map_err(map_sqlite_error)?;
    observer(&format!("{MIGRATION}.before_commit"))?;
    tx.commit().map_err(map_sqlite_error)?;
    observer(&format!("{MIGRATION}.after_commit"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "0123456789abcdef0123456789abcdef01234567";

    fn book() -> (tempfile::TempDir, crate::Book) {
        let root = tempfile::tempdir().unwrap();
        let book = crate::Book::create(root.path(), None).unwrap();
        book.connection
            .execute_batch(&format!(
                "DELETE FROM schema_migration_audits WHERE migration_name='{MIGRATION}';
                INSERT INTO workspaces VALUES('ws','W','fixed');
                INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','P','fixed');
                INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('w','ws','p','W','coreln','fixed');"
            ))
            .unwrap();
        (root, book)
    }

    fn backends(conn: &Connection) {
        conn.execute_batch(&format!(
            "INSERT INTO backends(name,kind,url,token,tor_proxy,config_json,created_at,updated_at)
               VALUES('shop','btcpay','https://pay.example','{KEY}','socks5h://u:proxy%2Fpw@127.0.0.1:9050','{{}}','f','f');
             INSERT INTO backends(name,kind,url,config_json,created_at,updated_at)
               VALUES('core','bitcoinrpc','http://127.0.0.1:8332','{{\"rpcuser\":\"rpc\",\"rpcpassword\":4242}}','f','f');
             INSERT INTO ai_providers(name,kind,base_url,api_key,created_at,updated_at)
               VALUES('ai','remote','https://ai.example/v1','ak-provider-key','f','f');"
        ))
        .unwrap();
    }

    fn audit(conn: &Connection) -> Option<String> {
        conn.query_row(
            "SELECT impact_json FROM schema_migration_audits WHERE migration_name=?",
            [MIGRATION],
            |r| r.get(0),
        )
        .ok()
    }

    fn text(conn: &Connection, sql: &str) -> String {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn every_error_column_loses_the_books_credentials_once() {
        let (_root, mut book) = book();
        backends(&book.connection);
        let upper = KEY.to_uppercase();
        book.connection
            .execute_batch(&format!(
                "INSERT INTO freshness_jobs(id,profile_id,job_type,source_key,source_type,source_label,status,error_json,created_at,updated_at)
                   VALUES('j','p','btcpay_wallet_source','k','wallet','L','error','{{\"message\":\"bad {upper} proxy/pw\"}}','f','f');
                 INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,last_error_message,updated_at)
                   VALUES('p','k','wallet','L','HTTP 401 cnBjOjQyNDI= rpc:4242 4242','f');
                 INSERT INTO btcpay_store_sync_states(profile_id,backend_name,store_id,last_error_message,updated_at)
                   VALUES('p','shop','S1','BTCPay returned HTTP 500: {KEY}','f');
                 INSERT INTO lightning_node_syncs(id,workspace_id,profile_id,wallet_id,backend_name,started_at,status,error_json,created_at)
                   VALUES('l','ws','p','w','node','f','partial','{{\"getinfo\":\"no ak-provider-key\"}}','f');
                 INSERT INTO ai_provider_checks(provider_name,check_kind,base_url,checked_at,stale,error_json,value_json)
                   VALUES('ai','models','https://ai.example/v1','f',0,'not json ak-provider-key','[]');"
            ))
            .unwrap();
        migrate(&mut book.connection, &mut |_| Ok(())).unwrap();
        let conn = &book.connection;
        assert_eq!(
            text(conn, "SELECT error_json FROM freshness_jobs"),
            r#"{"message":"bad <redacted> <redacted>"}"#
        );
        assert_eq!(
            text(
                conn,
                "SELECT last_error_message FROM freshness_source_states"
            ),
            "HTTP 401 <redacted> <redacted> <redacted>"
        );
        assert_eq!(
            text(
                conn,
                "SELECT last_error_message FROM btcpay_store_sync_states"
            ),
            "BTCPay returned HTTP 500: <redacted>"
        );
        assert_eq!(
            text(conn, "SELECT error_json FROM lightning_node_syncs"),
            r#"{"getinfo":"no <redacted>"}"#
        );
        assert_eq!(
            text(conn, "SELECT error_json FROM ai_provider_checks"),
            "not json <redacted>"
        );
        assert_eq!(
            audit(conn).unwrap(),
            r#"{"migration": "stored-error-credentials-v1", "schema_version": 1, "scrubbed_value_count": 5}"#
        );
        // The backend rows keep the credentials themselves.
        assert_eq!(
            text(conn, "SELECT token FROM backends WHERE name='shop'"),
            KEY
        );
        // Once only: text written later is left to the source-side scrub.
        book.connection
            .execute(
                "UPDATE btcpay_store_sync_states SET last_error_message=?",
                [KEY],
            )
            .unwrap();
        migrate(&mut book.connection, &mut |_| Ok(())).unwrap();
        assert_eq!(
            text(
                &book.connection,
                "SELECT last_error_message FROM btcpay_store_sync_states"
            ),
            KEY
        );
    }

    #[test]
    fn json_is_scrubbed_decoded_and_rewritten_compactly() {
        let (_root, mut book) = book();
        backends(&book.connection);
        let stored = format!(
            r#"{{"z": [1.5, "{KEY}", null], "a-{KEY}": 1, "a-ak-provider-key": 2, "s": "\ud800"}}"#
        );
        book.connection
            .execute(
                "INSERT INTO freshness_jobs(id,profile_id,job_type,source_key,source_type,source_label,status,error_json,created_at,updated_at)
                   VALUES('j','p','btcpay_wallet_source','k','wallet','L','error',?,'f','f')",
                [&stored],
            )
            .unwrap();
        migrate(&mut book.connection, &mut |_| Ok(())).unwrap();
        // Two keys that become equal keep the first one's place and the
        // later value, as a Python dict comprehension does.
        assert_eq!(
            text(&book.connection, "SELECT error_json FROM freshness_jobs"),
            r#"{"a-<redacted>":2,"s":"\ud800","z":[1.5,"<redacted>",null]}"#
        );
    }

    #[test]
    fn a_book_without_credentials_only_records_the_audit() {
        let (_root, mut book) = book();
        book.connection
            .execute(
                "INSERT INTO btcpay_store_sync_states(profile_id,backend_name,store_id,last_error_message,updated_at) VALUES('p','shop','S1',?,'f')",
                [KEY],
            )
            .unwrap();
        migrate(&mut book.connection, &mut |_| Ok(())).unwrap();
        assert_eq!(
            text(
                &book.connection,
                "SELECT last_error_message FROM btcpay_store_sync_states"
            ),
            KEY
        );
        assert_eq!(
            audit(&book.connection).unwrap(),
            r#"{"migration": "stored-error-credentials-v1", "schema_version": 1, "scrubbed_value_count": 0}"#
        );
    }

    #[test]
    fn a_failure_before_commit_leaves_no_change_and_no_audit() {
        let (_root, mut book) = book();
        backends(&book.connection);
        book.connection
            .execute(
                "INSERT INTO btcpay_store_sync_states(profile_id,backend_name,store_id,last_error_message,updated_at) VALUES('p','shop','S1',?,'f')",
                [KEY],
            )
            .unwrap();
        let failed = migrate(&mut book.connection, &mut |label| {
            if label.ends_with("before_commit") {
                Err(Error::new(ErrorKind::Validation, "injected"))
            } else {
                Ok(())
            }
        });
        assert!(failed.is_err());
        assert_eq!(
            text(
                &book.connection,
                "SELECT last_error_message FROM btcpay_store_sync_states"
            ),
            KEY
        );
        assert!(audit(&book.connection).is_none());
    }
}
