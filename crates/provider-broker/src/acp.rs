//! The GitHub Copilot adapter (`acp.ts`). Step 2 of plan 23 ports it; until then
//! the status probe finds the CLI and reports it as not ported, and a chat
//! fails before starting anything.

use std::path::Path;

use crate::executables::resolve_executable;
use crate::protocol::{
    provider_status, BrokerFailure, ChatRequest, ProviderId, ProviderState, ProviderStatus,
};
use crate::{Adapter, EventSink, ToolBridge};

/// The adapter [`crate::adapter`] returns for `copilot`.
pub static COPILOT: Copilot = Copilot;

pub struct Copilot;

const DISPLAY_NAME: &str = "GitHub Copilot";
const EXECUTABLE: &str = "copilot";

impl Adapter for Copilot {
    fn status(&self, _cwd: &Path) -> Result<ProviderStatus, BrokerFailure> {
        let Some(executable) = resolve_executable(EXECUTABLE) else {
            return Ok(provider_status(
                ProviderId::Copilot,
                DISPLAY_NAME,
                ProviderState::MissingExecutable,
                "Install the GitHub Copilot CLI, then sign in outside Kassiber.",
            ));
        };
        Ok(provider_status(
            ProviderId::Copilot,
            DISPLAY_NAME,
            ProviderState::Error,
            "The GitHub Copilot adapter is not ported to the Rust broker yet.",
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
            return Err(BrokerFailure::new("GitHub Copilot is not installed."));
        }
        Err(BrokerFailure::new(
            "The GitHub Copilot adapter is not ported to the Rust broker yet.",
        ))
    }
}
