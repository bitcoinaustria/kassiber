//! CSV export commands: prepare, publish and complete one artifact.
//!
//! Saved exports hold one write transaction from report admission through
//! the saved marker and its dependencies, so the registered summaries always
//! describe the admitted book version; a concurrent writer waits instead of
//! changing it in between. Observed transaction lists register nothing.
use super::*;
use crate::artifact_csv::Prepared;
use crate::artifact_publication::{self, PublishedFile};
use kassiber_setup::Sources;
use std::path::{Path, PathBuf};

/// `OSError.__str__` for a failed artifact write.
pub fn io_error(error: std::io::Error, path: Option<&Path>) -> AppError {
    let message = match error.raw_os_error() {
        Some(number) => {
            let text = error.to_string();
            let suffix = format!(" (os error {number})");
            let strerror = text.strip_suffix(&suffix).unwrap_or(&text);
            let label = if cfg!(windows) { "WinError" } else { "Errno" };
            let mut message = format!("[{label} {number}] {strerror}");
            if let Some(path) = path {
                message.push_str(": ");
                message.push_str(&kassiber_pytext::repr_str(&path.to_string_lossy()));
            }
            message
        }
        None => error.to_string(),
    };
    AppError::new("internal_error", message)
}

/// Run `operation` in the admitted write transaction (or a savepoint of the
/// caller's transaction), committing only after every step succeeded.
pub(crate) fn admitted<T>(conn: &Connection, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let owns = conn.is_autocommit();
    conn.execute_batch(if owns {
        "BEGIN IMMEDIATE"
    } else {
        "SAVEPOINT report_export"
    })
    .map_err(sql_error)?;
    let rollback = || {
        let _ = conn.execute_batch(if owns {
            "ROLLBACK"
        } else {
            "ROLLBACK TO report_export; RELEASE report_export"
        });
    };
    match operation() {
        Ok(value) => match conn.execute_batch(if owns {
            "COMMIT"
        } else {
            "RELEASE report_export"
        }) {
            Ok(()) => Ok(value),
            Err(error) => {
                rollback();
                Err(sql_error(error))
            }
        },
        Err(error) => {
            rollback();
            Err(error)
        }
    }
}

fn path_text(path: &Path) -> Value {
    s(path.to_string_lossy().into_owned())
}
fn file_entry(file: &PublishedFile) -> Value {
    dict([
        ("sheet", s(file.sheet.as_str())),
        ("file", path_text(&file.path)),
        ("bytes", Value::Int(file.bytes.into())),
        ("rows", Value::Int(file.rows.into())),
    ])
}

/// Path.mkdir(parents=True, exist_ok=True), retaining the actual failing
/// ancestor in the error rather than substituting the final artifact path.
pub(crate) fn create_directory(path: &Path) -> Result<()> {
    match std::fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let Some(parent) = path.parent().filter(|parent| *parent != path) else {
                return Err(io_error(error, Some(path)));
            };
            create_directory(if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            })?;
            match std::fs::create_dir(path) {
                Ok(()) => Ok(()),
                Err(_) if path.is_dir() => Ok(()),
                Err(error) => Err(io_error(error, Some(path))),
            }
        }
        Err(_) if path.is_dir() => Ok(()),
        Err(error) => Err(io_error(error, Some(path))),
    }
}
/// Publish `prepared`, then register its saved marker when it has one.
fn complete(
    conn: &Connection,
    profile: &Value,
    prepared: Prepared,
    target: &Path,
    bundle: bool,
    sources: &dyn Sources,
    published: &mut dyn FnMut() -> Result<()>,
) -> Result<Value> {
    let failed = |e| io_error(e, Some(target));
    let mut result;
    let mut files = None;
    let paths: Vec<PathBuf> = if bundle {
        create_directory(target)?;
        let published = artifact_publication::bundle(&prepared, target).map_err(failed)?;
        result = dict([("dir", path_text(&target.canonicalize().map_err(failed)?))]);
        files = Some(Value::List(published.iter().map(file_entry).collect()));
        published.into_iter().map(|file| file.path).collect()
    } else {
        create_directory(
            target
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?;
        let published = artifact_publication::file(&prepared, target).map_err(failed)?;
        result = dict([("file", path_text(&published.path))]);
        // PDF writers report their page count between the path and size.
        if let Some(pages) = get(&prepared.metadata, "pages") {
            replace(&mut result, "pages", pages.clone());
        }
        replace(&mut result, "bytes", Value::Int(published.bytes.into()));
        vec![target.to_owned()]
    };
    let Value::Dict(metadata) = &prepared.metadata else {
        unreachable!("prepared metadata is a dict")
    };
    for (key, item) in metadata {
        let key = match key {
            kassiber_pyjson::Key::Str(key) => py_str(&Value::Str(key.clone())),
            _ => unreachable!("metadata keys are strings"),
        };
        // The bundle lists its files just before the row counts.
        if key == "rows" {
            if let Some(files) = files.take() {
                replace(&mut result, "files", files);
            }
        }
        replace(&mut result, &key, item.clone());
    }
    published()?;
    if let Some(completion) = &prepared.completion {
        let snapshot = export_completion::register(
            conn,
            &text(profile, "workspace_id"),
            &text(profile, "id"),
            completion,
            &paths,
            sources,
        )?;
        replace(&mut result, "report_snapshot", snapshot);
    }
    Ok(result)
}

/// Publish a prepared native report or observed transaction list.
/// `target` is the caller's already expanded file or bundle directory.
pub fn export(
    conn: &Connection,
    command: &str,
    args: &Value,
    target: &Path,
    sources: &dyn Sources,
) -> Result<Value> {
    export_with(conn, command, args, target, sources, &mut || Ok(()))
}

/// `export` with a hook between publication and completion (tests only
/// observe the retained admission there).
pub fn export_with(
    conn: &Connection,
    command: &str,
    args: &Value,
    target: &Path,
    sources: &dyn Sources,
    published: &mut dyn FnMut() -> Result<()>,
) -> Result<Value> {
    let prepare = || {
        if command == "reports.export-year-handoff" {
            year_handoff_xlsx::prepare(conn, args, sources)
        } else if command.ends_with("-xlsx") {
            artifact_xlsx::prepare(conn, command, args, sources)
        } else {
            artifact_csv::prepare(conn, command, args, sources)
        }
    };
    if matches!(
        command,
        "reports.export-transactions-csv" | "reports.export-transactions-xlsx"
    ) {
        let prepared = prepare()?;
        return complete(
            conn,
            &Value::Null,
            prepared,
            target,
            false,
            sources,
            published,
        );
    }
    admitted(conn, || {
        let prepared = prepare()?;
        let (_, profile) = kassiber_setup::references::scope(conn, args)?;
        let bundle = command == "reports.export-austrian-e1kv-csv";
        complete(conn, &profile, prepared, target, bundle, sources, published)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_store::rusqlite;

    struct Fixed(String);
    impl Sources for Fixed {
        fn uuid(&self, _: &Connection) -> Result<String> {
            Ok(self.0.clone())
        }
        fn now(&self, _: &Connection) -> Result<String> {
            Ok("2026-01-02T03:04:05Z".into())
        }
    }
    fn encode(v: &Value) -> String {
        kassiber_pyjson::dumps(
            v,
            &kassiber_pyjson::Options {
                sort_keys: true,
                separators: Some((",".into(), ":".into())),
                ..Default::default()
            },
        )
        .unwrap()
    }
    fn dump(conn: &Connection) -> Value {
        let tables = rows(conn, "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name", []).unwrap();
        dict(tables.iter().map(|table| {
            let name = text(table, "name");
            let mut contents = rows(conn, &format!("SELECT * FROM \"{name}\""), []).unwrap();
            contents.sort_by_key(encode);
            (name, Value::List(contents))
        }))
    }
    /// Complete Python exports, repeated explicit filing, failures and drafts.
    /// Compare every table after each action, keeping stored JSON literal.
    #[test]
    fn csv_export_sequences_match_python_artifacts_and_database() {
        let vectors =
            kassiber_pytext::loads(include_str!("../tests/data/export_sequences.json")).unwrap();
        for case in artifact_csv::list(&vectors) {
            let (root, book) = book();
            let conn = book.connection();
            conn.execute_batch(&text(case, "sql")).unwrap();
            assert_eq!(
                encode(&dump(conn)),
                encode(value(case, "before")),
                "{}: starting database",
                text(case, "name")
            );
            for (index, action) in artifact_csv::list(value(case, "actions"))
                .iter()
                .enumerate()
            {
                let command = text(action, "command");
                let sources = Fixed(format!("00000000-0000-0000-0000-{:012x}", index + 1));
                let args = value(action, "args");
                let result = if command == "reports.filed-snapshots.create" {
                    crate::execute_with_sources(conn, &command, args, &sources)
                } else {
                    export(
                        conn,
                        &command,
                        args,
                        &root.path().join(text(action, "target")),
                        &sources,
                    )
                };
                let result = match result {
                    Ok(v) => dict([("ok", v)]),
                    Err(e) => dict([(
                        "error",
                        dict([
                            ("code", s(e.code)),
                            ("message", s(e.message)),
                            ("hint", e.hint.map(s).unwrap_or(Value::Null)),
                            ("details", e.details.unwrap_or(Value::Null)),
                            ("retryable", Value::Bool(e.retryable)),
                        ]),
                    )]),
                };
                assert_eq!(
                    encode(&result).replace(&root.path().to_string_lossy().to_string(), "<root>"),
                    encode(value(action, "expected")),
                    "{} action {index}",
                    text(case, "name")
                );
                assert_eq!(
                    encode(&dump(conn)),
                    encode(value(action, "tables")),
                    "{} action {index}: database",
                    text(case, "name")
                );
                if let Value::Dict(files) = value(action, "files") {
                    for (name, expected) in files {
                        let kassiber_pyjson::Key::Str(name) = name else {
                            panic!()
                        };
                        assert_eq!(
                            std::fs::read(root.path().join(name.as_str().unwrap())).unwrap(),
                            py_str(expected).as_bytes(),
                            "{} action {index}: {name:?}",
                            text(case, "name")
                        );
                    }
                }
            }
        }
    }

    fn book() -> (tempfile::TempDir, kassiber_store::Book) {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        book.connection()
            .execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        (root, book)
    }
    fn concurrent_write(database: &Path) -> rusqlite::Result<usize> {
        let other = rusqlite::Connection::open(database)?;
        other.busy_timeout(std::time::Duration::ZERO)?;
        other.execute(
            "UPDATE profiles SET journal_input_version=journal_input_version+1 WHERE id='p'",
            [],
        )
    }

    /// Admission is retained: another writer cannot change the admitted book
    /// between publication and completion, so the marker matches the artifact.
    #[test]
    fn saved_export_retains_admitted_book_through_completion() {
        let (root, book) = book();
        let conn = book.connection();
        // WAL readers do not block writers; only the write transaction does.
        let mode: String = conn
            .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let target = root.path().join("out/report.csv");
        let mut blocked = None;
        let result = export_with(
            conn,
            "reports.export-csv",
            &dict::<&str>([]),
            &target,
            &kassiber_setup::SystemSources,
            &mut || {
                blocked = Some(concurrent_write(book.database()).is_err());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            blocked,
            Some(true),
            "a concurrent writer changed the admitted book"
        );
        assert!(conn.is_autocommit());
        let snapshot = value(&result, "report_snapshot");
        assert_eq!(text(snapshot, "report_state"), "saved");
        assert_eq!(
            text(snapshot, "content_sha256"),
            export_completion::content_sha256(std::slice::from_ref(&target)).unwrap()
        );
        // The writer proceeds once completion committed; the next export is stale.
        assert_eq!(concurrent_write(book.database()).unwrap(), 1);
        let stale = export(
            conn,
            "reports.export-csv",
            &dict::<&str>([]),
            &root.path().join("out/second.csv"),
            &kassiber_setup::SystemSources,
        )
        .unwrap_err();
        assert_eq!(stale.code, "app_error");
        assert!(!root.path().join("out/second.csv").exists());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM filed_report_snapshots", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    /// A failed completion rolls back the marker while the published file
    /// remains, as Python leaves its artifact after a failed registration.
    #[test]
    fn failed_completion_keeps_artifact_and_writes_no_marker() {
        let (root, book) = book();
        let conn = book.connection();
        conn.execute_batch(
            "CREATE TRIGGER reject BEFORE INSERT ON filed_report_chain_captures BEGIN SELECT RAISE(ABORT,'rejected'); END;",
        )
        .unwrap();
        let target = root.path().join("report.csv");
        let error = export(
            conn,
            "reports.export-csv",
            &dict::<&str>([]),
            &target,
            &kassiber_setup::SystemSources,
        )
        .unwrap_err();
        assert_eq!(error.message, "rejected");
        assert!(target.is_file());
        assert!(conn.is_autocommit());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM filed_report_snapshots", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    /// Observed transaction lists never register a saved or filed marker.
    #[test]
    fn transaction_csv_export_registers_nothing() {
        let (root, book) = book();
        let conn = book.connection();
        let target = root.path().join("transactions.csv");
        let result = export(
            conn,
            "reports.export-transactions-csv",
            &dict::<&str>([]),
            &target,
            &kassiber_setup::SystemSources,
        )
        .unwrap();
        assert!(get(&result, "report_snapshot").is_none());
        assert!(target.is_file());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM filed_report_snapshots", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
}
