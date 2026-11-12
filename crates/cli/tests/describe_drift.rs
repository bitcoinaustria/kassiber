//! The Rust tree's catalog against Python's `commands describe`
//! (`tests/fixtures/rust_port/commands_describe.json`).
//!
//! Every leaf of the generated tree must describe exactly as Python
//! describes it, implemented or not, so a leaf ported later starts from the
//! right contract. Catalog metadata is independent of execution support.

use kassiber_cli::describe::{describe, describe_leaf, leaves, IMPLEMENTED};
use kassiber_envelope::get;
use kassiber_pyjson::{dumps, Options, Value};

/// Python's catalog with the two flag texts the Rust tax-country registry
/// writes (`docs/reference/rust-shell.md#tax-countries`), the Rust-only
/// `cli_proxy` provider kind on `ai providers create|update --kind`
/// (`docs/reference/rust-shell.md#cli-proxy-providers`), plus each
/// Rust-only leaf described as its Python sibling with the name changed
/// (`docs/reference/rust-shell.md#rust-only-commands`): every other byte
/// must still be Python's.
/// Each Rust-only leaf and the Python leaf it is described like.
const RUST_ONLY_SIBLINGS: &[(&str, &str)] = &[("sync-bitpanda", "sync-binance")];

fn python_catalog() -> Value {
    fn field<'a>(value: &'a mut Value, name: &str) -> &'a mut Value {
        let Value::Dict(items) = value else {
            panic!("{name}: not an object")
        };
        let key = kassiber_pyjson::Key::Str(name.into());
        &mut items.iter_mut().find(|(k, _)| *k == key).expect(name).1
    }
    let mut catalog = registry_catalog();
    let data = field(&mut catalog, "data");
    let Value::List(commands) = field(data, "commands") else {
        panic!("commands")
    };
    for (leaf, sibling) in RUST_ONLY_SIBLINGS {
        let at = commands
            .iter()
            .position(|c| compact(c).contains(&format!("\"wallets.{sibling}\"")))
            .expect("sibling");
        let text = compact(&commands[at]).replace(sibling, leaf);
        commands.insert(at + 1, kassiber_pytext::loads(&text).unwrap());
    }
    // The Rust-only backend kinds are extra, sorted `--kind` choices.
    for command in commands.iter_mut() {
        let Some(Value::List(arguments)) = get(command, "arguments").cloned() else {
            continue;
        };
        if !compact(command).contains("\"backends.create\"")
            && !compact(command).contains("\"backends.update\"")
        {
            continue;
        }
        let mut arguments = arguments;
        for argument in arguments.iter_mut() {
            if get(argument, "name") != Some(&Value::Str("kind".into())) {
                continue;
            }
            let Value::List(choices) = field(argument, "choices") else {
                panic!("kind choices")
            };
            for kind in kassiber_cli::spec::RUST_ONLY_BACKEND_KINDS {
                choices.push(Value::Str((*kind).into()));
            }
            choices.sort_by_key(compact);
        }
        *field(command, "arguments") = Value::List(arguments);
    }
    let added = kassiber_cli::spec::RUST_ONLY_LEAVES.len();
    let Value::Int(count) = field(data, "count") else {
        panic!("count")
    };
    *count += kassiber_pyjson::BigInt::from(added);
    catalog
}

fn registry_catalog() -> Value {
    let kind_choices = "\"choices\": [\n              \"local\",\n              \"remote\",\n              \"tee\"\n            ],";
    let text = include_str!("../../../tests/fixtures/rust_port/commands_describe.json");
    assert_eq!(
        text.matches(kind_choices).count(),
        2,
        "the provider kind choices"
    );
    let text = text
        .replace(
            "Tax country for the book (CLI profile; currently supported: generic, at)",
            kassiber_cli::spec::tax_countries_help(),
        )
        .replace(
            "Country-policy-controlled cost-basis pool scope (currently: global).",
            kassiber_cli::spec::pool_scopes_help(),
        )
        .replace(
            kind_choices,
            "\"choices\": [\"local\", \"remote\", \"tee\", \"cli_proxy\"],",
        )
        .replace(
            "local = on-machine, remote = data leaves the device, tee = encrypted attestation provider. Default: local for a loopback URL, else remote (tee is never inferred; remote still needs --acknowledge)",
            kassiber_cli::spec::AI_PROVIDER_KIND_HELP,
        )
        .replace(
            "Change the privacy kind. Unchanged by default, except that a local provider given a non-loopback --base-url becomes remote; leaving local always needs --acknowledge again",
            kassiber_cli::spec::AI_PROVIDER_KIND_CHANGE_HELP,
        );
    kassiber_pytext::loads(&text).unwrap()
}

fn compact(value: &Value) -> String {
    dumps(value, &Options::new().compact()).unwrap()
}

#[test]
fn every_leaf_describes_like_python() {
    let catalog = python_catalog();
    let data = get(&catalog, "data").unwrap();
    let Some(Value::List(python)) = get(data, "commands") else {
        panic!("no commands");
    };
    let rust = leaves();
    assert_eq!(rust.len(), python.len(), "leaf count");
    let mut unimplemented = Vec::new();
    for (leaf, expected) in rust.iter().zip(python) {
        assert_eq!(
            compact(&describe_leaf(leaf)),
            compact(expected),
            "{}",
            leaf.dotted()
        );
        if !leaf.implemented() {
            unimplemented.push(leaf.dotted());
        }
    }
    assert_eq!(unimplemented.len(), rust.len() - IMPLEMENTED.len());
    println!(
        "{} leaves implemented, {} not yet: {}",
        IMPLEMENTED.len(),
        unimplemented.len(),
        unimplemented.join(" ")
    );
}

#[test]
fn the_complete_binary_catalog_is_python_s() {
    let catalog = python_catalog();
    let data = get(&catalog, "data").unwrap();
    assert_eq!(compact(&describe(&[]).unwrap()), compact(data));
}

#[test]
fn queries_describe_unported_leaves_and_reject_unknown_paths() {
    let path = |parts: &[&str]| parts.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
    let status = describe(&path(&["status"])).unwrap();
    assert_eq!(get(&status, "count"), Some(&Value::from(1)));
    let unported = describe(&path(&["journals", "process"])).unwrap();
    assert_eq!(get(&unported, "count"), Some(&Value::from(1)));
    let unknown = describe(&path(&["nope"])).unwrap_err();
    assert_eq!(unknown.code, "unknown_command");
    assert_eq!(unknown.message, "unknown command path: nope");
}

#[test]
fn every_prefix_selects_the_python_entries_in_order() {
    let catalog = python_catalog();
    let data = get(&catalog, "data").unwrap();
    let Some(Value::List(python)) = get(data, "commands") else {
        panic!("no commands")
    };
    for leaf in leaves() {
        for length in 1..=leaf.path.len() {
            let query: Vec<String> = leaf.path[..length].iter().map(|p| (*p).into()).collect();
            let expected: Vec<Value> = python
                .iter()
                .filter(|entry| {
                    let Some(Value::List(path)) = get(entry, "path") else {
                        panic!("no path")
                    };
                    path.len() >= query.len()
                        && path
                            .iter()
                            .zip(&query)
                            .all(|(a, b)| *a == kassiber_envelope::s(b.as_str()))
                })
                .cloned()
                .collect();
            let actual = describe(&query).unwrap();
            assert_eq!(
                get(&actual, "commands"),
                Some(&Value::List(expected)),
                "{query:?}"
            );
        }
    }
}
