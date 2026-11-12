//! The harness binary end to end on synthetic books, pinned to golden
//! transcripts under `tests/black_box/golden/` (`KASSIBER_BLESS=1` rewrites
//! them). Each module replaces one former Python parity module; its tests
//! keep that module's test names so the mutation catalogs' trail reads.

#![cfg(feature = "test-harness")]

#[path = "black_box/support.rs"]
mod support;

#[path = "black_box/cli_leaves.rs"]
mod cli_leaves;

#[path = "black_box/desktop.rs"]
mod desktop;

#[path = "black_box/ai_chat.rs"]
mod ai_chat;
#[path = "black_box/ai_document.rs"]
mod ai_document;
#[path = "black_box/ai_tools.rs"]
mod ai_tools;
#[path = "black_box/audit_package.rs"]
mod audit_package;
#[path = "black_box/cli_proxy.rs"]
mod cli_proxy;
#[path = "black_box/evidence_summary.rs"]
mod evidence_summary;
#[path = "black_box/hidden_home.rs"]
mod hidden_home;
#[path = "black_box/legacy_ledger.rs"]
mod legacy_ledger;
#[path = "black_box/legacy_migrations.rs"]
mod legacy_migrations;
#[path = "black_box/maintenance_run.rs"]
mod maintenance_run;
#[path = "black_box/mcp.rs"]
mod mcp;
#[path = "black_box/privacy_mirror.rs"]
mod privacy_mirror;
#[path = "black_box/projects.rs"]
mod projects;
#[path = "black_box/release_cli.rs"]
mod release_cli;
#[path = "black_box/report_blockers.rs"]
mod report_blockers;
#[path = "black_box/report_read_sync.rs"]
mod report_read_sync;
#[path = "black_box/review_context.rs"]
mod review_context;
#[path = "black_box/shell.rs"]
mod shell;
#[path = "black_box/transaction_dashboard.rs"]
mod transaction_dashboard;
#[path = "black_box/transaction_graph.rs"]
mod transaction_graph;
#[path = "black_box/wallet_completeness.rs"]
mod wallet_completeness;
