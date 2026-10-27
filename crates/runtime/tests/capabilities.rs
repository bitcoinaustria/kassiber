//! The Rust capability tables against Python's registries
//! (`tests/data/capabilities.json`, from `scripts/gen_capabilities.py`).

use kassiber_envelope::get;
use kassiber_pyjson::{Key, Value};
use kassiber_runtime::capabilities::{
    cli_capabilities, cli_capability, daemon_capabilities, daemon_capability, Capability,
};

fn table(document: &Value, name: &str) -> Vec<(String, String)> {
    let Some(Value::Dict(pairs)) = get(document, name) else {
        panic!("missing {name}");
    };
    pairs
        .iter()
        .map(|(key, value)| match (key, value) {
            (Key::Str(k), Value::Str(v)) => (
                k.as_str().unwrap().to_owned(),
                v.as_str().unwrap().to_owned(),
            ),
            other => panic!("{other:?}"),
        })
        .collect()
}

fn check(
    expected: Vec<(String, String)>,
    actual: Vec<(&str, Capability)>,
    lookup: fn(&str) -> Option<Capability>,
) {
    assert_eq!(expected.len(), actual.len());
    for (path, capability) in expected {
        assert_eq!(
            lookup(&path).map(Capability::as_str),
            Some(capability.as_str()),
            "{path}"
        );
    }
}

#[test]
fn tables_match_python() {
    let document = kassiber_pytext::loads(include_str!("data/capabilities.json")).unwrap();
    check(
        table(&document, "cli"),
        cli_capabilities().collect(),
        cli_capability,
    );
    check(
        table(&document, "daemon"),
        daemon_capabilities().collect(),
        daemon_capability,
    );
    assert_eq!(cli_capability("status"), Some(Capability::Read));
    assert_eq!(daemon_capability("daemon.unlock"), Some(Capability::Admin));
    assert_eq!(daemon_capability("cancel"), None);
}
