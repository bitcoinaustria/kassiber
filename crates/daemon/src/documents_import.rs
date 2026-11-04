//! `DocumentImportSessions`: process-local grants for native-picker files.
//!
//! The renderer holds only an opaque token. Source paths and reviewed draft
//! rows stay inside the daemon until the import succeeds, the bounded session
//! expires, or the book is locked, switched or restored.
use kassiber_envelope::AppError;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const SESSION_TTL: Duration = Duration::from_secs(30 * 60);

pub(crate) struct Session {
    pub source_file: String,
    workspace_id: String,
    profile_id: String,
    data_root: String,
    last_accessed: Instant,
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

impl Sessions {
    fn prune(&mut self, now: Instant) {
        self.sessions
            .retain(|_, session| now.duration_since(session.last_accessed) < SESSION_TTL);
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

    pub(crate) fn source_for_preview(
        &mut self,
        token: &str,
        scope: &Scope<'_>,
    ) -> Result<String, AppError> {
        let session = self.require(token, scope)?;
        session.last_accessed = Instant::now();
        Ok(session.source_file.clone())
    }
}
