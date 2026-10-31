//! The Rust tree's catalog against Python's `commands describe`
//! (`tests/fixtures/rust_port/commands_describe.json`).
//!
//! Every leaf of the generated tree must describe exactly as Python
//! describes it, implemented or not, so a leaf ported later starts from the
//! right contract. Catalog metadata is independent of execution support.

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
