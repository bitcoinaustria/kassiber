//! The desktop boundaries agree with the daemon and with each other.
//!
//! The Tauri command allowlist (`ui-tauri/src-tauri/src/lib.rs`) and the dev
//! Vite bridge (`ui-tauri/vite.config.ts`) may forward only kinds the daemon
//! supports, must forward the same kinds, and must forward every `ui.*` kind
//! the React app invokes. The connection catalog
//! (`ui-tauri/src/lib/connectionCatalog.tsx`) may offer only wallet kinds and
//! source formats the daemon accepts.

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

fn tauri_allowlist() -> BTreeSet<String> {
    let text = read("ui-tauri/src-tauri/src/lib.rs");
    let at = text
        .find("ALLOWED_DAEMON_KINDS")
        .expect("Tauri daemon allowlist");
    quoted_list(&text[at..], "&[", "];")
}

fn vite_allowlist() -> BTreeSet<String> {
    let text = read("ui-tauri/vite.config.ts");
    let at = text
        .find("ALLOWED_BRIDGE_KINDS = new Set([")
        .expect("Vite bridge allowlist");
    quoted_list(&text[at..], "new Set([", "]);")
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
    "ui.transfers.payouts.list",
    "ui.transfers.payouts.create",
    "ui.transfers.payouts.delete",
    "ui.transfers.pair",
    "ui.transfers.unpair",
    "ui.transfers.bulk_pair",
    "ui.transfers.dismiss",
    "ui.transfers.dismissals.delete",
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
fn tauri_and_vite_bridge_allowlists_match() {
    // `pnpm dev:bridge` is the browser equivalent of the Tauri boundary; a
    // Tauri-only kind must be allowlisted explicitly here, not by drift.
    assert_eq!(tauri_allowlist(), vite_allowlist());
}

#[test]
fn desktop_allowlists_are_subset_of_daemon_supported_kinds() {
    let supported = supported();
    for list in [tauri_allowlist(), vite_allowlist()] {
        let extra: Vec<&String> = list.difference(&supported).collect();
        assert!(
            extra.is_empty(),
            "allowlisted kinds the daemon does not support: {extra:?}"
        );
    }
}

#[test]
fn desktop_kinds_are_supported_and_allowed() {
    let (supported, tauri, vite) = (supported(), tauri_allowlist(), vite_allowlist());
    for kind in DESKTOP_KINDS {
        assert!(supported.contains(*kind), "{kind} is not a daemon kind");
        assert!(
            tauri.contains(*kind),
            "{kind} is missing from the Tauri allowlist"
        );
        assert!(
            vite.contains(*kind),
            "{kind} is missing from the Vite bridge allowlist"
        );
    }
}

#[test]
fn removed_device_sync_kinds_stay_out_of_every_boundary() {
    // Device sync was removed (plan 19); wallet sync is `ui.wallets.sync`.
    for list in [supported(), tauri_allowlist(), vite_allowlist()] {
        assert!(list.iter().all(|k| !k.starts_with("ui.sync.")));
    }
}

#[test]
fn document_import_stage_kind_is_privileged() {
    let kind = "internal.document_import.stage";
    assert!(supported().contains(kind));
    assert!(!tauri_allowlist().contains(kind));
    assert!(!vite_allowlist().contains(kind));
}

#[test]
fn exchange_connection_kinds_are_desktop_kinds() {
    let native = read("crates/daemon/src/connections.rs");
    let kinds = quoted_list(&native, "pub const KINDS: &[&str] = &[", "];");
    let (supported, tauri, vite) = (supported(), tauri_allowlist(), vite_allowlist());
    for kind in kinds.iter().filter(|k| k.contains("exchange")) {
        assert!(supported.contains(kind), "{kind}");
        assert!(tauri.contains(kind) && vite.contains(kind), "{kind}");
    }
    let exchanges: Vec<String> = catalog_entries()
        .iter()
        .filter(|e| field(e, "setupKind").as_deref() == Some("exchange-api"))
        .filter_map(|e| field(e, "exchange"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    assert_eq!(exchanges, ["binance", "bitpanda", "coinbase", "kraken"]);
}

#[test]
fn refresh_kinds_are_stream_capable_in_desktop_boundaries() {
    let tauri = read("ui-tauri/src-tauri/src/lib.rs");
    let at = tauri
        .find("STREAMING_DAEMON_KINDS")
        .expect("Tauri streaming list");
    let tauri = quoted_list(&tauri[at..], "&[", "];");
    let vite = read("ui-tauri/vite.config.ts");
    let at = vite
        .find("STREAM_CAPABLE_BRIDGE_KINDS")
        .expect("Vite stream-capable list");
    let vite = quoted_list(&vite[at..], "new Set([", "]);");
    for kind in [
        "ui.wallets.sync",
        "ui.freshness.run",
        "ui.wallets.document_import.preview",
        "ui.wallets.document_import.import",
    ] {
        assert!(
            tauri.contains(kind),
            "{kind} emits progress records and must stream"
        );
        assert!(
            vite.contains(kind),
            "{kind} must be stream-capable in the dev bridge"
        );
    }
}

/// `ui.*` kinds the React app invokes through `useDaemon*` hooks or an inline
/// `transport.invoke`/`stream` call, outside tests and mocks.
fn invoked_ui_kinds() -> BTreeSet<(String, String)> {
    let mut found = BTreeSet::new();
    let mut pending = vec![root().join("ui-tauri/src")];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.to_string_lossy().replace('\\', "/");
            if path.is_dir() {
                if !name.ends_with("/mocks") {
                    pending.push(path);
                }
                continue;
            }
            if !(name.ends_with(".ts") || name.ends_with(".tsx")) || name.contains(".test.") {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            for kind in invoked_in(&text) {
                found.insert((kind, name.clone()));
            }
        }
    }
    found
}

fn literal_kind(text: &str) -> Option<String> {
    let text = text.trim_start();
    let quote = text
        .chars()
        .next()
        .filter(|c| matches!(c, '"' | '\'' | '`'))?;
    let rest = &text[1..];
    let end = rest.find(quote)?;
    let kind = &rest[..end];
    let valid = kind.starts_with("ui.")
        && kind
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.');
    valid.then(|| kind.to_owned())
}

fn skip_generics(text: &str) -> &str {
    match text.strip_prefix('<') {
        Some(rest) => rest.find('>').map(|end| &rest[end + 1..]).unwrap_or(rest),
        None => text,
    }
}

fn invoked_in(text: &str) -> Vec<String> {
    let mut kinds = Vec::new();
    for hook in [
        "useDaemonStreamMutation",
        "useDaemonMutation",
        "useDaemonInfinite",
        "useDaemon",
    ] {
        for (at, _) in text.match_indices(hook) {
            let after = &text[at + hook.len()..];
            let before_ok = text[..at]
                .chars()
                .last()
                .is_none_or(|c| !c.is_alphanumeric() && c != '_');
            let after_ok = after.chars().next().is_some_and(|c| c == '(' || c == '<');
            if !(before_ok && after_ok) {
                continue;
            }
            if let Some(call) = skip_generics(after).strip_prefix('(') {
                kinds.extend(literal_kind(call));
            }
        }
    }
    for method in [".invoke", ".stream"] {
        for (at, _) in text.match_indices(method) {
            let after = skip_generics(&text[at + method.len()..]);
            let Some(call) = after.strip_prefix('(') else {
                continue;
            };
            let Some(object) = call.trim_start().strip_prefix('{') else {
                continue;
            };
            let Some(kind) = object.trim_start().strip_prefix("kind:") else {
                continue;
            };
            kinds.extend(literal_kind(kind));
        }
    }
    kinds
}

#[test]
fn invoked_ui_kinds_are_in_desktop_allowlist() {
    let invoked = invoked_ui_kinds();
    // A refactor of the client hooks would otherwise make this a no-op.
    assert!(
        invoked.len() > 50,
        "only {} invoked ui.* kinds found",
        invoked.len()
    );
    let allowed = tauri_allowlist();
    let missing: Vec<&(String, String)> = invoked
        .iter()
        .filter(|(k, _)| !allowed.contains(k))
        .collect();
    assert!(
        missing.is_empty(),
        "the desktop invokes kinds missing from ALLOWED_DAEMON_KINDS (add them to lib.rs and vite.config.ts): {missing:?}"
    );
}

#[test]
fn invocation_scan_reads_hooks_and_inline_calls() {
    let text = r#"
        useDaemonMutation<Report>("ui.wallets.identify");
        useDaemon('ui.rates.summary', {});
        getTransport("real").invoke({
            kind: "ui.secrets.init", args: {} });
        transport.stream<X>({ kind: `ui.wallets.sync` });
        notAuseDaemon("ui.not.this");
        if (envelope.kind === "ui.freshness") {}
    "#;
    let mut kinds = invoked_in(text);
    kinds.sort();
    assert_eq!(
        kinds,
        [
            "ui.rates.summary",
            "ui.secrets.init",
            "ui.wallets.identify",
            "ui.wallets.sync"
        ]
    );
}

/// The object literals of `CONNECTION_SOURCES` in the connection catalog.
fn catalog_entries() -> Vec<String> {
    let text = read("ui-tauri/src/lib/connectionCatalog.tsx");
    let start = text.find("CONNECTION_SOURCES").expect("CONNECTION_SOURCES");
    // `CONNECTION_SOURCES: ConnectionSource[] = [`: the list after the `=`.
    let body = &text[start..];
    let body = &body[body.find('=').unwrap()..];
    let body = &body[body.find('[').unwrap() + 1..];
    let mut entries = Vec::new();
    let (mut depth, mut quote, mut escape, mut begin) = (0usize, None::<char>, false, None);
    for (index, c) in body.char_indices() {
        if let Some(q) = quote {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' | '`' => quote = Some(c),
            '{' => {
                if depth == 0 {
                    begin = Some(index);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    entries.push(body[begin.take().unwrap()..=index].to_owned());
                }
            }
            ']' if depth == 0 => break,
            _ => {}
        }
    }
    entries
}

fn field(entry: &str, name: &str) -> Option<String> {
    let mut search = entry;
    while let Some(at) = search.find(name) {
        let before_ok = search[..at]
            .chars()
            .last()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_');
        let rest = search[at + name.len()..].trim_start();
        if before_ok {
            if let Some(value) = rest
                .strip_prefix(':')
                .map(str::trim_start)
                .and_then(|v| v.strip_prefix('"'))
            {
                return value.split('"').next().map(str::to_owned);
            }
        }
        search = &search[at + name.len()..];
    }
    None
}

fn wallet_kinds() -> BTreeSet<String> {
    let catalog: serde_json::Value =
        serde_json::from_str(&read("crates/wallets/src/connections_catalog.json")).unwrap();
    catalog
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["kind"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn ready_entries_reference_known_wallet_kinds() {
    let entries = catalog_entries();
    assert!(!entries.is_empty(), "no source entries parsed");
    let kinds = wallet_kinds();
    let formats: BTreeSet<&str> = kassiber_wallets::connections::SOURCE_FORMATS
        .iter()
        .copied()
        .collect();
    for entry in &entries {
        if field(entry, "status").as_deref() != Some("ready") {
            continue;
        }
        let id = field(entry, "id").unwrap_or_default();
        let setup = field(entry, "setupKind");
        // Backend-only, label-import, evidence-archive, BTCPay and exchange
        // API entries name no wallet kind; the daemon owns their target.
        if matches!(
            setup.as_deref(),
            None | Some("backend-settings" | "bip329" | "btcpay" | "prior-report" | "exchange-api")
        ) {
            continue;
        }
        let kind = field(entry, "walletKind").unwrap_or_else(|| panic!("{id} has no walletKind"));
        assert!(
            kinds.contains(&kind),
            "{id} references unknown walletKind {kind}"
        );
        if let Some(format) = field(entry, "sourceFormat") {
            assert!(
                formats.contains(format.as_str()),
                "{id} references unknown sourceFormat {format}"
            );
        }
    }
}

#[test]
fn btcpay_csv_import_is_manual_file_wallet_source() {
    let entries = catalog_entries();
    let by_id = |id: &str| {
        entries
            .iter()
            .find(|e| field(e, "id").as_deref() == Some(id))
            .cloned()
    };
    let btcpay = by_id("btcpay").expect("btcpay entry");
    assert_eq!(field(&btcpay, "setupKind").as_deref(), Some("btcpay"));
    let csv = by_id("btcpay-csv").expect("manual BTCPay CSV source");
    assert_eq!(field(&csv, "setupKind").as_deref(), Some("file-wallet"));
    assert_eq!(field(&csv, "walletKind").as_deref(), Some("custom"));
    assert_eq!(field(&csv, "sourceFormat").as_deref(), Some("btcpay_csv"));
    assert!(csv.contains("invoice and payment-request provenance"));
    assert!(wallet_kinds().contains("custom"));
}
