//! The AI read executor's host (`kassiber-ai`): database work runs here, on
//! the request loop, against the open book (`_run_on_daemon_main_thread` and
//! `_drain_daemon_main_thread_tasks`), and each read reaches the same native
//! family the desktop kind uses, without the desktop's pre-request
//! maintenance. Chat (R6-7, `ai_chat.rs`) drives it from its worker through
//! request-loop tasks; the test-harness kinds `test.ai_tool` and
//! `test.ai_auto_read` drive it directly.

use std::path::Path;

use kassiber_ai::executor::Book as AiBook;
#[cfg(feature = "test-kinds")]
use kassiber_ai::executor::ReadHost;
use kassiber_ai::ToolError;
use kassiber_envelope::{s, AppError};
use kassiber_pyjson::Value;

use crate::Daemon;

/// The daemon as the executor's request loop.
#[cfg(feature = "test-kinds")]
pub(crate) struct Host<'a> {
    pub(crate) daemon: &'a mut Daemon,
}

#[cfg(feature = "test-kinds")]
impl ReadHost for Host<'_> {
    fn on_request_loop(
        &mut self,
        work: &mut dyn FnMut(&mut dyn AiBook) -> Result<Value, ToolError>,
    ) -> Result<Value, ToolError> {
        if self.daemon.book.is_none() {
            match self.daemon.open(None, false) {
                Ok(()) => {}
                Err(error) if error.code == "passphrase_required" => {
                    return Err(AppError::new(
                        "passphrase_required",
                        "database is locked; unlock the daemon before running AI tools",
                    )
                    .into());
                }
                Err(error) => return Err(tool_error(error)),
            }
        }
        work(&mut HostBook {
            daemon: &mut *self.daemon,
        })
    }
}

pub(crate) struct HostBook<'a> {
    pub(crate) daemon: &'a mut Daemon,
}

/// Python's executor turns non-`AppError` exceptions into a generic
/// `tool_error`; native code reports those as `internal_error`.
pub(crate) fn tool_error(error: AppError) -> ToolError {
    if error.sqlite_origin && (error.message.contains("locked") || error.message.contains("busy")) {
        return ToolError::Busy;
    }
    if error.code == "internal_error" {
        return ToolError::Unexpected;
    }
    ToolError::App(error)
}

fn resolved(path: &Path) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

impl AiBook for HostBook<'_> {
    fn database_in_scope(&mut self, data_root: &str) -> Result<bool, ToolError> {
        let facts = self
            .daemon
            .book_mut()
            .map_err(tool_error)?
            .status_facts()
            .map_err(tool_error)?;
        let expected = kassiber_paths::resolve_database_path(Path::new(data_root));
        Ok(resolved(&expected) == resolved(Path::new(&facts.paths.database)))
    }

    fn context(&mut self) -> Result<(Value, Value), ToolError> {
        let context = self
            .daemon
            .book_mut()
            .map_err(tool_error)?
            .context()
            .map_err(tool_error)?;
        Ok((s(context.workspace_id), s(context.profile_id)))
    }

    fn maintain_for_read(&mut self, sync_attempted: bool) -> Result<Option<Value>, ToolError> {
        self.daemon
            .book_mut()
            .map_err(tool_error)?
            .maintain_for_ai_read(sync_attempted)
            .map_err(tool_error)
    }

    fn read(
        &mut self,
        kind: &str,
        arguments: &Value,
        authored_source: &'static str,
    ) -> Result<Value, ToolError> {
        native_read(self.daemon, kind, arguments, authored_source).map_err(tool_error)
    }

    fn active_profile(&mut self) -> Result<String, ToolError> {
        let book = self.daemon.book_mut().map_err(tool_error)?;
        Ok(book.resolve_scope(None, None).map_err(tool_error)?.1)
    }

    fn decode_analysis_references(
        &mut self,
        profile: &str,
        arguments: &Value,
    ) -> Result<Value, ToolError> {
        let book = self.daemon.book_mut().map_err(tool_error)?;
        book.chain_analysis_decode_ai_args(profile, arguments)
            .map_err(tool_error)
    }

    fn issue_analysis_reference(
        &mut self,
        profile: &str,
        value: &str,
    ) -> Result<String, ToolError> {
        let book = self.daemon.book_mut().map_err(tool_error)?;
        let issued = book
            .chain_analysis_issue_ai_references(profile, &[value.to_owned()])
            .map_err(tool_error)?;
        issued.into_iter().next().ok_or(ToolError::Unexpected)
    }
}

/// One native read kind, routed as `Daemon::handle` routes it, for a book
/// that is already open.
fn native_read(
    daemon: &mut Daemon,
    kind: &str,
    args: &Value,
    source: &'static str,
) -> Result<Value, AppError> {
    let args = Some(args);
    if kind == "status" {
        // `_status_payload_from_parts`: the desktop's project fields are not
        // part of the AI view.
        let facts = daemon.book_mut()?.status_facts()?;
        if let Some(default) = daemon.book_mut()?.current_default_backend() {
            daemon.runtime_config.default_backend = Some(default);
        }
        return Ok(kassiber_runtime::status::daemon_status_payload(
            daemon.build.version,
            &facts,
            &daemon.runtime_config,
            None,
        ));
    }
    if crate::transactions::KINDS.contains(&kind)
        || crate::transactions::AUDIT_KINDS.contains(&kind)
    {
        return crate::transactions::execute(daemon.book_mut()?.as_mut(), kind, args);
    }
    if crate::documents::KINDS.contains(&kind) {
        return crate::documents::execute(daemon.book_mut()?.as_mut(), kind, args);
    }
    if crate::reports::KINDS.contains(&kind) {
        return crate::reports::execute(daemon.book_mut()?.as_mut(), kind, args);
    }
    if crate::metadata::KINDS.contains(&kind) {
        return crate::metadata::execute(daemon.book_mut()?.as_mut(), kind, args);
    }
    if crate::review::KINDS.contains(&kind) {
        return crate::review::execute(daemon.book_mut()?.as_mut(), kind, args);
    }
    if crate::transfers::KINDS.contains(&kind) {
        return crate::transfers::execute_as(daemon.book_mut()?.as_mut(), kind, args, source);
    }
    if crate::loans::KINDS.contains(&kind) {
        return crate::loans::execute(daemon.book_mut()?.as_mut(), kind, args);
    }
    if crate::attachments::KINDS.contains(&kind) {
        return crate::attachments::execute(daemon.book_mut()?.as_mut(), kind, args);
    }
    if crate::rates::KINDS.contains(&kind) {
        return crate::rates::run(daemon.book_mut()?.as_mut(), kind, args);
    }
    if crate::settings::KINDS.contains(&kind) {
        return crate::settings::execute(daemon.book_mut()?.as_mut(), kind, args);
    }
    if crate::chain_analysis_reads::KINDS.contains(&kind) {
        let runtime = std::sync::Arc::clone(&daemon.analysis);
        return crate::chain_analysis_reads::execute(
            daemon.book_mut()?.as_mut(),
            &runtime,
            kind,
            args,
        );
    }
    if crate::chain_analysis_ops::KINDS.contains(&kind) {
        return crate::chain_analysis_ops::execute(daemon.book_mut()?.as_mut(), kind, args);
    }
    if kind == "ui.wallets.list" {
        let default = daemon.runtime_config.default_backend.clone();
        return daemon.book_mut()?.wallets_snapshot(default.as_deref());
    }
    if kind == "ui.wallets.utxos" {
        let default = daemon.runtime_config.default_backend.clone();
        return daemon.book_mut()?.wallet_utxos(args, default.as_deref());
    }
    if crate::services::kinds().any(|served| served == kind) {
        return crate::services::execute(daemon.book_mut()?.as_mut(), kind, args);
    }
    Err(AppError::new(
        "not_implemented",
        format!("AI tool kind {kind} has no native read"),
    ))
}
