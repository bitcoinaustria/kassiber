//! The MCP grant registry against Python's (`tests/data/registry.json`,
//! `scripts/gen_registry.py`): every exposed tool's classification, the
//! listing and definitions at each grant and access mode, the refusals, the
//! instructions per grant, `call_capability` and client names.

use kassiber_envelope::{dict, get, s};
use kassiber_mcp::tools::{
    available_tools, call_capability, client_label, instructions_for, mcp_tools, refusal_for,
    tool_definitions,
};
use kassiber_pyjson::{Key, Value};

fn vectors() -> Value {
    kassiber_pytext::loads(include_str!("data/registry.json")).unwrap()
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap_or_else(|| panic!("missing {key}"))
}

fn text(value: &Value) -> &str {
    match value {
        Value::Str(text) => text.as_str().unwrap(),
        other => panic!("{other:?}"),
    }
}

fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        other => panic!("{other:?}"),
    }
}

const GRANTS: [&str; 3] = ["read", "operator", "accounting_decisions"];
const ACCESS: [&str; 2] = ["full", "restricted"];

#[test]
fn every_tool_is_classified_as_python_classifies_it() {
    let vectors = vectors();
    let have: Vec<Value> = mcp_tools()
        .iter()
        .map(|tool| {
            dict([
                ("name", s(tool.name.as_str())),
                ("entry", s(tool.entry.name.as_str())),
                ("required", s(tool.required.as_str())),
                ("egress", Value::Bool(tool.egress)),
                ("full_access_only", Value::Bool(tool.full_access_only)),
                ("destructive", Value::Bool(tool.destructive)),
                ("idempotent", Value::Bool(tool.idempotent)),
            ])
        })
        .collect();
    assert_eq!(have, list(field(&vectors, "tools")));
}

#[test]
fn listings_and_definitions_follow_the_grant() {
    let vectors = vectors();
    for grant in GRANTS {
        for access in ACCESS {
            let names: Vec<Value> = available_tools(grant, access)
                .map(|tool| s(tool.name.as_str()))
                .collect();
            let key = format!("{grant}/{access}");
            assert_eq!(
                names,
                list(field(field(&vectors, "listings"), &key)),
                "{key}"
            );
        }
    }
    assert_eq!(
        tool_definitions("accounting_decisions", "full"),
        list(field(&vectors, "definitions"))
    );
}

#[test]
fn refusals_name_what_the_call_needs() {
    let vectors = vectors();
    let Value::Dict(expected) = field(&vectors, "refusals") else {
        panic!()
    };
    let mut seen = 0;
    for tool in mcp_tools() {
        for grant in GRANTS {
            for access in ACCESS {
                let key = format!("{}/{grant}/{access}", tool.name);
                let want = expected
                    .iter()
                    .find(|(name, _)| *name == Key::from(key.as_str()))
                    .map(|(_, value)| value);
                match (refusal_for(tool, grant, access), want) {
                    (None, None) => {}
                    (Some(error), Some(want)) => {
                        seen += 1;
                        let have = dict([
                            ("code", s(error.code.as_str())),
                            ("message", s(error.message.as_str())),
                            ("hint", error.hint.as_deref().map_or(Value::Null, s)),
                            ("details", error.details.clone().unwrap_or(Value::Null)),
                        ]);
                        assert_eq!(&have, want, "{key}");
                    }
                    (have, want) => panic!("{key}: {have:?} vs {want:?}"),
                }
            }
        }
    }
    assert_eq!(seen, expected.len());
}

#[test]
fn instructions_capabilities_and_client_names_match_python() {
    let vectors = vectors();
    for grant in [GRANTS.as_slice(), &["admin"]].concat() {
        assert_eq!(
            instructions_for(grant),
            text(field(field(&vectors, "instructions"), grant)),
            "{grant}"
        );
    }
    for case in list(field(&vectors, "call_capability")) {
        let optional = |key: &str| match field(case, key) {
            Value::Null => None,
            value => Some(text(value).to_owned()),
        };
        let (tool, grant) = (optional("tool"), optional("grant"));
        assert_eq!(
            call_capability(tool.as_deref(), grant.as_deref()).as_str(),
            text(field(case, "capability")),
            "{case:?}"
        );
    }
    for case in list(field(&vectors, "client_labels")) {
        let have = client_label(Some(field(case, "value"))).map_or(Value::Null, s);
        assert_eq!(&have, field(case, "label"), "{case:?}");
    }
}
