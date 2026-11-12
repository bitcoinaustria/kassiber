//! The Rust parser against Python's argparse answers
//! (`tests/data/argparse_cases.txt`, from `scripts/gen_cli_fixtures.py`).

use kassiber_cli::argparse::{parse_args, Context, Stop};
use kassiber_cli::help::{format_help, format_usage};
use kassiber_envelope::{command_path, derive_kind, get};
use kassiber_pyjson::{dumps, Key, Options, PyStr, Value};
use kassiber_pytext::loads;

const VERSION: &str = "Kassiber 0.22.78";

fn text(value: &Value) -> String {
    match value {
        Value::Str(s) => s.as_str().expect("plain text").to_owned(),
        other => panic!("expected text, got {other:?}"),
    }
}

fn number(value: &Value) -> usize {
    match value {
        Value::Int(n) => n.to_string().parse().unwrap(),
        other => panic!("expected a number, got {other:?}"),
    }
}

/// The two flags whose help the Rust tax-country registry writes
/// (`docs/reference/rust-shell.md#tax-countries`): Python's sentences and the
/// Rust ones become one placeholder, whitespace collapsed, so the rest of
/// the help still has to match.
const PYTHON_REGISTRY_HELP: [&str; 2] = [
    "Tax country for the book (CLI profile; currently supported: generic, at)",
    "Country-policy-controlled cost-basis pool scope (currently: global).",
];

fn registry_help(got: String, expected: String) -> (String, String) {
    let collapse = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let collapsed = collapse(&expected);
    if !PYTHON_REGISTRY_HELP
        .iter()
        .any(|sentence| collapsed.contains(sentence))
    {
        return (got, expected);
    }
    let rust = [
        kassiber_cli::spec::tax_countries_help(),
        kassiber_cli::spec::pool_scopes_help(),
    ];
    let mut got = collapse(&got);
    let mut expected = collapsed;
    for (python, rust) in PYTHON_REGISTRY_HELP.iter().zip(rust) {
        expected = expected.replace(python, "<registry help>");
        got = got.replace(&collapse(rust), "<registry help>");
    }
    (got, expected)
}

/// Python's `ai providers create|update --kind` help, which the Rust tree
/// replaces to name the Rust-only `cli_proxy` kind
/// (`docs/reference/rust-shell.md#cli-proxy-providers`).
const PYTHON_KIND_HELP: [&str; 2] = [
    "local = on-machine, remote = data leaves the device, tee = encrypted attestation provider. Default: local for a loopback URL, else remote (tee is never inferred; remote still needs --acknowledge)",
    "Change the privacy kind. Unchanged by default, except that a local provider given a non-loopback --base-url becomes remote; leaving local always needs --acknowledge again",
];

/// Python's answer with `cli_proxy` added to the provider kinds: the
/// choices in usage and errors, and the `--kind` help. The longer text
/// rewraps, so whitespace is collapsed on both sides; everything else
/// must still match.
fn cli_proxy_kind(got: String, expected: String) -> (String, String) {
    if !expected.contains("{local,remote,tee}") && !expected.contains("'local', 'remote', 'tee')") {
        return (got, expected);
    }
    let collapse = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut expected = collapse(&expected)
        .replace("{local,remote,tee}", "{local,remote,tee,cli_proxy}")
        .replace(
            "'local', 'remote', 'tee')",
            "'local', 'remote', 'tee', 'cli_proxy')",
        );
    let rust = [
        kassiber_cli::spec::AI_PROVIDER_KIND_HELP,
        kassiber_cli::spec::AI_PROVIDER_KIND_CHANGE_HELP,
    ];
    for (python, rust) in PYTHON_KIND_HELP.iter().zip(rust) {
        expected = expected.replace(python, rust);
    }
    (collapse(&got), expected)
}

/// The Rust-only leaves and backend kinds (`spec::RUST_ONLY_LEAVES`,
/// `spec::RUST_ONLY_BACKEND_KINDS`) are extra choices: dropped from the Rust
/// text, whitespace collapsed, the rest of the usage, help or error must
/// still match Python's.
fn rust_only(got: String, expected: String) -> (String, String) {
    let names: Vec<&str> = kassiber_cli::spec::RUST_ONLY_LEAVES
        .iter()
        .map(|leaf| leaf.rsplit('.').next().unwrap())
        .chain(kassiber_cli::spec::RUST_ONLY_BACKEND_KINDS.iter().copied())
        .collect();
    if !names
        .iter()
        .any(|name| got.contains(name) && !expected.contains(name))
    {
        return (got, expected);
    }
    let collapse = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut got = got;
    for name in names {
        got = got
            .replace(&format!(",{name},"), ",")
            .replace(&format!(",{name}}}"), "}")
            .replace(&format!(", '{name}'"), "");
    }
    (collapse(&got), collapse(&expected))
}

/// `--cost-basis-pool-scope` has no parser default in Rust (the country
/// decides it); Python's is `global`.
fn registry_default(argv: &[String], got: &str, expected: String) -> String {
    let creates = argv
        .windows(2)
        .any(|w| w[0] == "profiles" && w[1] == "create");
    let explicit = argv.iter().any(|arg| arg.starts_with("--cost"));
    if creates && !explicit && got.contains("\"cost_basis_pool_scope\":null") {
        expected.replace(
            "\"cost_basis_pool_scope\":\"global\"",
            "\"cost_basis_pool_scope\":null",
        )
    } else {
        expected
    }
}

#[test]
fn matches_python_argparse() {
    let mut counts = std::collections::BTreeMap::new();
    let mut failures = Vec::new();
    for (number_, line) in include_str!("data/argparse_cases.txt").lines().enumerate() {
        let case = loads(line).unwrap();
        let op = text(get(&case, "op").unwrap());
        *counts.entry(op.clone()).or_insert(0usize) += 1;
        let at = format!("line {}", number_ + 1);
        match op.as_str() {
            "usage" => {
                let (got, expected) = cli_proxy_kind(
                    format_usage(
                        number(get(&case, "parser").unwrap()),
                        number(get(&case, "width").unwrap()),
                    ),
                    text(get(&case, "text").unwrap()),
                );
                let (got, expected) = rust_only(got, expected);
                if got != expected {
                    failures.push(format!("{at}: usage\n got: {got:?}\nwant: {expected:?}"));
                }
            }
            "help" => {
                let (got, expected) = registry_help(
                    format_help(
                        number(get(&case, "parser").unwrap()),
                        number(get(&case, "width").unwrap()),
                    ),
                    text(get(&case, "text").unwrap()),
                );
                let (got, expected) = cli_proxy_kind(got, expected);
                let (got, expected) = rust_only(got, expected);
                if got != expected {
                    failures.push(format!("{at}: help\n got: {got:?}\nwant: {expected:?}"));
                }
            }
            "kind" => {
                let Some(Value::Dict(dests)) = get(&case, "dests") else {
                    panic!("{at}: dests");
                };
                let dests: Vec<(String, String)> = dests
                    .iter()
                    .map(|(k, v)| match k {
                        Key::Str(k) => (k.as_str().unwrap().to_owned(), text(v)),
                        other => panic!("{other:?}"),
                    })
                    .collect();
                let attr = |name: &str| {
                    dests
                        .iter()
                        .find(|(k, _)| k == name)
                        .map(|(_, v)| v.as_str())
                };
                assert_eq!(
                    derive_kind(attr, None),
                    text(get(&case, "kind").unwrap()),
                    "{at}"
                );
                assert_eq!(
                    command_path(attr),
                    text(get(&case, "command_path").unwrap()),
                    "{at}"
                );
            }
            "parse" => {
                let Some(Value::List(argv)) = get(&case, "argv") else {
                    panic!("{at}: argv");
                };
                let argv: Vec<String> = argv.iter().map(text).collect();
                let width = get(&case, "width").map_or(78, number);
                let out = get(&case, "out").unwrap();
                let got = parse_args(
                    &argv,
                    Context {
                        width,
                        version: VERSION,
                    },
                );
                let matched = match (&got, get(out, "ok"), get(out, "error"), get(out, "exit")) {
                    (Ok(namespace), Some(ok), _, _) => {
                        let dict = Value::Dict(
                            namespace
                                .items()
                                .iter()
                                .map(|(k, v)| (Key::Str(PyStr::from(k.as_str())), v.clone()))
                                .collect(),
                        );
                        let got = dumps(&dict, &Options::new().compact().sort_keys(true)).unwrap();
                        got == registry_default(&argv, &got, text(ok))
                    }
                    (Err(Stop::Error { parser, message }), _, Some(error), _) => {
                        let (got, expected) =
                            cli_proxy_kind(message.clone(), text(get(error, "message").unwrap()));
                        let (got, expected) = rust_only(got, expected);
                        *parser == number(get(error, "parser").unwrap()) && got == expected
                    }
                    (Err(Stop::Exit { stdout }), _, _, Some(exit)) => {
                        let (stdout, expected) = cli_proxy_kind(stdout.clone(), text(exit));
                        let (stdout, expected) = rust_only(stdout, expected);
                        stdout == expected
                    }
                    _ => false,
                };
                if !matched {
                    failures.push(format!("{at}: {argv:?}\n got: {got:?}\nwant: {out:?}"));
                }
            }
            other => panic!("unknown op {other}"),
        }
    }
    if !failures.is_empty() {
        let shown: Vec<&String> = failures.iter().take(8).collect();
        panic!(
            "{} mismatches; first:\n{}",
            failures.len(),
            shown
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n\n")
        );
    }
    for (op, minimum) in [
        ("usage", 1000),
        ("help", 370),
        ("kind", 300),
        ("parse", 5000),
    ] {
        assert!(
            counts.get(op).copied().unwrap_or(0) >= minimum,
            "too few {op}"
        );
    }
}
