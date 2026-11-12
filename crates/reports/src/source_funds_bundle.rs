//! The source-of-funds bundle (`export_bundle` in Python's source-funds
//! core): the saved case's PDF in the shared report design, the disclosed
//! evidence files exactly as attached, and `manifest.json` with their
//! SHA-256 hashes, in one ZIP. Evidence ships only when the case's reveal
//! mode discloses it (`standard`, `full`).
use super::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The CLI leaf and the desktop kind.
pub const COMMANDS: [&str; 2] = [
    "reports.export-source-funds-bundle",
    "ui.source_funds.export_bundle",
];

const REVEAL_MODES: [&str; 4] = ["labels_only", "minimal", "standard", "full"];

const NOTE: &str = "This bundle contains the Kassiber source-of-funds report and the original evidence files attached to the disclosed sources. Verify each SHA-256 against the report's Disclosure Preview. Kassiber references originals; it does not transcribe or recompute them.";

/// `re.sub(r"[^A-Za-z0-9._-]+", "_", text)`.
fn collapse(text: &str) -> String {
    let mut out = String::new();
    let mut run = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
            run = false;
        } else if !run {
            out.push('_');
            run = true;
        }
    }
    out
}

/// `_bundle_safe_name`: an ASCII file name from the original name or label,
/// keeping its extension (else the stored file's), unique in the bundle.
fn safe_name(label: &str, original: &str, suffix: &str, used: &mut BTreeSet<String>) -> String {
    let base = kassiber_pytext::strip(if !original.is_empty() {
        original
    } else if !label.is_empty() {
        label
    } else {
        "evidence"
    });
    let cleaned = match collapse(base).trim_matches(['.', '_']) {
        "" => "evidence".to_owned(),
        trimmed => trimmed.to_owned(),
    };
    // `rpartition(".")`: no dot leaves the whole name as the extension.
    let (mut stem, mut ext) = match cleaned.rfind('.') {
        Some(i) => (cleaned[..i].to_owned(), cleaned[i + 1..].to_owned()),
        None => (String::new(), cleaned.clone()),
    };
    if !ext.is_empty() && stem.is_empty() {
        stem = cleaned.clone();
        ext = String::new();
    }
    if ext.is_empty() && !suffix.is_empty() {
        ext = suffix.trim_start_matches('.').to_owned();
    }
    let head = if stem.is_empty() { cleaned } else { stem };
    let candidate = if ext.is_empty() {
        head.clone()
    } else {
        format!("{head}.{ext}")
    };
    let mut index = 2;
    let mut name = candidate.clone();
    while used.contains(&name) {
        name = if ext.is_empty() {
            format!("{candidate}-{index}")
        } else {
            format!("{head}-{index}.{ext}")
        };
        index += 1;
    }
    used.insert(name.clone());
    name
}

/// `PurePath.suffix` of the stored file's name.
fn suffix(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match name.rfind('.') {
        Some(i) if i > 0 && i < name.len() - 1 => name[i..].to_owned(),
        _ => String::new(),
    }
}

/// `_resolve_stored_path`: the stored file inside the attachments root, or
/// nothing for an empty, absolute or escaping path.
fn stored(root: &Path, relative: &str) -> Option<PathBuf> {
    let relative = kassiber_pytext::strip(relative);
    if relative.is_empty() || Path::new(relative).is_absolute() {
        return None;
    }
    let root = kassiber_paths::canonicalize_allow_missing(root).ok()?;
    let path = kassiber_paths::canonicalize_allow_missing(&root.join(relative)).ok()?;
    path.starts_with(&root).then_some(path)
}

fn evidence_error(message: &str, code: &str, item: &Value) -> AppError {
    AppError::new(code, message)
        .with_hint(if code == "source_funds_evidence_changed" {
            "Restore the original file, or attach and review replacement evidence in a new case."
        } else {
            "Restore the original attachment before exporting this saved case."
        })
        .with_details(dict([("attachment_id", value(item, "id").clone())]))
}

/// A value, or `default` when Python's `x or default` would take it.
fn or(v: &Value, default: &str) -> String {
    if truthy(v) {
        py_str(v)
    } else {
        default.into()
    }
}

/// Write the bundle for the saved case in `args` to `target` (`.zip` is
/// added when missing) and return Python's result.
pub fn export(
    conn: &Connection,
    args: &Value,
    target: &Path,
    attachments_root: &Path,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let model = source_funds_pdf::admit(conn, args, sources, true)?;
    let report = value(&model, "report");
    let hash = text(&model, "snapshot_hash");
    let reveal = value(report, "reveal_mode");
    let mode = kassiber_pytext::lower(kassiber_pytext::strip(&or(reveal, "standard")));
    if !REVEAL_MODES.contains(&mode.as_str()) {
        return Err(AppError::new(
            "validation",
            format!("Unsupported reveal mode '{}'", py_str(reveal)),
        )
        .with_hint(format!("Choose one of: {}", REVEAL_MODES.join(", "))));
    }
    let include_files = matches!(mode.as_str(), "standard" | "full");

    let target = if target
        .extension()
        .is_some_and(|e| kassiber_pytext::lower(&e.to_string_lossy()) == "zip")
    {
        target.to_owned()
    } else {
        target.with_extension("zip")
    };
    export::create_directory(
        target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;

    let disclosed: Vec<&Value> =
        artifact_csv::list(value(value(report, "disclosure_preview"), "attachments"))
            .iter()
            .filter(|item| matches!(item, Value::Dict(_)))
            .collect();
    let ids: Vec<String> = disclosed
        .iter()
        .filter(|item| truthy(value(item, "id")))
        .map(|item| py_str(value(item, "id")))
        .collect();
    // `resolve_attachment_files`: the disclosed ids' rows and stored files.
    let mut resolved = BTreeMap::new();
    if include_files && !ids.is_empty() {
        std::fs::create_dir_all(attachments_root)
            .map_err(|e| export::io_error(e, Some(attachments_root)))?;
        for id in &ids {
            if resolved.contains_key(id) {
                continue;
            }
            let found = rows(
                conn,
                "SELECT original_filename,stored_relpath,source_url FROM attachments WHERE id=?",
                [id],
            )?;
            if let Some(row) = found.into_iter().next() {
                let path =
                    stored(attachments_root, &text(&row, "stored_relpath")).filter(|p| p.exists());
                resolved.insert(id.clone(), (row, path));
            }
        }
    }

    let (pdf, _) = report_design::pdf(&report_design::source_funds(&model))?;
    let pdf_sha = format!("{:x}", Sha256::digest(&pdf));
    let mut evidence = Vec::new();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut used = BTreeSet::new();
    let (mut file_count, mut url_count, mut withheld) = (0usize, 0usize, 0usize);
    for item in &disclosed {
        let label = or(value(item, "label"), "evidence");
        let kind = or(value(item, "attachment_type"), "");
        let entry = |source: &str| {
            dict([
                ("label", s(label.as_str())),
                ("attachment_type", s(kind.as_str())),
                ("source", s(source)),
                ("sha256", s("")),
            ])
        };
        if !include_files {
            withheld += 1;
            evidence.push(entry("withheld_by_reveal_mode"));
            continue;
        }
        let info = if truthy(value(item, "id")) {
            resolved.get(&py_str(value(item, "id")))
        } else {
            None
        };
        let path = info.and_then(|(_, path)| path.as_ref());
        let expected = kassiber_pytext::lower(&or(value(item, "sha256"), ""));
        if !expected.is_empty() && path.is_none() {
            return Err(evidence_error(
                "A disclosed evidence file is no longer available",
                "source_funds_evidence_unavailable",
                item,
            ));
        }
        if let (Some((row, _)), Some(path)) = (info, path) {
            let name = safe_name(
                &label,
                &text(row, "original_filename"),
                &suffix(path),
                &mut used,
            );
            let bytes = std::fs::read(path).map_err(|_| {
                evidence_error(
                    "A disclosed evidence file could not be read",
                    "source_funds_evidence_unavailable",
                    item,
                )
            })?;
            let sha = format!("{:x}", Sha256::digest(&bytes));
            let well_formed = expected.len() == 64
                && expected
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            if !well_formed || sha != expected {
                return Err(evidence_error(
                    "Evidence bytes do not match the saved source-of-funds case",
                    "source_funds_evidence_changed",
                    item,
                ));
            }
            evidence.push(dict([
                ("label", s(label.as_str())),
                ("attachment_type", s(kind.as_str())),
                ("source", s("file")),
                ("filename", s(format!("evidence/{name}"))),
                ("media_type", s(or(value(item, "media_type"), ""))),
                ("sha256", s(sha)),
            ]));
            files.push((format!("evidence/{name}"), bytes));
            file_count += 1;
        } else if info.is_some_and(|(row, _)| truthy(value(row, "source_url")))
            || truthy(value(item, "source_url"))
        {
            let mut manifest_item = entry("url");
            if mode == "full" {
                replace(
                    &mut manifest_item,
                    "source_url",
                    s(or(value(item, "source_url"), "")),
                );
            }
            evidence.push(manifest_item);
            url_count += 1;
        } else {
            withheld += 1;
            evidence.push(entry("unavailable"));
        }
    }
    let case_id = match value(value(report, "case"), "id") {
        id if truthy(id) => id.clone(),
        _ => value(args, "case").clone(),
    };
    let evidence_total = evidence.len();
    let manifest = dict([
        ("kind", s("kassiber.source_funds.bundle.manifest")),
        ("schema_version", Value::Int(1.into())),
        ("generated_at", value(&model, "generated_at").clone()),
        ("workspace", stored_or_empty(report, "workspace")),
        ("profile", stored_or_empty(report, "profile")),
        ("case_id", case_id.clone()),
        ("snapshot_hash", s(hash.as_str())),
        ("reveal_mode", s(mode.as_str())),
        (
            "report_pdf",
            dict([
                ("filename", s("source-of-funds-report.pdf")),
                ("sha256", s(pdf_sha)),
            ]),
        ),
        ("evidence", Value::List(evidence)),
        ("note", s(NOTE)),
    ]);
    let manifest = kassiber_pyjson::dumps(
        &kassiber_envelope::json_ready(manifest),
        &kassiber_pyjson::Options {
            sort_keys: true,
            indent: Some(kassiber_pyjson::Indent::Spaces(2)),
            ..Default::default()
        },
    )
    .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut members = vec![
        ("source-of-funds-report.pdf".to_owned(), pdf),
        ("manifest.json".to_owned(), manifest.into_bytes()),
    ];
    members.extend(files);
    let bytes = kassiber_xlsx::archive(&members)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    let prepared = artifact_csv::Prepared {
        files: vec![artifact_csv::File {
            name: "source-funds-bundle.zip".into(),
            sheet: String::new(),
            bytes,
            rows: 0,
        }],
        metadata: dict::<&str>([]),
        completion: None,
    };
    // Python stages a sibling file and replaces the target only when the
    // archive is complete.
    let failed = |e: std::io::Error| export::io_error(e, Some(&target));
    let published = artifact_publication::file(&prepared, &target)
        .map_err(|failure| failed(failure.into_io()))?;
    let file = kassiber_paths::canonicalize(&target).map_err(failed)?;
    Ok(dict([
        ("file", s(file.to_string_lossy().into_owned())),
        ("bytes", Value::Int(published.bytes.into())),
        ("format", s("zip")),
        ("scope", s("source_funds")),
        ("snapshot_hash", s(hash)),
        ("reveal_mode", s(mode)),
        ("case_id", case_id),
        (
            "target_transaction_id",
            value(value(report, "target"), "transaction_id").clone(),
        ),
        ("evidence_files", Value::Int(file_count.into())),
        ("evidence_urls", Value::Int(url_count.into())),
        ("evidence_withheld", Value::Int(withheld.into())),
        ("evidence_total", Value::Int(evidence_total.into())),
        ("exportable", Value::Bool(true)),
    ]))
}

/// `report.get(key, "")`: a missing key is empty, a present one as stored.
fn stored_or_empty(report: &Value, key: &str) -> Value {
    get(report, key).cloned().unwrap_or_else(|| s(""))
}

/// The CLI leaf (an expanded `--file`) or the desktop kind (a managed
/// `kassiber-source-funds-bundle-….zip`, plus its `filename`).
pub fn execute(
    conn: &Connection,
    command: &str,
    args: &Value,
    exports_root: &Path,
    attachments_root: &Path,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    if command == "reports.export-source-funds-bundle" {
        let target = PathBuf::from(text(args, "file"));
        return export(conn, args, &target, attachments_root, sources);
    }
    if !matches!(value(args, "case"), Value::Null | Value::Str(_)) {
        return Err(AppError::new(
            "validation",
            "ui.source_funds.export_bundle case must be a string",
        ));
    }
    let target = export_request::managed(
        exports_root,
        "kassiber-source-funds-bundle",
        ".zip",
        &sources.now(conn)?,
    )?;
    let mut args = args.clone();
    replace(&mut args, "workspace", Value::Null);
    replace(&mut args, "profile", Value::Null);
    let mut result = export(conn, &args, &target, attachments_root, sources)?;
    let name = PathBuf::from(text(&result, "file"));
    replace(
        &mut result,
        "filename",
        s(name
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()),
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_store::Book;

    struct Fixed;
    impl kassiber_setup::Sources for Fixed {
        fn now(&self, _: &Connection) -> Result<String> {
            Ok("2026-01-02T03:04:05Z".into())
        }
        fn uuid(&self, _: &Connection) -> Result<String> {
            panic!("the bundle requested an identity")
        }
    }

    #[test]
    fn safe_names_match_python() {
        let mut used = BTreeSet::new();
        let mut name = |label: &str, original: &str, suffix: &str| {
            safe_name(label, original, suffix, &mut used)
        };
        // Python's `_bundle_safe_name` over one shared set.
        assert_eq!(name("  Invoice\n\t Ω  ", "", ".pdf"), "Invoice.pdf");
        assert_eq!(name("  Invoice\n\t Ω  ", "", ".pdf"), "Invoice-2.pdf");
        assert_eq!(name("x", ".hidden", ""), "hidden");
        assert_eq!(name("x", "a.b.c", ""), "a.b.c");
        assert_eq!(name("x", "a.b.c", ""), "a.b-2.c");
        assert_eq!(name("", "", ""), "evidence");
        assert_eq!(name("lbl", "", ""), "lbl");
        assert_eq!(name("lbl", "", ""), "lbl-2");
        assert_eq!(name("x", "__..__", ".txt"), "evidence.txt");
        assert_eq!(name("x", "rep ort (1).PDF", ".pdf"), "rep_ort_1_.PDF");
        assert_eq!(name("x", "noext", ""), "noext");
        assert_eq!(suffix(Path::new("a/secret.pdf")), ".pdf");
        assert_eq!(suffix(Path::new("a/.hidden")), "");
        assert_eq!(suffix(Path::new("a/name.")), "");
    }

    /// The PDF fixtures' saved `case-1`, its evidence `a-a` disclosed with
    /// `sha` under `reveal`, and `evidence` (if any) stored for it.
    fn fixture(sha: &str, reveal: &str, evidence: Option<&[u8]>) -> (tempfile::TempDir, Book) {
        let vectors =
            kassiber_pytext::loads(include_str!("../tests/data/source_funds_pdf.json")).unwrap();
        let case = &artifact_csv::list(value(&vectors, "cases"))[0];
        assert_eq!(text(case, "name"), "standard");
        let root = tempfile::tempdir().unwrap();
        let book = Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        conn.execute_batch(&text(&vectors, "base_sql")).unwrap();
        conn.execute_batch(&text(case, "sql")).unwrap();
        let found = rows(
            conn,
            "SELECT snapshot_json FROM source_funds_cases WHERE id='case-1'",
            [],
        )
        .unwrap();
        let json = text(&found[0], "snapshot_json")
            .replace("\"sha256\": \"hash\"", &format!("\"sha256\": \"{sha}\""))
            .replace(
                "\"reveal_mode\": \"standard\"",
                &format!("\"reveal_mode\": \"{reveal}\""),
            );
        let hash =
            source_funds_pdf::snapshot_hash(&kassiber_pytext::loads(&json).unwrap()).unwrap();
        conn.execute(
            "UPDATE source_funds_cases SET snapshot_json=?1,snapshot_hash=?2 WHERE id='case-1'",
            [&json, &hash],
        )
        .unwrap();
        if let Some(bytes) = evidence {
            let stored = root.path().join("attachments/not-read/secret.pdf");
            std::fs::create_dir_all(stored.parent().unwrap()).unwrap();
            std::fs::write(stored, bytes).unwrap();
        }
        (root, book)
    }

    fn run(root: &Path, book: &Book) -> Result<Value> {
        export(
            book.connection(),
            &dict([("case", s("case-1"))]),
            &root.join("out/bundle"),
            &root.join("attachments"),
            &Fixed,
        )
    }

    #[test]
    fn the_bundle_ships_disclosed_evidence_only_when_its_hash_matches() {
        let evidence = b"%PDF-1.4 invoice evidence\n";
        let sha = format!("{:x}", Sha256::digest(evidence));

        let (root, book) = fixture(&sha, "standard", Some(evidence));
        let result = run(root.path(), &book).unwrap();
        let zip = root.path().join("out/bundle.zip");
        assert_eq!(
            text(&result, "file"),
            zip.canonicalize().unwrap().to_string_lossy()
        );
        for (key, expected) in [
            ("evidence_files", 1),
            ("evidence_urls", 1),
            ("evidence_withheld", 1),
            ("evidence_total", 3),
        ] {
            assert_eq!(value(&result, key), &Value::Int(expected.into()), "{key}");
        }
        let bytes = std::fs::read(&zip).unwrap();
        let member = b"evidence/invoice.pdf";
        assert!(bytes.windows(member.len()).any(|w| w == member));
        // The same case gives the same archive.
        run(root.path(), &book).unwrap();
        assert_eq!(std::fs::read(&zip).unwrap(), bytes);

        let (root, book) = fixture(&sha, "labels_only", Some(evidence));
        let result = run(root.path(), &book).unwrap();
        assert_eq!(value(&result, "evidence_files"), &Value::Int(0.into()));
        assert_eq!(value(&result, "evidence_withheld"), &Value::Int(3.into()));

        let (root, book) = fixture(&sha, "standard", Some(b"replaced"));
        let error = run(root.path(), &book).unwrap_err();
        assert_eq!(error.code, "source_funds_evidence_changed");
        assert!(!root.path().join("out/bundle.zip").exists());

        let (root, book) = fixture(&sha, "standard", None);
        let error = run(root.path(), &book).unwrap_err();
        assert_eq!(error.code, "source_funds_evidence_unavailable");
    }
}
