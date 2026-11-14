//! The Codex adapter (`codex.ts`). Step 2 of plan 23 ports it; until then
//! the status probe finds the CLI and reports it as not ported, and a chat
//! fails before starting anything.

use std::path::Path;

use crate::executables::resolve_executable;
use crate::protocol::{
    provider_status, BrokerFailure, ChatRequest, ProviderId, ProviderState, ProviderStatus,
};
use crate::{Adapter, EventSink, ToolBridge};

/// The adapter [`crate::adapter`] returns for `codex`.
pub static CODEX: Codex = Codex;

pub struct Codex;

const DISPLAY_NAME: &str = "Codex";
const EXECUTABLE: &str = "codex";

impl Adapter for Codex {
    fn status(&self, _cwd: &Path) -> Result<ProviderStatus, BrokerFailure> {
        let Some(executable) = resolve_executable(EXECUTABLE) else {
            return Ok(provider_status(
                ProviderId::Codex,
                DISPLAY_NAME,
                ProviderState::MissingExecutable,
                "Install Codex, then run `codex login` outside Kassiber.",
            ));
        };
        Ok(provider_status(
            ProviderId::Codex,
            DISPLAY_NAME,
            ProviderState::Error,
            "The Codex adapter is not ported to the Rust broker yet.",
        )
        .with_executable(&executable.to_string_lossy()))
    }

    fn chat(
        &self,
        _request: &ChatRequest,
        _cwd: &Path,
        _bridge: Option<&ToolBridge>,
        _events: &EventSink,
    ) -> Result<(), BrokerFailure> {
        if resolve_executable(EXECUTABLE).is_none() {
            return Err(BrokerFailure::new("Codex is not installed."));
        }
        Err(BrokerFailure::new(
            "The Codex adapter is not ported to the Rust broker yet.",
        ))
    }
}
