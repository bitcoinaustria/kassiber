//! A reviewed document supplies observations and managed evidence only.
use super::*;
use std::path::Path;

pub struct ImportDraft<'a> {
    pub source_file: &'a str,
    pub data_root: &'a Path,
    pub profile_id: &'a str,
    pub wallet: &'a str,
    pub rows: &'a [Py],
    pub include_quarantined: bool,
    pub selected_row_ids: Option<&'a [String]>,
    pub expected_source_sha256: Option<&'a str>,
    pub confidence_threshold: &'a Py,
    pub attach_evidence: bool,
    /// Refuse what attaching would refuse (the evidence budget) without
    /// copying the document: a chat plan simulates the apply's import.
    pub check_evidence_budget: bool,
}

/// Rebuild every record from the visible reviewed fields. A hidden import_record
/// and ready/status flags can never bypass the structural or confidence gates.
pub fn reviewed_records(
    rows: &[Py],
    include_quarantined: bool,
    selected_row_ids: Option<&[String]>,
    source_hash: &str,
    currency: &str,
    confidence_threshold: &Py,
) -> Result<(Vec<Py>, usize), AppError> {
    if rows.len() > 500 {
        return Err(
            AppError::new("validation", "Document draft contains too many rows").with_details(
                dict([
                    ("rows", Py::from(rows.len() as i64)),
                    ("max_rows", Py::from(500)),
                ]),
            ),
        );
    }
    let threshold = threshold(confidence_threshold)?;
    let mut records = Vec::new();
    let mut skipped = 0;
    for row in rows {
        let id = rendered(field(row, "id"));
        if selected_row_ids
            .is_some_and(|selected| !selected.iter().any(|s| s == &id && !s.is_empty()))
        {
            continue;
        }
        let flags = confidence_flags(row, &threshold);
        if !flags.is_empty() && !include_quarantined {
            skipped += 1;
            continue;
        }
        let Some(mut record) = reviewed_record(row, source_hash, currency) else {
            skipped += 1;
            continue;
        };
        if !flags.is_empty() {
            let mut raw = field(&record, "raw_json").clone();
            set(&mut raw, "confidence_override", Py::Bool(true));
            set(
                &mut raw,
                "review_flags",
                Py::List(flags.into_iter().map(s).collect()),
            );
            set(&mut record, "raw_json", raw);
        }
        records.push(record);
    }
    Ok((records, skipped))
}

/// The caller resolves scope and supplies reviewed rows; this function performs
/// no provider lookup or networking. Transaction writes and evidence copies share
/// one savepoint. It also composes under an existing caller transaction.
pub fn import_draft(
    conn: &Connection,
    request: ImportDraft<'_>,
    identity: Option<&Identity>,
) -> Result<Py, AppError> {
    let source = files::source_path(request.source_file)?;
    let snapshot = files::Snapshot::new(&source)?;
    let source_hash = files::hash(&snapshot.path)?;
    if let Some(expected) = request.expected_source_sha256 {
        let expected = kassiber_pytext::lower(kassiber_pytext::strip(expected));
        if expected.len() != 64
            || !expected
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(AppError::new(
                "validation",
                "Preview source hash is invalid",
            ));
        }
        if source_hash != expected {
            return Err(AppError::new(
                "document_import_source_changed",
                "The OCR source changed after preview",
            )
            .with_hint("Preview the document again before importing its rows.")
            .with_details(dict([(
                "filename",
                s(source.file_name().unwrap_or_default().to_string_lossy()),
            )])));
        }
    }
    let currency: String = conn
        .query_row(
            "SELECT fiat_currency FROM profiles WHERE id=?",
            [request.profile_id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let (records, skipped) = reviewed_records(
        request.rows,
        request.include_quarantined,
        request.selected_row_ids,
        &source_hash,
        &currency,
        request.confidence_threshold,
    )?;
    if records.is_empty() {
        return Err(AppError::new(
            "document_import_no_ready_rows",
            "Document draft has no importable rows",
        )
        .with_hint("Review the OCR draft or retry with a clearer image/local vision model.")
        .with_details(dict([("quarantined_skipped", Py::from(skipped as i64))])));
    }
    let source_bytes = std::fs::metadata(&snapshot.path).map_err(files::io)?.len();
    let projected = source_bytes.saturating_mul(records.len() as u64);
    if (request.attach_evidence || request.check_evidence_budget) && projected > 512 * 1024 * 1024 {
        return Err(AppError::new(
            "document_import_evidence_budget_exceeded",
            "Document evidence copies would exceed the import storage budget",
        )
        .with_hint("Import fewer rows at a time or use a smaller source image/PDF.")
        .with_details(dict([
            ("rows", Py::from(records.len() as i64)),
            ("source_bytes", Py::from(source_bytes as i64)),
            ("projected_bytes", Py::from(projected as i64)),
            ("max_projected_bytes", Py::from(512 * 1024 * 1024)),
        ])));
    }
    let rows = records
        .iter()
        .map(|record| csv::Row {
            headers: vec![],
            data: generic_json::projection(record),
            rest: None,
            original: Some(record.clone()),
        })
        .collect::<Vec<_>>();
    let records = rows.iter().map(normalize).collect::<Result<Vec<_>, _>>()?;
    let wallet = resolve(conn, request.profile_id, request.wallet)?;
    let mut ids = Ids { identity, at: 0 };
    let import_request = Request {
        profile_id: request.profile_id.into(),
        wallet: request.wallet.into(),
        file: request.source_file.into(),
        format: Format::Document,
        mode: None,
    };
    let paths =
        kassiber_paths::Context::from_env().map_err(|e| AppError::new(e.code, e.message))?;
    let data_root = paths
        .effective_data_root(request.data_root)
        .map_err(|e| AppError::new(e.code, e.message))?;
    let attachments_root = paths
        .state_root_for_data(&data_root)
        .map_err(|e| AppError::new(e.code, e.message))?
        .join("attachments");
    std::fs::create_dir_all(&attachments_root).map_err(files::io)?;
    let mut copied = Vec::new();
    kassiber_store::begin_write_savepoint(conn, "document_import").map_err(database_error)?;
    let outcome = (|| {
        let mut outcome = apply(
            conn,
            &import_request,
            &wallet,
            &rows,
            &records,
            &mut ids,
            None,
        )?;
        let mut attached = Vec::new();
        if request.attach_evidence {
            let changed =
                ["inserted_records", "updated_records"]
                    .iter()
                    .flat_map(|key| match field(&outcome, key) {
                        Py::List(rows) => rows.clone(),
                        _ => vec![],
                    });
            for changed in changed {
                let tx = rendered(field(&changed, "transaction_id"));
                if tx.is_empty() {
                    continue;
                }
                // Python's attachment hook resolves the active context, even
                // when the imported wallet was selected explicitly. Retain
                // that refusal boundary until the reference is corrected.
                let (workspace, profile) = kassiber_setup::references::scope(conn, &Py::Null)?;
                let workspace_id = rendered(field(&workspace, "id"));
                let profile_id = rendered(field(&profile, "id"));
                let scope = kassiber_transactions::Scope {
                    workspace_id: &workspace_id,
                    profile_id: &profile_id,
                };
                let attachment = kassiber_transactions::attachments::execute(
                    conn,
                    Some(&scope),
                    &attachments_root,
                    &kassiber_transactions::attachments::Command::Add {
                        transaction: tx.clone(),
                        file: Some(snapshot.path.to_string_lossy().into_owned()),
                        url: None,
                        label: Some(format!(
                            "OCR source: {}",
                            source.file_name().unwrap_or_default().to_string_lossy()
                        )),
                        media_type: Some(files::mime(&snapshot.path).into()),
                    },
                )?;
                // Record the path before verification so even a verification
                // failure cleans up the just-created file.
                let relative = rendered(field(&attachment, "stored_relpath"));
                if let Some(path) = files::managed_path(&attachments_root, &relative) {
                    copied.push(path);
                }
                if field(&attachment, "sha256") != &s(&source_hash) {
                    return Err(AppError::new(
                        "document_import_source_changed",
                        "Managed OCR evidence did not match the reviewed source bytes",
                    ));
                }
                attached.push(dict([
                    ("transaction_id", s(tx)),
                    ("attachment_id", field(&attachment, "id").clone()),
                    ("stored_relpath", s(relative)),
                ]));
            }
        }
        set(
            &mut outcome,
            "source",
            dict([
                ("path", s(source.to_string_lossy())),
                (
                    "filename",
                    s(source.file_name().unwrap_or_default().to_string_lossy()),
                ),
                ("sha256", s(&source_hash)),
            ]),
        );
        set(&mut outcome, "source_format", s("document_import"));
        set(
            &mut outcome,
            "draft_rows_imported",
            Py::from(records.len() as i64),
        );
        set(
            &mut outcome,
            "quarantined_skipped",
            Py::from(skipped as i64),
        );
        set(&mut outcome, "attached_evidence", Py::List(attached));
        conn.execute_batch("RELEASE SAVEPOINT document_import")
            .map_err(database_error)?;
        Ok(outcome)
    })();
    if outcome.is_err() {
        let _ = conn.execute_batch(
            "ROLLBACK TO SAVEPOINT document_import; RELEASE SAVEPOINT document_import",
        );
        for path in copied {
            let _ = std::fs::remove_file(path);
        }
    }
    outcome
}
