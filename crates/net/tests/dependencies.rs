//! Pin the reviewed ASCII-only URL dependency surface, not just URL behavior.
#[test]
fn bdk_wallet_uses_pinned_pure_state_without_transport_or_file_store() {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "kassiber-wallets",
            "--edges",
            "normal",
            "--prefix",
            "none",
            "--locked",
            "--offline",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let tree = String::from_utf8(output.stdout).unwrap();
    for required in ["bdk_wallet v3.0.0", "bdk_chain v0.23.3", "bdk_core v0.6.3"] {
        assert!(
            tree.lines()
                .any(|line| line == required || line == format!("{required} (*)")),
            "{required}"
        );
    }
    for line in tree.lines() {
        let name = line.split_whitespace().next().unwrap_or_default();
        assert!(
            ![
                "bdk_electrum",
                "bdk_esplora",
                "electrum-client",
                "esplora-client",
                "bdk_file_store",
                "reqwest",
                "ureq"
            ]
            .contains(&name),
            "ungated BDK dependency: {name}"
        );
    }
}

/// LWK reaches chain data only through kassiber-net: no LWK client feature,
/// no protocol crate, and no socket-capable runtime feature in its tree.
#[test]
fn lwk_wollet_is_pinned_without_clients_or_network_features() {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            "kassiber-wallets",
            "--edges",
            "normal,features",
            "--prefix",
            "none",
            "--locked",
            "--offline",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let tree = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = tree.lines().map(|l| l.trim_end_matches(" (*)")).collect();
    for required in ["lwk_wollet v0.18.0", "lwk_common v0.18.1"] {
        assert!(lines.contains(&required), "{required}");
    }
    for line in &lines {
        let name = line.split_whitespace().next().unwrap_or_default();
        assert!(
            ![
                "electrum-client",
                "esplora-client",
                "reqwest",
                "ureq",
                "hyper",
                "tokio-tungstenite",
                "tungstenite",
                "bitcoincore-rpc",
                "jsonrpc",
                "socket2",
                "mio",
                "lwk_jade",
                "lwk_signer",
                "lwk_registry",
                "lwk_prices",
                "lwk_boltz",
            ]
            .contains(&name),
            "network-capable dependency in the LWK tree: {name}"
        );
        // Default features are off, so lwk_wollet enables none of its own.
        assert!(!line.starts_with("lwk_wollet feature"), "{line}");
        if let Some(feature) = line.strip_prefix("lwk_common feature ") {
            assert!(["\"amp0\"", "\"default\""].contains(&feature), "{line}");
        }
        if let Some(feature) = line.strip_prefix("tokio feature ") {
            assert!(
                [
                    "\"macros\"",
                    "\"rt\"",
                    "\"rt-multi-thread\"",
                    "\"sync\"",
                    "\"time\"",
                    "\"tokio-macros\""
                ]
                .contains(&feature),
                "{line}"
            );
        }
    }
}

#[test]
fn ascii_url_dependencies_do_not_restore_icu_or_unicode_mapping() {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "--package",
            "kassiber-net",
            "--edges",
            "normal",
            "--prefix",
            "none",
            "--locked",
            "--offline",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tree = String::from_utf8(output.stdout).unwrap();
    assert!(tree
        .lines()
        .any(|line| line.starts_with("idna_adapter v1.0.")));
    for line in tree.lines() {
        let name = line.split_whitespace().next().unwrap_or_default();
        assert!(
            !name.starts_with("icu_")
                && ![
                    "idna_mapping",
                    "unicode-normalization",
                    "unicode-bidi",
                    "yoke",
                    "zerovec",
                    "zerofrom",
                    "zerotrie"
                ]
                .contains(&name),
            "unreviewed Unicode/ICU URL dependency: {name}"
        );
    }
}
