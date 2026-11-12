//! `export_audit_package` (`ui.reports.export_audit_package`): the evidence
//! summary written as `manifest.json` beside copies of the managed evidence
//! files it names. Nothing is rendered: the package is the auditor's index of
//! what the book holds and what it lacks. URL references stay literal and are
//! never fetched; secret-bearing ones are redacted before they are written.
//! Descriptors, xpubs, credentials and wallet files never reach the package,
//! because the summary does not carry them.
//!
//! Python creates the final directory first and writes into it as it goes.
//! Here the directory is created at the same point (so a refused export
//! leaves the same empty directory and the next managed name is the same),
//! but the package is staged in a hidden sibling, every file and directory
//! synced, and renamed over that empty directory only when complete: a
//! failure never leaves a partial package. Storage failures answer
//! `export_write_failed`.
use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use bitcoin_hashes::{sha256, Hash, HashEngine};
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_store::rusqlite::Connection;

use crate::evidence_summary::{
    self, attachment_rows_by_ids, or_empty, sha256_hex, text, url_safety_full, warning, Options,
};
use crate::report_blockers::val;

type Result<T> = std::result::Result<T, AppError>;

pub const KIND: &str = "ui.reports.export_audit_package";
const SCHEMA_VERSION: i64 = 2;
const MANIFEST: &str = "manifest.json";

/// The desktop kind: a managed `kassiber-audit-package-…` directory under
/// `exports_root`, then `_audit_package_options(args)`, as Python orders them
/// (so a refused request still creates the reports directory).
pub fn execute(
    conn: &Connection,
    args: &Value,
    exports_root: &Path,
    attachments_root: &Path,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let directory = kassiber_reports::export_request::managed(
        exports_root,
        "kassiber-audit-package",
        "",
        &sources.now(conn)?,
    )?;
    let options = evidence_summary::options(args)?;
    // `build_evidence_summary` creates the attachments root it reads.
    fs::create_dir_all(attachments_root)
        .map_err(|e| kassiber_reports::export::io_error(e, Some(attachments_root)))?;
    export(conn, &directory, attachments_root, &options, &|| {
        sources.now(conn)
    })
}

/// `export_audit_package(conn, data_root, None, None, directory, hooks,
/// **options)`; `now` is `hooks.now_iso`.
pub fn export(
    conn: &Connection,
    directory: &Path,
    attachments_root: &Path,
    options: &Options,
    now: &dyn Fn() -> Result<String>,
) -> Result<Value> {
    match fs::read_dir(directory).map(|mut entries| entries.next().is_some()) {
        Ok(true) => {
            return Err(AppError::new(
                "conflict",
                "Audit package export directory already exists and is not empty",
            ))
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(write_failed(error, directory)),
    }
    fs::create_dir_all(directory).map_err(|e| write_failed(e, directory))?;
    let summary = evidence_summary::summary(conn, attachments_root, options)?;
    let profile = text(&val(&summary, "profile"), "id");
    let ids: Vec<String> = attachment_ids(&summary, options.include_review_state)
        .into_iter()
        .collect();
    let rows = attachment_rows_by_ids(conn, &profile, &ids)?;
    let staging = Staging::create(directory)?;
    let built = (|| {
        let mut package_warnings = Vec::new();
        let mut evidence_files = Vec::new();
        if options.include_copied_attachments {
            let (copied, warnings) = copy_evidence(&rows, attachments_root, &staging.path)?;
            evidence_files = copied;
            package_warnings.extend(warnings);
        } else if rows
            .iter()
            .any(|row| text(row, "attachment_type") == "file")
        {
            package_warnings.push(warning(
                "copied_attachments_excluded",
                "info",
                "Managed copied evidence files exist but were excluded by export options.",
                None,
            ));
        }
        let (references, warnings) = url_references(&rows, options.include_url_references)?;
        package_warnings.extend(warnings);
        let totals = val(&summary, "summary");
        if !options.include_edit_history && truthy(&val(&totals, "edit_history_event_count")) {
            package_warnings.push(warning(
                "edit_history_excluded",
                "info",
                "Transaction edit history exists but was excluded by export options.",
                None,
            ));
        }
        let presumed = val(&val(&summary, "custody_quantity"), "presumed_external");
        let slices = val(&presumed, "slice_count");
        if truthy(&presumed) && truthy(&slices) {
            package_warnings.push(warning(
                "external_custody_presumed",
                "warning",
                format!(
                    "{} unmatched custody slice(s) are treated as presumed external. This is disclosed but does not block reports.",
                    py_str(&slices)
                ),
                None,
            ));
        }
        package_warnings.push(warning(
            "sensitive_material_excluded",
            "info",
            "Descriptors, xpubs, backend URLs, credentials, wallet files, logs, AI settings, unrelated books, and raw technical wallet evidence were excluded; authored evidence is represented by hashes only.",
            None,
        ));
        let mut manifest = if options.include_url_references {
            summary.clone()
        } else {
            without_url_attachments(&summary)
        };
        let reference_count = references.len();
        let evidence_count = evidence_files.len();
        set_key(
            &mut manifest,
            "package",
            dict([
                ("schema_version", SCHEMA_VERSION.into()),
                ("generated_at", s(now()?)),
                (
                    "options",
                    dict([
                        (
                            "include_copied_attachments",
                            Value::Bool(options.include_copied_attachments),
                        ),
                        (
                            "include_url_references",
                            Value::Bool(options.include_url_references),
                        ),
                        (
                            "include_journal_state",
                            Value::Bool(options.include_journal_state),
                        ),
                        (
                            "include_review_state",
                            Value::Bool(options.include_review_state),
                        ),
                        (
                            "include_edit_history",
                            Value::Bool(options.include_edit_history),
                        ),
                    ]),
                ),
                ("evidence_files", Value::List(evidence_files)),
                ("url_references", Value::List(references)),
                ("warnings", Value::List(package_warnings.clone())),
            ]),
        );
        let mut bytes = kassiber_pyjson::dumps(
            &kassiber_envelope::json_ready(manifest),
            &kassiber_pyjson::Options {
                sort_keys: true,
                indent: Some(kassiber_pyjson::Indent::Spaces(2)),
                ..Default::default()
            },
        )
        .map_err(|e| AppError::new("internal_error", e.to_string()))?
        .into_bytes();
        bytes.push(b'\n');
        let manifest_path = staging.path.join(MANIFEST);
        write_synced(&manifest_path, &bytes).map_err(|e| write_failed(e, directory))?;
        Ok((package_warnings, evidence_count, reference_count))
    })();
    let (package_warnings, evidence_count, reference_count) = match built {
        Ok(built) => built,
        Err(error) => {
            staging.discard();
            return Err(error);
        }
    };
    staging.publish(directory)?;
    let resolved = directory
        .canonicalize()
        .map_err(|e| write_failed(e, directory))?;
    let totals = val(&summary, "summary");
    Ok(dict([
        ("dir", path_text(&resolved)),
        ("manifest", path_text(&resolved.join(MANIFEST))),
        ("format", s("directory")),
        ("scope", s("audit_package")),
        (
            "filename",
            s(directory
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()),
        ),
        ("transaction_count", val(&totals, "transaction_count")),
        ("ready_count", val(&totals, "ready_count")),
        ("blocked_count", val(&totals, "blocked_count")),
        ("evidence_file_count", evidence_count.into()),
        ("url_reference_count", reference_count.into()),
        ("warnings", Value::List(package_warnings)),
    ]))
}

fn path_text(path: &Path) -> Value {
    s(path.to_string_lossy().into_owned())
}

/// `{**value, key: item}`: replaces in place or appends.
fn set_key(value: &mut Value, key: &str, item: Value) {
    if let Value::Dict(pairs) = value {
        match pairs.iter_mut().find(|(k, _)| *k == Key::from(key)) {
            Some((_, slot)) => *slot = item,
            None => pairs.push((key.into(), item)),
        }
    }
}

fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        _ => &[],
    }
}

/// `_collect_attachment_ids(summary, include_review_state=...)`: direct
/// evidence, and the evidence of reviewed links and their root sources.
fn attachment_ids(summary: &Value, include_review_state: bool) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for item in list(&val(summary, "transactions")) {
        for attachment in list(&val(item, "direct_attachments")) {
            ids.insert(py_str(&val(attachment, "id")));
        }
        if !include_review_state {
            continue;
        }
        for link in list(&val(item, "source_funds_links")) {
            if text(link, "state") != "reviewed" {
                continue;
            }
            for attachment in list(&val(link, "attachments")) {
                ids.insert(py_str(&val(attachment, "id")));
            }
            let source = val(link, "from_source");
            for attachment in list(&val(&source, "attachments")) {
                ids.insert(py_str(&val(attachment, "id")));
            }
        }
    }
    ids
}

/// `_without_url_attachment_summaries(summary)`.
fn without_url_attachments(summary: &Value) -> Value {
    fn files_only(value: &Value) -> Value {
        Value::List(
            list(value)
                .iter()
                .filter(|item| text(item, "attachment_type") != "url")
                .cloned()
                .collect(),
        )
    }
    let mut scrubbed = summary.clone();
    let transactions = list(&val(summary, "transactions"))
        .iter()
        .map(|item| {
            let mut item = item.clone();
            let direct = files_only(&val(&item, "direct_attachments"));
            set_key(&mut item, "direct_attachments", direct);
            let links = list(&val(&item, "source_funds_links"))
                .iter()
                .map(|link| {
                    let mut link = link.clone();
                    let attachments = files_only(&val(&link, "attachments"));
                    set_key(&mut link, "attachments", attachments);
                    if let Some(source @ Value::Dict(_)) = get(&link, "from_source").cloned() {
                        let mut source = source;
                        let attachments = files_only(&val(&source, "attachments"));
                        set_key(&mut source, "attachments", attachments);
                        set_key(&mut link, "from_source", source);
                    }
                    link
                })
                .collect();
            set_key(&mut item, "source_funds_links", Value::List(links));
            item
        })
        .collect();
    set_key(&mut scrubbed, "transactions", Value::List(transactions));
    scrubbed
}

// ------------------------------------------------------------ evidence

/// `_copy_evidence_files(rows, attachments_root, output_dir)`: every managed
/// file whose stored hash still matches, under a collision-resistant name.
fn copy_evidence(
    rows: &[Value],
    attachments_root: &Path,
    output: &Path,
) -> Result<(Vec<Value>, Vec<Value>)> {
    let mut included = Vec::new();
    let mut warnings = Vec::new();
    for row in rows {
        if text(row, "attachment_type") != "file" {
            continue;
        }
        let id = py_str(&val(row, "id"));
        let (stored, valid) = kassiber_transactions::attachments::stored_path(
            attachments_root,
            &text(row, "stored_relpath"),
        );
        let Some(stored) = stored.filter(|_| valid) else {
            warnings.push(warning(
                "attachment_storage_path_invalid",
                "blocker",
                format!("Attachment {id} has an invalid managed storage path."),
                None,
            ));
            continue;
        };
        if !stored.exists() {
            warnings.push(warning(
                "attachment_file_missing",
                "blocker",
                format!("Attachment {id} points to a missing managed file."),
                None,
            ));
            continue;
        }
        let (size, digest) = hash_file(&stored).map_err(|e| write_failed(e, output))?;
        let expected = val(row, "sha256");
        if truthy(&expected) && digest != py_str(&expected) {
            warnings.push(warning(
                "attachment_hash_mismatch",
                "blocker",
                format!("Attachment {id} hash does not match the copied file."),
                None,
            ));
            continue;
        }
        let label = kassiber_transactions::attachments::display_label(row);
        let original = val(row, "original_filename");
        let name = safe_filename(&if truthy(&original) {
            py_str(&original)
        } else {
            label.clone()
        });
        let relative = format!(
            "evidence/{}-{}-{}",
            safe_component(&id, "attachment"),
            &sha256_hex(&id)[..12],
            name
        );
        copy_verified(&stored, &output.join(&relative), &digest)
            .map_err(|e| write_failed(e, output))?;
        included.push(dict([
            ("attachment_id", val(row, "id")),
            ("label", s(label)),
            ("path", s(relative)),
            ("sha256", s(digest)),
            ("size_bytes", Value::Int(size.into())),
            ("media_type", or_empty(row, "media_type")),
            (
                "copied_from_attachment_id",
                or_empty(row, "copied_from_attachment_id"),
            ),
            (
                "copied_from_transaction_id",
                or_empty(row, "copied_from_transaction_id"),
            ),
        ]));
    }
    included.sort_by_key(|item| py_str(&val(item, "attachment_id")));
    if !included.is_empty() {
        sync_directory(&output.join("evidence")).map_err(|e| write_failed(e, output))?;
    }
    Ok((included, warnings))
}

/// `_hash_file(path)`: size and SHA-256 hex digest.
fn hash_file(path: &Path) -> std::io::Result<(u64, String)> {
    let mut file = fs::File::open(path)?;
    let mut engine = sha256::Hash::engine();
    let mut buffer = vec![0; 1 << 20];
    let mut size = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        engine.input(&buffer[..read]);
        size += read as u64;
    }
    Ok((size, sha256::Hash::from_engine(engine).to_string()))
}

/// `shutil.copy2(source, destination)`: contents, permission bits and
/// access/modification times. The copy is hashed as it is written and must
/// match the digest the manifest records, so a file replaced between the
/// check and the copy fails the export instead of shipping other bytes.
fn copy_verified(source: &Path, destination: &Path, digest: &str) -> std::io::Result<()> {
    if let Some(parent) = destination.parent() {
        match fs::create_dir(parent) {
            Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => return Err(error),
            _ => {}
        }
    }
    let mut input = fs::File::open(source)?;
    let mut output = fs::File::create(destination)?;
    let mut engine = sha256::Hash::engine();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        engine.input(&buffer[..read]);
        output.write_all(&buffer[..read])?;
    }
    if sha256::Hash::from_engine(engine).to_string() != digest {
        return Err(std::io::Error::other(
            "the evidence file changed while it was copied",
        ));
    }
    let metadata = input.metadata()?;
    output.set_permissions(metadata.permissions())?;
    let mut times = fs::FileTimes::new().set_modified(metadata.modified()?);
    if let Ok(accessed) = metadata.accessed() {
        times = times.set_accessed(accessed);
    }
    output.set_times(times)?;
    output.sync_all()
}

/// `_safe_filename(value)`: the last path component, cleaned.
fn safe_filename(value: &str) -> String {
    // `Path(value).name`: POSIX components without empty or `.` parts.
    let name = value
        .split('/')
        .rfind(|part| !part.is_empty() && *part != ".")
        .unwrap_or_default();
    let name = kassiber_pytext::strip(name);
    safe_component(
        if name.is_empty() {
            "evidence.bin"
        } else {
            name
        },
        "evidence.bin",
    )
}

/// `_safe_path_component(value, fallback=...)`: CPython `str.isalnum`
/// characters and `._-` kept, everything else `_`, then `._` trimmed.
fn safe_component(value: &str, fallback: &str) -> String {
    let raw = kassiber_pytext::strip(value);
    let raw = if raw.is_empty() { fallback } else { raw };
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if kassiber_custody::python_text::isalnum(c) || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(|c| c == '.' || c == '_');
    if cleaned.is_empty() {
        fallback.to_owned()
    } else {
        cleaned.to_owned()
    }
}

/// `_url_references(rows, include_url_references=...)`.
fn url_references(rows: &[Value], include: bool) -> Result<(Vec<Value>, Vec<Value>)> {
    let urls: Vec<&Value> = rows
        .iter()
        .filter(|row| text(row, "attachment_type") == "url")
        .collect();
    if !include {
        let warnings = if urls.is_empty() {
            Vec::new()
        } else {
            vec![warning(
                "url_references_excluded",
                "info",
                "URL references exist but were excluded by export options.",
                None,
            )]
        };
        return Ok((Vec::new(), warnings));
    }
    let mut references = Vec::new();
    let mut warnings = Vec::new();
    for row in urls {
        let safety = url_safety_full(&text(row, "source_url"))?;
        let mut item = dict([
            ("attachment_id", val(row, "id")),
            (
                "label",
                s(kassiber_transactions::attachments::display_label(row)),
            ),
            ("host", s(safety.host)),
            ("scheme", s(safety.scheme)),
            ("redacted", Value::Bool(safety.redacted)),
            (
                "copied_from_attachment_id",
                or_empty(row, "copied_from_attachment_id"),
            ),
            (
                "copied_from_transaction_id",
                or_empty(row, "copied_from_transaction_id"),
            ),
            ("url", s(safety.url)),
            (
                "redacted_url",
                s(if safety.redacted {
                    safety.redacted_url
                } else {
                    String::new()
                }),
            ),
        ]);
        if safety.redacted {
            set_key(&mut item, "redaction_reason", s(safety.reason.as_str()));
            warnings.push(warning(
                "secret_bearing_url_redacted",
                "warning",
                format!(
                    "URL reference {} was redacted because it contains {}.",
                    py_str(&val(row, "id")),
                    safety.reason
                ),
                None,
            ));
        }
        references.push(item);
    }
    references.sort_by_key(|item| py_str(&val(item, "attachment_id")));
    Ok((references, warnings))
}

// --------------------------------------------------------- publication

/// The hidden sibling the package is assembled in.
struct Staging {
    path: PathBuf,
}

impl Staging {
    fn create(directory: &Path) -> Result<Staging> {
        let parent = parent(directory);
        let name = directory
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut attempt = 1;
        loop {
            let suffix = if attempt == 1 {
                String::new()
            } else {
                format!("-{attempt}")
            };
            let path = parent.join(format!(".{name}.staging{suffix}"));
            match fs::create_dir_all(parent).and_then(|()| fs::create_dir(&path)) {
                Ok(()) => return Ok(Staging { path }),
                Err(error)
                    if error.kind() == std::io::ErrorKind::AlreadyExists && attempt < 100 =>
                {
                    attempt += 1
                }
                Err(error) => return Err(write_failed(error, directory)),
            }
        }
    }

    /// Rename the complete package over the empty directory `export`
    /// created; anything written there since is a conflict.
    fn publish(self, directory: &Path) -> Result<()> {
        let result = (|| {
            sync_directory(&self.path)?;
            match fs::remove_dir(directory) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
            fs::rename(&self.path, directory)?;
            sync_directory(parent(directory))
        })();
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                let conflict = directory.exists();
                self.discard();
                Err(if conflict {
                    AppError::new(
                        "conflict",
                        "Audit package export directory already exists and is not empty",
                    )
                } else {
                    write_failed(error, directory)
                })
            }
        }
    }

    /// Remove the staged tree; only ever the directory `create` made.
    fn discard(self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(not(windows))]
    fs::File::open(path)?.sync_all()?;
    #[cfg(windows)]
    let _ = path;
    Ok(())
}

/// A storage failure while staging or publishing the package: the
/// destination directory and errno, as the other exports report them.
fn write_failed(error: std::io::Error, destination: &Path) -> AppError {
    let destination = destination.to_string_lossy().into_owned();
    let reason = match error.raw_os_error() {
        Some(number) => {
            let text = error.to_string();
            text.strip_suffix(&format!(" (os error {number})"))
                .unwrap_or(&text)
                .to_owned()
        }
        None => error.to_string(),
    };
    AppError::new(
        "export_write_failed",
        format!("Could not write audit package '{destination}': {reason}"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_names_follow_python_path_and_isalnum_rules() {
        assert_eq!(safe_filename("dir/sub/Grüße 2024.pdf"), "Grüße_2024.pdf");
        assert_eq!(safe_filename("a/."), "a");
        assert_eq!(safe_filename("a/b/"), "b");
        assert_eq!(safe_filename(""), "evidence.bin");
        assert_eq!(safe_filename("  "), "evidence.bin");
        assert_eq!(safe_filename(".."), "evidence.bin");
        assert_eq!(safe_filename("._x_."), "x");
        assert_eq!(safe_filename("..\\evil"), "evil");
        // Combining marks are never alphanumeric in CPython; U+093E and
        // U+0345 are `Alphabetic` (so `char::is_alphanumeric`) in Rust.
        assert_eq!(safe_filename("a\u{0301}b"), "a_b");
        assert_eq!(safe_filename("a\u{093E}b"), "a_b");
        assert_eq!(safe_filename("a\u{0345}b"), "a_b");
        assert_eq!(safe_component("", "attachment"), "attachment");
        assert_eq!(safe_component("../att 1", "attachment"), "att_1");
    }
}
