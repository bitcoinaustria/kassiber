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
const RUST_ONLY_SIBLINGS: &[(&str, &str)] = &[
    ("sync-bitpanda", "sync-binance"),
    ("import-binance-history", "import-river"),
    ("sync-bitvavo", "sync-bitpanda"),
    ("import-bitvavo", "import-strike"),
    ("sync-bitstamp", "sync-bitvavo"),
    ("import-bitstamp", "import-bitvavo"),
    ("sync-bitfinex", "sync-bitstamp"),
    ("import-bitfinex", "import-bitstamp"),
    ("sync-bitcoinde", "sync-bitfinex"),
    ("import-bitcoinde", "import-bitfinex"),
];
/// Rust-only leaves with a grammar of their own and the Python leaf they
/// follow: Python has nothing to compare, so they describe themselves.
const RUST_ONLY_OWN: &[(&str, &str)] = &[("import-bisq", "import-strike")];

/// Rust-only leaves with no Python sibling to be described like, each with
/// the leaf it follows and its reviewed description
/// (docs/reference/backends.md#self-signed-certificates).
const RUST_ONLY_ENTRIES: &[(&str, &str)] = &[(
    "backends.reveal-token",
    r#"{"path":["backends","trust-certificate"],"command":"backends trust-certificate","kind":"backends.trust-certificate","help":"Fetch the TLS certificate a backend's server presents (an explicit network request) and print its SHA-256 fingerprint. Nothing is trusted until you compare that fingerprint with the server and re-run with --fingerprint; the certificate is then pinned for this backend only.","effect":"mutating","capability":"admin","needs_database":true,"supports_cursor":false,"supports_dry_run":false,"scope_flags":[],"may_prompt":false,"arguments":[{"name":"name","flags":[],"positional":true,"required":true,"repeatable":false,"choices":null,"secret_input":false,"help":"Backend name"},{"name":"fingerprint","flags":["--fingerprint"],"positional":false,"required":false,"repeatable":false,"choices":null,"secret_input":false,"help":"The SHA-256 fingerprint you confirmed (colons optional); pins the certificate only if the server still presents it"}]}"#,
)];

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
    for (leaf, sibling) in RUST_ONLY_OWN {
        let at = commands
            .iter()
            .position(|c| compact(c).contains(&format!("\"wallets.{sibling}\"")))
            .expect("sibling");
        let own = leaves()
            .into_iter()
            .find(|l| l.dotted() == format!("wallets.{leaf}"))
            .expect("rust-only leaf");
        commands.insert(at + 1, describe_leaf(&own));
    }
    for (after, entry) in RUST_ONLY_ENTRIES {
        let at = commands
            .iter()
            .position(|c| compact(c).contains(&format!("\"kind\":\"{after}\"")))
            .expect("preceding leaf");
        commands.insert(at + 1, kassiber_pytext::loads(entry).unwrap());
    }
    // `--changed-rows` is a Rust-only flag, the last argument of each leaf
    // that takes it (docs/reference/imports.md#re-importing-a-newer-export).
    let flag = kassiber_pytext::loads(&format!(
        r#"{{"name":"changed_rows","flags":["--changed-rows"],"positional":false,"required":false,"repeatable":false,"choices":["skip","import"],"secret_input":false,"help":{}}}"#,
        dumps(&Value::Str(kassiber_cli::spec::CHANGED_ROWS_HELP.into()), &Options::new()).unwrap()
    ))
    .unwrap();
    for command in commands.iter_mut() {
        let Some(Value::Str(kind)) = get(command, "kind").cloned() else {
            continue;
        };
        if !kassiber_cli::spec::CHANGED_ROWS_LEAVES.contains(&kind.as_str().unwrap()) {
            continue;
        }
        let Value::List(arguments) = field(command, "arguments") else {
            panic!("arguments")
        };
        arguments.push(flag.clone());
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
    // Rust-only leaves without a Python sibling are described by their own
    // reviewed entry (`data/rust_only_describe.jsonl`), placed after the last
    // Python leaf of their command group, where the grammar appends them.
    for line in include_str!("data/rust_only_describe.jsonl").lines() {
        // Their registry flag texts are written as Python's and replaced as
        // the Python catalog's are.
        let line = line
            .replace(
                "Tax country for the book (CLI profile; currently supported: generic, at)",
                kassiber_cli::spec::tax_countries_help(),
            )
            .replace(
                "Country-policy-controlled cost-basis pool scope (currently: global).",
                kassiber_cli::spec::pool_scopes_help(),
            );
        let entry = kassiber_pytext::loads(&line).unwrap();
        let Some(Value::List(path)) = get(&entry, "path").cloned() else {
            panic!("rust-only path")
        };
        let at = commands
            .iter()
            .rposition(
                |c| matches!(get(c, "path"), Some(Value::List(p)) if p.first() == path.first()),
            )
            .expect("command group");
        commands.insert(at + 1, entry);
    }
    // Commands new in Rust describe themselves, in the Rust tree's order.
    let mut python_entries = std::mem::take(commands).into_iter();
    let mut added_new = 0;
    for leaf in leaves() {
        if kassiber_cli::spec::rust_only_new(&leaf.dotted()) {
            commands.push(describe_leaf(&leaf));
            added_new += 1;
        } else {
            commands.push(python_entries.next().expect("python entry"));
        }
    }
    commands.extend(python_entries);
    let added = kassiber_cli::spec::RUST_ONLY_LEAVES.len() + added_new;
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
