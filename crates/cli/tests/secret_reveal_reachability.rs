//! `wallets reveal-descriptor` prints a wallet's raw descriptors, blinding
//! key and configuration. It is a CLI leaf only: no AI tool and no MCP tool
//! names it or its daemon kind, and only the CLI's wallet family calls the
//! store method that reads the secrets.

use std::path::{Path, PathBuf};

const NAMES: [&str; 7] = [
    "wallets.reveal-descriptor",
    "wallets.reveal_descriptor",
    "wallets_reveal_descriptor",
    "ui.wallets.reveal_descriptor",
    "ui_wallets_reveal_descriptor",
    "backends.reveal-token",
    "backends.reveal_token",
];

#[test]
fn no_ai_or_mcp_tool_reveals_wallet_or_backend_secrets() {
    for name in NAMES {
        assert!(kassiber_ai::get_tool(name).is_none(), "AI tool {name}");
        let wire = kassiber_pyjson::PyStr::from(name);
        assert!(
            kassiber_mcp::tools::entry_for(&wire).is_none(),
            "MCP tool {name}"
        );
    }
    for tool in kassiber_ai::tools() {
        let kind = tool.daemon_kind.as_deref().unwrap_or_default();
        assert!(
            !kind.contains("reveal") && !tool.name.contains("reveal"),
            "AI tool {} reaches {kind}",
            tool.name
        );
    }
    for tool in kassiber_mcp::tools::mcp_tools() {
        let kind = tool.entry.daemon_kind.as_deref().unwrap_or_default();
        assert!(
            !kind.contains("reveal") && !tool.name.contains("reveal"),
            "MCP tool {} reaches {kind}",
            tool.name
        );
    }
}

fn sources(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "tests") {
                sources(&path, found);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

#[test]
fn only_the_cli_wallet_family_reads_wallet_secrets() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(crates).unwrap() {
        let source = entry.unwrap().path().join("src");
        if source.is_dir() {
            sources(&source, &mut files);
        }
    }
    let mut callers = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        let relative = file
            .strip_prefix(crates)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        for (needle, label) in [
            (".reveal_wallet_secrets(", "trait call"),
            ("kassiber_wallets::wallet_secrets(", "store read"),
        ] {
            for _ in text.matches(needle) {
                callers.push(format!("{relative}: {label}"));
            }
        }
    }
    callers.sort();
    assert_eq!(
        callers,
        [
            "cli/src/wallets.rs: trait call",
            "kassiber/src/real_store.rs: store read",
        ],
    );
}
