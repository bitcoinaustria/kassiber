//! The daemon kinds the desktop may send: the desktop's privilege boundary, checked before every
//! request leaves the app (docs/reference/daemon.md, desktop invoke contract).
//!
//! - [`ALLOWED_DAEMON_KINDS`]: anything else is refused locally with `kind_not_allowed`.
//! - [`AI_RUNTIME_KINDS`]: refused with `ai_features_disabled` while AI features are off (the
//!   switch is a privacy promise, not a hidden button); `ai.providers.*` stays configurable.
//! - [`STREAMING_DAEMON_KINDS`]: kinds whose intermediate records are progress, not violations.
//!
//! These lists are the canonical desktop allowlist. `crates/daemon/tests/desktop_boundaries.rs`
//! checks them against the daemon's supported kinds, and keeps the Tauri shell's copies in
//! `ui-tauri/src-tauri/src/lib.rs` equal to them until that shell is removed.

use crate::daemon::DaemonError;

pub const ALLOWED_DAEMON_KINDS: &[&str] = &[
    "ui.chain_analysis.ai_context",
    "ui.chain_analysis.query",
    "ui.chain_analysis.entropy",
    "ui.chain_analysis.entropy.start",
    "ui.chain_analysis.jobs.get",
    "ui.chain_analysis.jobs.cancel",
    "ui.chain_analysis.psbt.analyze",
    "ui.chain_analysis.psbt.compare",
    "ui.chain_analysis.psbt.entropy.start",
    "ui.chain_analysis.datasets.query",
    "ui.chain_analysis.datasets.list",
    "ui.chain_analysis.datasets.get",
    "ui.chain_analysis.datasets.preview.start",
    "ui.chain_analysis.datasets.import.start",
    "ui.chain_analysis.datasets.revoke",
    "ui.chain_analysis.datasets.discard.start",
    "ui.chain_analysis.watches.preview",
    "ui.chain_analysis.watches.create",
    "ui.chain_analysis.watches.list",
    "ui.chain_analysis.watches.configure",
    "ui.chain_analysis.watches.delete",
    "ui.chain_analysis.watches.inbox",
    "ui.chain_analysis.watches.acknowledge",
    "ui.chain_analysis.watches.evaluate",
    "ui.chain_analysis.cases.list",
    "ui.chain_analysis.cases.get",
    "ui.chain_analysis.cases.compare",
    "ui.chain_analysis.labels.list",
    "ui.chain_analysis.acquire.plan",
    "ui.chain_analysis.cases.save",
    "ui.chain_analysis.cases.delete",
    "ui.chain_analysis.labels.upsert",
    "ui.chain_analysis.labels.delete",
    "ui.chain_analysis.labels.import",
    "ui.chain_analysis.acquire.apply",
    "ui.chain_analysis.sources.plan",
    "ui.chain_analysis.sources.authorize",
    "ui.chain_analysis.sources.list",
    "ui.chain_analysis.sources.revoke",
    "ui.chain_analysis.sources.run",
    "status",
    "ui.egress.snapshot",
    "ui.network.offline",
    "ui.network.offline.set",
    "ui.overview.snapshot",
    "ui.workspace.overview.snapshot",
    "ui.transactions.list",
    "ui.transactions.dashboard",
    "ui.transactions.metadata.update",
    "ui.transactions.resolve",
    "ui.transactions.graph",
    "ui.transactions.history",
    "ui.transactions.history.revert",
    "ui.activity.history",
    "ui.activity.stale",
    "ui.attachments.list",
    "ui.attachments.add",
    "ui.attachments.copy",
    "ui.attachments.rename",
    "ui.attachments.remove",
    "ui.attachments.open",
    "ui.wallets.list",
    "ui.backends.list",
    "ui.backends.options",
    "ui.backends.public_defaults",
    "ui.backends.settings.list",
    "ui.backends.create",
    "ui.backends.update",
    "ui.backends.delete",
    "ui.backends.set_default",
    "ui.backends.bitcoinrpc.test",
    "ui.backends.btcpay.test",
    "ui.backends.detect_core",
    "ui.backends.electrum.test",
    "ui.backends.certificate.fetch",
    "ui.backends.certificate.trust",
    "ui.backends.http.test",
    "ui.backends.lightning.test",
    "ui.profiles.snapshot",
    // Desktop-only (desktop_boundaries.rs `DESKTOP_ONLY_KINDS`): Book settings reads any book's settings.
    "ui.profiles.settings",
    "ui.networks.binding",
    "ui.networks.inventory",
    "ui.networks.plan",
    "ui.networks.bind",
    "ui.networks.partition_plan",
    "ui.networks.partition_export",
    "ui.onboarding.complete",
    "ui.profiles.create",
    "ui.profiles.rename",
    "ui.profiles.update",
    "ui.profiles.switch",
    "ui.profiles.reset_data",
    "ui.report.blockers",
    "ui.reports.capital_gains",
    "ui.reports.explain_capital_gain",
    "ui.reports.summary",
    "ui.reports.balance_sheet",
    "ui.reports.portfolio_summary",
    "ui.reports.balance_history",
    "ui.reports.tax_summary",
    "ui.reports.privacy_hygiene",
    "ui.reports.privacy_mirror",
    "ui.reports.psbt_privacy",
    "ui.reports.exit_tax_preview",
    "ui.reports.export_exit_tax_pdf",
    "ui.reports.export_exit_tax_xlsx",
    "ui.reports.export_year_handoff",
    "ui.reports.export_cross_check",
    "ui.reports.export_pdf",
    "ui.reports.export_summary_pdf",
    "ui.reports.export_csv",
    "ui.reports.export_xlsx",
    "ui.reports.export_capital_gains_csv",
    "ui.reports.export_austrian_e1kv_pdf",
    "ui.reports.export_austrian_e1kv_xlsx",
    "ui.reports.export_austrian_e1kv_csv",
    "ui.reports.export_audit_package",
    "ui.transactions.export_csv",
    "ui.transactions.export_xlsx",
    "ui.transactions.export_platform_csv",
    "ui.transactions.ledger_template",
    "ui.journals.snapshot",
    "ui.journals.verify",
    "ui.diagnostics.replay_bundle.export",
    "ui.journals.events.list",
    "ui.journals.quarantine",
    "ui.journals.transfers.list",
    "ui.journals.process",
    "ui.custody.coverage.snapshot",
    "ui.custody.lineage.snapshot",
    "ui.custody.gaps.list",
    "ui.custody.gaps.review_context",
    "ui.custody.gaps.history",
    "ui.custody.review.plan",
    "ui.custody.review.apply",
    "ui.transfers.suggest",
    "ui.transfers.list",
    "ui.transfers.pair_options",
    "ui.transfers.payouts.list",
    "ui.transfers.payouts.create",
    "ui.transfers.payouts.delete",
    "ui.transfers.pair",
    "ui.transfers.unpair",
    "ui.transfers.update",
    "ui.transfers.bulk_pair",
    "ui.transfers.dismiss",
    "ui.transfers.dismissals.list",
    "ui.transfers.dismissals.delete",
    "ui.transfers.components.list",
    "ui.transfers.components.plan",
    "ui.transfers.components.apply",
    "ui.transfers.rules.list",
    "ui.transfers.rules.create",
    "ui.transfers.rules.delete",
    "ui.transfers.rules.set_enabled",
    "ui.transfers.rules.apply",
    "ui.saved_views.list",
    "ui.saved_views.create",
    "ui.saved_views.delete",
    "ui.rates.summary",
    "ui.rates.coverage",
    "ui.rates.kraken_csv.import",
    "ui.rates.latest",
    "ui.rates.rebuild",
    "ui.maintenance.settings",
    "ui.maintenance.configure",
    "ui.maintenance.run",
    "ui.workspace.health",
    "ui.workspace.freshness.run",
    "ui.audit.evidence.summary",
    "ui.workspace.create",
    "ui.workspace.rename",
    "ui.workspace.delete",
    "ui.projects.list",
    "ui.projects.create",
    "ui.projects.select",
    "ui.backup.export",
    "ui.backup.preview",
    "ui.backup.apply",
    "ui.backup.cancel",
    "ui.secrets.init",
    "ui.secrets.change_passphrase",
    "ui.secrets.forget_cli_unlock",
    "ui.next_actions",
    "ui.review.badges",
    "ui.review.cases",
    "ui.review.plan",
    "ui.review.apply",
    "ui.review.request_input",
    "ui.wallets.utxos",
    "ui.privacy_hygiene.snapshot",
    "ui.loans.list",
    "ui.loans.link",
    "ui.loans.mark",
    "ui.loans.unmark",
    "ui.wallets.create",
    "ui.wallets.import_file",
    "ui.wallets.document_import.preview",
    "ui.wallets.document_import.import",
    "ui.wallets.import_samourai",
    "ui.wallets.ledger_preview",
    "ui.imports.list",
    "ui.imports.rollback",
    "ui.wallets.preview_descriptor",
    "ui.wallets.detect_script_types",
    "ui.wallets.identify",
    "ui.wallets.identify_onchain",
    "ui.wallets.identify_onchain.cancel",
    "ui.connections.sources",
    "ui.connections.btcpay.create",
    "ui.connections.btcpay.discover",
    "ui.connections.btcpay.key_guide",
    "ui.connections.btcpay.test",
    // Bull Bitcoin Wallet CSV import: the user-picked file becomes a wallet.
    "ui.connections.bullbitcoin_wallet.create",
    "ui.connections.exchange.status",
    "ui.connections.exchange.test",
    "ui.connections.exchange.create",
    "ui.connections.exchange.sync",
    "ui.connections.node.snapshot",
    "ui.reports.lightning_profitability",
    "ui.metadata.bip329.preview",
    "ui.metadata.bip329.import",
    "ui.metadata.bip329.export",
    "ui.exports.preview",
    "ui.reports.cross_check",
    "ui.wallets.update",
    "ui.wallets.delete",
    "ui.wallets.sync",
    "ui.freshness.status",
    "ui.freshness.configure",
    "ui.freshness.run",
    "ui.freshness.cancel",
    "ui.freshness.pause",
    "ui.freshness.resume",
    "wallets.reveal_descriptor",
    "daemon.lock",
    "daemon.unlock",
    "ai.providers.list",
    "ai.providers.get",
    "ai.providers.create",
    "ai.providers.update",
    "ai.providers.set_api_key",
    "ai.providers.move_api_key",
    "ai.providers.delete",
    "ai.providers.set_default",
    "ai.providers.clear_default",
    "ai.providers.acknowledge",
    "ai.provider_runtime.status",
    "ai.list_models",
    "ai.test_connection",
    "ai.chat",
    "ai.chat.cancel",
    "ai.tool_call.consent",
    "ai.chat.permission_mode",
    "ui.chat.attachments.upload",
    "ui.chat.sessions.list",
    "ui.chat.sessions.get",
    "ui.chat.sessions.delete",
    "ui.chat.sessions.clear",
    "ui.chat.history.configure",
    "ui.source_funds.preview",
    "ui.source_funds.review_context",
    "ui.source_funds.request_input",
    "ui.source_funds.cases.save",
    "ui.source_funds.cases.list",
    "ui.source_funds.sources.list",
    "ui.source_funds.sources.create",
    "ui.source_funds.sources.attach",
    "ui.source_funds.links.list",
    "ui.source_funds.links.create",
    "ui.source_funds.links.review",
    "ui.source_funds.links.bulk_review",
    "ui.source_funds.links.attach",
    "ui.source_funds.suggest",
    "ui.source_funds.assemble",
    "ui.source_funds.evidence.list",
    "ui.source_funds.export_pdf",
    "ui.source_funds.export_bundle",
    "ui.source_funds.coverage",
    "ui.source_funds.recipients.list",
    "ui.source_funds.recipients.create",
    "ui.source_funds.recipients.update",
    "ui.source_funds.recipients.delete",
    "ui.btcpay.provenance.sync",
    "ui.btcpay.provenance.list",
    "ui.btcpay.provenance.suggest",
    "ui.btcpay.provenance.links",
    "ui.btcpay.provenance.review",
    "ui.transactions.commercial_context",
    "ui.documents.list",
    "ui.documents.create",
    "ui.documents.attach",
    "ui.documents.import_report",
    "ui.logs.snapshot",
    "ui.agent_access.status",
    "ui.agent_access.configure",
    "ui.agent_access.unlock",
    "ui.agent_access.lock",
    "ui.agent_access.pairing",
];

pub const STREAMING_DAEMON_KINDS: &[&str] = &[
    "ai.chat",
    "ui.wallets.sync",
    "ui.freshness.run",
    "ui.workspace.freshness.run",
    "ui.journals.process",
    "ui.rates.rebuild",
    "ui.source_funds.assemble",
    "ui.wallets.document_import.preview",
    "ui.wallets.document_import.import",
    "ui.backup.export",
    "ui.backup.preview",
    "ui.backup.apply",
    "ui.review.plan",
    "ui.review.apply",
    "ui.wallets.identify_onchain",
];

pub const AI_RUNTIME_KINDS: &[&str] = &[
    "ai.provider_runtime.status",
    "ai.list_models",
    "ai.test_connection",
    "ui.wallets.document_import.preview",
    "ai.chat",
    "ai.chat.cancel",
    "ai.tool_call.consent",
    "ai.chat.permission_mode",
    "ui.chat.attachments.upload",
];

/// Kinds the app may send while it is locked: the lock and unlock themselves, the project
/// catalog the lock screen's book switch uses, and the machine-wide offline switch. Anything else
/// waits for the unlock (the web mounts no screen behind its lock screen); a plaintext book would
/// otherwise reopen itself on the first read.
pub const ALLOWED_WHILE_LOCKED: &[&str] = &[
    "daemon.lock",
    "daemon.unlock",
    "ui.projects.list",
    "ui.projects.select",
    "ui.network.offline",
    "ui.network.offline.set",
];

/// `daemon_invoke`'s two refusals, in its order.
pub fn check(kind: &str, ai_features: bool) -> Result<(), DaemonError> {
    if !ALLOWED_DAEMON_KINDS.contains(&kind) {
        return Err(DaemonError::new(
            "kind_not_allowed",
            format!("daemon kind {kind:?} is not allowed by the desktop app"),
        )
        .hint("Add the kind to ALLOWED_DAEMON_KINDS (crates/kassiber-desktop/src/kinds.rs) before the desktop sends it.")
        .details(serde_json::json!({"kind": kind})));
    }
    if AI_RUNTIME_KINDS.contains(&kind) && !ai_features {
        return Err(DaemonError::new(
            "ai_features_disabled",
            "AI features are disabled in Settings.",
        )
        .hint("Enable AI features in Settings before invoking AI runtime kinds.")
        .details(serde_json::json!({"kind": kind})));
    }
    Ok(())
}

pub fn is_streaming(kind: &str) -> bool {
    STREAMING_DAEMON_KINDS.contains(&kind)
}

/// Idempotent reads (supervisor.rs `timeout_is_safe_to_retry`, plus `status` and `.get`): the
/// reads a book refresh cycle holds back until the whole screen can swap at once.
pub fn is_read(kind: &str) -> bool {
    matches!(
        kind,
        "status"
            | "ui.audit.evidence.summary"
            | "ui.backends.options"
            | "ui.backends.public_defaults"
            | "ui.documents.list"
            | "ui.egress.snapshot"
            | "ui.freshness.status"
            | "ui.logs.snapshot"
            | "ui.network.offline"
            | "ui.next_actions"
            | "ui.overview.snapshot"
            | "ui.profiles.snapshot"
            | "ui.profiles.settings"
            | "ui.activity.history"
            | "ui.activity.stale"
            | "ui.reports.balance_history"
            | "ui.reports.lightning_profitability"
            | "ui.source_funds.preview"
            | "ui.transactions.commercial_context"
            | "ui.transactions.history"
            | "ui.transactions.resolve"
            | "ui.transactions.dashboard"
            | "ui.review.badges"
            | "ui.transfers.suggest"
            | "ui.workspace.health"
            | "ui.workspace.overview.snapshot"
            | "ui.wallets.utxos"
    ) || kind.ends_with(".list")
        || kind.ends_with(".snapshot")
        || kind.ends_with(".summary")
        || kind.ends_with(".coverage")
        || kind.ends_with(".quarantine")
        || kind.ends_with(".get")
}

/// Kinds that are not book-bound writes: reads, what works behind the lock, and the app's
/// machine-wide or book-less surfaces (the book set itself, onboarding, secrets, the network
/// switch, AI providers and chat, backend probes, backups, staging, cancels of running jobs and
/// the book switch). Everything else changes, or acts on, the open book.
const NOT_BOOK_BOUND: &[&str] = &[
    "daemon.",
    "ui.projects.",
    "ui.network.",
    "ai.",
    "ui.chat.",
    "ui.secrets.",
    "ui.onboarding.",
    "ui.agent_access.",
    "ui.backends.",
    "ui.backup.",
    "internal.",
    "wallets.reveal_",
    "ui.logs.",
    "ui.egress.",
];

/// A write bound to the open book: it carries `expected_scope` (`crate::app::scope_args`).
pub fn is_book_mutation(kind: &str) -> bool {
    !is_read(kind)
        && !ALLOWED_WHILE_LOCKED.contains(&kind)
        && !NOT_BOOK_BOUND.iter().any(|p| kind.starts_with(p))
        && !kind.ends_with(".cancel")
        && kind != "ui.profiles.switch"
}

#[cfg(test)]
mod tests;
