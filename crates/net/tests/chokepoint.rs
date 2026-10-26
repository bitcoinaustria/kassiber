//! A conservative, intentionally simple dependency/source guard for backend crates.
//! This catches direct imports/aliases too, not only a fully spelled connect call.
//! It is a review guard, not an OS sandbox or a transitive SDK audit.
use std::fs;
use std::path::Path;

fn bypass(source: &str) -> bool {
    let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    ["TcpStream", "TcpSocket", "UdpSocket", "ToSocketAddrs", "to_socket_addrs", "reqwest", "ureq", "tokio::net", "socket2", "getaddrinfo", "gethostbyname"]
        .iter().any(|needle| compact.contains(needle))
        // Conservatively refuse grouped/aliased tokio imports and the net feature.
        || (compact.contains("tokio") && (compact.contains("net::") || compact.contains("netas") || compact.contains("\"net\"")))
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
    } else if (path.extension().is_some_and(|ext| ext == "rs")
        || path.file_name().is_some_and(|name| name == "Cargo.toml"))
        && bypass(&fs::read_to_string(path).unwrap())
    {
        failures.push(path.display().to_string());
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
    ] {
        assert!(bypass(source));
    }
    assert!(!bypass("use kassiber_net::{Route, Request};"));
}
