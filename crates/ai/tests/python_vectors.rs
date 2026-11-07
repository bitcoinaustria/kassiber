//! The pure AI parts against Python's answers (`tests/data/vectors.json`,
//! from `scripts/gen_vectors.py`).

use std::sync::OnceLock;

use kassiber_ai::executor::{parse_tool_call, tool_result_denied};
use kassiber_ai::planner::{auto_tool_context_for_model, planned_auto_reads};
use kassiber_ai::provenance::{answer_provenance, record_tool_usage};
use kassiber_ai::redact::{redact_ai_tool_result, redact_tool_arguments};
use kassiber_ai::select::{build_responses_tools, select_tool_capabilities, tool_capabilities};
use kassiber_ai::summary::summarize_tool_call;
use kassiber_ai::validate::{validate_arguments, validate_schema_value};
use kassiber_ai::{get_tool, tools, ToolError};
use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::{PyStr, Value};

fn vectors() -> &'static Value {
    static VECTORS: OnceLock<Value> = OnceLock::new();
    VECTORS.get_or_init(|| {
        kassiber_pytext::loads(include_str!("data/vectors.json")).expect("vectors parse")
    })
}

fn section(name: &str) -> &'static [Value] {
    match get(vectors(), name) {
        Some(Value::List(items)) => items,
        _ => panic!("missing section {name}"),
    }
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap_or_else(|| panic!("missing {key} in {value:?}"))
}

fn text(value: &Value) -> &str {
    match value {
        Value::Str(text) => text.as_str().expect("UTF-8 text"),
        other => panic!("not text: {other:?}"),
    }
}

fn optional_text(value: &Value) -> Option<&str> {
    match value {
        Value::Null => None,
        other => Some(text(other)),
    }
}

fn message(error: &AppError) -> Value {
    match &error.message_codepoints {
        Some(message) => Value::Str((**message).clone()),
        None => s(error.message.as_str()),
    }
}

/// The Python `outcome()` record for a Rust result.
fn outcome(result: Result<(), ToolError>) -> Value {
    match result {
        Ok(()) => dict([("ok", Value::Null)]),
        Err(ToolError::App(error)) => dict([(
            "error",
            dict([
                ("code", s(error.code.as_str())),
                ("message", message(&error)),
                ("details", error.details.clone().unwrap_or(Value::Null)),
                ("hint", error.hint.as_deref().map(s).unwrap_or(Value::Null)),
                ("retryable", Value::Bool(error.retryable)),
            ]),
        )]),
        Err(other) => dict([("exception", s(format!("{other:?}")))]),
    }
}

fn messages(value: &Value) -> Option<&[Value]> {
    match value {
        Value::List(items) => Some(items),
        Value::Null => None,
        other => panic!("messages: {other:?}"),
    }
}

#[test]
fn tool_capabilities_match_python() {
    let Some(Value::Dict(expected)) = get(vectors(), "capabilities") else {
        panic!("capabilities");
    };
    assert_eq!(expected.len(), tools().len());
    for tool in tools() {
        let want: Vec<Value> = match get(vectors(), "capabilities").and_then(|c| get(c, &tool.name))
        {
            Some(Value::List(items)) => items.clone(),
            other => panic!("{}: {other:?}", tool.name),
        };
        let have: Vec<Value> = tool_capabilities(tool).into_iter().map(s).collect();
        assert_eq!(have, want, "{}", tool.name);
    }
}

#[test]
fn selection_matches_python() {
    for case in section("selection") {
        let screen = match field(case, "screen_context") {
            Value::Null => None,
            other => Some(other),
        };
        let profile = optional_text(field(case, "profile"));
        let have = match build_responses_tools(messages(field(case, "messages")), screen, profile) {
            Ok(selected) => {
                let indexes: Vec<Value> = selected
                    .iter()
                    .map(|tool| {
                        Value::from(
                            tools()
                                .iter()
                                .position(|entry| std::ptr::eq(entry, *tool))
                                .unwrap(),
                        )
                    })
                    .collect();
                dict([("tools", Value::List(indexes))])
            }
            Err(error) => outcome(Err(ToolError::App(error))),
        };
        assert_eq!(&have, field(case, "result"), "{case:?}");
        if profile != Some("bogus") {
            let capabilities = select_tool_capabilities(messages(field(case, "messages")), screen)
                .map(|set| Value::List(set.into_iter().map(s).collect()))
                .unwrap_or(Value::Null);
            assert_eq!(&capabilities, field(case, "capabilities"), "{case:?}");
        }
    }
}

#[test]
fn catalog_validation_matches_python() {
    for case in section("validation") {
        let tool = get_tool(text(field(case, "tool"))).unwrap();
        let have = outcome(validate_arguments(tool, field(case, "arguments")));
        assert_eq!(&have, field(case, "result"), "{case:?}");
    }
}

#[test]
fn synthetic_validation_matches_python() {
    for case in section("synthetic_validation") {
        let have = outcome(validate_schema_value(
            field(case, "value"),
            field(case, "schema"),
            "t",
        ));
        assert_eq!(&have, field(case, "result"), "{case:?}");
    }
}

#[test]
fn redaction_matches_python() {
    for case in section("redaction") {
        let value = field(case, "value");
        assert_eq!(
            &redact_tool_arguments(value),
            field(case, "arguments"),
            "{value:?}"
        );
        assert_eq!(
            &redact_ai_tool_result(value),
            field(case, "result"),
            "{value:?}"
        );
    }
}

#[test]
fn summaries_match_python() {
    for case in section("summaries") {
        let tool = get_tool(text(field(case, "tool"))).unwrap();
        match summarize_tool_call(tool, field(case, "arguments")) {
            Ok(summary) => assert_eq!(&Value::Str(summary), field(case, "summary"), "{case:?}"),
            // A call the consent card cannot describe is refused.
            Err(error) => assert_eq!(
                &outcome(Err(ToolError::App(error))),
                field(case, "outcome"),
                "{case:?}"
            ),
        }
    }
}

#[test]
fn call_parsing_and_denials_match_python() {
    for case in section("parse") {
        let index = match field(case, "index") {
            Value::Int(n) => usize::try_from(n.clone()).unwrap(),
            other => panic!("{other:?}"),
        };
        let call = parse_tool_call(field(case, "raw"), index);
        let have = dict([
            ("call_id", Value::Str(call.call_id)),
            ("name", Value::Str(call.name)),
            ("arguments", call.arguments),
            (
                "argument_error",
                call.argument_error.map(s).unwrap_or(Value::Null),
            ),
        ]);
        // Compared as JSON text: NaN is not equal to itself.
        let json = |value: &Value| {
            kassiber_pyjson::dumps(value, &kassiber_pyjson::Options::new()).unwrap()
        };
        assert_eq!(json(&have), json(field(case, "call")), "{case:?}");
    }
    for case in section("denied") {
        let message = match field(case, "message") {
            Value::Str(text) => Some(text.clone()),
            _ => None,
        };
        let have = tool_result_denied(text(field(case, "reason")), message.as_ref());
        assert_eq!(&have, field(case, "result"), "{case:?}");
    }
}

#[test]
fn planner_matches_python() {
    for case in section("planner") {
        let kind = optional_text(field(case, "system_prompt_kind"));
        let planned: Vec<Value> =
            planned_auto_reads(kind, messages(field(case, "messages")).unwrap())
                .into_iter()
                .map(|read| Value::List(vec![s(read.name), read.arguments]))
                .collect();
        assert_eq!(&Value::List(planned), field(case, "planned"), "{case:?}");
    }
}

#[test]
fn context_messages_match_python() {
    for case in section("context") {
        let Value::List(context) = field(case, "context") else {
            panic!("context");
        };
        let have = auto_tool_context_for_model(context);
        assert_eq!(have, text(field(case, "message")), "{case:?}");
    }
}

#[test]
fn provenance_matches_python() {
    for case in section("provenance") {
        let mut state = field(case, "initial").clone();
        let Value::List(steps) = field(case, "steps") else {
            panic!("steps");
        };
        for step in steps {
            let Value::List(step) = step else {
                panic!("step")
            };
            record_tool_usage(&mut state, &PyStr::from(text(&step[0])), &step[1]);
        }
        assert_eq!(&state, field(case, "state"), "{case:?}");
        let have = answer_provenance(
            field(case, "provider"),
            field(case, "validated"),
            &state,
            field(case, "egress"),
            "2026-11-05T00:00:00Z",
        );
        assert_eq!(&have, field(case, "provenance"), "{case:?}");
    }
}

#[test]
fn surrogate_text_survives_redaction() {
    let raw = PyStr::from_code_points([0xD800, 0x20, 0x2F, 0x61, 0x2F, 0x62]).unwrap();
    let redacted = redact_ai_tool_result(&Value::Str(raw));
    assert_eq!(
        redacted,
        Value::Str(
            PyStr::from_code_points(
                [0xD800, 0x20]
                    .into_iter()
                    .chain("<redacted-path>".chars().map(u32::from))
            )
            .unwrap()
        )
    );
}

#[test]
fn chain_analysis_projection_matches_python() {
    for case in section("projection") {
        let projected =
            kassiber_ai::analysis::project_ai_result(field(case, "value"), &mut |value| {
                Ok(format!("ref({value})"))
            })
            .unwrap();
        assert_eq!(&projected, field(case, "projected"), "{case:?}");
    }
}

/// The Python `outcome()` record for a Rust result with a value; a Python
/// exception other than `AppError` compares by its presence only.
fn value_outcome(result: Result<Value, ToolError>, expected: &Value) -> Value {
    match result {
        Ok(value) => dict([("ok", value)]),
        Err(ToolError::App(error)) => outcome(Err(ToolError::App(error))),
        Err(_) if get(expected, "exception").is_some() => expected.clone(),
        Err(other) => dict([("exception", s(format!("{other:?}")))]),
    }
}

#[test]
fn ai_review_operation_rules_match_python() {
    use kassiber_ai::review::validate_operations;
    for case in section("review_operations") {
        let operations = field(case, "operations");
        let operations = (!matches!(operations, Value::Null)).then_some(operations);
        let full_access = field(case, "full_access") == &Value::Bool(true);
        // The generator's book: two components and nothing else.
        let mut lookup = |id: &str| match id {
            "conv" => Ok(dict([("conservation_mode", s("conversion"))])),
            "qty" => Ok(dict([("conservation_mode", s("quantity"))])),
            _ => Err(AppError::new(
                "not_found",
                "custody component was not found",
            )),
        };
        let have = outcome(validate_operations(
            operations,
            full_access,
            Some(&mut lookup),
        ));
        assert_eq!(&have, field(case, "outcome"), "{case:?}");
    }
}

#[test]
fn report_export_targets_match_python() {
    use kassiber_ai::mutate::report_export_target;
    for case in section("export_target") {
        let expected = field(case, "outcome");
        let have = value_outcome(
            report_export_target(field(case, "arguments"))
                .map(|(kind, args)| Value::List(vec![s(kind), args]))
                .map_err(ToolError::App),
            expected,
        );
        assert_eq!(&have, expected, "{case:?}");
    }
}

#[test]
fn strict_wallet_sync_arguments_match_python() {
    use kassiber_ai::mutate::strict_wallets_sync_args;
    for case in section("wallets_sync") {
        let expected = field(case, "outcome");
        let have = value_outcome(
            strict_wallets_sync_args(field(case, "arguments")).map_err(ToolError::App),
            expected,
        );
        assert_eq!(&have, expected, "{case:?}");
    }
}

#[test]
fn sync_failures_match_python() {
    use kassiber_ai::executor::sync_payload_has_errors;
    for case in section("sync_errors") {
        let payload = field(case, "payload");
        let payload = (!matches!(payload, Value::Null)).then_some(payload);
        assert_eq!(
            Value::Bool(sync_payload_has_errors(payload)),
            *field(case, "errors"),
            "{case:?}"
        );
    }
}

#[test]
fn ai_egress_view_matches_python() {
    use kassiber_ai::projections::egress_view;
    for case in section("egress_view") {
        let expected = field(case, "outcome");
        let mut calls = Vec::new();
        let ledger = field(case, "ledger").clone();
        let have = value_outcome(
            egress_view(field(case, "arguments"), &mut |after_id, limit| {
                calls.push(Value::List(vec![Value::from(after_id), Value::from(limit)]));
                Ok(ledger.clone())
            }),
            expected,
        );
        assert_eq!(&have, expected, "{case:?}");
        assert_eq!(&Value::List(calls), field(case, "calls"), "{case:?}");
    }
}

#[test]
fn file_analysis_egress_matches_python() {
    use kassiber_ai::file_analysis::redact_for_egress;
    for case in section("file_egress") {
        assert_eq!(
            &redact_for_egress(field(case, "analysis")),
            field(case, "redacted"),
            "{case:?}"
        );
    }
}

#[test]
fn ownership_results_for_ai_match_python() {
    use kassiber_ai::projections::identify_result_for_ai;
    for case in section("identify") {
        assert_eq!(
            &identify_result_for_ai(field(case, "item")),
            field(case, "redacted"),
            "{case:?}"
        );
    }
}

#[test]
fn planned_review_artifact_checks_match_python() {
    use kassiber_ai::review::check_planned_artifact;
    for case in section("planned_artifact") {
        let payload = field(case, "payload");
        let expected = field(case, "outcome");
        let have = value_outcome(
            check_planned_artifact(payload)
                .map(|()| payload.clone())
                .map_err(ToolError::App),
            expected,
        );
        assert_eq!(&have, expected, "{case:?}");
    }
}
