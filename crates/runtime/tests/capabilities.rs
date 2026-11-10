//! The Rust capability tables against Python's registries
//! (`tests/data/capabilities.json`, from `scripts/gen_capabilities.py`).

use kassiber_envelope::get;
use kassiber_pyjson::{Key, Value};
use kassiber_runtime::capabilities::{
    cli_capabilities, cli_capability, daemon_capabilities, daemon_capability, Capability,
};

fn table(document: &Value, name: &str) -> Vec<(String, String)> {
    let Some(Value::Dict(pairs)) = get(document, name) else {
        panic!("missing {name}");
    };
    pairs
        .iter()
        .map(|(key, value)| match (key, value) {
            (Key::Str(k), Value::Str(v)) => (
                k.as_str().unwrap().to_owned(),
                v.as_str().unwrap().to_owned(),
            ),
            other => panic!("{other:?}"),
        })
        .collect()
}

fn check(
    expected: Vec<(String, String)>,
    actual: Vec<(&str, Capability)>,
    lookup: fn(&str) -> Option<Capability>,
) {
    assert_eq!(expected.len(), actual.len());
    for (path, capability) in expected {
        assert_eq!(
            lookup(&path).map(Capability::as_str),
            Some(capability.as_str()),
            "{path}"
        );
    }
}

#[test]
fn tables_match_python() {
    let document = kassiber_pytext::loads(include_str!("data/capabilities.json")).unwrap();
    // Rust-only commands (docs/reference/rust-shell.md#rust-only-commands)
    // carry their siblings' capability; Python has no entry for them.
    let rust_only = [("wallets.sync-bitpanda", "wallets.sync-binance")];
    for (leaf, sibling) in rust_only {
        assert_eq!(cli_capability(leaf), cli_capability(sibling), "{leaf}");
    }
    check(
        table(&document, "cli"),
        cli_capabilities()
            .filter(|(path, _)| !rust_only.iter().any(|(leaf, _)| leaf == path))
            .collect(),
        cli_capability,
    );
    // The desktop's exchange connection kinds are Rust only too
    // (docs/reference/exchange-desktop.md): the BTCPay connection kinds'
    // capabilities, a read for the status and operator for the rest.
    let rust_only_kinds = [
        (
            "ui.connections.exchange.status",
            "ui.connections.btcpay.key_guide",
        ),
        ("ui.connections.exchange.test", "ui.connections.btcpay.test"),
        (
            "ui.connections.exchange.create",
            "ui.connections.btcpay.create",
        ),
        ("ui.connections.exchange.sync", "ui.connections.btcpay.test"),
    ];
    for (kind, sibling) in rust_only_kinds {
        assert!(daemon_capability(kind).is_some(), "{kind}");
        assert_eq!(
            daemon_capability(kind),
            daemon_capability(sibling),
            "{kind}"
        );
    }
    check(
        table(&document, "daemon"),
        daemon_capabilities()
            .filter(|(kind, _)| {
                !rust_only_kinds
                    .iter()
                    .any(|(rust_only, _)| rust_only == kind)
            })
            .collect(),
        daemon_capability,
    );
    assert_eq!(cli_capability("status"), Some(Capability::Read));
    assert_eq!(daemon_capability("daemon.unlock"), Some(Capability::Admin));
    assert_eq!(daemon_capability("cancel"), None);
}
