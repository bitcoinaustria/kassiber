//! The Rust tree's catalog against Python's `commands describe`
//! (`tests/fixtures/rust_port/commands_describe.json`).
//!
//! Every leaf of the generated tree must describe exactly as Python
//! describes it, implemented or not, so a leaf ported later starts from the
//! right contract. The binary's own catalog lists only the implemented
//! leaves; the rest are reported here as unimplemented.

use kassiber_cli::describe::{describe, describe_leaf, leaves, IMPLEMENTED};
use kassiber_envelope::get;
use kassiber_pyjson::{dumps, Options, Value};

fn python_catalog() -> Value {
    let text = include_str!("../../../tests/fixtures/rust_port/commands_describe.json");
    kassiber_pytext::loads(text).unwrap()
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
fn the_binary_catalog_is_python_s_for_implemented_leaves() {
    let catalog = python_catalog();
    let data = get(&catalog, "data").unwrap();
    let Some(Value::List(python)) = get(data, "commands") else {
        panic!("no commands");
    };
    let rust = describe(&[]).unwrap();
    let Some(Value::List(commands)) = get(&rust, "commands") else {
        panic!("no commands");
    };
    let expected: Vec<String> = python
        .iter()
        .filter(|entry| match get(entry, "kind") {
            Some(Value::Str(kind)) => IMPLEMENTED.contains(&kind.as_str().unwrap()),
            _ => false,
        })
        .map(compact)
        .collect();
    let actual: Vec<String> = commands.iter().map(compact).collect();
    assert_eq!(actual, expected);
    assert_eq!(get(&rust, "count"), Some(&Value::from(IMPLEMENTED.len())));
    assert_eq!(
        compact(get(&rust, "global").unwrap()),
        compact(get(data, "global").unwrap())
    );
}

#[test]
fn queries_distinguish_unported_from_unknown() {
    let path = |parts: &[&str]| parts.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
    let status = describe(&path(&["status"])).unwrap();
    assert_eq!(get(&status, "count"), Some(&Value::from(1)));
    let unported = describe(&path(&["wallets", "sync"])).unwrap_err();
    assert_eq!(unported.code, "not_implemented");
    let unknown = describe(&path(&["nope"])).unwrap_err();
    assert_eq!(unknown.code, "unknown_command");
    assert_eq!(unknown.message, "unknown command path: nope");
}
