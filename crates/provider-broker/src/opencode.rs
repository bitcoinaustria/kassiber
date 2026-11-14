//! The OpenCode adapter (`opencode.ts`, `opencode2.ts`). Step 2 of plan 23 ports it; until then
//! the status probe finds the CLI and reports it as not ported, and a chat
//! fails before starting anything.

use std::path::Path;

use crate::executables::resolve_executable;
use crate::protocol::{
    provider_status, BrokerFailure, ChatRequest, ProviderId, ProviderState, ProviderStatus,
};
use crate::{Adapter, EventSink, ToolBridge};

/// The adapter [`crate::adapter`] returns for `opencode`.
pub static OPENCODE: OpenCode = OpenCode;

pub struct OpenCode;

const DISPLAY_NAME: &str = "OpenCode";
const EXECUTABLE: &str = "opencode";

impl Adapter for OpenCode {
    fn status(&self, _cwd: &Path) -> Result<ProviderStatus, BrokerFailure> {
        let Some(executable) = resolve_executable(EXECUTABLE) else {
            return Ok(provider_status(
                ProviderId::OpenCode,
                DISPLAY_NAME,
                ProviderState::MissingExecutable,
                "Install OpenCode, then run `opencode auth login` outside Kassiber.",
            ));
        };
        Ok(provider_status(
            ProviderId::OpenCode,
            DISPLAY_NAME,
            ProviderState::Error,
            "The OpenCode adapter is not ported to the Rust broker yet.",
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
            return Err(BrokerFailure::new("OpenCode is not installed."));
        }
        Err(BrokerFailure::new(
            "The OpenCode adapter is not ported to the Rust broker yet.",
        ))
    }
}
