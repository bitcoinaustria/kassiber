//! Desktop document import: `internal.document_import.stage`,
//! `ui.wallets.document_import.preview` and `ui.wallets.document_import.import`.
//!
//! `DocumentImportSessions` are process-local grants for native-picker files.
//! The renderer holds only an opaque token. Source paths and reviewed draft
//! rows stay inside the daemon until the import succeeds, the bounded session
//! expires, or the book is locked, switched or restored. A document grants no
//! ownership, basis or tax fact; its rows are observations like any import.
use crate::chat_uploads::{self, ChatUploads, Received};
use crate::{
    imports::optional_string,
    output::{with_request_id, RequestId},
    Daemon, Handled,
};
use kassiber_envelope::{dict, get, py_str, s, success, truthy, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_runtime::store::ReviewedDocument;
use kassiber_wallets::file_import::document::{staged_source, DocumentPreview};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub const KINDS: &[&str] = &[
    "internal.document_import.stage",
    chat_uploads::KIND,
    "ui.wallets.document_import.preview",
    "ui.wallets.document_import.import",
];

const SESSION_TTL: Duration = Duration::from_secs(30 * 60);
/// `MAX_DOCUMENT_IMPORT_SESSIONS`: sixteen since a chat holds up to eight files (#698).
const MAX_SESSIONS: usize = 16;
const RENDERER_FIELDS: [&str; 6] = [
    "source_file",
    "file",
    "file_path",
    "draft",
    "rows",
    "expected_source_sha256",
];

#[derive(Clone)]
pub(crate) struct Session {
    pub source_file: String,
    workspace_id: String,
    profile_id: String,
    data_root: String,
    last_accessed: Instant,
    /// The reviewed preview a preview token stands for.
    draft: Option<Value>,
}

/// The book a session is bound to.
pub(crate) struct Scope<'a> {
    pub workspace_id: &'a str,
    pub profile_id: &'a str,
    pub data_root: &'a str,
}

#[derive(Default)]
pub(crate) struct Sessions {
    sessions: HashMap<String, Session>,
    /// Files the daemon wrote for a pasted attachment; deleted with the last
    /// session that refers to them.
    owned_files: HashSet<String>,
    pub(crate) uploads: ChatUploads,
}

pub(crate) fn unavailable() -> AppError {
    AppError::new(
        "document_import_session_expired",
        "Document import session is unavailable",
    )
    .with_hint("Choose the document and preview it again.")
}

pub(crate) fn source_unavailable() -> AppError {
    AppError::new(
        "document_import_source_unavailable",
        "Selected document is no longer available for local OCR",
    )
    .with_hint("Choose the document again and create a new preview.")
}

/// `secrets.token_urlsafe(32)`.
fn token() -> Result<String, AppError> {
    token_urlsafe(32)
}

/// `secrets.token_urlsafe(n)`.
pub(crate) fn token_urlsafe(n: usize) -> Result<String, AppError> {
    use std::io::Read;
    let mut bytes = vec![0u8; n];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|_| AppError::new("internal_error", "Could not create a document session"))?;
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(n * 4 / 3 + 1);
    for chunk in bytes.chunks(3) {
        let n =
            chunk.iter().fold(0u32, |acc, b| acc << 8 | u32::from(*b)) << (8 * (3 - chunk.len()));
        for index in 0..=chunk.len() {
            out.push(ALPHABET[(n >> (18 - 6 * index) & 63) as usize] as char);
        }
    }
    Ok(out)
}

impl Sessions {
    fn prune(&mut self, now: Instant) {
        let expired: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, session)| now.duration_since(session.last_accessed) >= SESSION_TTL)
            .map(|(token, _)| token.clone())
            .collect();
        for token in expired {
            self.drop_session(&token);
        }
    }

    /// `_drop_locked(token)`: an uploaded file goes with the last session
    /// that refers to it.
    fn drop_session(&mut self, token: &str) {
        let Some(session) = self.sessions.remove(token) else {
            return;
        };
        if !self.owned_files.contains(&session.source_file)
            || self
                .sessions
                .values()
                .any(|other| other.source_file == session.source_file)
        {
            return;
        }
        self.owned_files.remove(&session.source_file);
        chat_uploads::discard_upload(&session.source_file);
    }

    fn evict(&mut self) {
        while self.sessions.len() > MAX_SESSIONS {
            let oldest = self
                .sessions
                .iter()
                .min_by_key(|(_, session)| session.last_accessed)
                .map(|(token, _)| token.clone())
                .expect("sessions");
            self.drop_session(&oldest);
        }
    }

    fn require(&mut self, token: &str, scope: &Scope<'_>) -> Result<&mut Session, AppError> {
        self.prune(Instant::now());
        match self.sessions.get_mut(token) {
            Some(session)
                if session.workspace_id == scope.workspace_id
                    && session.profile_id == scope.profile_id
                    && session.data_root == scope.data_root =>
            {
                Ok(session)
            }
            _ => Err(unavailable()),
        }
    }

    fn stage(
        &mut self,
        source_file: String,
        scope: &Scope<'_>,
        owned: bool,
    ) -> Result<String, AppError> {
        let now = Instant::now();
        let token = token()?;
        self.prune(now);
        if owned {
            self.owned_files.insert(source_file.clone());
        }
        self.sessions.insert(
            token.clone(),
            Session {
                source_file,
                workspace_id: scope.workspace_id.into(),
                profile_id: scope.profile_id.into(),
                data_root: scope.data_root.into(),
                last_accessed: now,
                draft: None,
            },
        );
        self.evict();
        Ok(token)
    }

    pub(crate) fn source_for_preview(
        &mut self,
        token: &str,
        scope: &Scope<'_>,
    ) -> Result<String, AppError> {
        let session = self.require(token, scope)?;
        session.last_accessed = Instant::now();
        Ok(session.source_file.clone())
    }

    fn create_preview(
        &mut self,
        token: &str,
        draft: Value,
        scope: &Scope<'_>,
    ) -> Result<String, AppError> {
        let source = self.require(token, scope)?;
        let now = Instant::now();
        source.last_accessed = now;
        let preview = Session {
            last_accessed: now,
            draft: Some(draft),
            ..source.clone()
        };
        let token = self::token()?;
        self.sessions.insert(token.clone(), preview);
        self.evict();
        Ok(token)
    }

    fn preview_for_import(&mut self, token: &str, scope: &Scope<'_>) -> Result<Session, AppError> {
        let session = self.require(token, scope)?;
        if session.draft.is_none() {
            return Err(AppError::new(
                "document_import_preview_required",
                "Document import must be previewed before it can be imported",
            )
            .with_hint("Preview the selected document, then import the reviewed rows."));
        }
        session.last_accessed = Instant::now();
        Ok(session.clone())
    }

    fn consume(&mut self, token: &str) {
        self.drop_session(token);
    }

    pub(crate) fn clear(&mut self) {
        let tokens: Vec<String> = self.sessions.keys().cloned().collect();
        for token in tokens {
            self.drop_session(&token);
        }
        self.uploads.clear();
    }
}

fn object(args: Option<&Value>) -> Result<Value, AppError> {
    match args {
        None | Some(Value::Null) => Ok(dict::<&str>([])),
        Some(value @ Value::Dict(_)) => Ok(value.clone()),
        Some(value) => Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(kassiber_envelope::type_name(value)))]))),
    }
}

fn required(args: &Value, key: &str, label: &str) -> Result<String, AppError> {
    optional_string(args, key)?.ok_or_else(|| {
        AppError::new("validation", format!("{label} is required."))
            .with_hint(format!("Enter {}.", label.to_lowercase()))
    })
}

fn has(args: &Value, key: &str) -> bool {
    get(args, key).is_some()
}

/// `_reject_document_import_renderer_fields`: the renderer never names paths,
/// rows or digests; the session holds them.
fn reject_renderer_fields(args: &Value, importing: bool) -> Result<(), AppError> {
    let mut forbidden: Vec<&str> = RENDERER_FIELDS.to_vec();
    if importing {
        forbidden.extend(["include_quarantined", "attach_evidence", "row_ids"]);
    }
    let mut supplied: Vec<&str> = forbidden.into_iter().filter(|key| has(args, key)).collect();
    supplied.sort_unstable();
    if supplied.is_empty() {
        return Ok(());
    }
    Err(AppError::new(
        "validation",
        "Document import accepts only a trusted document session",
    )
    .with_hint("Choose the document again and use the reviewed preview.")
    .with_details(dict([(
        "forbidden_fields",
        Value::List(supplied.into_iter().map(s).collect()),
    )])))
}

/// `_document_import_token`.
fn document_token(args: &Value) -> Result<String, AppError> {
    if let Some(Value::Str(text)) = get(args, "document_token") {
        let token = kassiber_pytext::strip(&py_str(&Value::Str(text.clone()))).to_owned();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    Err(AppError::new("validation", "document_token is required")
        .with_hint("Choose the local document before previewing or importing it."))
}

/// `_document_import_selected_row_ids`: unique, in first-seen order.
fn selected_row_ids(args: &Value) -> Result<Option<Vec<String>>, AppError> {
    let items = match get(args, "selected_row_ids") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::List(items)) => items,
        Some(_) => {
            return Err(AppError::new(
                "validation",
                "selected_row_ids must be a list",
            ))
        }
    };
    let mut selected: Vec<String> = vec![];
    for item in items {
        match item {
            Value::Str(text) if text.code_points().next().is_some() => {
                let text = py_str(item);
                if !selected.contains(&text) {
                    selected.push(text);
                }
            }
            _ => {
                return Err(AppError::new(
                    "validation",
                    "selected_row_ids must contain non-empty strings",
                ))
            }
        }
    }
    Ok(Some(selected))
}

/// A local-source failure that names the selected file means the user must
/// choose it again.
fn source_error(error: AppError, source: &str) -> AppError {
    if error.code == "not_found" && error.message.contains(source) {
        source_unavailable()
    } else {
        error
    }
}

fn drop_key(value: &mut Value, key: &str) {
    if let Value::Dict(pairs) = value {
        pairs.retain(|(name, _)| *name != Key::from(key));
    }
}

fn field_mut<'a>(value: &'a mut Value, key: &str) -> Option<&'a mut Value> {
    match value {
        Value::Dict(pairs) => pairs
            .iter_mut()
            .find(|(name, _)| *name == Key::from(key))
            .map(|(_, value)| value),
        _ => None,
    }
}

impl Daemon {
    fn document_scope(&mut self) -> Result<(String, String), AppError> {
        self.book_mut()?.resolve_scope(None, None)
    }

    /// The staged file behind a picker token, for this book only.
    pub(crate) fn document_source(&mut self, token: &str) -> Result<String, AppError> {
        let (workspace_id, profile_id) = self.document_scope()?;
        let data_root = self.data_root.clone();
        self.document_sessions.source_for_preview(
            token,
            &Scope {
                workspace_id: &workspace_id,
                profile_id: &profile_id,
                data_root: &data_root,
            },
        )
    }

    /// `_document_import_stage_payload`.
    fn stage_document(&mut self, args: &Value) -> Result<Value, AppError> {
        let source = required(args, "source_file", "Selected document")?;
        let chat = match get(args, "purpose") {
            None => false,
            Some(purpose) if *purpose == s("document_import") => false,
            Some(purpose) if *purpose == s("chat_attachment") => true,
            // Python's set membership raises on a list or object purpose
            // (tests/rust_port/repro_stage_purpose_unhashable.py); refused.
            Some(_) => return Err(AppError::new("validation", "Unknown staging purpose")),
        };
        let (path, size) = if chat {
            // Exports as well as statements and photos; the OCR preview still
            // refuses anything but an image or PDF when it reads the token.
            let path = chat_uploads::chat_attachment_source(&source)?;
            let size = std::fs::metadata(&path)
                .map_err(|_| source_unavailable())?
                .len();
            (path, size)
        } else {
            let (path, size, _) = staged_source(&source).map_err(|_| source_unavailable())?;
            (path, size)
        };
        let media_type = kassiber_wallets::file_import::document::media_type(&path);
        let (workspace_id, profile_id) = self.document_scope()?;
        if !has(args, "review_case_id")
            && (has(args, "review_recipe") || has(args, "expected_review_fingerprint"))
        {
            return Err(AppError::new(
                "validation",
                "Review recipe requires a source-funds case ID",
            ));
        }
        if has(args, "review_case_id") {
            let case_id = required(args, "review_case_id", "Review case")?;
            if case_id.starts_with("source_funds:") {
                if !has(args, "review_recipe") || !has(args, "expected_review_fingerprint") {
                    return Err(AppError::new(
                        "validation",
                        "Source-funds attachment requires its reviewed recipe and fingerprint",
                    ));
                }
            } else if has(args, "review_recipe") || has(args, "expected_review_fingerprint") {
                return Err(AppError::new(
                    "validation",
                    "Only source-funds evidence accepts a review recipe",
                ));
            }
            // Attaching the document to a review case is not native yet.
            return Err(kassiber_runtime::store::not_implemented(
                "documents",
                "review-case document staging",
            ));
        }
        let data_root = self.data_root.clone();
        let source_file = path.to_string_lossy().into_owned();
        let token = self.document_sessions.stage(
            source_file.clone(),
            &Scope {
                workspace_id: &workspace_id,
                profile_id: &profile_id,
                data_root: &data_root,
            },
            false,
        )?;
        // Python adds a `thumbnail` of a chat image when Pillow is present;
        // the Rust daemon has no image codec and, like Python without
        // Pillow, sends none.
        let kind = kassiber_wallets::file_import::analysis::file_kind(&source_file);
        Ok(dict([
            ("document_token", s(token)),
            (
                "source",
                dict([
                    (
                        "filename",
                        s(path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .as_ref()),
                    ),
                    ("media_type", s(media_type)),
                    ("size_bytes", Value::from(size as i64)),
                    ("kind", s(kind)),
                ]),
            ),
        ]))
    }

    /// `_chat_attachment_upload_payload`: take one chunk of a pasted file;
    /// the last one stages it like a pick.
    fn upload_chat_attachment(&mut self, args: &Value) -> Result<Value, AppError> {
        let (workspace_id, profile_id) = self.document_scope()?;
        let data_root = self.data_root.clone();
        let received =
            self.document_sessions
                .uploads
                .receive(args, &workspace_id, &profile_id, &data_root)?;
        let completed = match received {
            Received::Progress {
                upload_id,
                received_bytes,
            } => {
                return Ok(dict([
                    ("complete", Value::Bool(false)),
                    ("upload_id", s(upload_id)),
                    ("received_bytes", Value::from(received_bytes)),
                ]));
            }
            Received::Complete(completed) => completed,
        };
        let token = self.document_sessions.stage(
            completed.path.clone(),
            &Scope {
                workspace_id: &workspace_id,
                profile_id: &profile_id,
                data_root: &data_root,
            },
            true,
        )?;
        let path = std::path::Path::new(&completed.path);
        Ok(dict([
            ("complete", Value::Bool(true)),
            ("document_token", s(token)),
            (
                "source",
                dict([
                    ("filename", s(completed.filename.as_str())),
                    (
                        "media_type",
                        s(kassiber_wallets::file_import::document::media_type(path)),
                    ),
                    ("size_bytes", Value::from(completed.size_bytes)),
                    (
                        "kind",
                        s(kassiber_wallets::file_import::analysis::file_kind(
                            &completed.path,
                        )),
                    ),
                ]),
            ),
        ]))
    }

    /// `_document_import_preview_payload`: local OCR through the native
    /// provider service, then a preview grant for the reviewed rows.
    fn preview_document(&mut self, args: &Value) -> Result<Value, AppError> {
        reject_renderer_fields(args, false)?;
        let token = document_token(args)?;
        let (workspace_id, profile_id) = self.document_scope()?;
        let data_root = self.data_root.clone();
        let scope = Scope {
            workspace_id: &workspace_id,
            profile_id: &profile_id,
            data_root: &data_root,
        };
        let source = self.document_sessions.source_for_preview(&token, &scope)?;
        let provider_name = optional_string(args, "provider")?;
        let model = optional_string(args, "model")?;
        let raw = |key| get(args, key).cloned().unwrap_or(Value::Null);
        let fiat = self
            .book_mut()?
            .document_target(&profile_id, None)?
            .fiat_currency;
        let draft = self
            .book_mut()?
            .preview_document(DocumentPreview {
                source_file: source.clone(),
                provider_name,
                model,
                confidence_threshold: raw("confidence_threshold"),
                max_pages: raw("max_pages"),
                pages: raw("pages"),
                expected_fiat_currency: Some(fiat),
            })
            .map_err(|error| source_error(error, &source))?;
        let preview_token = self
            .document_sessions
            .create_preview(&token, draft.clone(), &scope)?;
        let mut public = draft;
        if let Some(source) = field_mut(&mut public, "source") {
            drop_key(source, "path");
        }
        kassiber_envelope::set(&mut public, "document_token", s(preview_token));
        Ok(public)
    }

    /// `_document_import_import_payload` (without the journal step).
    fn import_document(&mut self, args: &Value) -> Result<Value, AppError> {
        reject_renderer_fields(args, true)?;
        let token = document_token(args)?;
        let wallet = required(args, "wallet", "Wallet")?;
        let (workspace_id, profile_id) = self.document_scope()?;
        let data_root = self.data_root.clone();
        let session = self.document_sessions.preview_for_import(
            &token,
            &Scope {
                workspace_id: &workspace_id,
                profile_id: &profile_id,
                data_root: &data_root,
            },
        )?;
        let draft = session.draft.clone().unwrap_or(Value::Null);
        let rows = match get(&draft, "rows") {
            Some(Value::List(rows)) => Some(rows.clone()),
            _ => None,
        };
        let digest = match get(&draft, "source").and_then(|source| get(source, "sha256")) {
            Some(Value::Str(text)) => Some(py_str(&Value::Str(text.clone()))),
            _ => None,
        };
        let (Some(rows), Some(digest)) = (rows, digest) else {
            return Err(AppError::new(
                "document_import_preview_required",
                "Document import preview is incomplete",
            )
            .with_hint("Preview the local document again before importing selected rows."));
        };
        let Some(selected) = selected_row_ids(args)? else {
            return Err(AppError::new("validation", "selected_row_ids is required")
                .with_hint("Select one or more reviewed rows to import."));
        };
        let ready: Vec<String> = rows
            .iter()
            .filter(|row| {
                matches!(row, Value::Dict(_))
                    && get(row, "status") == Some(&s("ready"))
                    && get(row, "id").is_some_and(truthy)
            })
            .map(|row| py_str(get(row, "id").expect("ready id")))
            .collect();
        let invalid = selected.iter().filter(|id| !ready.contains(id)).count();
        if invalid > 0 {
            return Err(
                AppError::new("validation", "Selected document rows are not importable")
                    .with_hint("Select only ready rows from the current preview.")
                    .with_details(dict([("invalid_row_count", Value::from(invalid as i64))])),
            );
        }
        // Python resolves the wallet before reading the document again.
        self.book_mut()?
            .document_target(&profile_id, Some(&wallet))?;
        let rows: Vec<Value> = rows
            .into_iter()
            .filter(|row| matches!(row, Value::Dict(_)))
            .collect();
        let threshold = get(&draft, "confidence_threshold")
            .cloned()
            .unwrap_or(Value::Null);
        let mut outcome = self
            .book_mut()?
            .import_document(ReviewedDocument {
                source_file: &session.source_file,
                profile_id: &profile_id,
                wallet: &wallet,
                rows: &rows,
                selected_row_ids: &selected,
                source_sha256: &digest,
                include_quarantined: false,
                confidence_threshold: &threshold,
            })
            .map_err(|error| source_error(error, &session.source_file))?;
        self.document_sessions.consume(&token);
        if let Some(source) = field_mut(&mut outcome, "source") {
            drop_key(source, "path");
        }
        if let Some(Value::List(attachments)) = field_mut(&mut outcome, "attached_evidence") {
            for attachment in attachments {
                drop_key(attachment, "stored_relpath");
            }
        }
        Ok(outcome)
    }
}

pub(super) fn handle(
    daemon: &mut Daemon,
    kind: &str,
    args: Option<&Value>,
    request_id: &RequestId,
) -> Handled {
    if let Some(locked) = daemon.open_lazily(request_id)? {
        return Ok((Some(locked), false));
    }
    let args = object(args)?;
    match kind {
        "internal.document_import.stage" => {
            let payload = daemon.stage_document(&args)?;
            Ok((
                Some(with_request_id(success(kind, payload), request_id)),
                false,
            ))
        }
        chat_uploads::KIND => {
            let payload = daemon.upload_chat_attachment(&args)?;
            Ok((
                Some(with_request_id(success(kind, payload), request_id)),
                false,
            ))
        }
        "ui.wallets.document_import.preview" => {
            let payload = daemon.preview_document(&args)?;
            Ok((
                Some(with_request_id(success(kind, payload), request_id)),
                false,
            ))
        }
        _ => {
            // The book is captured before the import, as Python does.
            let profile_id = daemon.book_mut()?.context()?.profile_id;
            let outcome = daemon.import_document(&args)?;
            let payload = daemon
                .book_mut()?
                .import_journal_step(outcome, &profile_id)?;
            // Python ends the session after a committed desktop import.
            Ok((
                Some(with_request_id(success(kind, payload), request_id)),
                true,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(profile: &str) -> Scope<'_> {
        Scope {
            workspace_id: "w",
            profile_id: profile,
            data_root: "/books/a",
        }
    }

    #[test]
    fn document_grants_stay_bound_to_their_book_and_preview() {
        let mut sessions = Sessions::default();
        let token = sessions
            .stage("/files/receipt.png".into(), &scope("p"), false)
            .unwrap();
        assert_eq!(token.len(), 43);
        assert!(token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert_eq!(
            sessions.source_for_preview(&token, &scope("p")).unwrap(),
            "/files/receipt.png"
        );
        assert_eq!(
            sessions
                .source_for_preview(&token, &scope("q"))
                .unwrap_err()
                .code,
            "document_import_session_expired"
        );
        for other in [
            Scope {
                workspace_id: "x",
                ..scope("p")
            },
            Scope {
                data_root: "/books/b",
                ..scope("p")
            },
        ] {
            assert!(sessions.source_for_preview(&token, &other).is_err());
        }
        assert_eq!(
            sessions
                .preview_for_import(&token, &scope("p"))
                .err()
                .unwrap()
                .code,
            "document_import_preview_required"
        );
        let preview = sessions
            .create_preview(&token, dict([("rows", Value::List(vec![]))]), &scope("p"))
            .unwrap();
        assert_ne!(preview, token);
        assert!(sessions
            .preview_for_import(&preview, &scope("p"))
            .unwrap()
            .draft
            .is_some());
        sessions.consume(&preview);
        assert!(sessions.preview_for_import(&preview, &scope("p")).is_err());
        sessions.clear();
        assert!(sessions.source_for_preview(&token, &scope("p")).is_err());
    }

    #[test]
    fn document_grants_keep_only_the_most_recently_used() {
        let mut sessions = Sessions::default();
        let tokens: Vec<String> = (0..=MAX_SESSIONS)
            .map(|index| {
                sessions
                    .stage(format!("/files/{index}.png"), &scope("p"), false)
                    .unwrap()
            })
            .collect();
        assert!(sessions
            .source_for_preview(&tokens[0], &scope("p"))
            .is_err());
        assert!(sessions
            .source_for_preview(&tokens[MAX_SESSIONS], &scope("p"))
            .is_ok());
    }

    #[test]
    fn an_uploaded_file_goes_with_the_last_grant_that_names_it() {
        let directory = tempfile::tempdir().unwrap();
        let holder = directory.path().join("upload");
        std::fs::create_dir(&holder).unwrap();
        let file = holder.join("paste.csv");
        std::fs::write(&file, b"a").unwrap();
        let path = file.to_string_lossy().into_owned();
        let mut sessions = Sessions::default();
        let token = sessions.stage(path.clone(), &scope("p"), true).unwrap();
        let preview = sessions
            .create_preview(&token, dict([("rows", Value::List(vec![]))]), &scope("p"))
            .unwrap();
        sessions.consume(&token);
        assert!(file.exists(), "the preview grant still names it");
        sessions.consume(&preview);
        assert!(!file.exists() && !holder.exists());
        // A picked file is the user's own and is never deleted.
        let picked = directory.path().join("picked.csv");
        std::fs::write(&picked, b"a").unwrap();
        let token = sessions
            .stage(picked.to_string_lossy().into_owned(), &scope("p"), false)
            .unwrap();
        sessions.consume(&token);
        sessions.clear();
        assert!(picked.exists());
    }

    #[test]
    fn document_requests_refuse_renderer_authority() {
        let args = dict([
            ("rows", Value::List(vec![])),
            ("draft", Value::Null),
            ("row_ids", Value::Null),
        ]);
        let error = reject_renderer_fields(&args, false).unwrap_err();
        assert_eq!(
            error.details,
            Some(dict([(
                "forbidden_fields",
                Value::List(vec![s("draft"), s("rows")])
            )]))
        );
        let error =
            reject_renderer_fields(&dict([("include_quarantined", Value::Bool(false))]), true)
                .unwrap_err();
        assert_eq!(error.code, "validation");
        assert!(reject_renderer_fields(
            &dict([("include_quarantined", Value::Bool(false))]),
            false
        )
        .is_ok());
        assert_eq!(
            document_token(&dict([("document_token", s(" t "))])).unwrap(),
            "t"
        );
        assert!(document_token(&dict([("document_token", s(" "))])).is_err());
        let rows =
            |items: Vec<Value>| selected_row_ids(&dict([("selected_row_ids", Value::List(items))]));
        assert_eq!(
            rows(vec![s("b"), s("a"), s("b")]).unwrap(),
            Some(vec!["b".into(), "a".into()])
        );
        assert!(rows(vec![s("")]).is_err());
        assert!(rows(vec![Value::from(1)]).is_err());
        assert_eq!(selected_row_ids(&dict::<&str>([])).unwrap(), None);
    }
}
