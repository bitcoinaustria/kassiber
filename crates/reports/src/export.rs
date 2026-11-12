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

/// Write `bytes` like `fs::write` (create or truncate in place), then flush
/// the file: these reference exporters keep Python's direct write but must
/// not lose a finished artifact to a crash.
pub(crate) fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    sync_written(&file, path)
}

/// Flush a file written in place to the device (`F_FULLFSYNC` on macOS), then
/// its directory entry. Only regular files: a FIFO or device the user named
/// has nothing to flush. The directory sync is best effort, as Python's
/// `_fsync_csv_directory` is where the file system refuses it.
pub(crate) fn sync_written(file: &std::fs::File, path: &Path) -> std::io::Result<()> {
    if !file.metadata()?.is_file() {
        return Ok(());
    }
    #[cfg(test)]
    tests::SYNCED.with(|synced| synced.borrow_mut().push(path.to_owned()));
    file.sync_all()?;
    #[cfg(not(windows))]
    if let Ok(directory) = std::fs::File::open(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    ) {
        let _ = directory.sync_all();
    }
    Ok(())
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
/// `str(PurePosixPath(path))`: repeated separators and `.` components drop,
/// a trailing separator goes, exactly two leading separators stay, `..` stays.
pub(crate) fn python_path_text(path: &str) -> String {
    let root = if path.starts_with("//") && !path.starts_with("///") {
        "//"
    } else if path.starts_with('/') {
        "/"
    } else {
        ""
    };
    let parts = path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>();
    if root.is_empty() && parts.is_empty() {
        return ".".into();
    }
    format!("{root}{}", parts.join("/"))
}

/// `_export_csv_rows`'s storage refusal: the requested destination and errno,
/// never a private staging name. `destination` is the text Python formats:
/// the report exports pass a normalized `Path`, Lightning its raw argument.
pub(crate) fn export_write_failed(error: std::io::Error, destination: &str) -> AppError {
    let destination = destination.to_owned();
    let reason = match error.raw_os_error() {
        Some(number) => {
            let text = error.to_string();
            text.strip_suffix(&format!(" (os error {number})"))
                .unwrap_or(&text)
                .to_owned()
        }
        None => "I/O error".to_owned(),
    };
    AppError::new(
        "export_write_failed",
        format!("Could not write CSV export '{destination}': {reason}"),
    )
    .with_details(dict([
        ("destination", s(destination)),
        (
            "errno",
            error
                .raw_os_error()
                .map_or(Value::Null, |n| Value::Int(n.into())),
        ),
    ]))
}

/// `_register_saved_export`'s answer for a device, pipe or other
/// non-regular artifact: written, but no saved snapshot.
fn non_regular_target() -> Value {
    dict([
        ("status", s("skipped")),
        ("reason", s("non_regular_target")),
        (
            "message",
            s("Saved-artifact registration skipped for non-regular output target."),
        ),
    ])
}

/// Publish `prepared`, then register its saved marker when it has one.
/// `shared_csv` exports go through Python's `_export_csv_rows`: each file
/// creates its own parents and every storage failure is `export_write_failed`
/// against the file it concerns.
#[allow(clippy::too_many_arguments)]
fn complete(
    conn: &Connection,
    profile: &Value,
    prepared: Prepared,
    target: &Path,
    bundle: bool,
    shared_csv: bool,
    sources: &dyn Sources,
    published: &mut dyn FnMut() -> Result<()>,
) -> Result<Value> {
    let failed = |failure: artifact_publication::Failure| match failure {
        artifact_publication::Failure::Write(path, error) if shared_csv => {
            export_write_failed(error, &python_path_text(&path.to_string_lossy()))
        }
        failure => io_error(failure.into_io(), Some(target)),
    };
    let mut result;
    let mut files = None;
    let paths: Vec<PathBuf> = if bundle {
        if !shared_csv {
            create_directory(target)?;
        }
        let published = artifact_publication::bundle(&prepared, target).map_err(failed)?;
        let directory =
            artifact_publication::resolve(target).map_err(|e| io_error(e, Some(target)))?;
        result = dict([("dir", path_text(&directory))]);
        files = Some(Value::List(published.iter().map(file_entry).collect()));
        published.into_iter().map(|file| file.path).collect()
    } else {
        if !shared_csv {
            create_directory(
                target
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new(".")),
            )?;
        }
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
        let mut regular = true;
        for path in &paths {
            regular &= std::fs::metadata(path)
                .map_err(|e| io_error(e, Some(path)))?
                .is_file();
        }
        let snapshot = if regular {
            export_completion::register(
                conn,
                &text(profile, "workspace_id"),
                &text(profile, "id"),
                completion,
                &paths,
                sources,
            )?
        } else {
            non_regular_target()
        };
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
    // The report CSVs Python writes through `_export_csv_rows`; platform
    // CSVs and workbooks keep their own writers' failures.
    let shared_csv = matches!(
        command,
        "reports.export-csv"
            | "reports.export-transactions-csv"
            | "reports.export-austrian-e1kv-csv"
    );
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
            shared_csv,
            sources,
            published,
        );
    }
    admitted(conn, || {
        let prepared = prepare()?;
        let (_, profile) = kassiber_setup::references::scope(conn, args)?;
        let bundle = command == "reports.export-austrian-e1kv-csv";
        complete(
            conn, &profile, prepared, target, bundle, shared_csv, sources, published,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_store::rusqlite;
    use std::collections::BTreeMap;

    thread_local! {
        /// Files `sync_written` flushed on this test thread.
        pub(crate) static SYNCED: std::cell::RefCell<Vec<PathBuf>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    #[test]
    fn in_place_exports_are_flushed_but_special_files_are_not() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("artifact.csv");
        std::fs::write(&file, b"previous, longer content").unwrap();
        SYNCED.with(|synced| synced.borrow_mut().clear());
        write_synced(&file, b"a,b\r\n").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"a,b\r\n");
        #[cfg(unix)]
        write_synced(Path::new("/dev/null"), b"discarded").unwrap();
        assert_eq!(SYNCED.with(|synced| synced.take()), vec![file]);
    }

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
                // The CSVs are the shared design's machine format, so the
                // artifact bytes, their hashes and the layout's names differ
                // from Python's; every other value and table must not.
                assert_eq!(
                    encode(&masked(&result, &command))
                        .replace(&root.path().to_string_lossy().to_string(), "<root>"),
                    encode(&masked(value(action, "expected"), &command)),
                    "{} action {index}",
                    text(case, "name")
                );
                assert_eq!(
                    encode(&masked(&dump(conn), &command)),
                    encode(&masked(value(action, "tables"), &command)),
                    "{} action {index}: database",
                    text(case, "name")
                );
                if let Value::Dict(files) = value(action, "files") {
                    for (name, _) in files {
                        let kassiber_pyjson::Key::Str(name) = name else {
                            panic!()
                        };
                        let name = name.as_str().unwrap();
                        // The Austrian bundle's files are the design's.
                        let path = root.path().join(name);
                        assert!(
                            path.exists()
                                || (name.starts_with("bundle/")
                                    && path.parent().is_some_and(Path::is_dir)),
                            "{} action {index}: {name:?}",
                            text(case, "name")
                        );
                    }
                }
            }
        }
    }

    /// Values the shared design's CSV changes: artifact sizes and hashes,
    /// and the names and counts of its tables and files.
    fn masked(v: &Value, command: &str) -> Value {
        let layout = matches!(
            command,
            "reports.export-csv" | "reports.export-austrian-e1kv-csv"
        );
        match v {
            Value::Dict(fields) => Value::Dict(
                fields
                    .iter()
                    .map(|(k, v)| {
                        let key = match k {
                            kassiber_pyjson::Key::Str(k) => k.as_str().unwrap_or_default(),
                            _ => "",
                        };
                        let hide = matches!(key, "content_sha256" | "bytes")
                            || (layout && matches!(key, "sections" | "sheets" | "files" | "rows"));
                        (
                            k.clone(),
                            if hide {
                                s("<design>")
                            } else {
                                masked(v, command)
                            },
                        )
                    })
                    .collect(),
            ),
            Value::List(items) => Value::List(items.iter().map(|v| masked(v, command)).collect()),
            other => other.clone(),
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

    /// `str(pathlib.Path(text))` under CPython 3.11 for each input.
    #[test]
    fn destinations_format_like_pathlib() {
        for (raw, python) in [
            ("a//b/./c", "a/b/c"),
            ("//a//b", "//a/b"),
            ("///a/b/", "/a/b"),
            ("./a/", "a"),
            (".", "."),
            ("a/../b", "a/../b"),
            ("", "."),
            ("/./", "/"),
            ("x/.", "x"),
            ("~/a//b", "~/a/b"),
        ] {
            assert_eq!(python_path_text(raw), python, "{raw:?}");
        }
    }
    /// The shared report design (docs/reference/report-design.md) renders a
    /// report's layout natively while Python stays the reference for its
    /// data, so a successful export's layout (byte and row counts, section
    /// names, the content hash, and the text and rows that carry it) is left
    /// out of the writer comparison. Where the file lands, which tables
    /// change, the data summaries and every refusal stay exact.
    fn without_layout(value: &Value) -> Value {
        let mut value = value.clone();
        if let Value::Dict(fields) = &mut value {
            fields.retain(|(key, _)| {
                !["bytes", "rows", "sections", "files", "sheets"]
                    .iter()
                    .any(|n| *key == (*n).into())
            });
            for (key, snapshot) in fields.iter_mut() {
                if *key == "report_snapshot".into() {
                    if let Value::Dict(snapshot) = snapshot {
                        snapshot.retain(|(key, _)| *key != "content_sha256".into());
                    }
                }
            }
        }
        value
    }
    /// A bundle's member names are the design's: a refusal while writing
    /// the Austrian bundle names the design's first member (`summary.csv`)
    /// where Python names its first (`00_uebersicht.csv`).
    fn without_member_names(text: String) -> String {
        text.replace("/summary.csv", "/<first-member>")
            .replace("/00_uebersicht.csv", "/<first-member>")
    }
    fn row_counts(value: &Value) -> Value {
        match value {
            Value::Dict(tables) => Value::Dict(
                tables
                    .iter()
                    .map(|(table, rows)| {
                        let count = match rows {
                            Value::List(rows) => rows.len() as i64,
                            _ => -1,
                        };
                        (table.clone(), Value::from(count))
                    })
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    fn text_presence(value: &Value) -> Value {
        match value {
            Value::Dict(pairs) => Value::Dict(
                pairs
                    .iter()
                    .map(|(key, item)| {
                        if *key == "text".into() {
                            (key.clone(), Value::Bool(true))
                        } else {
                            (key.clone(), text_presence(item))
                        }
                    })
                    .collect(),
            ),
            Value::List(items) => Value::List(items.iter().map(|_| Value::Bool(true)).collect()),
            other => other.clone(),
        }
    }

    /// Python's shared report CSV writer at every `_export_csv_rows` export:
    /// parents, special targets, staging, typed failures and registration.
    #[cfg(unix)]
    #[test]
    fn csv_exports_follow_the_python_report_writer() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let vectors = kassiber_pytext::loads(include_str!("../tests/data/csv_write.json")).unwrap();
        let cases = artifact_csv::list(&vectors);
        assert!(cases.len() >= 20);
        // One book, each case in a savepoint rolled back afterwards: creating
        // a book per case only multiplies its syncs on a busy disk.
        let (_data, book) = book();
        let conn = book.connection();
        for case in cases {
            let name = text(case, "name");
            let root = tempfile::tempdir().unwrap();
            conn.execute_batch("SAVEPOINT csv_case").unwrap();
            conn.execute_batch(&text(case, "sql")).unwrap();
            let before = dump(conn);
            let artifacts = root.path().canonicalize().unwrap().join("artifacts");
            std::fs::create_dir(&artifacts).unwrap();
            let target = match text(case, "target").as_str() {
                "new-nested" => artifacts.join("a/b/report.csv"),
                "existing" => {
                    std::fs::write(artifacts.join("existing.csv"), b"old\r\n").unwrap();
                    artifacts.join("existing.csv")
                }
                "final-symlink" => {
                    std::fs::write(artifacts.join("real.csv"), b"old\r\n").unwrap();
                    symlink("real.csv", artifacts.join("link.csv")).unwrap();
                    artifacts.join("link.csv")
                }
                "parent-file" => {
                    std::fs::write(artifacts.join("parent"), "file").unwrap();
                    artifacts.join("parent/missing/report.csv")
                }
                // Raw text, as a CLI argument arrives.
                "unnormalized-parent-file" => {
                    std::fs::write(artifacts.join("parent"), "file").unwrap();
                    PathBuf::from(format!(
                        "{}/parent//missing/./report.csv",
                        artifacts.display()
                    ))
                }
                "trailing-separator-directory" => {
                    std::fs::create_dir(artifacts.join("directory.csv")).unwrap();
                    PathBuf::from(format!("{}/directory.csv/", artifacts.display()))
                }
                "read-only-parent" => {
                    let locked = artifacts.join("locked");
                    std::fs::create_dir(&locked).unwrap();
                    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500))
                        .unwrap();
                    locked.join("missing/report.csv")
                }
                "directory" => {
                    std::fs::create_dir(artifacts.join("directory.csv")).unwrap();
                    artifacts.join("directory.csv")
                }
                "long-name" => artifacts.join(format!("{}.csv", "r".repeat(240))),
                "dev-null" => PathBuf::from("/dev/null"),
                "fifo" => {
                    let fifo = artifacts.join("report.pipe");
                    assert!(std::process::Command::new("mkfifo")
                        .arg(&fifo)
                        .status()
                        .unwrap()
                        .success());
                    fifo
                }
                other => panic!("unknown target {other}"),
            };
            let reader = (text(case, "target") == "fifo").then(|| {
                let fifo = target.clone();
                std::thread::spawn(move || {
                    String::from_utf8_lossy(&std::fs::read(fifo).unwrap()).into_owned()
                })
            });
            let result = match export(
                conn,
                &text(case, "command"),
                value(case, "args"),
                &target,
                &Fixed("00000000-0000-0000-0000-000000000001".into()),
            ) {
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
            if let Some(reader) = &reader {
                // An export that never opened the pipe must not leave the
                // reader blocked: a non-blocking writer releases it.
                use std::os::unix::fs::OpenOptionsExt;
                if !reader.is_finished() {
                    let _ = std::fs::OpenOptions::new()
                        .write(true)
                        .custom_flags(0o4000)
                        .open(&target);
                }
            }
            let received = reader.map(|r| r.join().unwrap());
            if artifacts.join("locked").exists() {
                std::fs::set_permissions(
                    artifacts.join("locked"),
                    std::fs::Permissions::from_mode(0o700),
                )
                .unwrap();
            }
            let root_text = root.path().canonicalize().unwrap();
            let expected = value(case, "expected");
            let succeeded = get(expected, "ok").is_some();
            let shown = |value: &Value| {
                if succeeded {
                    dict([(
                        "ok",
                        without_layout(get(value, "ok").unwrap_or(&Value::Null)),
                    )])
                } else {
                    value.clone()
                }
            };
            assert_eq!(
                without_member_names(
                    encode(&shown(&result))
                        .replace(&root_text.to_string_lossy().to_string(), "<root>")
                ),
                without_member_names(encode(&shown(expected))),
                "{name}"
            );
            let after = dump(conn);
            let changed = match (&before, &after) {
                (Value::Dict(before), Value::Dict(after)) => Value::Dict(
                    after
                        .iter()
                        .filter(|(table, rows)| {
                            before.iter().find(|(t, _)| t == table).map(|(_, r)| r) != Some(rows)
                        })
                        .cloned()
                        .collect(),
                ),
                _ => unreachable!(),
            };
            let (changed, expected_changed) = if succeeded {
                (row_counts(&changed), row_counts(value(case, "changed")))
            } else {
                (changed, value(case, "changed").clone())
            };
            assert_eq!(
                encode(&changed),
                encode(&expected_changed),
                "{name}: tables"
            );
            let received = received.map_or(Value::List(vec![]), |text| Value::List(vec![s(text)]));
            assert_eq!(
                text_presence(&received),
                text_presence(value(case, "received")),
                "{name}: pipe"
            );
            let mut files = BTreeMap::new();
            let mut pending = vec![artifacts.clone()];
            while let Some(directory) = pending.pop() {
                for entry in std::fs::read_dir(&directory).unwrap() {
                    let path = entry.unwrap().path();
                    let key = path
                        .strip_prefix(&artifacts)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    let metadata = std::fs::symlink_metadata(&path).unwrap();
                    let kind =
                        if metadata.file_type().is_symlink() {
                            dict([("symlink", path_text(&std::fs::read_link(&path).unwrap()))])
                        } else if metadata.is_dir() {
                            pending.push(path);
                            dict([("directory", Value::Bool(true))])
                        } else if metadata.is_file() {
                            dict([(
                                "text",
                                s(String::from_utf8_lossy(&std::fs::read(&path).unwrap())
                                    .into_owned()),
                            )])
                        } else {
                            dict([("special", Value::Bool(true))])
                        };
                    files.insert(key, kind);
                }
            }
            let expected_files = match value(case, "files") {
                Value::Dict(files) => files
                    .iter()
                    .map(|(k, v)| {
                        (
                            py_str(&match k {
                                kassiber_pyjson::Key::Str(k) => Value::Str(k.clone()),
                                _ => unreachable!(),
                            }),
                            encode(&text_presence(v)),
                        )
                    })
                    .collect::<BTreeMap<_, _>>(),
                _ => unreachable!(),
            };
            // Nothing staged is left behind, whatever the design names its files.
            assert!(
                files
                    .keys()
                    .all(|key| !key.split('/').any(|part| part.starts_with('.'))),
                "{name}: staged leftovers {:?}",
                files.keys().collect::<Vec<_>>()
            );
            // A successful bundle's member files are the design's; its
            // directory is still compared.
            let bundle = succeeded
                .then(|| get(get(expected, "ok").unwrap(), "dir"))
                .flatten()
                .map(|dir| format!("{}/", py_str(dir).trim_start_matches("<root>/artifacts/")));
            let outside = |key: &String| {
                bundle
                    .as_ref()
                    .is_none_or(|dir| !key.starts_with(dir.as_str()))
            };
            assert_eq!(
                files
                    .into_iter()
                    .filter(|(k, _)| outside(k))
                    .map(|(k, v)| (k, encode(&text_presence(&v))))
                    .collect::<BTreeMap<_, _>>(),
                expected_files
                    .into_iter()
                    .filter(|(k, _)| outside(k))
                    .collect::<BTreeMap<_, _>>(),
                "{name}: artifacts"
            );
            conn.execute_batch("ROLLBACK TO csv_case; RELEASE csv_case")
                .unwrap();
        }
    }
}
