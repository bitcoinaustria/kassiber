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

/// Rust-only tools (docs/reference/desktop.md#sources-live-sync-and-file-snapshots):
/// Python's registry lacks them, so they are compared to their own
/// classification here and left out of the Python comparison.
const RUST_ONLY: [(&str, &str); 3] = [
    ("ui.sources.coverage", "read"),
    ("ui.sources.attest_coverage", "accounting_decisions"),
    ("ui.reports.wealth_preview", "operator"),
];

/// The reproducibility check, also Rust only (`docs/reference/rust-shell.md#rust-only-commands`),
/// is checked on its own in `the_journal_check_is_a_read_for_every_grant`.
const JOURNAL_CHECK: &str = "ui.journals.verify";

fn rust_only(entry: &str) -> bool {
    entry == JOURNAL_CHECK
        || RUST_ONLY.iter().any(|(name, _)| *name == entry)
        || CHECKPOINT_TOOLS.iter().any(|(name, ..)| *name == entry)
        || bank_statement_tool(entry)
}

fn rust_only_names() -> Vec<String> {
    mcp_tools()
        .iter()
        .filter(|tool| rust_only(tool.entry.name.as_str()))
        .map(|tool| tool.name.clone())
        .collect()
}

#[test]
fn rust_only_tools_are_exposed_at_their_grant() {
    for (entry, required) in RUST_ONLY {
        let tool = mcp_tools()
            .iter()
            .find(|tool| tool.entry.name == entry)
            .unwrap_or_else(|| panic!("{entry} not exposed"));
        assert_eq!(tool.required.as_str(), required, "{entry}");
        assert!(
            !tool.egress && !tool.destructive && tool.idempotent,
            "{entry}"
        );
    }
}

/// The balance checkpoint tools (docs/reference/balance-checkpoints.md), Rust
/// only: `(entry, grant, destructive, idempotent)`. The read is listed at every
/// grant, the consented writes from operator up.
const CHECKPOINT_TOOLS: [(&str, &str, bool, bool); 3] = [
    ("ui.wallets.checkpoints.list", "read", false, true),
    ("ui.wallets.checkpoints.add", "operator", false, false),
    ("ui.wallets.checkpoints.remove", "operator", true, true),
];

#[test]
fn balance_checkpoint_tools_are_exposed_at_their_grant() {
    for (entry, required, destructive, idempotent) in CHECKPOINT_TOOLS {
        let tool = mcp_tools()
            .iter()
            .find(|tool| tool.entry.name == entry)
            .unwrap_or_else(|| panic!("{entry} not exposed"));
        assert_eq!(tool.required.as_str(), required, "{entry}");
        assert!(!tool.egress, "{entry}");
        assert_eq!(
            (tool.destructive, tool.idempotent),
            (destructive, idempotent),
            "{entry}"
        );
    }
}

const GRANTS: [&str; 3] = ["read", "operator", "accounting_decisions"];
const ACCESS: [&str; 2] = ["full", "restricted"];

#[test]
fn every_tool_is_classified_as_python_classifies_it() {
    let vectors = vectors();
    let have: Vec<Value> = mcp_tools()
        .iter()
        .filter(|tool| !rust_only(tool.entry.name.as_str()))
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
                .filter(|tool| !rust_only(tool.entry.name.as_str()))
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
    let skipped = rust_only_names();
    let definitions: Vec<Value> = tool_definitions("accounting_decisions", "full")
        .into_iter()
        .filter(|definition| match get(definition, "name") {
            Some(Value::Str(name)) => !skipped.iter().any(|n| Some(n.as_str()) == name.as_str()),
            _ => true,
        })
        .map(|mut definition| {
            // `reports_export` gained Rust-only wealth/draft/privacy options after Python froze its schema.
            if get(&definition, "name") == Some(&s("reports_export")) {
                if let Value::Dict(ref mut top) = definition {
                    if let Some((_, Value::Dict(schema))) =
                        top.iter_mut().find(|(k, _)| *k == Key::from("inputSchema"))
                    {
                        if let Some((_, Value::Dict(props))) = schema
                            .iter_mut()
                            .find(|(k, _)| *k == Key::from("properties"))
                        {
                            props.retain(|(k, _)| {
                                ![
                                    "wallets",
                                    "as_of",
                                    "include_regime",
                                    "include_valuation",
                                    "include_wallets",
                                    "privacy_mode",
                                    "draft",
                                ]
                                .iter()
                                .any(|name| *k == Key::from(*name))
                            });
                            if let Some((_, Value::Dict(report))) =
                                props.iter_mut().find(|(k, _)| *k == Key::from("report"))
                            {
                                if let Some((_, Value::List(variants))) =
                                    report.iter_mut().find(|(k, _)| *k == Key::from("enum"))
                                {
                                    variants.retain(|v| *v != s("wealth"));
                                }
                            }
                        }
                    }
                }
            }
            definition
        })
        .collect();
    assert_eq!(definitions, list(field(&vectors, "definitions")));
}

#[test]
fn refusals_name_what_the_call_needs() {
    let vectors = vectors();
    let Value::Dict(expected) = field(&vectors, "refusals") else {
        panic!()
    };
    let mut seen = 0;
    for tool in mcp_tools()
        .iter()
        .filter(|tool| !rust_only(tool.entry.name.as_str()))
    {
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

/// The bank-statement tools new in Rust (`kassiber_ai::catalog::RUST_ONLY_TOOLS`),
/// absent from Python's vectors; compared separately below.
fn bank_statement_tool(entry: &str) -> bool {
    kassiber_ai::catalog::RUST_ONLY_TOOLS.contains(&entry)
}

#[test]
fn rust_only_bank_statement_tools_follow_the_grant() {
    let tools: Vec<_> = mcp_tools()
        .iter()
        .filter(|tool| bank_statement_tool(tool.entry.name.as_str()))
        .collect();
    assert_eq!(tools.len(), 3);
    for tool in &tools {
        let read = tool.entry.name.ends_with(".suggest");
        assert_eq!(
            tool.required.as_str(),
            if read { "read" } else { "accounting_decisions" },
            "{}",
            tool.name
        );
        assert!(!tool.egress && !tool.destructive);
        assert_eq!(refusal_for(tool, "read", "full").is_some(), !read);
        assert!(refusal_for(tool, "accounting_decisions", "full").is_none());
    }
}

/// The reproducibility check is a read: every grant and access mode lists it,
/// it never egresses and never writes.
#[test]
fn the_journal_check_is_a_read_for_every_grant() {
    let tool = mcp_tools()
        .iter()
        .find(|tool| tool.name.as_str() == "journals_verify")
        .expect("journals_verify");
    assert_eq!(tool.entry.name.as_str(), "ui.journals.verify");
    assert_eq!(tool.required.as_str(), "read");
    assert!(!tool.egress && !tool.destructive && !tool.full_access_only);
    for grant in GRANTS {
        for access in ACCESS {
            assert!(
                available_tools(grant, access).any(|t| t.name.as_str() == "journals_verify"),
                "{grant}/{access}"
            );
        }
    }
}
