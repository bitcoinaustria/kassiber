//! The desktop app's boundaries agree with the daemon.
//!
//! The desktop app's request allowlists (`crates/kassiber-desktop/src/kinds.rs`:
//! `ALLOWED_DAEMON_KINDS`, `STREAMING_DAEMON_KINDS`, `AI_RUNTIME_KINDS`) are the
//! desktop boundary: they may name only kinds the daemon supports, and must
//! carry the kinds the desktop's flows depend on. The app's own tests
//! (`src/kinds/tests.rs`) check that every kind it sends is allowed, and
//! `src/wallets/add/meta.rs` checks its connection catalog against the wallet
//! kinds and source formats the daemon accepts.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The double-quoted strings between `start` and the first `end` after it.
fn quoted_list(text: &str, start: &str, end: &str) -> BTreeSet<String> {
    let body = text
        .split_once(start)
        .unwrap_or_else(|| panic!("{start} not found"))
        .1
        .split_once(end)
        .unwrap_or_else(|| panic!("{end} not found after {start}"))
        .0;
    body.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// The quoted kinds of one `pub const NAME: &[&str] = &[ … ];` in the desktop
/// app's `kinds.rs`, line comments dropped.
fn desktop_list(name: &str) -> BTreeSet<String> {
    let text = read("crates/kassiber-desktop/src/kinds.rs");
    let start = text
        .find(&format!("pub const {name}: &[&str] = &["))
        .unwrap_or_else(|| panic!("{name} not found in the desktop kinds.rs"));
    let body = &text[start..];
    let end = body.find("\n];").expect("list end");
    body[..end]
        .lines()
        .skip(1)
        .map(|line| line.split("//").next().unwrap_or(""))
        .flat_map(|line| {
            line.split('"')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn desktop_allowlist() -> BTreeSet<String> {
    desktop_list("ALLOWED_DAEMON_KINDS")
}

#[test]
fn desktop_stream_and_ai_lists_are_allowed_kinds() {
    let allowed = desktop_allowlist();
    for name in ["STREAMING_DAEMON_KINDS", "AI_RUNTIME_KINDS"] {
        let extra: Vec<String> = desktop_list(name).difference(&allowed).cloned().collect();
        assert!(
            extra.is_empty(),
            "{name} names kinds outside ALLOWED_DAEMON_KINDS: {extra:?}"
        );
    }
}

fn supported() -> BTreeSet<String> {
    kassiber_daemon::supported_kinds()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

const DESKTOP_KINDS: &[&str] = &[
    // Mutations.
    "ui.backends.options",
    "ui.backends.public_defaults",
    "ui.backends.btcpay.test",
    "ui.backends.electrum.test",
    "ui.backends.certificate.fetch",
    "ui.backends.certificate.trust",
    "ui.transactions.metadata.update",
    "ui.onboarding.complete",
    "ui.wallets.create",
    "ui.wallets.preview_descriptor",
    "ui.profiles.create",
    "ui.profiles.rename",
    "ui.profiles.switch",
    "ui.profiles.reset_data",
    "ui.workspace.create",
    "ui.workspace.rename",
    "ui.connections.sources",
    "ui.connections.btcpay.create",
    "ui.connections.btcpay.discover",
    "ui.connections.btcpay.test",
    "ui.connections.bullbitcoin_wallet.create",
    "ui.metadata.bip329.import",
    "ui.rates.kraken_csv.import",
    "ui.rates.latest",
    "ui.rates.rebuild",
    "ui.wallets.update",
    "ui.wallets.delete",
    "wallets.reveal_descriptor",
    "ai.providers.set_api_key",
    "ai.providers.move_api_key",
    // Rate reads.
    "ui.rates.summary",
    "ui.rates.coverage",
    // Swap matching.
    "ui.transfers.suggest",
    "ui.transfers.list",
    "ui.transfers.pair_options",
    "ui.transfers.payouts.list",
    "ui.transfers.payouts.create",
    "ui.transfers.payouts.delete",
    "ui.transfers.pair",
    "ui.transfers.unpair",
    "ui.transfers.bulk_pair",
    "ui.transfers.dismiss",
    "ui.transfers.dismissals.delete",
    // The Transfers screen lists undone automatic pairings so they can be restored.
    "ui.transfers.dismissals.list",
    "ui.transfers.rules.list",
    "ui.transfers.rules.create",
    "ui.transfers.rules.delete",
    "ui.transfers.rules.set_enabled",
    "ui.transfers.rules.apply",
    "ui.saved_views.list",
    "ui.saved_views.create",
    "ui.saved_views.delete",
    // Commercial reconciliation.
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
    // RAM-only reads and lineage.
    "ui.logs.snapshot",
    "ui.egress.snapshot",
    "ui.custody.lineage.snapshot",
];

#[test]
fn desktop_allowlists_are_subset_of_daemon_supported_kinds() {
    let extra: Vec<String> = desktop_allowlist()
        .difference(&supported())
        .cloned()
        .collect();
    assert!(
        extra.is_empty(),
        "allowlisted kinds the daemon does not support: {extra:?}"
    );
}

#[test]
fn desktop_kinds_are_supported_and_allowed() {
    let (supported, desktop) = (supported(), desktop_allowlist());
    for kind in DESKTOP_KINDS {
        assert!(supported.contains(*kind), "{kind} is not a daemon kind");
        assert!(
            desktop.contains(*kind),
            "{kind} is missing from the desktop allowlist"
        );
    }
}

#[test]
fn removed_device_sync_kinds_stay_out_of_every_boundary() {
    // Device sync was removed (plan 19); wallet sync is `ui.wallets.sync`.
    for list in [supported(), desktop_allowlist()] {
        assert!(list.iter().all(|k| !k.starts_with("ui.sync.")));
    }
}

#[test]
fn document_import_stage_kind_is_privileged() {
    let kind = "internal.document_import.stage";
    assert!(supported().contains(kind));
    assert!(!desktop_allowlist().contains(kind));
}

#[test]
fn exchange_connection_kinds_are_desktop_kinds() {
    let native = read("crates/daemon/src/connections.rs");
    let kinds = quoted_list(&native, "pub const KINDS: &[&str] = &[", "];");
    let (supported, desktop) = (supported(), desktop_allowlist());
    let exchange: Vec<&String> = kinds.iter().filter(|k| k.contains("exchange")).collect();
    assert!(!exchange.is_empty(), "no exchange connection kinds parsed");
    for kind in exchange {
        assert!(supported.contains(kind), "{kind}");
        assert!(desktop.contains(kind), "{kind}");
    }
    // The exchanges the desktop app sets up with a read-only API key.
    let model = read("crates/kassiber-desktop/src/wallets/add/exchange/model.rs");
    let exchanges: Vec<String> = quoted_list(&model, "pub const KINDS: [&str; ", "];")
        .into_iter()
        .collect();
    assert_eq!(
        exchanges,
        [
            "binance",
            "bitcoinde",
            "bitfinex",
            "bitpanda",
            "bitstamp",
            "bitvavo",
            "coinbase",
            "kraken"
        ]
    );
}

#[test]
fn asset_scope_toggle_stays_out_of_ai_tools() {
    // docs/plan/24-crypto-assets.md: the desktop sets a wallet's asset scope
    // through `ui.wallets.update`; assistant tools do not touch wallet
    // configuration, so neither that kind nor the setting is an AI tool.
    assert!(desktop_allowlist().contains("ui.wallets.update"));
    let catalog = read("crates/ai/src/data/catalog.json");
    assert!(!catalog.contains("ui.wallets.update"));
    assert!(!catalog.contains("asset_scope"));
}

#[test]
fn refresh_kinds_are_stream_capable_in_desktop_boundaries() {
    let desktop = desktop_list("STREAMING_DAEMON_KINDS");
    for kind in [
        "ui.wallets.sync",
        "ui.freshness.run",
        "ui.wallets.document_import.preview",
        "ui.wallets.document_import.import",
    ] {
        assert!(
            desktop.contains(kind),
            "{kind} emits progress records and must stream"
        );
    }
}
