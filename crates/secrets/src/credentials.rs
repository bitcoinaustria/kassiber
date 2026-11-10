//! Move dotenv credentials only after each target row has durably committed.
//! The original snapshot remains plaintext, exactly as Python's recovery copy.
use crate::service::storage_error;
use kassiber_envelope::{dict, s, set, AppError};
use kassiber_pyjson::{dumps, Key, Options, Value};
use kassiber_pytext::{loads, lower, strip};
use kassiber_store::rusqlite::{params, Connection, OptionalExtension};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::Path,
};
use zeroize::Zeroizing;

type Result<T> = std::result::Result<T, AppError>;
fn sql(error: kassiber_store::rusqlite::Error) -> AppError {
    storage_error(kassiber_store::map_sqlite_error(error))
}
fn now(conn: &Connection, stamp: bool) -> Result<String> {
    #[cfg(feature = "test-hooks")]
    if let Ok(value) = std::env::var("KASSIBER_RUST_TEST_SECRETS_TIME") {
        return Ok(if stamp {
            value.replace(['-', ':'], "")
        } else {
            value
        });
    }
    conn.query_row(
        if stamp {
            "SELECT strftime('%Y%m%dT%H%M%SZ','now')"
        } else {
            "SELECT strftime('%Y-%m-%dT%H:%M:%SZ','now')"
        },
        [],
        |row| row.get(0),
    )
    .map_err(sql)
}
fn io(error: std::io::Error) -> AppError {
    storage_error(kassiber_paths::Error::io(error))
}

struct Finding {
    backend: String,
    field: String,
    env_key: String,
    lineno: usize,
    value: Zeroizing<String>,
}
impl Finding {
    fn public(&self) -> Value {
        dict([
            ("backend", s(self.backend.as_str())),
            ("field", s(self.field.as_str())),
            ("env_key", s(self.env_key.as_str())),
            ("lineno", Value::from(self.lineno)),
        ])
    }
}
fn splitlines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut iter = text.char_indices().peekable();
    while let Some((index, ch)) = iter.next() {
        if matches!(
            ch,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        ) {
            out.push(&text[start..index]);
            start = index + ch.len_utf8();
            if ch == '\r' && iter.peek().is_some_and(|(_, ch)| *ch == '\n') {
                let (i, _) = iter.next().unwrap();
                start = i + 1;
            }
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}
fn findings(text: &str) -> Vec<Finding> {
    splitlines(text)
        .into_iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = strip(line);
            if line.starts_with('#') {
                return None;
            }
            let (key, raw) = line.split_once('=')?;
            let key = strip(key);
            let suffix = key
                .strip_prefix("KASSIBER_BACKEND_")
                .or_else(|| key.strip_prefix("SATBOOKS_BACKEND_"))?;
            let (backend, field) = suffix.split_once('_')?;
            let backend = lower(strip(backend));
            if backend.is_empty() {
                return None;
            }
            let field = lower(strip(field)).replace('-', "_");
            let field = match field.as_str() {
                "rpcpassword" | "rpc_password" => "password",
                "rpcuser" | "rpc_user" => "username",
                v => v,
            };
            if !["auth_header", "password", "token", "username"].contains(&field) {
                return None;
            }
            let mut value = strip(raw);
            if value.len() >= 2
                && (value.starts_with('\'') && value.ends_with('\'')
                    || value.starts_with('"') && value.ends_with('"'))
            {
                value = &value[1..value.len() - 1];
            }
            Some(Finding {
                backend,
                field: field.into(),
                env_key: key.into(),
                lineno: index + 1,
                value: Zeroizing::new(value.into()),
            })
        })
        .collect()
}
fn read(path: &Path) -> Result<Zeroizing<String>> {
    match fs::read_to_string(path) {
        // Python's text mode performs universal newline translation before
        // splitlines and before deciding whether to retain a final LF.
        Ok(text) => Ok(Zeroizing::new(
            text.replace("\r\n", "\n").replace('\r', "\n"),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(Zeroizing::new(String::new()))
        }
        Err(error) => Err(io(error)),
    }
}
pub fn scan(path: &Path) -> Result<Value> {
    Ok(Value::List(
        findings(&read(path)?).iter().map(Finding::public).collect(),
    ))
}

pub fn migrate(conn: &Connection, path: &Path) -> Result<Value> {
    migrate_observed(conn, path, &mut |_| Ok(()))
}
/// Observer injection is explicit; production never reads fault settings.
pub fn migrate_observed(
    conn: &Connection,
    path: &Path,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<Value> {
    let text = read(path)?;
    let entries = findings(&text);
    let mut groups: BTreeMap<(&str, &str), Vec<&Finding>> = BTreeMap::new();
    for entry in &entries {
        groups
            .entry((&entry.backend, &entry.field))
            .or_default()
            .push(entry);
    }
    let duplicates = groups
        .into_iter()
        .filter(|(_, members)| members.len() > 1)
        .map(|((backend, field), members)| {
            dict([
                ("backend", s(backend)),
                ("field", s(field)),
                (
                    "entries",
                    Value::List(
                        members
                            .into_iter()
                            .map(|v| {
                                dict([
                                    ("env_key", s(v.env_key.as_str())),
                                    ("lineno", Value::from(v.lineno)),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ])
        })
        .collect::<Vec<_>>();
    if !duplicates.is_empty() {
        return Err(AppError::new("validation", "Duplicate backend credential dotenv entries must be resolved before migration")
            .with_hint("Keep exactly one secret env entry per backend credential field, then rerun `kassiber secrets migrate-credentials`.")
            .with_details(dict([("duplicates", Value::List(duplicates))])));
    }
    let mut migrated = Vec::new();
    let mut skipped = Vec::new();
    let mut drop_lines = BTreeSet::new();
    for finding in &entries {
        let config: Option<Option<String>> = conn
            .query_row(
                "SELECT config_json FROM backends WHERE name=?",
                [&finding.backend],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql)?;
        let mut public = finding.public();
        let Some(config) = config else {
            set(&mut public, "reason", s("backend_not_in_db"));
            skipped.push(public);
            continue;
        };
        let mut config = config
            .as_deref()
            .and_then(|v| loads(v).ok())
            .filter(|v| matches!(v, Value::Dict(_)))
            .unwrap_or_else(|| dict::<&str>([]));
        if ["username", "password"].contains(&finding.field.as_str()) {
            let value = strip(&finding.value);
            if value.is_empty() {
                if let Value::Dict(pairs) = &mut config {
                    pairs.retain(|(k, _)| *k != Key::from(finding.field.as_str()));
                }
            } else {
                set(&mut config, &finding.field, s(value));
            }
        }
        let config = Zeroizing::new(
            dumps(
                &config,
                &Options {
                    sort_keys: true,
                    ..Default::default()
                },
            )
            .map_err(|_| AppError::new("internal_error", "could not encode backend config"))?,
        );
        // Each Python update_db_backend commits independently. Preserve its
        // resumability: a repeated migration applies the same exact value.
        let tx = kassiber_store::write_transaction(conn).map_err(sql)?;
        let column = match finding.field.as_str() {
            "auth_header" => "auth_header=?",
            "token" => "token=?",
            _ => "config_json=?",
        };
        let value = if column == "config_json=?" {
            config.as_str()
        } else {
            finding.value.as_str()
        };
        tx.execute(
            &format!("UPDATE backends SET {column}, config_json=?2, updated_at=?3 WHERE name=?4"),
            params![value, config.as_str(), now(conn, false)?, &finding.backend],
        )
        .map_err(sql)?;
        tx.commit().map_err(sql)?;
        observer("credentials.after_row_commit")?;
        set(&mut public, "applied", Value::Bool(true));
        migrated.push(public);
        drop_lines.insert(finding.lineno);
    }
    let mut backup_path = Value::Null;
    if !drop_lines.is_empty() {
        let stamp = now(conn, true)?;
        let backup = path.with_file_name(format!(
            "{}.pre-credentials-migration-{stamp}.bak",
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
        // Stage both files privately. Publication cannot leave a partially
        // truncated dotenv; interruption leaves either original or sanitized.
        let parent = path.parent().unwrap_or(Path::new("."));
        let original = Zeroizing::new(fs::read(path).map_err(io)?);
        let captured = std::str::from_utf8(&original).map_err(|_| {
            AppError::new("validation", "dotenv changed during credential migration")
        })?;
        if Zeroizing::new(captured.replace("\r\n", "\n").replace('\r', "\n")) != text {
            return Err(AppError::new(
                "validation",
                "dotenv changed during credential migration",
            ));
        }
        let permissions = fs::metadata(path).map_err(io)?.permissions();
        let mut snapshot = tempfile::NamedTempFile::new_in(parent).map_err(io)?;
        snapshot.write_all(&original).map_err(io)?;
        snapshot
            .as_file()
            .set_permissions(permissions.clone())
            .map_err(io)?;
        snapshot.as_file().sync_all().map_err(io)?;
        observer("credentials.after_backup_sync")?;
        match snapshot.persist_noclobber(&backup) {
            Ok(_) => (),
            Err(error)
                if error.error.kind() == std::io::ErrorKind::AlreadyExists
                    && fs::read(&backup).ok().as_deref() == Some(original.as_slice()) => {}
            Err(error) => return Err(io(error.error)),
        }
        sync_directory(parent)?;
        observer("credentials.after_backup_publish")?;
        let mut body = Zeroizing::new(
            splitlines(&text)
                .into_iter()
                .enumerate()
                .filter(|(index, _)| !drop_lines.contains(&(index + 1)))
                .map(|(_, line)| line)
                .collect::<Vec<_>>()
                .join("\n"),
        );
        if text.ends_with('\n') && !body.ends_with('\n') {
            body.push('\n');
        }
        let mut candidate = tempfile::NamedTempFile::new_in(parent).map_err(io)?;
        candidate.write_all(body.as_bytes()).map_err(io)?;
        candidate
            .as_file()
            .set_permissions(permissions)
            .map_err(io)?;
        candidate.as_file().sync_all().map_err(io)?;
        observer("credentials.after_sanitized_sync")?;
        if fs::read(path).map_err(io)? != *original {
            return Err(AppError::new(
                "validation",
                "dotenv changed during credential migration",
            ));
        }
        observer("credentials.before_sanitized_publish")?;
        candidate.persist(path).map_err(|e| io(e.error))?;
        observer("credentials.after_sanitized_publish")?;
        sync_directory(parent)?;
        observer("credentials.after_directory_sync")?;
        backup_path = s(backup.to_string_lossy().as_ref());
    }
    Ok(dict([
        ("dotenv_path", s(path.to_string_lossy().as_ref())),
        ("migrated", Value::List(migrated)),
        ("skipped", Value::List(skipped)),
        ("backup_path", backup_path),
        ("rewritten", Value::Bool(!drop_lines.is_empty())),
    ]))
}
fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(not(windows))]
    fs::File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(io)?;
    #[cfg(windows)]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_envelope::get;
    const BODY: &str = "# keep\r\nKASSIBER_BACKEND_CORE_RPCUSER=' user '\r\nSATBOOKS_BACKEND_CORE_RPC_PASSWORD=\" pw \"\r\nKASSIBER_BACKEND_CORE_AUTH-HEADER=' Bearer token '\r\nKASSIBER_BACKEND_CORE_TOKEN=\r\nKASSIBER_BACKEND_CORE_COOKIEFILE=/keep/cookie\r\nKASSIBER_BACKEND_GONE_TOKEN=retained\r\nopaque\r\n";
    fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE backends(name TEXT PRIMARY KEY, config_json TEXT, token TEXT, auth_header TEXT, updated_at TEXT); INSERT INTO backends VALUES('core','{\"preserved\": 7, \"password\": \"old\"}','old','old','old');").unwrap();
        conn
    }
    #[test]
    fn dotenv_aliases_quotes_line_numbers_and_secret_free_scan_match_python() {
        let entries = findings(BODY);
        assert_eq!(entries.len(), 5);
        assert_eq!(
            entries
                .iter()
                .map(|v| (
                    v.backend.as_str(),
                    v.field.as_str(),
                    v.lineno,
                    v.value.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![
                ("core", "username", 2, " user "),
                ("core", "password", 3, " pw "),
                ("core", "auth_header", 4, " Bearer token "),
                ("core", "token", 5, ""),
                ("gone", "token", 7, "retained")
            ]
        );
        let public = dumps(
            &Value::List(entries.iter().map(Finding::public).collect()),
            &Default::default(),
        )
        .unwrap();
        assert!(!public.contains("retained"));
        assert!(!public.contains("Bearer token"));
        assert_eq!(
            splitlines("a\r\nb\rc\nd\x0be\x0cf\x1cg\x1dh\x1ei\u{85}j\u{2028}k\u{2029}"),
            vec!["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k"]
        );
    }
    #[test]
    fn credential_move_preserves_unknown_fields_and_skipped_dotenv_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("backends.env");
        fs::write(&path, BODY).unwrap();
        let conn = database();
        let result = migrate(&conn, &path).unwrap();
        assert_eq!(get(&result, "rewritten"), Some(&Value::Bool(true)));
        let (config, token, auth): (String, String, String) = conn
            .query_row(
                "SELECT config_json,token,auth_header FROM backends",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            config,
            "{\"password\": \"pw\", \"preserved\": 7, \"username\": \"user\"}"
        );
        assert_eq!(token, "");
        assert_eq!(auth, " Bearer token ");
        assert_eq!(fs::read_to_string(&path).unwrap(), "# keep\nKASSIBER_BACKEND_CORE_COOKIEFILE=/keep/cookie\nKASSIBER_BACKEND_GONE_TOKEN=retained\nopaque\n");
        let backup = get(&result, "backup_path")
            .and_then(|v| {
                if let Value::Str(v) = v {
                    v.as_str()
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(fs::read(backup).unwrap(), BODY.as_bytes());
        let again = migrate(&conn, &path).unwrap();
        assert_eq!(get(&again, "rewritten"), Some(&Value::Bool(false)));
    }
    #[test]
    fn duplicate_credential_targets_fail_before_database_or_dotenv_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("backends.env");
        let body =
            "KASSIBER_BACKEND_CORE_PASSWORD=first\nSATBOOKS_BACKEND_CORE_RPCPASSWORD=second\n";
        fs::write(&path, body).unwrap();
        let conn = database();
        assert_eq!(migrate(&conn, &path).unwrap_err().code, "validation");
        assert_eq!(
            conn.query_row("SELECT config_json FROM backends", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "{\"preserved\": 7, \"password\": \"old\"}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), body);
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
    }
    #[test]
    fn changed_dotenv_is_preserved_before_capture_and_before_publication() {
        for point in [
            "credentials.after_row_commit",
            "credentials.after_sanitized_sync",
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join("backends.env");
            fs::write(&path, BODY).unwrap();
            let conn = database();
            let changed = format!("{BODY}# concurrent edit\n");
            let error = migrate_observed(&conn, &path, &mut |label| {
                if label == point {
                    fs::write(&path, &changed).unwrap();
                }
                Ok(())
            })
            .unwrap_err();
            assert_eq!(error.code, "validation", "{point}");
            assert_eq!(fs::read_to_string(&path).unwrap(), changed, "{point}");
            let config: String = conn
                .query_row("SELECT config_json FROM backends", [], |row| row.get(0))
                .unwrap();
            assert!(config.contains("\"password\": \"pw\""), "{point}");
        }
    }
    #[test]
    fn every_credential_publication_boundary_retains_committed_values_and_is_repeatable() {
        for point in [
            "credentials.after_row_commit",
            "credentials.after_backup_sync",
            "credentials.after_backup_publish",
            "credentials.after_sanitized_sync",
            "credentials.before_sanitized_publish",
            "credentials.after_sanitized_publish",
            "credentials.after_directory_sync",
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join("backends.env");
            fs::write(&path, BODY).unwrap();
            let conn = database();
            assert!(
                migrate_observed(&conn, &path, &mut |label| if label == point {
                    Err(AppError::new("internal_error", "injected interruption"))
                } else {
                    Ok(())
                })
                .is_err()
            );
            let body = fs::read_to_string(&path).unwrap();
            assert!(body==BODY || body=="# keep\nKASSIBER_BACKEND_CORE_COOKIEFILE=/keep/cookie\nKASSIBER_BACKEND_GONE_TOKEN=retained\nopaque\n");
            migrate(&conn, &path).unwrap();
            let config: String = conn
                .query_row("SELECT config_json FROM backends", [], |r| r.get(0))
                .unwrap();
            assert!(config.contains("\"password\": \"pw\""), "{point}");
            assert_eq!(
                conn.query_row("SELECT token FROM backends", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                ""
            );
        }
    }
}
