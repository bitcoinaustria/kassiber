//! A conservative, intentionally simple dependency/source guard for backend crates.
//! This catches direct imports/aliases too, not only a fully spelled connect call.
//! It is a review guard, not an OS sandbox or a transitive SDK audit.
use std::fs;
use std::path::Path;

fn bypass(source: &str) -> bool {
    let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    ["TcpStream", "TcpListener", "TcpSocket", "UdpSocket", "ToSocketAddrs", "to_socket_addrs", "reqwest", "ureq", "tokio::net", "socket2", "getaddrinfo", "gethostbyname", "std::net", "::net::", "::net{", "::netas", "::net;", "{net::", "{net,", "{net}", "{netas", ",net::", ",net,", ",net}", ",netas", "hyper", "isahc", "surf::", "attohttpc", "rustls", "native_tls", "openssl::ssl", "tungstenite", "websocket", "trust_dns", "hickory", "libc::socket", "libc::connect", "libc::send", "WinSock"]
        .iter().any(|needle| compact.contains(needle))
        // Conservatively refuse grouped/aliased tokio imports and the net feature.
        || (compact.contains("tokio") && (compact.contains("net::") || compact.contains("netas") || compact.contains("\"net\"") || compact.contains("\"full\"")))
}

fn unreviewed_dependencies(source: &str) -> bool {
    let reviewed: Vec<_> = include_str!("reviewed-dependencies.txt")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();
    let mut dependencies = false;
    for line in source.lines() {
        let line = line.split('#').next().unwrap().trim();
        if line.starts_with('[') {
            if line.contains("dependencies.") {
                return true;
            }
            dependencies = line.ends_with("dependencies]");
        } else if dependencies {
            if let Some((name, _)) = line.split_once('=') {
                let name = name.trim().trim_matches('"');
                if !reviewed.contains(&name) || line.split_once('=').unwrap().1.contains("package")
                {
                    return true;
                }
            }
        }
    }
    false
}

fn scan(path: &Path, own: &Path, failures: &mut Vec<String>) {
    if path == own {
        return;
    }
    if path.is_dir() {
        if matches!(
            path.file_name().and_then(|n| n.to_str()),
            Some("target" | "node_modules" | ".git" | ".venv" | "dist" | "build" | "vendor")
        ) {
            return;
        }
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_symlink() {
                scan(&entry.path(), own, failures);
            }
        }
    } else {
        let manifest = path.file_name().is_some_and(|name| name == "Cargo.toml");
        if manifest || path.extension().is_some_and(|ext| ext == "rs") {
            let source = fs::read_to_string(path).unwrap();
            let net_identifier = !manifest
                && source
                    .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .any(|word| word == "net");
            if bypass(&source) || net_identifier || (manifest && unreviewed_dependencies(&source)) {
                failures.push(path.display().to_string());
            }
        }
    }
}

#[test]
fn other_crates_cannot_open_connections_or_depend_on_network_clients() {
    let own = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = own.parent().unwrap().parent().unwrap();
    let mut failures = Vec::new();
    // Includes future workspace crates and existing tax/Tauri crates; no source
    // exception for tests in other crates. Loopback fakes belong in kassiber-net.
    scan(root, own, &mut failures);
    assert!(
        failures.is_empty(),
        "route networking through kassiber-net: {failures:?}"
    );
}

#[test]
fn scanner_catches_common_import_and_dependency_forms() {
    for source in [
        "use std::net::TcpStream as Stream;",
        "use std::{net::{ToSocketAddrs}};",
        "use std::net::*; fn resolve() { let _ = (\"host\", 443).to_socket_addrs(); }",
        "tokio :: net :: TcpStream",
        "reqwest = { version = \"1\" }",
        "use tokio::{net as sockets};",
        "tokio = {features = [\"net\"]}",
        "ureq::get(url)",
        "use std as system; use system::{net as sockets};",
        "use std::{net};",
        "use hyper_util::client;",
        "use rustls::StreamOwned;",
        "use hickory_resolver::Resolver;",
        "use tokio_tungstenite::connect_async;",
        "use libc::connect as send;",
    ] {
        assert!(bypass(source));
    }
    assert!(!bypass("use kassiber_net::{Route, Request};"));
    for source in [
        "[dependencies]\nunknown-network = \"1\"",
        "[dependencies]\nalias = { package = \"unknown-network\", version = \"1\" }",
        "[target.'cfg(unix)'.dependencies]\nalias = \"1\"",
        "[build-dependencies]\nalias = \"1\"",
        "[dependencies]\nkassiber-new-network = \"1\"",
        "[dependencies.unknown]\nversion = \"1\"",
        "[dependencies]\nserde = { package = \"new-network\", version = \"1\" }",
    ] {
        assert!(unreviewed_dependencies(source));
    }
    assert!(!unreviewed_dependencies(
        "[dependencies]\nkassiber-net = { path = \"../net\" }"
    ));
}

#[test]
fn injected_file_and_dependency_are_found_by_the_real_tree_walk() {
    // Standard library only so this scanner can run even without Cargo's cache:
    // CARGO_MANIFEST_DIR=$PWD/crates/net rustc --test crates/net/tests/chokepoint.rs …
    let root = std::env::temp_dir().join(format!("kassiber-net-scanner-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    fs::write(root.join("bypass.rs"), "use std::{/* gap */ net as wire};").unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[dependencies]\nnew-network = \"1\"\n",
    )
    .unwrap();
    let mut failures = Vec::new();
    scan(&root, &root.join("net"), &mut failures);
    fs::remove_dir_all(root).unwrap();
    assert_eq!(failures.len(), 2);
}
