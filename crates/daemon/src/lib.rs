//! `kassiber daemon`: the JSONL protocol the desktop supervisor speaks.
//!
//! One JSON object per `\n`-terminated stdin line, one compact envelope per
//! stdout line. The first record is `daemon.ready` (no `request_id`) with
//! the kinds this daemon implements; then each request
//! `{request_id, kind, args}` gets terminal records echoing its
//! `request_id`, unsolicited records carry `event: true` and never a
//! `request_id`, and end of input or `daemon.shutdown` ends the process
//! after the book is closed and ownership released. This follows
//! `kassiber/daemon.py`'s `run` loop and `handle_request` for the protocol
//! and the R1 kinds (`status`, `daemon.lock`, `daemon.unlock`,
//! `daemon.shutdown`); every other classified kind answers
//! `not_implemented`, and an unclassified one `unsupported_kind`, as in
//! Python. See docs/reference/rust-shell.md.

mod attachments;
pub mod control;
pub mod input;
mod loans;
mod metadata;
pub mod output;
mod rates;
#[cfg(feature = "test-kinds")]
mod test_kinds;

use std::io::{Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use kassiber_envelope::redaction::{redact_error_details, redact_secret_text};
use kassiber_envelope::{build_error_envelope, dict, get, s, success, truthy, type_name, AppError};
use kassiber_pyjson::{Key, PyStr, Value};
use kassiber_pytext::{loads, repr_py, strip};
use kassiber_runtime::capabilities::daemon_capability;
use kassiber_runtime::status::daemon_status_payload;
use kassiber_runtime::store::{
    Book, BookStore, Bootstrap, OwnerKind, ProjectOwner, RuntimeConfig, Secret, Unlock,
};
use kassiber_runtime::BuildInfo;

use crate::control::{BridgeError, SECRET_STORE_CONTROL_RESPONSE_KIND};
use crate::input::{spawn_reader, Chunk, Input, MAX_REQUEST_LINE_CHARS};
use crate::output::{with_request_id, Output, OutputClosed, RequestId};

/// The kinds this daemon implements, in Python's `SUPPORTED_KINDS` order.
pub const SUPPORTED_KINDS: &[&str] = &[
    "status",
    "ui.attachments.list",
    "ui.attachments.add",
    "ui.attachments.copy",
    "ui.attachments.rename",
    "ui.attachments.remove",
    "ui.attachments.open",
    "ui.loans.list",
    "ui.loans.link",
    "ui.loans.mark",
    "ui.loans.unmark",
    "ui.activity.stale",
    "daemon.lock",
    "daemon.unlock",
    "daemon.shutdown",
];

const UNLOCK_SCOPE: &str = "unlock_database";
const UNLOCK_LABEL: &str = "Enter the SQLCipher database passphrase to unlock Kassiber.";

/// What `kassiber daemon` was started with.
pub struct DaemonConfig {
    pub build: BuildInfo,
    /// `bootstrap_runtime(args, needs_db=False)` for the selected project.
    pub bootstrap: Bootstrap,
}

/// Every kind `daemon.ready` advertises.
pub fn supported_kinds() -> Vec<&'static str> {
    let mut kinds = vec![SUPPORTED_KINDS[0]];
    kinds.extend(services::kinds());
    kinds.extend_from_slice(&SUPPORTED_KINDS[1..]);
    #[cfg(feature = "test-kinds")]
    kinds.extend(test_kinds::KINDS);
    kinds.extend(rates::KINDS);
    kinds
}

/// Serves the protocol until end of input or `daemon.shutdown`; returns the
/// exit code (1 when stdout fails).
pub fn serve(
    store: Arc<dyn BookStore>,
    config: DaemonConfig,
    input: Box<dyn Read + Send>,
    output: Box<dyn Write + Send>,
) -> i32 {
    let output = Output::new(output);
    let mut input = Input::new(spawn_reader(input));
    let mut daemon = Daemon::new(store, config, output.clone());
    let kinds = supported_kinds().into_iter().map(s).collect();
    let ready = success(
        "daemon.ready",
        dict([
            ("version", s(daemon.build.version)),
            ("supported_kinds", Value::List(kinds)),
        ]),
    );
    let code = match output.write(&ready) {
        Ok(()) => daemon.run(&mut input),
        Err(OutputClosed) => 1,
    };
    daemon.shutdown_resources();
    code
}

/// Why handling a request stopped short of a response.
enum Failure {
    App(AppError),
    Closed,
}

impl From<AppError> for Failure {
    fn from(error: AppError) -> Self {
        Failure::App(error)
    }
}

impl From<BridgeError> for Failure {
    fn from(error: BridgeError) -> Self {
        match error {
            BridgeError::App(error) => Failure::App(error),
            BridgeError::Closed(_) => Failure::Closed,
        }
    }
}

impl From<OutputClosed> for Failure {
    fn from(_: OutputClosed) -> Self {
        Failure::Closed
    }
}

/// A handler's result: the terminal record (or none when it writes its own,
/// as streaming kinds do) and whether to shut down.
type Handled = Result<(Option<Value>, bool), Failure>;

/// `_error_envelope`: redacted message, hint, and details.
pub fn error_envelope(
    code: &str,
    message: &str,
    request_id: &RequestId,
    details: Option<&Value>,
    hint: Option<&str>,
    retryable: bool,
) -> Value {
    let message = redact_secret_text(message);
    let hint = hint.map(redact_secret_text);
    with_request_id(
        build_error_envelope(
            code,
            &message,
            details.map(redact_error_details),
            hint.as_deref(),
            retryable,
            None,
        ),
        request_id,
    )
}

fn app_error_envelope(error: &AppError, request_id: &RequestId) -> Value {
    let code = if error.code.is_empty() {
        "app_error"
    } else {
        &error.code
    };
    error_envelope(
        code,
        &error.message,
        request_id,
        error.details.as_ref(),
        error.hint.as_deref(),
        error.retryable,
    )
}

/// `_locked_envelope(scope, label, request_id)`.
fn locked_envelope(scope: &str, label: &str, request_id: &RequestId) -> Value {
    with_request_id(
        success(
            "auth_required",
            dict([("scope", s(scope)), ("label", s(label))]),
        ),
        request_id,
    )
}

/// A random-enough token for the ownership generation
/// (`secrets.token_hex(8)` in Python).
fn token_hex8() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    format!("{:016x}", hasher.finish())
}

/// A connection and owner kept for another close or release attempt.
struct Retired {
    book: Option<Box<dyn Book>>,
    owner: Option<Box<dyn ProjectOwner>>,
}

/// `DaemonContext`, for the R1 kinds.
pub(crate) struct Daemon {
    store: Arc<dyn BookStore>,
    build: BuildInfo,
    pub(crate) output: Output,
    data_root: String,
    /// `ctx.runtime_config`: loaded at bootstrap, merged on open.
    runtime_config: RuntimeConfig,
    project_id: Option<String>,
    project_root: Option<String>,
    select_project_on_open: bool,
    book: Option<Box<dyn Book>>,
    owner: Option<Box<dyn ProjectOwner>>,
    retired: Vec<Retired>,
    ownership_generation: String,
    /// The passphrase that unlocked the book, for later background opens.
    unlocked_passphrase: Option<Secret>,
}

impl Daemon {
    fn new(store: Arc<dyn BookStore>, config: DaemonConfig, output: Output) -> Self {
        let paths = &config.bootstrap.paths;
        Daemon {
            store,
            build: config.build,
            output,
            data_root: paths.data_root.clone(),
            runtime_config: config.bootstrap.runtime_config.clone(),
            project_id: paths.project_id.clone(),
            project_root: paths.project_root.clone(),
            select_project_on_open: !config.bootstrap.selection_explicit,
            book: None,
            owner: None,
            retired: Vec::new(),
            ownership_generation: format!("desktop-{}-{}", std::process::id(), token_hex8()),
            unlocked_passphrase: None,
        }
    }

    fn write(&self, record: &Value) -> Result<(), OutputClosed> {
        self.output.write(record)
    }

    fn run(&mut self, input: &mut Input) -> i32 {
        loop {
            let line = match input.next_chunk() {
                Chunk::Eof => return 0,
                Chunk::Undecodable => {
                    let details = dict([("error", s("request line is not valid UTF-8"))]);
                    let record = error_envelope(
                        "invalid_json",
                        "daemon request line is not valid JSON",
                        &RequestId::Value(Value::Null),
                        Some(&details),
                        None,
                        false,
                    );
                    if self.write(&record).is_err() {
                        return 1;
                    }
                    continue;
                }
                Chunk::Text(line) => line,
            };
            if line.chars().count() > MAX_REQUEST_LINE_CHARS {
                let mut chunk = Chunk::Text(line);
                while !chunk.ends_line() {
                    chunk = input.next_chunk();
                }
                let details = dict([("max_chars", Value::from(MAX_REQUEST_LINE_CHARS))]);
                let record = error_envelope(
                    "request_too_large",
                    "daemon request line is too large",
                    &RequestId::Value(Value::Null),
                    Some(&details),
                    None,
                    false,
                );
                if self.write(&record).is_err() {
                    return 1;
                }
                // Python's loop never sees an end of input it drained here
                // and polls forever; stop instead.
                if chunk == Chunk::Eof {
                    return 0;
                }
                continue;
            }
            let raw = strip(&line);
            if raw.is_empty() {
                continue;
            }
            let request = match loads(raw) {
                Ok(request) => request,
                Err(error) => {
                    let details = dict([("error", s(error.to_string()))]);
                    let record = error_envelope(
                        "invalid_json",
                        "daemon request line is not valid JSON",
                        &RequestId::Value(Value::Null),
                        Some(&details),
                        None,
                        false,
                    );
                    if self.write(&record).is_err() {
                        return 1;
                    }
                    continue;
                }
            };
            if !matches!(request, Value::Dict(_)) {
                let details = dict([("type", s(type_name(&request)))]);
                let record = error_envelope(
                    "validation",
                    "daemon request must be a JSON object",
                    &RequestId::Value(Value::Null),
                    Some(&details),
                    None,
                    false,
                );
                if self.write(&record).is_err() {
                    return 1;
                }
                continue;
            }
            if matches!(get(&request, "kind"), Some(Value::Str(k)) if k.as_str() == Some(SECRET_STORE_CONTROL_RESPONSE_KIND))
            {
                continue;
            }
            let outcome = catch_unwind(AssertUnwindSafe(|| self.handle(&request, input)));
            let (response, shutdown) = match outcome {
                Ok(Ok(handled)) => handled,
                Ok(Err(Failure::Closed)) => return 1,
                Ok(Err(Failure::App(error))) => {
                    self.rollback();
                    (
                        Some(app_error_envelope(&error, &RequestId::or_null(&request))),
                        false,
                    )
                }
                Err(panic) => {
                    self.rollback();
                    let message = panic
                        .downcast_ref::<&str>()
                        .map(|m| (*m).to_owned())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "panic".to_owned());
                    // Not eprintln!: a closed stderr must not panic again
                    // outside catch_unwind.
                    let _ = writeln!(
                        std::io::stderr(),
                        "kassiber daemon: request handler panicked: {message}"
                    );
                    let record = error_envelope(
                        "internal_error",
                        &message,
                        &RequestId::or_null(&request),
                        None,
                        None,
                        false,
                    );
                    (Some(record), false)
                }
            };
            if let Some(response) = response {
                if self.write(&response).is_err() {
                    return 1;
                }
            }
            if shutdown {
                return 0;
            }
        }
    }

    fn rollback(&mut self) {
        if let Some(book) = self.book.as_mut() {
            let _ = book.rollback();
        }
    }

    /// `handle_request(ctx, request, out)`.
    fn handle(&mut self, request: &Value, input: &mut Input) -> Handled {
        let request_id = RequestId::of(request);
        let kind = match get(request, "kind") {
            Some(Value::Str(kind)) if kind.code_points().next().is_some() => kind.clone(),
            _ => {
                let Value::Dict(pairs) = request else {
                    return Ok((None, false));
                };
                // JSON object keys are always strings.
                let mut keys: Vec<PyStr> = pairs
                    .iter()
                    .filter_map(|(key, _)| match key {
                        Key::Str(key) => Some(key.clone()),
                        _ => None,
                    })
                    .collect();
                keys.sort();
                let details = dict([(
                    "keys",
                    Value::List(keys.into_iter().map(Value::Str).collect()),
                )]);
                let record = error_envelope(
                    "validation",
                    "daemon request requires a non-empty string kind",
                    &request_id,
                    Some(&details),
                    None,
                    false,
                );
                return Ok((Some(record), false));
            }
        };
        #[cfg(feature = "test-kinds")]
        if let Some(handled) = test_kinds::handle(self, &kind, request, &request_id, input) {
            return handled;
        }
        let Some(kind_text) = kind.as_str().filter(|k| daemon_capability(k).is_some()) else {
            let details = dict([("kind", Value::Str(kind.clone()))]);
            let record = error_envelope(
                "unsupported_kind",
                &format!(
                    "daemon kind {} is not classified or supported",
                    repr_py(&kind)
                ),
                &request_id,
                Some(&details),
                None,
                false,
            );
            return Ok((Some(record), false));
        };
        let kind_text = kind_text.to_owned();
        if !SUPPORTED_KINDS.contains(&kind_text.as_str())
            && !rates::KINDS.contains(&kind_text.as_str())
            && !services::kinds().any(|k| k == kind_text)
        {
            let details = dict([("kind", s(kind_text.as_str()))]);
            let record = error_envelope(
                "not_implemented",
                &format!(
                    "daemon kind {} is not implemented in the Rust daemon yet",
                    repr_py(&kind)
                ),
                &request_id,
                Some(&details),
                None,
                false,
            );
            return Ok((Some(record), false));
        }
        let mut args = get(request, "args").cloned();
        if let Some(Value::Dict(pairs)) = &args {
            if pairs
                .iter()
                .any(|(key, _)| *key == Key::from("expected_scope"))
            {
                if let Some(locked) = self.check_expected_scope(args.as_ref(), &request_id)? {
                    return Ok((Some(locked), false));
                }
                args = Some(Value::Dict(
                    pairs
                        .iter()
                        .filter(|(key, _)| *key != Key::from("expected_scope"))
                        .cloned()
                        .collect(),
                ));
            }
        }
        if metadata::KINDS.contains(&kind_text.as_str()) {
            if let Some(locked) = self.open_lazily(&request_id)? {
                return Ok((Some(locked), false));
            }
            let payload = metadata::execute(self.book_mut()?.as_mut(), args.as_ref())?;
            return Ok((
                Some(with_request_id(success(&kind_text, payload), &request_id)),
                false,
            ));
        }
        if loans::KINDS.contains(&kind_text.as_str()) {
            if let Some(locked) = self.open_lazily(&request_id)? {
                return Ok((Some(locked), false));
            }
            let payload = loans::execute(self.book_mut()?.as_mut(), &kind_text, args.as_ref())?;
            return Ok((
                Some(with_request_id(success(&kind_text, payload), &request_id)),
                false,
            ));
        }
        if attachments::KINDS.contains(&kind_text.as_str()) {
            if let Some(locked) = self.open_lazily(&request_id)? {
                return Ok((Some(locked), false));
            }
            let payload =
                attachments::execute(self.book_mut()?.as_mut(), &kind_text, args.as_ref())?;
            return Ok((
                Some(with_request_id(success(&kind_text, payload), &request_id)),
                false,
            ));
        }
        let _ = input;
        if rates::KINDS.contains(&kind_text.as_str()) {
            if let Some(locked) = self.open_lazily(&request_id)? {
                return Ok((Some(locked), false));
            }
            let payload = rates::run(self.book_mut()?.as_mut(), &kind_text, args.as_ref())?;
            return Ok((
                Some(with_request_id(success(&kind_text, payload), &request_id)),
                false,
            ));
        }
        match kind_text.as_str() {
            kind if crate::offline::KINDS.contains(&kind) => {
                let payload = crate::offline::execute(self.store.as_ref(), kind, args.as_ref())?;
                Ok((
                    Some(with_request_id(success(kind, payload), &request_id)),
                    false,
                ))
            }
            "daemon.shutdown" => Ok((
                Some(with_request_id(
                    success("daemon.shutdown", dict::<&str>([])),
                    &request_id,
                )),
                true,
            )),
            "daemon.lock" => {
                self.lock()?;
                Ok((
                    Some(with_request_id(
                        success("daemon.lock", dict([("locked", Value::Bool(true))])),
                        &request_id,
                    )),
                    false,
                ))
            }
            "daemon.unlock" => self.unlock(args.as_ref(), &request_id),
            "status" => {
                if let Some(locked) = self.open_lazily(&request_id)? {
                    return Ok((Some(locked), false));
                }
                let payload = self.status_payload()?;
                Ok((
                    Some(with_request_id(success("status", payload), &request_id)),
                    false,
                ))
            }
            _ => {
                if let Some(locked) = self.open_lazily(&request_id)? {
                    return Ok((Some(locked), false));
                }
                let payload =
                    services::execute(self.book_mut()?.as_mut(), &kind_text, args.as_ref())?;
                Ok((
                    Some(with_request_id(success(&kind_text, payload), &request_id)),
                    false,
                ))
            }
        }
    }

    /// `args.expected_scope`: refuse a request meant for another book
    /// before any handler runs. Returns the `auth_required` record when the
    /// book is locked.
    fn check_expected_scope(
        &mut self,
        args: Option<&Value>,
        request_id: &RequestId,
    ) -> Result<Option<Value>, Failure> {
        let Some(args) = args else {
            return Ok(None);
        };
        let expected = get(args, "expected_scope").cloned().unwrap_or(Value::Null);
        let field = |key: &str| match get(&expected, key) {
            Some(Value::Str(text)) => text.as_str().map(str::to_owned),
            _ => None,
        };
        let well_formed = match &expected {
            Value::Dict(pairs) => {
                let mut keys: Vec<Option<&str>> = pairs
                    .iter()
                    .map(|(key, _)| match key {
                        Key::Str(key) => key.as_str(),
                        _ => None,
                    })
                    .collect();
                keys.sort();
                keys == [Some("profile_id"), Some("workspace_id")]
                    && pairs.iter().all(|(_, value)| match value {
                        Value::Str(text) => kassiber_pytext::strip_py(text)
                            .code_points()
                            .next()
                            .is_some(),
                        _ => false,
                    })
            }
            _ => false,
        };
        if !well_formed {
            return Err(AppError::new("validation", "Expected book scope is malformed").into());
        }
        if self.book.is_none() {
            if let Some(locked) = self.open_lazily(request_id)? {
                return Ok(Some(locked));
            }
        }
        let current = self.book_mut()?.context()?;
        let expected_workspace = get(&expected, "workspace_id")
            .cloned()
            .unwrap_or(Value::Null);
        let expected_profile = get(&expected, "profile_id").cloned().unwrap_or(Value::Null);
        if s(current.workspace_id.as_str()) != expected_workspace
            || s(current.profile_id.as_str()) != expected_profile
        {
            return Err(AppError::new(
                "stale_context",
                "The active book changed before the requested input was supplied",
            )
            .retryable(true)
            .with_hint("Return to the original book and restart this input step.")
            .into());
        }
        let stale = || AppError::new("stale_context", "Requested input targets another book");
        if get(args, "workspace").is_some() || get(args, "profile").is_some() {
            let (workspace_id, profile_id) = self
                .book_mut()?
                .resolve_scope(get(args, "workspace"), get(args, "profile"))?;
            if Some(workspace_id) != field("workspace_id")
                || Some(profile_id) != field("profile_id")
            {
                return Err(stale().into());
            }
        }
        for (key, wanted) in [
            ("workspace_id", &expected_workspace),
            ("profile_id", &expected_profile),
        ] {
            if get(args, key).is_some_and(|given| given != wanted) {
                return Err(stale().into());
            }
        }
        Ok(None)
    }

    fn book_mut(&mut self) -> Result<&mut Box<dyn Book>, AppError> {
        self.book.as_mut().ok_or_else(|| {
            AppError::new(
                "passphrase_required",
                "database is locked; unlock the daemon before accessing your books",
            )
            .with_hint(
                "Enter the SQLCipher database passphrase to unlock the local daemon session.",
            )
        })
    }

    /// Opens a plaintext book on first use; an encrypted one answers
    /// `auth_required`.
    fn open_lazily(&mut self, request_id: &RequestId) -> Result<Option<Value>, Failure> {
        if self.book.is_some() {
            return Ok(None);
        }
        match self.open(None, false) {
            Ok(()) => Ok(None),
            Err(error) if error.code == "passphrase_required" => Ok(Some(locked_envelope(
                UNLOCK_SCOPE,
                UNLOCK_LABEL,
                request_id,
            ))),
            Err(error) => Err(error.into()),
        }
    }

    /// `_ensure_daemon_project_owner(ctx)`.
    fn ensure_owner(&mut self) -> Result<(), AppError> {
        let project = self.store.canonical_project(&self.data_root)?;
        if let Some(owner) = &self.owner {
            if owner.project().identity != project.identity {
                return Err(AppError::new(
                    "project_owner_mismatch",
                    "the desktop daemon owns a different project",
                )
                .with_details(dict([("project", s(project.public_id.as_str()))])));
            }
            return Ok(());
        }
        self.owner = Some(self.store.acquire_ownership(
            &project,
            OwnerKind::Desktop,
            &self.ownership_generation,
        )?);
        Ok(())
    }

    /// `_release_daemon_project_owner(ctx)`.
    fn release_owner(&mut self) -> Result<(), AppError> {
        if let Some(mut owner) = self.owner.take() {
            if let Err(error) = owner.release() {
                self.retired.push(Retired {
                    book: None,
                    owner: Some(owner),
                });
                return Err(error);
            }
        }
        Ok(())
    }

    /// `_retry_retired_project_resources(ctx)`: close, then release.
    fn retry_retired(&mut self) {
        let mut remaining = Vec::new();
        for mut resource in std::mem::take(&mut self.retired) {
            if let Some(book) = resource.book.as_mut() {
                if book.close().is_err() {
                    remaining.push(resource);
                    continue;
                }
                resource.book = None;
            }
            if let Some(owner) = resource.owner.as_mut() {
                if owner.release().is_err() {
                    remaining.push(resource);
                    continue;
                }
                resource.owner = None;
            }
        }
        self.retired = remaining;
    }

    fn remember_passphrase(&mut self, passphrase: Option<&Secret>) {
        if let Some(passphrase) = passphrase {
            self.unlocked_passphrase = Some(Secret::new(passphrase.expose().to_owned()));
        }
    }

    /// `_open_daemon_connection(ctx, passphrase, require_existing_schema)`.
    fn open(
        &mut self,
        passphrase: Option<&Secret>,
        require_existing_schema: bool,
    ) -> Result<(), AppError> {
        self.retry_retired();
        if self.book.is_some() {
            self.ensure_owner()?;
            self.remember_passphrase(passphrase);
            return Ok(());
        }
        let acquired_here = self.owner.is_none();
        self.ensure_owner()?;
        let canonical = self
            .owner
            .as_ref()
            .map(|owner| owner.project().data_root.clone())
            .unwrap_or_else(|| self.data_root.clone());
        let store = Arc::clone(&self.store);
        let opened = store.open_book(
            &canonical,
            Unlock::Daemon {
                passphrase,
                require_existing_schema,
                expected_project_identity: &self
                    .owner
                    .as_ref()
                    .expect("owner acquired")
                    .project()
                    .identity,
            },
            &mut self.runtime_config,
        );
        let book = match opened {
            Ok(book) => book,
            Err(error) => {
                if acquired_here {
                    self.release_owner()?;
                }
                return Err(error);
            }
        };
        self.data_root = canonical;
        self.book = Some(book);
        if let Some(owner) = &mut self.owner {
            if let Err(error) = owner.refresh_after_open() {
                // A failed inode reservation must not leave a cached book
                // accessible on the next request. Preserve close-before-release.
                self.lock()?;
                return Err(error);
            }
        }
        if let Some(project_id) = self.project_id.clone() {
            self.store.mark_project_opened(
                &project_id,
                &self.data_root,
                self.select_project_on_open,
            )?;
        }
        self.remember_passphrase(passphrase);
        Ok(())
    }

    /// `daemon.lock`: close the book, release ownership, forget the
    /// passphrase.
    fn lock(&mut self) -> Result<(), AppError> {
        if let Some(mut book) = self.book.take() {
            if let Err(error) = book.close() {
                self.book = Some(book);
                return Err(error);
            }
        }
        self.release_owner()?;
        self.unlocked_passphrase = None;
        Ok(())
    }

    /// `daemon.unlock`.
    fn unlock(&mut self, args: Option<&Value>, request_id: &RequestId) -> Handled {
        let args = match args {
            None | Some(Value::Null) => dict::<&str>([]),
            Some(args @ Value::Dict(_)) => args.clone(),
            Some(other) => {
                return Err(AppError::new("validation", "daemon args must be an object")
                    .with_details(dict([("type", s(type_name(other)))]))
                    .into())
            }
        };
        // `_passphrase_from_auth`: a non-empty `auth_response.passphrase_secret`.
        // A passphrase with lone surrogates cannot be encoded for SQLCipher;
        // it is refused like a wrong one.
        let supplied =
            get(&args, "auth_response").and_then(|auth| match get(auth, "passphrase_secret") {
                Some(Value::Str(text)) if text.code_points().next().is_some() => Some(text.clone()),
                _ => None,
            });
        let require_existing_schema = get(&args, "require_existing_project").is_some_and(truthy);
        let denied = || {
            error_envelope(
                "local_auth_denied",
                "passphrase verification failed",
                request_id,
                None,
                None,
                true,
            )
        };
        if self.store.database_is_encrypted(&self.data_root)? {
            let Some(supplied) = supplied else {
                return Ok((
                    Some(locked_envelope(UNLOCK_SCOPE, UNLOCK_LABEL, request_id)),
                    false,
                ));
            };
            let Some(text) = supplied.as_str() else {
                return Ok((Some(denied()), false));
            };
            let passphrase = Secret::new(text.to_owned());
            if !self
                .store
                .verify_passphrase(&self.data_root, UNLOCK_SCOPE, &passphrase)?
            {
                return Ok((Some(denied()), false));
            }
            if self.book.is_none() {
                match self.open(Some(&passphrase), require_existing_schema) {
                    Ok(()) => {}
                    Err(error) if error.code == "unlock_failed" => {
                        return Ok((Some(denied()), false))
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        } else {
            self.open(None, require_existing_schema)?;
        }
        let status = self.status_payload()?;
        let data = dict([("unlocked", Value::Bool(true)), ("status", status)]);
        Ok((
            Some(with_request_id(success("daemon.unlock", data), request_id)),
            false,
        ))
    }

    /// `_status_payload(ctx)`.
    fn status_payload(&mut self) -> Result<Value, AppError> {
        let facts = self.book_mut()?.status_facts()?;
        let project = self
            .project_id
            .as_deref()
            .map(|id| (id, self.project_root.as_deref()));
        Ok(daemon_status_payload(
            self.build.version,
            &facts,
            &self.runtime_config,
            project,
        ))
    }

    /// The `finally` of Python's `run`: forget the passphrase, close the
    /// book, and release ownership.
    fn shutdown_resources(&mut self) {
        self.unlocked_passphrase = None;
        self.retry_retired();
        let book = self.book.take();
        let owner = self.owner.take();
        if book.is_some() || owner.is_some() {
            self.retired.push(Retired { book, owner });
            self.retry_retired();
        }
    }
}

mod accounts;
mod context;
mod networks;
mod offline;
mod profiles;
mod services;
mod views;
mod workspaces;
