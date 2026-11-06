//! Conservative source/dependency review guard, not a sandbox or macro expander.
//! Parse Cargo TOML instead of guessing its table/feature syntax. Cargo metadata
//! separately verifies workspace declarations; neither path starts a connector.
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use toml_edit::{DocumentMut, Item, TableLike};

fn compact(source: &str) -> String {
    source.chars().filter(|c| !c.is_whitespace()).collect()
}

fn bypass(source: &str) -> bool {
    // Parsing an address literal is pure; keep every connector/import check below.
    let source = compact(source)
        .replace(
            "usestd::net::{IpAddr,Ipv4Addr,Ipv6Addr};",
            "useReviewedAddressLiteral;",
        )
        .replace(
            "parse::<std::net::Ipv4Addr>()",
            "parse::<ReviewedAddressLiteral>()",
        )
        .replace(
            "parse::<std::net::Ipv6Addr>()",
            "parse::<ReviewedAddressLiteral>()",
        )
        .replace(
            "parse::<std::net::IpAddr>()",
            "parse::<ReviewedAddressLiteral>()",
        )
        .replace("std::net::IpAddr::V4", "ReviewedV4Literal")
        .replace("std::net::IpAddr::V6", "ReviewedV6Literal");
    [
        "TcpStream",
        "TcpListener",
        "TcpSocket",
        "UdpSocket",
        "ToSocketAddrs",
        "to_socket_addrs",
        "reqwest",
        "ureq",
        "tokio::net",
        "socket2",
        "getaddrinfo",
        "gethostbyname",
        "std::net",
        "::net::",
        "::net{",
        "::netas",
        "::net;",
        "{net::",
        "{net,",
        "{net}",
        "{netas",
        ",net::",
        ",net,",
        ",net}",
        ",netas",
        "hyper",
        "isahc",
        "surf::",
        "attohttpc",
        "rustls",
        "native_tls",
        "openssl::ssl",
        "tungstenite",
        "websocket",
        "trust_dns",
        "hickory",
        "libc::socket",
        "libc::connect",
        "libc::send",
        "libc::*",
        "libc::{",
        "syscall",
        "nix::sys::socket",
        "WinSock",
        "WinHttp",
        "WinInet",
        "NSURLSession",
        ".import(",
        "::import(",
        ".import_bound(",
        "::import_bound(",
        "Command::new",
        "tokio/net",
        "tokio?/net",
        "tokio/full",
    ]
    .iter()
    .any(|needle| source.contains(needle))
        || (source.contains("tokio")
            && (source.contains("net::")
                || source.contains("netas")
                || source.contains("\"net\"")
                || source.contains("\"full\"")))
}

fn reviewed() -> BTreeMap<&'static str, Vec<&'static str>> {
    include_str!("reviewed-dependencies.txt")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let mut words = line.split_whitespace();
            (words.next().unwrap(), words.collect())
        })
        .collect()
}

fn allowed_feature(dependency: &str, feature: &str) -> bool {
    reviewed()
        .get(dependency)
        .is_some_and(|features| features.contains(&feature))
}

fn forwarded_feature(value: &str) -> bool {
    if let Some((dependency, feature)) = value.split_once('/') {
        !allowed_feature(dependency.trim_end_matches('?'), feature)
    } else if let Some(dependency) = value.strip_prefix("dep:") {
        !reviewed().contains_key(dependency)
    } else {
        false
    }
}

fn dependency_invalid(name: &str, item: &Item, own: bool) -> bool {
    const NET_ONLY: &[&str] = &[
        "httparse",
        "idna_adapter",
        "rustls",
        "rustls-native-certs",
        "url",
        "webpki-roots",
        "toml_edit",
    ];
    if !reviewed().contains_key(name) || (!own && NET_ONLY.contains(&name)) {
        return true;
    }
    if let Some(table) = item.as_table_like() {
        if table.get("package").is_some()
            || table.get("git").is_some()
            || table.get("registry").is_some()
        {
            return true;
        }
        if let Some(features) = table.get("features") {
            let Some(features) = features.as_array() else {
                return true;
            };
            return features.iter().any(|feature| {
                feature
                    .as_str()
                    .is_none_or(|feature| !allowed_feature(name, feature))
            });
        }
    } else if item.as_str().is_none() {
        return true;
    }
    false
}

fn tables_invalid(table: &dyn TableLike, own: bool) -> bool {
    table.iter().any(|(key, item)| {
        if matches!(key, "patch" | "replace" | "source") {
            return true;
        }
        if matches!(
            key,
            "dependencies" | "dev-dependencies" | "build-dependencies"
        ) {
            return item.as_table_like().is_none_or(|deps| {
                deps.iter()
                    .any(|(name, dep)| dependency_invalid(name, dep, own))
            });
        }
        if key == "features" {
            return item.as_table_like().is_none_or(|features| {
                features.iter().any(|(_, values)| {
                    values.as_array().is_none_or(|values| {
                        values
                            .iter()
                            .any(|value| value.as_str().is_none_or(forwarded_feature))
                    })
                })
            });
        }
        item.as_table_like()
            .is_some_and(|nested| tables_invalid(nested, own))
    })
}

fn unreviewed_dependencies(source: &str) -> bool {
    manifest_invalid(source, false)
}

fn manifest_invalid(source: &str, own: bool) -> bool {
    source
        .parse::<DocumentMut>()
        .map_or(true, |doc| tables_invalid(doc.as_table(), own))
}

// File, enclosing function, exact call line. Each entry grants one occurrence,
// not permission to launch arbitrary children elsewhere in a reviewed file.
fn source_without_reviewed_children(relative: &Path, source: &str) -> String {
    let calls = include_str!("reviewed-process-calls.txt");
    let mut remaining: Vec<_> = calls
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .collect();
    let mut function = "";
    source
        .lines()
        .map(|line| {
            if let Some((_, definition)) = line.split_once("fn ") {
                function = definition.split(['(', '<']).next().unwrap().trim();
            }
            let entry = format!(
                "{}|{}|{}",
                relative
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/"),
                function,
                line.trim()
            );
            if let Some(index) = remaining.iter().position(|allowed| *allowed == entry) {
                remaining.remove(index);
                line.replacen("Command::new", "ReviewedChild", 1)
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn scan(path: &Path, own: &Path, failures: &mut Vec<String>) {
    walk(path, path, own, false, failures);
}

fn walk(path: &Path, root: &Path, own: &Path, in_crate: bool, failures: &mut Vec<String>) {
    // Only exact generated checkout roots are excluded, never arbitrary nested
    // directories named build/dist/vendor (or even target/node_modules).
    let relative = path.strip_prefix(root).unwrap();
    if [
        ".git",
        ".venv",
        "target",
        "ui-tauri/node_modules",
        "ui-tauri/dist",
        "ui-tauri/src-tauri/target",
    ]
    .iter()
    .any(|name| relative == Path::new(name))
    {
        return;
    }
    let manifest = path.file_name().is_some_and(|name| name == "Cargo.toml");
    let cargo_config = matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("config" | "config.toml")
    ) && path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == ".cargo");
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && !in_crate
                && !manifest
                && !cargo_config =>
        {
            return
        }
        Err(_) => {
            failures.push(format!(
                "{}: could not inspect covered file",
                path.display()
            ));
            return;
        }
    };
    if metadata.file_type().is_symlink() {
        failures.push(format!("{}: symlink refused", path.display()));
        return;
    }
    if metadata.is_dir() {
        let manifest = path.join("Cargo.toml");
        let package = fs::symlink_metadata(&manifest).is_ok_and(|meta| meta.is_file())
            && fs::read_to_string(&manifest)
                .ok()
                .and_then(|source| source.parse::<DocumentMut>().ok())
                .is_some_and(|doc| doc.contains_key("package"));
        let in_crate = in_crate || package;
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !in_crate => return,
            Err(_) => {
                failures.push(format!("{}: could not inspect directory", path.display()));
                return;
            }
        };
        for entry in entries {
            walk(&entry.unwrap().path(), root, own, in_crate, failures);
        }
    } else if metadata.is_file() {
        if manifest || cargo_config {
            let source = fs::read_to_string(path).unwrap();
            if manifest_invalid(&source, path.parent() == Some(own)) {
                failures.push(path.display().to_string());
            }
        } else if in_crate
            && !path.starts_with(own)
            && path.file_name().is_none_or(|name| name != "Cargo.lock")
        {
            let bytes = fs::read(path).unwrap();
            let source = String::from_utf8_lossy(&bytes);
            let source = source_without_reviewed_children(relative, &source);
            let literal_checked = source
                .replace(
                    "use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};",
                    "use ReviewedAddressLiteral;",
                )
                .replace(
                    "parse::<std::net::Ipv4Addr>()",
                    "parse::<ReviewedAddressLiteral>()",
                )
                .replace(
                    "parse::<std::net::Ipv6Addr>()",
                    "parse::<ReviewedAddressLiteral>()",
                )
                .replace(
                    "parse::<std::net::IpAddr>()",
                    "parse::<ReviewedAddressLiteral>()",
                )
                .replace("std::net::IpAddr::V4", "ReviewedV4Literal")
                .replace("std::net::IpAddr::V6", "ReviewedV6Literal");
            let net_identifier = path.extension().is_some_and(|ext| ext == "rs")
                && literal_checked
                    .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .any(|word| word == "net");
            if bypass(&source) || net_identifier {
                failures.push(path.display().to_string());
            }
        }
    } else {
        failures.push(format!("{}: special file refused", path.display()));
    }
}

#[test]
fn cargo_metadata_dependency_features_are_reviewed() {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
            "--locked",
        ])
        .current_dir(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap(),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    for package in metadata["packages"].as_array().unwrap() {
        for dep in package["dependencies"].as_array().unwrap() {
            let name = dep["name"].as_str().unwrap();
            assert!(dep["rename"].is_null(), "dependency alias needs review");
            assert!(
                reviewed().contains_key(name),
                "unreviewed dependency {name}"
            );
            for feature in dep["features"].as_array().unwrap() {
                assert!(
                    allowed_feature(name, feature.as_str().unwrap()),
                    "unreviewed {name}/{feature}"
                );
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
    assert!(!bypass("use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};"));
    assert!(bypass(
        "use std::net::{IpAddr, Ipv4Addr, Ipv6Addr}; std::net::TcpStream::connect(addr)"
    ));
    assert!(!bypass("address.parse::<std::net::Ipv4Addr>().is_ok()"));
    assert!(!bypass("address.parse::<std::net::Ipv6Addr>().is_ok()"));
    assert!(!bypass("address.parse::<std::net::IpAddr>().is_ok()"));
    assert!(!bypass("match ip {std::net::IpAddr::V4(v)=>v.is_private(),std::net::IpAddr::V6(v)=>v.is_loopback()}"));
    assert!(bypass(
        "std::net::IpAddr::V4(ip); std::net::TcpStream::connect(addr)"
    ));
    assert!(bypass(
        "address.parse::<std::net::IpAddr>(); std::net::TcpStream::connect(addr)"
    ));
    assert!(bypass(
        "address.parse::<std::net::Ipv6Addr>(); std::net::TcpStream::connect(addr)"
    ));
    assert!(bypass(
        "address.parse::<std::net::Ipv4Addr>(); std::net::TcpStream::connect(addr)"
    ));
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
        "[package]\nname = \"example\"\nversion = \"0.1.0\"\n[dependencies]\nnew-network = \"1\"\n",
    )
    .unwrap();
    let mut failures = Vec::new();
    scan(&root, &root.join("net"), &mut failures);
    fs::remove_dir_all(root).unwrap();
    assert_eq!(failures.len(), 2);
}

#[test]
fn scanner_rejects_reviewed_evasion_forms() {
    let mut missed = Vec::new();
    for source in [
        "use libc::*; unsafe { socket(2,1,0); connect(fd, addr, len); }",
        "use libc::{socket as open};",
        "syscall(SYS_socket)",
        "nix::sys::socket::socket()",
        "WinHttpOpen()",
        "WinInet",
        "NSURLSession",
        "py.import(\"urllib.request\")",
        "std::process::Command::new(\"curl\")",
    ] {
        if !bypass(source) {
            missed.push(source);
        }
    }
    for source in [
        "[features]\nx = [\"tokio/net\"]",
        "[dependencies]\nnix = { version = \"0.30\", features = [\"socket\"] }",
        "[dependencies]\nwindows-sys = { version = \"0.61\", features = [\"Win32_Networking_WinHttp\"] }",
        "[patch.crates-io]\nlibc = { path = \"vendor/libc\" }",
        "[replace]\n\"libc:0.2.0\" = { path = \"vendor/libc\" }",
        "[source.crates-io]\nreplace-with = \"alternate\"",
    ] { if !unreviewed_dependencies(source) { missed.push(source); } }
    assert!(missed.is_empty(), "missed bypasses: {missed:?}");
}

#[test]
fn scanner_finds_includes_nested_build_directories_and_symlinks() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname = \"example\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(
        root.path().join("lib.rs"),
        "#[path = \"x.inc\"] mod x; include!(\"other.code\");",
    )
    .unwrap();
    for name in [
        "x.inc",
        "other.code",
        "build/x.rs",
        "dist/x.rs",
        "vendor/x.rs",
        "nested/build/x.rs",
    ] {
        let path = root.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "std::net::TcpStream::connect(\"localhost:9\");").unwrap();
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink("x.inc", root.path().join("linked")).unwrap();
    let mut failures = Vec::new();
    scan(root.path(), &root.path().join("net"), &mut failures);
    assert_eq!(
        failures.len(),
        if cfg!(unix) { 7 } else { 6 },
        "{failures:?}"
    );
}

#[test]
fn cargo_override_configs_and_forwarded_features_fail_in_real_tree_walk() {
    for source in [
        "[source.crates-io]\nreplace-with = 'vendor'\n[source.vendor]\ndirectory = 'vendor'",
        "[patch.crates-io]\nserde = { path = 'vendor' }",
        "[replace]\n'serde:1.0.0' = { path = 'vendor' }",
    ] {
        for filename in ["Cargo.toml", ".cargo/config", ".cargo/config.toml"] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join(filename);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, source).unwrap();
            let mut failures = Vec::new();
            scan(root.path(), &root.path().join("net"), &mut failures);
            assert_eq!(failures.len(), 1, "{filename}: {source}");
        }
    }
    for source in [
        "[features]\nx = [\n  'tokio/net',\n]",
        "[features]\nx = ['nix?/socket']",
        "[dependencies.nix]\nversion = '0.30'\nfeatures = ['net']",
        "[dependencies.windows-sys]\nversion = '0.61'\nfeatures = ['Win32_Networking_WinInet']",
        "[workspace.dependencies]\nnix = { version = '0.30', features = ['socket'] }",
    ] {
        assert!(unreviewed_dependencies(source), "{source}");
    }
    assert!(!unreviewed_dependencies(
        "[dependencies.nix]\nversion = '0.30'\nfeatures = ['fs']\n[features]\nx = ['nix/user']"
    ));
}

#[test]
fn reviewed_process_call_is_bound_to_function_file_line_and_occurrence() {
    let path = Path::new("ui-tauri/src-tauri/src/app_update.rs");
    let source = "async fn run_sidecar_update_check() {\nlet mut child = tokio::process::Command::new(&program)\n}";
    assert!(!bypass(&source_without_reviewed_children(path, source)));
    for (path, source) in [
        (Path::new("other.rs"), source.to_owned()),
        (
            path,
            source.replace("run_sidecar_update_check", "unreviewed"),
        ),
        (path, source.replace("&program", "\"curl\"")),
        (path, source.to_owned() + "\nCommand::new(\"curl\");"),
        (path, source.to_owned() + source),
    ] {
        assert!(bypass(&source_without_reviewed_children(path, &source)));
    }
}

#[test]
fn disappearing_unrelated_cache_files_do_not_mask_missing_crate_sources() {
    let root = tempfile::tempdir().unwrap();
    let own = root.path().join("net");
    let mut failures = Vec::new();
    walk(
        &root.path().join("cache.pyc.temporary"),
        root.path(),
        &own,
        false,
        &mut failures,
    );
    assert!(failures.is_empty());
    for (filename, in_crate) in [
        ("source.inc", true),
        ("Cargo.toml", false),
        (".cargo/config", false),
        (".cargo/config.toml", false),
    ] {
        walk(
            &root.path().join(filename),
            root.path(),
            &own,
            in_crate,
            &mut failures,
        );
    }
    assert_eq!(failures.len(), 4);
}
