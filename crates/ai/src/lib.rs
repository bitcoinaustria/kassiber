//! Kassiber's in-product AI tools (plan 21, stage R6-5): the catalog the
//! assistant can be offered, per-turn tool selection, the schema validator,
//! argument and result redaction, consent summaries, the consent state
//! machine, the read executor, the auto-read planner with its context
//! trimming, provenance and the privacy receipt, the embedded skill
//! references, and the read-side projections.
//!
//! Python (`kassiber/ai/tools.py`, `kassiber/ai/prompt.py` and the AI half of
//! `kassiber/daemon.py`) is the reference; docs/reference/r6-ai-operator-port.md
//! is the design. The provider store, the HTTP client, discovery (R6-6a) and
//! the provider broker client (R6-6b) reach the network or start the broker
//! only through `kassiber_net` routes the caller's gate authorizes. The
//! executor reaches the book only through [`executor::ReadHost`], which the
//! daemon serves on its request loop. Egressing tools and writes never run
//! without a consent bound to the call.

pub mod analysis;
pub mod broker;
pub mod catalog;
pub mod chat;
pub mod chat_imports;
pub mod clock;
pub mod consent;
pub mod discovery;
pub mod executor;
pub mod file_analysis;
pub mod import_runs;
pub mod mutate;
pub mod planner;
pub mod previews;
pub mod projections;
pub mod prompt;
pub mod provenance;
pub mod providers;
mod pyre;
mod pyval;
pub mod redact;
pub mod review;
pub mod select;
pub mod service;
pub mod skills;
pub mod summary;
pub mod transport;
pub mod undo;
pub mod validate;
pub mod visuals;

use kassiber_envelope::AppError;

pub use catalog::{get_tool, tools, KindClass, ToolEntry};

/// Why a tool did not produce a result.
#[derive(Debug)]
pub enum ToolError {
    /// Python's `AppError`: the call is denied with its code and message.
    App(AppError),
    /// SQLite lock contention from another Kassiber process.
    Busy,
    /// Anything else Python would catch as an unexpected exception.
    Unexpected,
}

impl From<AppError> for ToolError {
    fn from(error: AppError) -> Self {
        ToolError::App(error)
    }
}
