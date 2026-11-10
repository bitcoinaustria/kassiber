//! `kassiber mcp serve`'s protocol core and its tool projection (plan 21,
//! stage R6-8; `kassiber/mcp/protocol.py` and `kassiber/mcp/tools.py`).
//!
//! [`protocol`] speaks newline-delimited JSON-RPC in both MCP eras and
//! knows nothing about Kassiber: a [`protocol::ToolProvider`] supplies the
//! era-neutral definitions and runs calls. [`tools`] derives the 44
//! read-only definitions from the in-product AI catalog and shapes results
//! and refusals the way every MCP surface reports them. Nothing here opens a
//! file or a socket; the provider in `kassiber-cli` reaches the book.

pub mod protocol;
pub mod tools;

pub use protocol::{serve, Cancelled, McpServer, ToolOutcome, ToolProvider};
