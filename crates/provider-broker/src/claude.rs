//! The Claude adapter (`claude.ts`). Step 2 of plan 23 ports it; until then
//! the status probe finds the CLI and reports it as not ported, and a chat
//! fails before starting anything.

use std::path::Path;

use crate::executables::resolve_executable;
use crate::protocol::{
    provider_status, BrokerFailure, ChatRequest, ProviderId, ProviderState, ProviderStatus,
};
use crate::{Adapter, EventSink, ToolBridge};

/// The adapter [`crate::adapter`] returns for `claude`.
pub static CLAUDE: Claude = Claude;

pub struct Claude;

const DISPLAY_NAME: &str = "Claude";
const EXECUTABLE: &str = "claude";

impl Adapter for Claude {
    fn status(&self, _cwd: &Path) -> Result<ProviderStatus, BrokerFailure> {
        let Some(executable) = resolve_executable(EXECUTABLE) else {
            return Ok(provider_status(
                ProviderId::Claude,
                DISPLAY_NAME,
                ProviderState::MissingExecutable,
                "Install Claude Code, then run `claude login` outside Kassiber.",
            ));
        };
        Ok(provider_status(
            ProviderId::Claude,
            DISPLAY_NAME,
            ProviderState::Error,
            "The Claude adapter is not ported to the Rust broker yet.",
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
            return Err(BrokerFailure::new("Claude is not installed."));
        }
        Err(BrokerFailure::new(
            "The Claude adapter is not ported to the Rust broker yet.",
        ))
    }
}
