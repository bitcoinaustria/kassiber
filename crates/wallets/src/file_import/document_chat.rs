//! `core/document_chat_import.py`: plan and apply importing a chat document
//! draft's ready rows, bound to its plan.
//!
//! A photo or PDF the user attached to a chat is read by the local vision
//! model into a draft of rows, each `ready` or held back for review. This
//! answers, without changing the book, what importing the ready rows would
//! do, and applies exactly that after the user's consent:
//!
//! - Only `ready` rows are imported; the rest stay for the desktop's document
//!   import, where the user reviews them against the page.
//! - The plan is bound to the draft's rows, the document's SHA-256, the
//!   book's input version and a digest of the effect.
//! - Targets follow the spreadsheet import. The document is attached to each
//!   imported row as evidence and the import is recorded as an import run;
//!   because a rollback would also delete that evidence, undoing it is the
//!   desktop's, not the assistant's.
use super::attachment::{
    batch_effects, blocker, book_changed, create_target_wallet, digest, input_version,
    normalize_target, resolve_target, same_int, Identities, Target,
};
use super::document::{import_draft, ImportDraft};
use crate::database_error;
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::{Key, Value as Py};
use kassiber_store::rusqlite::{params, Connection};

pub const PLAN_SCHEMA_VERSION: i64 = 1;
/// `DOCUMENT_IMPORT_FORMAT`.
pub const SOURCE_FORMAT: &str = "document_import";
const SAVEPOINT: &str = "document_chat_import_plan";
const APPLY_SAVEPOINT: &str = "document_chat_import_apply";
pub const PLAN_TOOL: &str = "ui_wallets_import_document_plan";
const NO_SIMULATION: [&str; 3] = [
    "nothing_ready",
    "target_not_file_wallet",
    "wallet_label_taken",
];

/// One plan or apply request over a stored draft.
pub struct DocumentChatImport<'a> {
    pub profile_id: &'a str,
    pub data_root: &'a std::path::Path,
    /// The grant's staged document.
    pub source_file: &'a str,
    /// The draft the reader stored for the grant.
    pub draft: &'a Py,
    /// `wallet` or `new_wallet`.
    pub request: &'a Py,
    /// `plan_digest`, `expected_source_sha256`, `expected_input_version`.
    pub expected: Option<&'a Py>,
}

fn rows(draft: &Py) -> Vec<Py> {
    match get(draft, "rows") {
        Some(Py::List(rows)) => rows.clone(),
        _ => Vec::new(),
    }
}

/// `ready_row_ids(draft)`.
pub fn ready_row_ids(draft: &Py) -> Vec<String> {
    rows(draft)
        .iter()
        .filter(|row| {
            matches!(row, Py::Dict(_))
                && get(row, "status") == Some(&s("ready"))
                && get(row, "id").is_some_and(truthy)
        })
        .map(|row| py_str(get(row, "id").expect("ready id")))
        .collect()
}

/// `_draft_digest(draft)`: everything the import reads from the draft.
fn draft_digest(draft: &Py) -> Result<String, AppError> {
    digest(&dict([
        (
            "rows",
            match get(draft, "rows") {
                Some(rows) if truthy(rows) => rows.clone(),
                _ => Py::List(vec![]),
            },
        ),
        (
            "confidence_threshold",
            get(draft, "confidence_threshold")
                .cloned()
                .unwrap_or(Py::Null),
        ),
    ]))
}

fn mapping<'a>(draft: &'a Py, key: &str) -> Option<&'a Py> {
    get(draft, key).filter(|value| matches!(value, Py::Dict(_)))
}

fn int(value: Option<&Py>) -> i64 {
    match value {
        Some(Py::Int(number)) => i64::try_from(number.clone()).unwrap_or(0),
        Some(Py::Bool(flag)) => i64::from(*flag),
        _ => 0,
    }
}

/// `str(value or "")`.
fn text_or_empty(value: Option<&Py>) -> String {
    match value {
        Some(value) if truthy(value) => py_str(value),
        _ => String::new(),
    }
}

/// `record_batch(conn, profile, wallet_id=..., source_format=...,
/// source_filename=..., column_map=None, outcome=...)`: the run's id, or
/// `None` when it created nothing.
fn record_batch(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    source_filename: &str,
    outcome: &Py,
) -> Result<Option<String>, AppError> {
    let inserted: Vec<String> = match get(outcome, "inserted_records") {
        Some(Py::List(records)) => records
            .iter()
            .filter_map(|record| get(record, "transaction_id").filter(|id| truthy(id)))
            .map(py_str)
            .collect(),
        _ => Vec::new(),
    };
    if inserted.is_empty() {
        return Ok(None);
    }
    let batch = crate::lifecycle::uuid(conn)?;
    let imported_at = crate::lifecycle::now(conn)?;
    let workspace_id: String = conn
        .query_row(
            "SELECT workspace_id FROM profiles WHERE id = ?",
            [profile_id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    conn.execute(
        "INSERT INTO import_batches(id, workspace_id, profile_id, wallet_id, source_format, \
         source_filename, column_map_json, imported_at, rows_inserted, rows_updated, rows_skipped) \
         VALUES(?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, ?)",
        params![
            batch,
            workspace_id,
            profile_id,
            wallet_id,
            SOURCE_FORMAT,
            source_filename,
            imported_at,
            inserted.len() as i64,
            int(get(outcome, "updated")),
            int(get(outcome, "skipped")),
        ],
    )
    .map_err(database_error)?;
    for (position, id) in (1_i64..).zip(&inserted) {
        conn.execute(
            "INSERT OR IGNORE INTO import_batch_transactions(batch_id, transaction_id, source_position) VALUES(?, ?, ?)",
            params![batch, id, position],
        )
        .map_err(database_error)?;
    }
    Ok(Some(batch))
}

/// `_import(...)`: the draft's ready rows and the run that records them; the
/// caller's savepoint holds both.
fn import(
    conn: &Connection,
    request: &DocumentChatImport<'_>,
    wallet_id: &str,
    attach_evidence: bool,
    identities: &Identities<'_>,
) -> Result<Py, AppError> {
    let empty = dict::<&str>([]);
    let source = mapping(request.draft, "source").unwrap_or(&empty);
    let rows: Vec<Py> = rows(request.draft)
        .into_iter()
        .filter(|row| matches!(row, Py::Dict(_)))
        .collect();
    let ready = ready_row_ids(request.draft);
    let sha256 = text_or_empty(get(source, "sha256"));
    let threshold = get(request.draft, "confidence_threshold")
        .cloned()
        .unwrap_or(Py::Null);
    let mut outcome = import_draft(
        conn,
        ImportDraft {
            source_file: request.source_file,
            data_root: request.data_root,
            profile_id: request.profile_id,
            wallet: wallet_id,
            rows: &rows,
            include_quarantined: false,
            selected_row_ids: Some(&ready),
            expected_source_sha256: Some(&sha256),
            confidence_threshold: &threshold,
            attach_evidence,
            // The plan runs without copying the document, but must still
            // refuse what the apply would refuse.
            check_evidence_budget: true,
        },
        identities.import,
    )?;
    let filename = match get(source, "filename") {
        Some(name) if truthy(name) => py_str(name),
        _ => request.source_file.to_owned(),
    };
    let filename = super::attachment::basename(&filename).to_owned();
    match record_batch(conn, request.profile_id, wallet_id, &filename, &outcome) {
        Ok(Some(batch)) => kassiber_envelope::set(&mut outcome, "import_batch_id", s(batch)),
        Ok(None) => {}
        Err(error) => {
            // The caller rolls the rows back; the evidence copies are files.
            if let Some(Py::List(attached)) = get(&outcome, "attached_evidence") {
                let root = attachments_root(request.data_root);
                for item in attached {
                    let relative = text_or_empty(get(item, "stored_relpath"));
                    if let Some(path) = root
                        .as_deref()
                        .and_then(|root| super::document::files::managed_path(root, &relative))
                    {
                        let _ = std::fs::remove_file(path);
                    }
                }
            }
            return Err(error);
        }
    }
    Ok(outcome)
}

fn attachments_root(data_root: &std::path::Path) -> Option<std::path::PathBuf> {
    let paths = kassiber_paths::Context::from_env().ok()?;
    let data_root = paths.effective_data_root(data_root).ok()?;
    Some(
        paths
            .state_root_for_data(&data_root)
            .ok()?
            .join("attachments"),
    )
}

/// `plan_or_apply(conn, profile, source_file=..., draft=..., request=...,
/// data_root=..., expected=...)`.
pub fn plan_or_apply(
    conn: &Connection,
    request: DocumentChatImport<'_>,
    identities: &Identities<'_>,
) -> Result<Py, AppError> {
    let profile_id = request.profile_id;
    let target = normalize_target(request.request)?;
    let version = input_version(conn, profile_id)?;
    let empty = dict::<&str>([]);
    let source = mapping(request.draft, "source").unwrap_or(&empty).clone();
    let source_sha256 = text_or_empty(get(&source, "sha256"));
    if let Some(expected) = request.expected {
        if get(expected, "expected_source_sha256") != Some(&s(source_sha256.as_str())) {
            return Err(AppError::new(
                "attachment_changed",
                "The document changed since it was read",
            )
            .with_hint("Read the document again, plan again and ask for consent again."));
        }
        if !same_int(get(expected, "expected_input_version"), version) {
            return Err(book_changed(expected, version, PLAN_TOOL));
        }
    }
    let summary = mapping(request.draft, "summary").unwrap_or(&empty).clone();
    let ready = ready_row_ids(request.draft);
    let (described, mut blockers, existing) =
        resolve_target(conn, profile_id, &target, PLAN_TOOL, SOURCE_FORMAT)?;
    let mut warnings: Vec<Py> = Vec::new();
    if ready.is_empty() {
        blockers.push(blocker(
            "nothing_ready",
            "The reader was sure of no row; review the document in the desktop's document import.",
            vec![
                ("action", s("review_in_desktop")),
                ("desktop_only", Py::Bool(true)),
            ],
        ));
    }
    let held_back = int(get(&summary, "quarantined"));
    if held_back != 0 {
        warnings.push(s("rows_held_for_review"));
    }
    let mut rows = vec![
        ("read", Py::from(int(get(&summary, "rows")))),
        ("ready", Py::from(ready.len() as i64)),
        ("held_for_review", Py::from(held_back)),
        ("to_import", Py::from(0)),
        ("already_present", Py::from(0)),
    ];
    let mut effects = batch_effects(conn, profile_id, "", None)?;
    let simulate = !blockers.iter().any(|blocker| {
        get(blocker, "code").is_some_and(|code| NO_SIMULATION.iter().any(|name| *code == s(*name)))
    });
    let workspace_id: String = conn
        .query_row(
            "SELECT workspace_id FROM profiles WHERE id = ?",
            [profile_id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    kassiber_store::begin_write_savepoint(conn, SAVEPOINT).map_err(database_error)?;
    let planned = (|| -> Result<(Py, String), AppError> {
        if simulate {
            let run = (|| -> Result<(String, Py), AppError> {
                let (wallet_id, _) = match &existing {
                    Some(wallet) => wallet.clone(),
                    None => create_target_wallet(
                        conn,
                        profile_id,
                        &target,
                        SOURCE_FORMAT,
                        identities.wallet,
                    )?,
                };
                let outcome = import(conn, &request, &wallet_id, false, identities)?;
                Ok((wallet_id, outcome))
            })();
            match run {
                Err(error) if error.code == "internal_error" => return Err(error),
                Err(error) => {
                    let code = if error.code.is_empty() {
                        "import_failed".to_owned()
                    } else {
                        error.code.clone()
                    };
                    blockers.push(blocker(&code, &error.message, vec![]));
                }
                Ok((wallet_id, outcome)) if truthy(&outcome) => {
                    let to_import = int(get(&outcome, "imported"));
                    let already_present = int(get(&outcome, "unchanged"));
                    rows[3].1 = Py::from(to_import);
                    rows[4].1 = Py::from(already_present);
                    let batch = get(&outcome, "import_batch_id").map(py_str);
                    effects = batch_effects(conn, profile_id, &wallet_id, batch.as_deref())?;
                    if get(&outcome, "updated_records").is_some_and(truthy) {
                        warnings.push(s("updates_existing_rows_not_rolled_back"));
                    }
                    if to_import == 0 {
                        blockers.push(blocker(
                            "nothing_to_import",
                            "No new rows: every ready row is already in this wallet.",
                            vec![("action", s("none"))],
                        ));
                    }
                    if already_present != 0 {
                        warnings.push(s("rows_already_present"));
                    }
                    if int(get(&effects[2].1, "existing_rows_in_date_range")) != 0 {
                        warnings.push(s("overlaps_existing_history"));
                    }
                }
                Ok(_) => {}
            }
        }
        let mut plan = vec![
            ("schema_version", Py::from(PLAN_SCHEMA_VERSION)),
            ("workspace_id", s(workspace_id.as_str())),
            ("profile_id", s(profile_id)),
            ("input_version", Py::from(version)),
            (
                "document",
                dict([
                    ("sha256", s(source_sha256.as_str())),
                    ("kind", get(&source, "kind").cloned().unwrap_or(Py::Null)),
                ]),
            ),
            ("source_format", s(SOURCE_FORMAT)),
            (
                "row_ids",
                Py::List(ready.iter().map(|id| s(id.as_str())).collect()),
            ),
            ("target", described.clone()),
            ("rows", dict(rows.clone())),
        ];
        plan.extend(effects.iter().cloned());
        let ready_flag = blockers.is_empty();
        plan.extend([
            ("warnings", Py::List(warnings.clone())),
            ("blockers", Py::List(blockers.clone())),
            ("ready", Py::Bool(ready_flag)),
        ]);
        let plan = dict(plan);
        // The draft's digest binds the plan without crossing to the model.
        let mut bound = plan.clone();
        kassiber_envelope::set(&mut bound, "draft", s(draft_digest(request.draft)?));
        let plan_digest = digest(&bound)?;
        Ok((plan, plan_digest))
    })();
    // The simulation never stays, even when applying: the real import below
    // runs once more with the document attached as evidence.
    conn.execute_batch(&format!(
        "ROLLBACK TO SAVEPOINT {SAVEPOINT}; RELEASE SAVEPOINT {SAVEPOINT}"
    ))
    .map_err(database_error)?;
    let (plan, plan_digest) = planned?;
    let Some(expected) = request.expected else {
        let Py::Dict(mut payload) = plan else {
            unreachable!("plan is a dict")
        };
        payload.push((Key::from("plan_digest"), s(plan_digest.as_str())));
        payload.push((
            Key::from("apply_with"),
            dict([
                ("plan_digest", s(plan_digest.as_str())),
                ("expected_source_sha256", s(source_sha256.as_str())),
                ("expected_input_version", Py::from(version)),
            ]),
        ));
        return Ok(Py::Dict(payload));
    };
    if let Some(refusal) =
        super::attachment::apply_refusal(&plan, &plan_digest, expected, PLAN_TOOL)
    {
        return Err(refusal);
    }
    // One savepoint holds the wallet, the rows, their evidence and the run.
    kassiber_store::begin_write_savepoint(conn, APPLY_SAVEPOINT).map_err(database_error)?;
    let applied = (|| -> Result<((String, String), Py), AppError> {
        let wallet = match &existing {
            Some(wallet) => wallet.clone(),
            None => {
                create_target_wallet(conn, profile_id, &target, SOURCE_FORMAT, identities.wallet)?
            }
        };
        let outcome = import(conn, &request, &wallet.0, true, identities)?;
        Ok((wallet, outcome))
    })();
    let ((wallet_id, wallet_label), outcome) = match applied {
        Ok(applied) => {
            conn.execute_batch(&format!("RELEASE SAVEPOINT {APPLY_SAVEPOINT}"))
                .map_err(database_error)?;
            applied
        }
        Err(error) => {
            let _ = conn.execute_batch(&format!(
                "ROLLBACK TO SAVEPOINT {APPLY_SAVEPOINT}; RELEASE SAVEPOINT {APPLY_SAVEPOINT}"
            ));
            return Err(error);
        }
    };
    let batch = get(&outcome, "import_batch_id")
        .cloned()
        .unwrap_or(Py::Null);
    let mut applied_rows = match get(&plan, "rows") {
        Some(Py::Dict(rows)) => rows.clone(),
        _ => Vec::new(),
    };
    applied_rows.push((
        Key::from("imported"),
        Py::from(int(get(&outcome, "imported"))),
    ));
    let evidence = match get(&outcome, "attached_evidence") {
        Some(Py::List(items)) => items.len(),
        _ => 0,
    };
    let mut result = vec![
        ("applied", Py::Bool(true)),
        ("plan_digest", s(plan_digest)),
        ("import_batch_id", batch.clone()),
        (
            "wallet",
            dict([
                ("id", s(wallet_id)),
                ("label", s(wallet_label)),
                ("created", Py::Bool(matches!(target, Target::Create { .. }))),
            ]),
        ),
        ("rows", Py::Dict(applied_rows)),
        ("evidence_attached", Py::from(evidence as i64)),
    ];
    result.extend(effects);
    result.extend([
        (
            "warnings",
            get(&plan, "warnings").cloned().unwrap_or(Py::Null),
        ),
        (
            "journals_invalidated",
            Py::Bool(get(&outcome, "journal_invalidated").is_some_and(truthy)),
        ),
        (
            "rollback",
            dict([
                ("batch", batch),
                ("desktop_only", Py::Bool(true)),
                (
                    "why",
                    s("the document is kept as evidence on the imported rows"),
                ),
            ]),
        ),
    ]);
    Ok(dict(result))
}
