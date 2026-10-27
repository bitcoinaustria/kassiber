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
                let got = format_usage(
                    number(get(&case, "parser").unwrap()),
                    number(get(&case, "width").unwrap()),
                );
                let expected = text(get(&case, "text").unwrap());
                if got != expected {
                    failures.push(format!("{at}: usage\n got: {got:?}\nwant: {expected:?}"));
                }
            }
            "help" => {
                let got = format_help(
                    number(get(&case, "parser").unwrap()),
                    number(get(&case, "width").unwrap()),
                );
                let expected = text(get(&case, "text").unwrap());
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
                        dumps(&dict, &Options::new().compact().sort_keys(true)).unwrap() == text(ok)
                    }
                    (Err(Stop::Error { parser, message }), _, Some(error), _) => {
                        *parser == number(get(error, "parser").unwrap())
                            && *message == text(get(error, "message").unwrap())
                    }
                    (Err(Stop::Exit { stdout }), _, _, Some(exit)) => *stdout == text(exit),
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
