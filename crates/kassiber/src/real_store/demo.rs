//! `demo.create`: writes the bundled demo book into a fresh directory
//! (docs/reference/demo.md). The payload is a gzip'd SQL dump built by
//! `scripts/build-demo-payload.sh`: no backend rows, no default backend,
//! manual backend setup with the built-ins tombstoned, attachment files in
//! the staging table `demo_payload_files`. Nothing here opens a socket.
//!
//! Each call writes a new generation `<base>/<stamp>-<hex>/` and then removes
//! earlier generations under `<base>`: only directories that carry the demo
//! marker, never anything else, and never by following a symlink. A copy that
//! cannot be removed (still open on Windows) stays until the next call.
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use kassiber_envelope::{dict, s, AppError};
use kassiber_paths::Context;
use kassiber_pyjson::Value;
use kassiber_store::map_sqlite_error;
use kassiber_store::rusqlite::Connection;

/// The bundled demo book.
static PAYLOAD: &[u8] = include_bytes!("../../assets/demo-book.sql.gz");
/// Marks a finished copy (`config/demo.json`).
const MARKER: &str = "demo.json";
/// Marks a copy still being written (`<tmp>/.kassiber-demo-partial`).
const PARTIAL: &str = ".kassiber-demo-partial";
const STAGING: &str = "demo_payload_files";

fn io_error(context: &str) -> impl Fn(std::io::Error) -> AppError + '_ {
    move |e| AppError::new("demo_write_failed", format!("{context}: {e}"))
}

fn sql_error(e: kassiber_store::rusqlite::Error) -> AppError {
    let mapped = map_sqlite_error(e);
    AppError::new("demo_write_failed", mapped.message.to_string())
}

/// Owner-only directory, as the state root's own directories are.
fn make_dir(path: &Path) -> Result<(), AppError> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(io_error("could not create a demo directory"))
}

/// Owner-only file, created new.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(io_error("could not write a demo file"))?;
    std::io::Write::write_all(&mut file, bytes).map_err(io_error("could not write a demo file"))
}

/// A stored attachment path: relative, plain components only.
fn safe_relpath(relpath: &str) -> Option<PathBuf> {
    if relpath.is_empty() || relpath.contains('\\') {
        return None;
    }
    let path = Path::new(relpath);
    path.components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then(|| path.to_path_buf())
}

/// Whether `dir` is a finished or partial demo copy this module wrote. A
/// symlink is never one.
fn is_demo_copy(dir: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(dir) else {
        return false;
    };
    if !meta.is_dir() {
        return false;
    }
    if dir.join(PARTIAL).is_file() {
        return true;
    }
    let Some(raw) = kassiber_paths::read_small_regular_file(&dir.join("config").join(MARKER), 4096)
    else {
        return false;
    };
    std::str::from_utf8(&raw)
        .ok()
        .and_then(|text| kassiber_pytext::loads(text).ok())
        .and_then(|value| kassiber_envelope::get(&value, "kassiber_demo").cloned())
        .is_some_and(|v| v == Value::Int(1.into()))
}

/// The directory that holds demo copies: `--path`, or `demo/` under the
/// state root.
fn base(context: &Context, args: &Value) -> Result<PathBuf, AppError> {
    let requested = match kassiber_envelope::get(args, "path") {
        Some(Value::Str(text)) => text
            .as_str()
            .filter(|t| !t.trim().is_empty())
            .map(str::to_owned),
        _ => None,
    };
    let path = match requested {
        Some(path) => context
            .expand_user(Path::new(&path))
            .map_err(|e| AppError::new("invalid_path", e.message.to_owned()))?,
        None => context
            .default_state_root()
            .map_err(|e| AppError::new("invalid_path", e.message.to_owned()))?
            .join("demo"),
    };
    kassiber_paths::abspath(&path).map_err(io_error("could not resolve the demo path"))
}

/// Copies the current machine-wide preference into the demo, so the user's
/// offline and agent choices carry in while changes made in the demo stay
/// there. An absent preference stays absent (its default applies).
fn seed_preference(source: Option<PathBuf>, target: &Path) -> Result<bool, AppError> {
    let Some(source) = source else {
        return Ok(false);
    };
    match kassiber_paths::read_small_regular_file(&source, 64 * 1024) {
        Some(bytes) => write_private(target, &bytes).map(|_| true),
        None => Ok(false),
    }
}

fn restore(database: &Path, attachments: &Path) -> Result<(), AppError> {
    let mut sql = String::new();
    flate2::read::GzDecoder::new(PAYLOAD)
        .read_to_string(&mut sql)
        .map_err(|e| AppError::new("demo_payload_invalid", format!("bundled demo book: {e}")))?;
    let conn = Connection::open(database).map_err(sql_error)?;
    conn.execute_batch(&sql).map_err(sql_error)?;
    let files: Vec<(String, Vec<u8>)> = {
        let mut statement = conn
            .prepare(&format!(
                "SELECT relpath, content FROM {STAGING} ORDER BY relpath"
            ))
            .map_err(sql_error)?;
        let rows = statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(sql_error)?;
        rows.collect::<Result<_, _>>().map_err(sql_error)?
    };
    for (relpath, content) in files {
        let rel = safe_relpath(&relpath).ok_or_else(|| {
            AppError::new(
                "demo_payload_invalid",
                "bundled demo attachment path is not relative",
            )
        })?;
        let target = attachments.join(rel);
        if let Some(parent) = target.parent() {
            make_dir(parent)?;
        }
        write_private(&target, &content)?;
    }
    // A fresh identity per copy: copies never look like the same database to
    // device sync or the operator broker.
    conn.execute_batch(&format!(
        "BEGIN IMMEDIATE;
         DROP TABLE {STAGING};
         UPDATE settings SET value = lower(hex(randomblob(16))) WHERE key = 'database_instance_id';
         COMMIT;"
    ))
    .map_err(sql_error)?;
    conn.execute_batch("VACUUM").map_err(sql_error)?;
    conn.close().map_err(|(_, e)| sql_error(e))
}

fn generation_name() -> Result<String, AppError> {
    let conn = Connection::open_in_memory().map_err(sql_error)?;
    conn.query_row(
        "SELECT strftime('%Y%m%dT%H%M%SZ', 'now') || '-' || lower(hex(randomblob(4)))",
        [],
        |r| r.get(0),
    )
    .map_err(sql_error)
}

/// The demo copy's own wallet-backend lookup preference ("Fill in
/// transaction details from the wallet's backend"), seeded from the user's
/// like offline mode and agent access, so a change inside the demo never
/// reaches the user's books.
const LOOKUP_FILE: &str = "wallet-backend-lookup.json";

pub(super) fn create(context: &Context, args: &Value) -> Result<Value, AppError> {
    let base = base(context, args)?;
    if base.exists() {
        let meta =
            fs::symlink_metadata(&base).map_err(io_error("could not inspect the demo path"))?;
        if !meta.is_dir() {
            return Err(AppError::new(
                "demo_path_invalid",
                "The demo path exists and is not a directory",
            )
            .with_hint("Choose an empty directory, or one that only holds earlier demo copies."));
        }
    }
    make_dir(&base)?;
    // Refuse a directory that holds anything but demo copies: removal below
    // must never reach the user's own files.
    let mut earlier = Vec::new();
    for entry in fs::read_dir(&base).map_err(io_error("could not list the demo path"))? {
        let path = entry
            .map_err(io_error("could not list the demo path"))?
            .path();
        if is_demo_copy(&path) {
            earlier.push(path);
        } else if path.file_name().is_some_and(|n| n == ".DS_Store") {
            continue;
        } else {
            return Err(AppError::new(
                "demo_path_not_empty",
                "The demo path holds files that are not Kassiber demo copies",
            )
            .with_hint("Choose an empty directory for `--path`; Kassiber only replaces its own demo copies."));
        }
    }
    let name = generation_name()?;
    let partial = base.join(format!(".tmp-{name}"));
    make_dir(&partial)?;
    let written = (|| {
        write_private(&partial.join(PARTIAL), b"")?;
        for dir in ["config", "data", "attachments", "exports"] {
            make_dir(&partial.join(dir))?;
        }
        restore(
            &kassiber_paths::resolve_database_path(&partial.join("data")),
            &partial.join("attachments"),
        )?;
        let offline = seed_preference(
            kassiber_setup::offline::preference_path(context).ok(),
            &partial.join("config").join("offline-mode.json"),
        )?;
        let agents = seed_preference(
            kassiber_setup::agent_access::preference_path(context).ok(),
            &partial.join("config").join("agent-access.json"),
        )?;
        let lookup = seed_preference(
            kassiber_setup::wallet_backend_lookup::preference_path(context).ok(),
            &partial.join("config").join(LOOKUP_FILE),
        )?;
        let marker = kassiber_pyjson::dumps(
            &dict([
                ("kassiber_demo", Value::Int(1.into())),
                ("schema_version", Value::Int(1.into())),
                ("generation", s(name.as_str())),
            ]),
            &kassiber_pyjson::Options {
                sort_keys: true,
                ..Default::default()
            },
        )
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
        write_private(&partial.join("config").join(MARKER), marker.as_bytes())?;
        fs::remove_file(partial.join(PARTIAL))
            .map_err(io_error("could not finish the demo copy"))?;
        Ok::<_, AppError>((offline, agents, lookup))
    })();
    let (offline_seeded, agents_seeded, lookup_seeded) = match written {
        Ok(seeded) => seeded,
        Err(error) => {
            let _ = fs::remove_dir_all(&partial);
            return Err(error);
        }
    };
    let root = base.join(&name);
    fs::rename(&partial, &root).map_err(io_error("could not finish the demo copy"))?;
    let mut removed = 0i64;
    let mut kept = 0i64;
    for path in earlier {
        if fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        } else {
            kept += 1;
        }
    }
    let text = |p: &Path| s(p.to_string_lossy().as_ref());
    let data_root = root.join("data");
    let offline_file = root.join("config").join("offline-mode.json");
    let agents_file = root.join("config").join("agent-access.json");
    let lookup_file = root.join("config").join(LOOKUP_FILE);
    let preferences = dict([
        ("KASSIBER_OFFLINE_PREFERENCE_FILE", text(&offline_file)),
        ("KASSIBER_AGENT_ACCESS_FILE", text(&agents_file)),
        (
            kassiber_setup::wallet_backend_lookup::ENV,
            text(&lookup_file),
        ),
    ]);
    Ok(dict([
        ("schema_version", Value::Int(1.into())),
        ("generation", s(name.as_str())),
        ("path", text(&base)),
        ("state_root", text(&root)),
        ("data_root", text(&data_root)),
        ("preference_environment", preferences.clone()),
        (
            "preferences_seeded",
            dict([
                ("offline_mode", Value::Bool(offline_seeded)),
                ("agent_access", Value::Bool(agents_seeded)),
                ("wallet_backend_lookup", Value::Bool(lookup_seeded)),
            ]),
        ),
        (
            "mcp_server",
            dict([
                ("command", s("kassiber")),
                (
                    "args",
                    Value::List(vec![
                        s("--data-root"),
                        text(&data_root),
                        s("mcp"),
                        s("serve"),
                    ]),
                ),
                ("env", preferences),
            ]),
        ),
        ("removed_copies", Value::Int(removed.into())),
        ("kept_copies", Value::Int(kept.into())),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A context whose home and preferences live in `dir`: tests never read
    /// the user's state root.
    fn context(dir: &Path) -> Context {
        let mut environ = std::collections::BTreeMap::new();
        environ.insert(
            "KASSIBER_OFFLINE_PREFERENCE_FILE".into(),
            dir.join("offline.json").into_os_string(),
        );
        environ.insert(
            "KASSIBER_AGENT_ACCESS_FILE".into(),
            dir.join("agents.json").into_os_string(),
        );
        environ.insert(
            kassiber_setup::wallet_backend_lookup::ENV.into(),
            dir.join("lookup.json").into_os_string(),
        );
        Context {
            platform: kassiber_paths::Platform::current(),
            home: dir.join("home"),
            environ,
        }
    }

    #[test]
    fn attachment_paths_stay_relative() {
        assert!(safe_relpath("a/b.csv").is_some());
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "a\\b", "./a"] {
            assert!(safe_relpath(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn payload_has_no_backend_configuration() {
        let mut sql = String::new();
        flate2::read::GzDecoder::new(PAYLOAD)
            .read_to_string(&mut sql)
            .unwrap();
        assert!(!sql.contains("INSERT INTO backends"), "a backend row ships");
        assert!(!sql.contains("\"password\": "), "a credential ships");
        assert!(!sql.contains("/var/folders/"), "a generator path ships");
        assert!(
            !sql.contains("'default_backend'"),
            "a default backend ships"
        );
        assert!(sql.contains("'backend_bootstrap_mode','manual'"));
        assert!(!sql.contains("INSERT INTO ai_providers"));
        assert!(!sql.contains("INSERT INTO ai_chat_sessions"));
        // The regtest stack's invented quote is labelled synthetic, never as
        // a provider's market data.
        let rates: Vec<&str> = sql
            .lines()
            .filter(|line| line.starts_with("INSERT INTO rates_cache"))
            .collect();
        assert!(!rates.is_empty());
        for row in rates {
            assert!(
                !row.contains("'mempool'") && !row.contains("'mempool_prices'"),
                "a rate claims mempool market data: {row}"
            );
        }
    }

    #[test]
    fn a_copy_restores_and_old_copies_go() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("demo");
        let context = context(dir.path());
        fs::write(
            dir.path().join("offline.json"),
            r#"{"schema_version": 1, "enabled": true}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("lookup.json"),
            r#"{"schema_version": 1, "enabled": false}"#,
        )
        .unwrap();
        let args = dict([("path", s(base.to_string_lossy().as_ref()))]);
        let first = create(&context, &args).unwrap();
        let first_root = PathBuf::from(kassiber_envelope::py_str(
            kassiber_envelope::get(&first, "state_root").unwrap(),
        ));
        let db = kassiber_paths::resolve_database_path(&first_root.join("data"));
        // The user's offline choice carries in; an absent agent file stays absent.
        assert!(kassiber_setup::offline::enabled(
            &first_root.join("config/offline-mode.json")
        ));
        assert!(!first_root.join("config/agent-access.json").exists());
        // The user's lookup choice (off) carries in too.
        assert!(!kassiber_setup::wallet_backend_lookup::enabled(
            &first_root.join("config/wallet-backend-lookup.json")
        ));
        assert_eq!(
            kassiber_envelope::get(
                kassiber_envelope::get(&first, "preference_environment").unwrap(),
                kassiber_setup::wallet_backend_lookup::ENV
            ),
            Some(&s(first_root
                .join("config/wallet-backend-lookup.json")
                .to_string_lossy()
                .as_ref()))
        );
        let conn = Connection::open(&db).unwrap();
        let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
        assert!(count("SELECT count(*) FROM transactions") > 1000);
        assert_eq!(count("SELECT count(*) FROM backends"), 0);
        assert_eq!(
            count("SELECT count(*) FROM sqlite_master WHERE name = 'demo_payload_files'"),
            0
        );
        assert_eq!(
            count("SELECT count(*) FROM wallets WHERE config_json LIKE '%\"backend\"%'"),
            0
        );
        let attachment: String = conn
            .query_row("SELECT stored_relpath FROM attachments LIMIT 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(first_root.join("attachments").join(attachment).is_file());
        drop(conn);
        assert!(is_demo_copy(&first_root));

        let second = create(&context, &args).unwrap();
        assert_eq!(
            kassiber_envelope::get(&second, "removed_copies"),
            Some(&Value::Int(1.into()))
        );
        assert!(!first_root.exists());
        let entries: Vec<_> = fs::read_dir(&base).unwrap().collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn foreign_content_is_never_removed() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("mine");
        fs::create_dir_all(base.join("photos")).unwrap();
        fs::write(base.join("notes.txt"), "keep").unwrap();
        let context = context(dir.path());
        let error = create(
            &context,
            &dict([("path", s(base.to_string_lossy().as_ref()))]),
        )
        .unwrap_err();
        assert_eq!(error.code, "demo_path_not_empty");
        assert!(base.join("notes.txt").is_file());
        assert!(base.join("photos").is_dir());
        // A directory that only claims a marker it lacks is foreign too.
        assert!(!is_demo_copy(&base.join("photos")));
    }
}
