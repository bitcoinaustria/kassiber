//! Ports of `tests/test_ai_core_http_transport.py` and the client classes of
//! `tests/test_ai_core_api_format.py` (`AutoFallbackTest`) and
//! `tests/test_ai_core.py`, against the scripted loopback provider: request
//! bodies, retries, fallback, stream parsing and error mapping run end to
//! end without leaving the machine.

use kassiber_ai::transport::{
    run_connection_test, ChatDelta, ChatRequest, ClientConfig, HttpClient, RequestContext,
};
use kassiber_envelope::{dict, get, set, AppError};
use kassiber_net::test_support::{Reply, ScriptedProvider};
use kassiber_net::{Gate, Ledger, Offline};
use kassiber_pyjson::Value;

fn load(raw: &str) -> Value {
    kassiber_pytext::loads(raw).unwrap()
}

fn s(text: &str) -> Value {
    kassiber_envelope::s(text)
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap_or(&Value::Null)
}

fn dumps(value: &Value) -> String {
    kassiber_pyjson::dumps(value, &Default::default()).unwrap()
}

fn online() -> Gate {
    Gate::new(|| Offline::Online, Ledger::default())
}

fn client(provider: &ScriptedProvider, configure: impl FnOnce(&mut ClientConfig)) -> HttpClient {
    let mut config = ClientConfig::new(provider.base_url());
    configure(&mut config);
    HttpClient::new(config, online())
}

fn chat_client(provider: &ScriptedProvider) -> HttpClient {
    client(provider, |c| c.api_format = "chat_completions".into())
}

fn user(text: &str, items: &[Value]) -> RequestContext {
    let mut input = vec![dict([
        ("type", s("message")),
        ("role", s("user")),
        ("content", s(text)),
    ])];
    input.extend_from_slice(items);
    RequestContext {
        instructions: None,
        input_items: input,
    }
}

struct Turn<'a> {
    context: RequestContext,
    options: Value,
    tools: Option<Vec<Value>>,
    tool_choice: Value,
    model: &'a str,
}

impl Default for Turn<'_> {
    fn default() -> Self {
        Self {
            context: user("hello", &[]),
            options: dict::<&str>([]),
            tools: None,
            tool_choice: Value::Null,
            model: "test-model",
        }
    }
}

fn run(client: &mut HttpClient, turn: &Turn<'_>) -> (Vec<ChatDelta>, Result<(), AppError>) {
    let mut chunks = Vec::new();
    let request = ChatRequest {
        context: &turn.context,
        model: turn.model,
        options: &turn.options,
        tools: turn.tools.as_deref(),
        tool_choice: &turn.tool_choice,
    };
    let result = client.stream_chat(&request, &mut |chunk| {
        chunks.push(chunk);
        Ok(())
    });
    (chunks, result)
}

fn stream(client: &mut HttpClient, turn: &Turn<'_>) -> Vec<ChatDelta> {
    let (chunks, result) = run(client, turn);
    result.unwrap();
    chunks
}

fn until_error(client: &mut HttpClient, turn: &Turn<'_>) -> (Vec<ChatDelta>, AppError) {
    let (chunks, result) = run(client, turn);
    (chunks, result.unwrap_err())
}

fn options(raw: &str) -> Turn<'static> {
    Turn {
        options: load(raw),
        ..Default::default()
    }
}

fn status_tool() -> Vec<Value> {
    vec![load(
        r#"{"type":"function","function":{"name":"status","parameters":{"type":"object"}}}"#,
    )]
}

fn bodies(provider: &ScriptedProvider, path: &str) -> Vec<Value> {
    provider
        .bodies(path)
        .iter()
        .map(|body| kassiber_pytext::loads(&String::from_utf8_lossy(body)).unwrap())
        .collect()
}

fn paths(provider: &ScriptedProvider) -> Vec<String> {
    provider.requests().into_iter().map(|r| r.path).collect()
}

fn content(chunks: &[ChatDelta], key: &str) -> String {
    chunks
        .iter()
        .filter_map(|c| match field(&c.delta, key) {
            Value::Str(text) => text.as_str().map(str::to_owned),
            _ => None,
        })
        .collect()
}

fn notices(chunks: &[ChatDelta]) -> Vec<String> {
    chunks
        .iter()
        .filter_map(|c| c.status_phase.clone())
        .collect()
}

fn terminal(chunks: &[ChatDelta]) -> &ChatDelta {
    let terminal: Vec<_> = chunks
        .iter()
        .filter(|c| c.response_output.is_some())
        .collect();
    assert_eq!(terminal.len(), 1);
    terminal[0]
}

fn error_reply(status: u16, message: &str, extra: &str) -> Reply {
    let mut error = dict([("message", s(message))]);
    if !extra.is_empty() {
        for (key, value) in match load(extra) {
            Value::Dict(pairs) => pairs,
            _ => panic!(),
        } {
            if let kassiber_pyjson::Key::Str(key) = key {
                set(&mut error, key.as_str().unwrap(), value);
            }
        }
    }
    Reply::json(status, &dumps(&dict([("error", error)])))
}

fn message_item(text: &str) -> String {
    format!(
        r#"{{"type": "message", "role": "assistant", "status": "completed", "content": [{{"type": "output_text", "text": {}}}]}}"#,
        dumps(&s(text))
    )
}

fn responses_stream(output: &[String], text: Option<&str>) -> Reply {
    let mut events = Vec::new();
    if let Some(text) = text {
        events.push(format!(
            r#"{{"type": "response.output_text.delta", "output_index": 0, "delta": {}}}"#,
            dumps(&s(text))
        ));
    }
    events.push(format!(
        r#"{{"type": "response.completed", "response": {{"id": "resp_1", "status": "completed", "output": [{}]}}}}"#,
        output.join(", ")
    ));
    let events: Vec<&str> = events.iter().map(String::as_str).collect();
    Reply::sse(&events, true)
}

fn chunk(delta: &str, finish: Option<&str>) -> String {
    format!(
        r#"{{"id": "chatcmpl-1", "object": "chat.completion.chunk", "choices": [{{"index": 0, "delta": {delta}, "finish_reason": {}}}]}}"#,
        finish.map_or("null".to_owned(), |f| dumps(&s(f)))
    )
}

fn chat_stream(chunks: &[String], done: bool) -> Reply {
    let events: Vec<&str> = chunks.iter().map(String::as_str).collect();
    Reply::sse(&events, done)
}

fn answer(text: &str) -> Reply {
    chat_stream(
        &[
            chunk(&format!(r#"{{"content": {}}}"#, dumps(&s(text))), None),
            chunk("{}", Some("stop")),
        ],
        true,
    )
}

fn reason(error: &AppError) -> Value {
    field(error.details.as_ref().unwrap_or(&Value::Null), "reason").clone()
}

// --- ResponsesReasoningReplayTest -----------------------------------------

#[test]
fn effort_asks_for_encrypted_reasoning() {
    let provider = ScriptedProvider::new().unwrap();
    let ok = || responses_stream(&[message_item("ok")], Some("ok"));
    provider.queue("POST", "responses", vec![ok(), ok()]);
    let mut client = client(&provider, |_| {});
    stream(&mut client, &options(r#"{"reasoning_effort": "high"}"#));
    stream(&mut client, &Turn::default());
    let sent = bodies(&provider, "responses");
    assert_eq!(*field(&sent[0], "store"), Value::Bool(false));
    assert_eq!(*field(&sent[0], "reasoning"), load(r#"{"effort": "high"}"#));
    assert_eq!(
        *field(&sent[0], "include"),
        load(r#"["reasoning.encrypted_content"]"#)
    );
    assert!(get(&sent[1], "reasoning").is_none() && get(&sent[1], "include").is_none());
    // The Responses body in Python's key order.
    assert_eq!(
        provider.bodies("responses")[1],
        br#"{"model": "test-model", "input": [{"type": "message", "role": "user", "content": "hello"}], "stream": true, "store": false}"#
    );
}

#[test]
fn a_tool_round_replays_encrypted_reasoning_without_an_effort() {
    let provider = ScriptedProvider::new().unwrap();
    let reasoning = r#"{"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "gAAAA-sealed"}"#;
    let call = r#"{"type": "function_call", "status": "completed", "call_id": "call_1", "name": "status", "arguments": "{}"}"#;
    provider.queue(
        "POST",
        "responses",
        vec![
            responses_stream(&[reasoning.into(), call.into()], None),
            responses_stream(&[message_item("done")], Some("done")),
            responses_stream(&[message_item("plain")], Some("plain")),
        ],
    );
    let mut client = client(&provider, |_| {});
    let first = stream(
        &mut client,
        &Turn {
            tools: Some(status_tool()),
            ..Default::default()
        },
    );
    let output = terminal(&first).response_output.clone().unwrap();
    let mut items = output.clone();
    items.push(load(
        r#"{"type": "function_call_output", "call_id": "call_1", "output": "{}"}"#,
    ));
    stream(
        &mut client,
        &Turn {
            tools: Some(status_tool()),
            context: user("hello", &items),
            ..Default::default()
        },
    );
    stream(&mut client, &Turn::default());
    let sent = bodies(&provider, "responses");
    assert!(get(&sent[0], "reasoning").is_none());
    assert_eq!(
        *field(&sent[0], "include"),
        load(r#"["reasoning.encrypted_content"]"#)
    );
    let Value::List(input) = field(&sent[1], "input") else {
        panic!()
    };
    assert!(input.contains(&load(reasoning)));
    let kinds: Vec<Value> = input.iter().map(|i| field(i, "type").clone()).collect();
    assert_eq!(
        kinds,
        [
            s("message"),
            s("reasoning"),
            s("function_call"),
            s("function_call_output")
        ]
    );
    assert!(get(&sent[2], "include").is_none());
}

#[test]
fn a_refused_include_is_retried_once_and_then_left_out() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            error_reply(
                400,
                "Unknown parameter: 'include'.",
                r#"{"param": "include"}"#,
            ),
            responses_stream(&[message_item("ok")], Some("ok")),
            responses_stream(&[message_item("again")], Some("again")),
        ],
    );
    let mut client = client(&provider, |_| {});
    let turn = options(r#"{"reasoning_effort": "low"}"#);
    assert_eq!(content(&stream(&mut client, &turn), "content"), "ok");
    stream(&mut client, &turn);
    let sent = bodies(&provider, "responses");
    assert!(get(&sent[0], "include").is_some());
    assert!(get(&sent[1], "include").is_none());
    assert_eq!(*field(&sent[1], "reasoning"), load(r#"{"effort": "low"}"#));
    assert!(get(&sent[2], "include").is_none());

    let provider = ScriptedProvider::new().unwrap();
    let refusal = || {
        error_reply(
            400,
            "Unknown parameter: 'include'.",
            r#"{"param": "include"}"#,
        )
    };
    provider.queue("POST", "responses", vec![refusal(), refusal()]);
    let (_, error) = until_error(&mut client_for(&provider), &turn);
    assert_eq!(error.code, "ai_request_invalid");
    assert_eq!(bodies(&provider, "responses").len(), 2);

    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![error_reply(
            400,
            "Invalid value for 'temperature'.",
            r#"{"param": "temperature"}"#,
        )],
    );
    until_error(&mut client_for(&provider), &turn);
    assert_eq!(bodies(&provider, "responses").len(), 1);
}

fn client_for(provider: &ScriptedProvider) -> HttpClient {
    client(provider, |_| {})
}

#[test]
fn an_echoed_request_drops_nothing() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            Reply::json(422, r#"{"detail": [{"type": "less_than_equal", "loc": ["body", "temperature"], "msg": "Input should be less than or equal to 2", "input": {"include": ["reasoning.encrypted_content"], "reasoning": {"effort": "high"}, "max_tokens": 16}}]}"#),
            responses_stream(&[message_item("ok")], Some("ok")),
        ],
    );
    let turn = Turn {
        options: load(r#"{"reasoning_effort": "high", "temperature": 9}"#),
        tools: Some(status_tool()),
        ..Default::default()
    };
    until_error(&mut client_for(&provider), &turn);
    let sent = bodies(&provider, "responses");
    assert_eq!(sent.len(), 1);
    assert!(get(&sent[0], "include").is_some());
}

#[test]
fn only_encrypted_reasoning_items_are_replayed() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![responses_stream(&[message_item("ok")], None)],
    );
    let items = [
        load(r#"{"type": "reasoning", "id": "rs_plain", "summary": []}"#),
        load(r#"{"type": "reasoning", "id": "rs_empty", "summary": [], "encrypted_content": ""}"#),
        load(
            r#"{"type": "reasoning", "id": "rs_sealed", "summary": [], "encrypted_content": "gAAAA-sealed"}"#,
        ),
        load(
            r#"{"type": "function_call", "call_id": "call_1", "name": "status", "arguments": "{}"}"#,
        ),
        load(r#"{"type": "function_call_output", "call_id": "call_1", "output": "{}"}"#),
    ];
    stream(
        &mut client_for(&provider),
        &Turn {
            context: user("hello", &items),
            options: load(r#"{"reasoning_effort": "medium"}"#),
            ..Default::default()
        },
    );
    let Value::List(input) = field(&bodies(&provider, "responses")[0], "input").clone() else {
        panic!()
    };
    let ids: Vec<Value> = input
        .iter()
        .map(|i| {
            if get(i, "id").is_some() {
                field(i, "id").clone()
            } else {
                field(i, "type").clone()
            }
        })
        .collect();
    assert_eq!(
        ids,
        [
            s("message"),
            s("rs_sealed"),
            s("function_call"),
            s("function_call_output")
        ]
    );
}

// --- ReasoningRefusalTest -------------------------------------------------

#[test]
fn refused_effort_is_retried_without_reasoning_and_noticed() {
    for refusal in [
        error_reply(
            400,
            "Unsupported parameter: 'reasoning.effort' is not supported with this model.",
            r#"{"param": "reasoning.effort", "code": "unsupported_parameter"}"#,
        ),
        error_reply(400, "\"llama3.2\" does not support thinking", ""),
        Reply::json(
            422,
            r#"{"detail": [{"type": "extra_forbidden", "loc": ["body", "reasoning"], "msg": "Extra inputs are not permitted"}]}"#,
        ),
    ] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue(
            "POST",
            "responses",
            vec![
                refusal,
                responses_stream(&[message_item("ok")], Some("ok")),
                responses_stream(&[message_item("again")], Some("again")),
            ],
        );
        let mut client = client_for(&provider);
        let turn = options(r#"{"reasoning_effort": "max"}"#);
        let chunks = stream(&mut client, &turn);
        assert_eq!(notices(&chunks), ["reasoning_effort_unavailable"]);
        assert!(chunks[0].status_phase.is_some());
        assert_eq!(chunks[0].delta, dict::<&str>([]));
        assert_eq!(content(&chunks, "content"), "ok");
        let sent = bodies(&provider, "responses");
        assert_eq!(*field(&sent[0], "reasoning"), load(r#"{"effort": "high"}"#));
        assert!(get(&sent[1], "reasoning").is_none() && get(&sent[1], "include").is_none());
        let later = stream(&mut client, &turn);
        assert!(notices(&later).is_empty());
        assert!(get(&bodies(&provider, "responses")[2], "reasoning").is_none());
    }
}

#[test]
fn reasoning_refusals_are_bounded_and_only_400s_count() {
    let provider = ScriptedProvider::new().unwrap();
    let refusal = || error_reply(400, "Unsupported parameter: 'reasoning.effort'.", "");
    provider.queue("POST", "responses", vec![refusal(), refusal()]);
    until_error(
        &mut client_for(&provider),
        &options(r#"{"reasoning_effort": "low"}"#),
    );
    assert_eq!(bodies(&provider, "responses").len(), 2);

    for status in [401, 404, 429, 500] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue(
            "POST",
            "responses",
            vec![error_reply(status, "reasoning effort trouble", "")],
        );
        until_error(
            &mut client(&provider, |c| c.api_format = "responses".into()),
            &options(r#"{"reasoning_effort": "low"}"#),
        );
        assert_eq!(bodies(&provider, "responses").len(), 1, "{status}");
    }

    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            error_reply(
                400,
                "Invalid input item: reasoning items are not supported.",
                "",
            ),
            responses_stream(&[message_item("ok")], Some("ok")),
        ],
    );
    let sealed = load(
        r#"{"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "sealed"}"#,
    );
    let chunks = stream(
        &mut client_for(&provider),
        &Turn {
            context: user("hello", &[sealed]),
            ..Default::default()
        },
    );
    assert!(notices(&chunks).is_empty());
    let sent = bodies(&provider, "responses");
    assert!(dumps(&sent[0]).contains("rs_1") && !dumps(&sent[1]).contains("rs_1"));
}

#[test]
fn blocking_chat_records_the_notice_and_reports_length() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            error_reply(400, "Unsupported parameter: 'reasoning.effort'.", ""),
            Reply::json(
                200,
                &format!(
                    r#"{{"status": "completed", "output": [{}]}}"#,
                    message_item("ok")
                ),
            ),
        ],
    );
    let mut client = client_for(&provider);
    let context = RequestContext::from_messages(&[load(r#"{"role": "user", "content": "hello"}"#)]);
    let effort = load(r#"{"reasoning_effort": "medium"}"#);
    let result = client
        .chat(
            &ChatRequest {
                context: &context,
                model: "test-model",
                options: &effort,
                tools: None,
                tool_choice: &Value::Null,
            },
            None,
        )
        .unwrap();
    assert_eq!(*field(&result, "content"), s("ok"));
    assert_eq!(client.notices, ["reasoning_effort_unavailable"]);

    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![Reply::json(200, &format!(
            r#"{{"status": "incomplete", "incomplete_details": {{"reason": "max_output_tokens"}}, "output": [{}, {{"type": "function_call", "status": "in_progress", "call_id": "call_cut", "name": "status", "arguments": "{{"}}]}}"#,
            message_item("Partial")
        ))],
    );
    let none = dict::<&str>([]);
    let result = client_for(&provider)
        .chat(
            &ChatRequest {
                context: &context,
                model: "m",
                options: &none,
                tools: None,
                tool_choice: &Value::Null,
            },
            None,
        )
        .unwrap();
    assert_eq!(*field(&result, "finish_reason"), s("length"));
    assert_eq!(*field(&result, "content"), s("Partial"));
    assert!(get(&result, "tool_calls").is_none());
}

// --- ResponsesIncompleteTest ---------------------------------------------

#[test]
fn incomplete_answers_finish_with_length_or_content_filter_and_withhold_cut_calls() {
    for (why, finish) in [
        ("max_output_tokens", "length"),
        ("content_filter", "content_filter"),
    ] {
        let provider = ScriptedProvider::new().unwrap();
        let events = [
            r#"{"type": "response.output_text.delta", "output_index": 0, "delta": "Partial answ"}"#
                .to_owned(),
            format!(
                r#"{{"type": "response.incomplete", "response": {{"id": "resp_1", "status": "incomplete", "output": [{}], "incomplete_details": {{"reason": "{why}"}}}}}}"#,
                message_item("Partial answ")
            ),
        ];
        let events: Vec<&str> = events.iter().map(String::as_str).collect();
        provider.queue("POST", "responses", vec![Reply::sse(&events, true)]);
        let chunks = stream(&mut client_for(&provider), &Turn::default());
        assert_eq!(content(&chunks, "content"), "Partial answ");
        assert_eq!(terminal(&chunks).finish_reason.as_deref(), Some(finish));
    }
    let finished = r#"{"type": "function_call", "status": "completed", "call_id": "call_done", "name": "status", "arguments": "{}"}"#;
    let cut = r#"{"type": "function_call", "status": "incomplete", "call_id": "call_cut", "name": "ui_journals_process", "arguments": "{\"wallet\": \"va"}"#;
    let provider = ScriptedProvider::new().unwrap();
    let incomplete = format!(
        r#"{{"type": "response.incomplete", "response": {{"status": "incomplete", "incomplete_details": {{"reason": "max_output_tokens"}}, "output": [{finished}, {cut}]}}}}"#
    );
    provider.queue(
        "POST",
        "responses",
        vec![Reply::sse(
            &[
                r#"{"type": "response.output_item.added", "output_index": 1, "item": {"type": "function_call", "status": "incomplete", "call_id": "call_cut", "name": "ui_journals_process", "arguments": ""}}"#,
                &incomplete,
            ],
            true,
        )],
    );
    let chunks = stream(&mut client_for(&provider), &Turn::default());
    let last = terminal(&chunks);
    assert_eq!(last.finish_reason.as_deref(), Some("length"));
    let Value::List(calls) = field(&last.delta, "tool_calls") else {
        panic!()
    };
    assert_eq!(
        calls
            .iter()
            .map(|c| field(c, "id").clone())
            .collect::<Vec<_>>(),
        [s("call_done")]
    );
    assert_eq!(last.response_output.as_ref().unwrap().len(), 1);

    // A call without a status in an incomplete answer is unfinished.
    let provider = ScriptedProvider::new().unwrap();
    let statusless =
        r#"{"type": "function_call", "call_id": "call_cut", "name": "x", "arguments": "{"}"#;
    let done = format!(
        r#"{{"type": "response.output_item.done", "output_index": 0, "item": {statusless}}}"#
    );
    let end = format!(
        r#"{{"type": "response.incomplete", "response": {{"status": "incomplete", "incomplete_details": {{"reason": "max_output_tokens"}}, "output": [{statusless}]}}}}"#
    );
    provider.queue("POST", "responses", vec![Reply::sse(&[&done, &end], true)]);
    let chunks = stream(&mut client_for(&provider), &Turn::default());
    let last = terminal(&chunks);
    assert_eq!(*field(&last.delta, "tool_calls"), Value::List(vec![]));
    assert_eq!(last.response_output, Some(vec![]));
}

#[test]
fn responses_streams_that_end_early_or_empty_are_no_answer() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![Reply::sse(
            &[
                r#"{"type": "response.output_item.added", "output_index": 0, "item": {"type": "function_call", "call_id": "c1", "name": "status", "arguments": ""}}"#,
                r#"{"type": "response.function_call_arguments.delta", "output_index": 0, "delta": "{\"a"}"#,
            ],
            false,
        )],
    );
    let (chunks, error) = until_error(&mut client_for(&provider), &Turn::default());
    let Value::List(streamed) = field(&chunks[chunks.len() - 2].delta, "tool_calls") else {
        panic!()
    };
    assert_eq!(*field(&streamed[0], "id"), s("c1"));
    assert_eq!(
        chunks[chunks.len() - 1].delta,
        load(r#"{"tool_calls": []}"#)
    );
    assert_eq!(reason(&error), s("stream_incomplete"));

    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses_stream(&[], None),
            responses_stream(&[], Some("ok")),
        ],
    );
    let (chunks, error) = until_error(&mut client_for(&provider), &Turn::default());
    assert_eq!(reason(&error), s("no_answer"));
    assert!(chunks.iter().all(|c| c.response_output.is_none()));
    assert_eq!(
        content(
            &stream(&mut client_for(&provider), &Turn::default()),
            "content"
        ),
        "ok"
    );

    for done in [false, true] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue(
            "POST",
            "responses",
            vec![Reply::sse(&[r#"{"type": "response.output_text.delta", "output_index": 0, "delta": "Half an ans"}"#], done)],
        );
        let (chunks, error) = until_error(&mut client_for(&provider), &Turn::default());
        assert_eq!(chunks[0].delta, load(r#"{"content": "Half an ans"}"#));
        assert_eq!(
            (error.code.as_str(), error.retryable),
            ("ai_unavailable", true)
        );
        assert_eq!(reason(&error), s("stream_incomplete"));
    }

    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![Reply::raw(
            200,
            "text/plain",
            br#"{"error": "Unexpected endpoint"}"#,
        )],
    );
    let (_, error) = until_error(
        &mut client(&provider, |c| c.api_format = "responses".into()),
        &Turn::default(),
    );
    assert_eq!(reason(&error), s("no_stream_events"));
}

// --- ChatCompletionsStreamTest -------------------------------------------

#[test]
fn chat_streams_text_with_keep_alive_comments_and_a_usage_chunk() {
    let provider = ScriptedProvider::new().unwrap();
    let mut body = String::from(": OPENROUTER PROCESSING\n\n");
    for event in [
        chunk(r#"{"role": "assistant", "content": ""}"#, None),
        chunk(r#"{"content": "Hel"}"#, None),
        chunk(r#"{"content": "lo"}"#, None),
        chunk("{}", Some("stop")),
        r#"{"id": "chatcmpl-1", "choices": [], "usage": {"total_tokens": 9}}"#.to_owned(),
    ] {
        body.push_str(&format!("data: {event}\n\n: OPENROUTER PROCESSING\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    provider.queue(
        "POST",
        "chat/completions",
        vec![Reply::raw(200, "text/event-stream", body.as_bytes()).chunked(7)],
    );
    let context = RequestContext {
        instructions: Some("Be brief.".into()),
        input_items: vec![load(
            r#"{"type": "message", "role": "user", "content": "hi"}"#,
        )],
    };
    let chunks = stream(
        &mut chat_client(&provider),
        &Turn {
            context,
            ..Default::default()
        },
    );
    assert_eq!(content(&chunks, "content"), "Hello");
    let last = terminal(&chunks);
    assert!(std::ptr::eq(last, chunks.last().unwrap()));
    assert_eq!(last.finish_reason.as_deref(), Some("stop"));
    assert_eq!(last.delta, dict::<&str>([]));
    assert_eq!(last.raw, load(r#"{"usage": {"total_tokens": 9}}"#));
    assert_eq!(
        Value::List(last.response_output.clone().unwrap()),
        load(
            r#"[{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Hello"}]}]"#
        )
    );
    assert_eq!(
        provider.bodies("chat/completions")[0],
        br#"{"model": "test-model", "messages": [{"role": "system", "content": "Be brief."}, {"role": "user", "content": "hi"}], "stream": true, "store": false}"#
    );
    assert!(provider.bodies("responses").is_empty());
}

#[test]
fn chat_reasoning_fields_stay_reasoning() {
    for name in ["reasoning_content", "reasoning"] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue(
            "POST",
            "chat/completions",
            vec![chat_stream(
                &[
                    chunk(&format!(r#"{{"{name}": "Let me think. "}}"#), None),
                    chunk(&format!(r#"{{"{name}": "Done."}}"#), None),
                    chunk(r#"{"content": "Answer"}"#, None),
                    chunk("{}", Some("stop")),
                ],
                true,
            )],
        );
        let chunks = stream(&mut chat_client(&provider), &Turn::default());
        assert_eq!(content(&chunks, "reasoning"), "Let me think. Done.");
        assert_eq!(content(&chunks, "content"), "Answer");
        let item = &terminal(&chunks).response_output.as_ref().unwrap()[0];
        assert_eq!(
            *field(item, "chat_completions_replay"),
            dict([("field", s(name))])
        );
    }
}

#[test]
fn chat_tool_calls_split_by_index_with_parallel_calls() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "chat/completions",
        vec![chat_stream(
            &[
                chunk(r#"{"role": "assistant", "content": null}"#, None),
                chunk(r#"{"tool_calls": [{"index": 0, "id": "call_a", "type": "function", "function": {"name": "status", "arguments": ""}}]}"#, None),
                chunk(r#"{"tool_calls": [{"index": 0, "function": {"arguments": "{\"ver"}}]}"#, None),
                chunk(r#"{"tool_calls": [{"index": 1, "id": "call_b", "type": "function", "function": {"name": "ui_wallets_list", "arguments": "{}"}}]}"#, None),
                chunk(r#"{"tool_calls": [{"index": 0, "function": {"arguments": "bose\": true}"}}]}"#, None),
                chunk("{}", Some("tool_calls")),
            ],
            true,
        )],
    );
    let turn = Turn {
        tools: Some(vec![load(
            r#"{"type": "function", "name": "status", "parameters": {"type": "object"}, "strict": false}"#,
        )]),
        tool_choice: s("auto"),
        ..Default::default()
    };
    let chunks = stream(&mut chat_client(&provider), &turn);
    let last = terminal(&chunks);
    assert_eq!(last.finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(
        *field(&last.delta, "tool_calls"),
        load(
            r#"[{"id": "call_a", "type": "function", "function": {"name": "status", "arguments": "{\"verbose\": true}"}}, {"id": "call_b", "type": "function", "function": {"name": "ui_wallets_list", "arguments": "{}"}}]"#
        )
    );
    let body = &bodies(&provider, "chat/completions")[0];
    assert_eq!(
        *field(body, "tools"),
        load(
            r#"[{"type": "function", "function": {"name": "status", "parameters": {"type": "object"}}}]"#
        )
    );
    assert_eq!(*field(body, "tool_choice"), s("auto"));
}

#[test]
fn chat_duplicate_ids_fail_closed_and_finish_reasons_map() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "chat/completions",
        vec![chat_stream(
            &[
                chunk(r#"{"tool_calls": [{"index": 0, "id": "dup", "function": {"name": "ui_transactions_list", "arguments": "{\"limit\":"}}]}"#, None),
                chunk(r#"{"tool_calls": [{"index": 1, "id": "dup", "function": {"arguments": "5}"}}]}"#, None),
                chunk("{}", Some("tool_calls")),
            ],
            true,
        )],
    );
    let (chunks, error) = until_error(&mut chat_client(&provider), &Turn::default());
    assert_eq!(error.code, "ai_request_invalid");
    assert_eq!(reason(&error), s("duplicate_tool_call_id"));
    assert!(chunks.iter().all(|c| c.response_output.is_none()));
    assert!(!chunks
        .iter()
        .any(|c| dumps(&c.delta).contains(r#"{\"limit\":5}"#)));

    for (finish, expected) in [
        (Some("length"), "length"),
        (Some("content_filter"), "content_filter"),
        (Some("function_call"), "tool_calls"),
        (None, "stop"),
    ] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue(
            "POST",
            "chat/completions",
            vec![chat_stream(
                &[
                    chunk(r#"{"content": "partial"}"#, None),
                    chunk("{}", finish),
                ],
                true,
            )],
        );
        assert_eq!(
            terminal(&stream(&mut chat_client(&provider), &Turn::default()))
                .finish_reason
                .as_deref(),
            Some(expected)
        );
    }

    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "chat/completions",
        vec![chat_stream(&[chunk(r#"{"tool_calls": [{"index": 0, "id": "c1", "function": {"name": "status", "arguments": "{\"a"}}]}"#, None), chunk("{}", Some("length"))], true)],
    );
    let last = stream(&mut chat_client(&provider), &Turn::default())
        .pop()
        .unwrap();
    assert_eq!(last.delta, load(r#"{"tool_calls": []}"#));
    assert_eq!(last.response_output, Some(vec![]));
}

#[test]
fn chat_streams_that_end_early_or_empty_fail() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "chat/completions",
        vec![chat_stream(&[chunk(r#"{"content": "half"}"#, None)], false)],
    );
    let (chunks, error) = until_error(&mut chat_client(&provider), &Turn::default());
    assert_eq!(content(&chunks, "content"), "half");
    assert_eq!(
        (error.code.as_str(), reason(&error)),
        ("ai_unavailable", s("stream_incomplete"))
    );

    for reply in [
        chat_stream(&[], true),
        chat_stream(
            &[r#"{"id": "chatcmpl-1", "object": "chat.completion.chunk", "choices": []}"#.into()],
            true,
        ),
        chat_stream(&[chunk("{}", None)], true),
    ] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue("POST", "chat/completions", vec![reply]);
        let (chunks, error) = until_error(&mut chat_client(&provider), &Turn::default());
        assert!(chunks.is_empty());
        assert_eq!(reason(&error), s("no_answer"));
    }
    for reply in [
        chat_stream(
            &[
                chunk(r#"{"content": "ok"}"#, None),
                chunk("{}", Some("stop")),
            ],
            false,
        ),
        chat_stream(&[chunk(r#"{"content": "ok"}"#, None)], true),
        chat_stream(&[chunk("{}", Some("stop"))], true),
    ] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue("POST", "chat/completions", vec![reply]);
        assert_eq!(
            stream(&mut chat_client(&provider), &Turn::default())
                .last()
                .unwrap()
                .finish_reason
                .as_deref(),
            Some("stop")
        );
    }

    let provider = ScriptedProvider::new().unwrap();
    provider.queue("POST", "chat/completions", vec![chat_stream(&[chunk(r#"{"tool_calls": [{"index": 0, "id": "c1", "function": {"name": "status", "arguments": "{\"a"}}]}"#, None)], false)]);
    let (chunks, error) = until_error(&mut chat_client(&provider), &Turn::default());
    assert_eq!(chunks.last().unwrap().delta, load(r#"{"tool_calls": []}"#));
    assert_eq!(reason(&error), s("stream_incomplete"));

    // [DONE] after call fragments without a finish reason: nothing runs.
    let provider = ScriptedProvider::new().unwrap();
    provider.queue("POST", "chat/completions", vec![chat_stream(&[chunk(r#"{"tool_calls": [{"index": 0, "id": "call_1", "function": {"name": "status", "arguments": "{\"wal"}}]}"#, None)], true)]);
    let (chunks, error) = until_error(&mut chat_client(&provider), &Turn::default());
    assert_eq!(reason(&error), s("no_finish_reason"));
    assert_eq!(chunks.last().unwrap().delta, load(r#"{"tool_calls": []}"#));

    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "chat/completions",
        vec![Reply::raw(
            200,
            "text/plain",
            br#"{"error":"Unexpected endpoint or method."}"#,
        )],
    );
    assert_eq!(
        reason(&until_error(&mut chat_client(&provider), &Turn::default()).1),
        s("no_stream_events")
    );
}

#[test]
fn chat_error_payloads_are_mapped() {
    for (event, code) in [
        (
            r#"{"id": "chatcmpl-1", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": ""}, "finish_reason": "error"}], "error": {"code": "server_error", "message": "Provider disconnected"}}"#,
            "ai_unavailable",
        ),
        (
            r#"{"error": {"code": 429, "message": "Rate limit exceeded"}}"#,
            "ai_rate_limited",
        ),
        (
            r#"{"object": "error", "message": "bad", "type": "BadRequestError", "code": 400}"#,
            "ai_request_invalid",
        ),
    ] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue("POST", "chat/completions", vec![Reply::sse(&[event], true)]);
        assert_eq!(
            until_error(&mut chat_client(&provider), &Turn::default())
                .1
                .code,
            code
        );
    }
}

// --- ChatCompletionsRequestTest ------------------------------------------

#[test]
fn two_round_tool_loop_replays_calls_outputs_and_reasoning() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "chat/completions",
        vec![
            chat_stream(
                &[
                    chunk(r#"{"reasoning_content": "Need the status."}"#, None),
                    chunk(r#"{"content": "Checking."}"#, None),
                    chunk(r#"{"tool_calls": [{"index": 0, "id": "call_1", "function": {"name": "status", "arguments": "{}"}}]}"#, None),
                    chunk(r#"{"tool_calls": [{"index": 1, "id": "call_2", "function": {"name": "health", "arguments": "{}"}}]}"#, None),
                    chunk("{}", Some("tool_calls")),
                ],
                true,
            ),
            answer("All good."),
        ],
    );
    let mut client = chat_client(&provider);
    let mut context = RequestContext {
        instructions: Some("Kassiber prompt".into()),
        input_items: vec![
            load(r#"{"type": "message", "role": "user", "content": "earlier"}"#),
            load(r#"{"type": "message", "role": "assistant", "content": "earlier answer"}"#),
            load(r#"{"type": "message", "role": "user", "content": "status?"}"#),
        ],
    };
    let tools = Some(vec![load(r#"{"type": "function", "name": "status"}"#)]);
    let first = stream(
        &mut client,
        &Turn {
            context: context.clone(),
            tools: tools.clone(),
            ..Default::default()
        },
    );
    let last = terminal(&first);
    context
        .input_items
        .extend(last.response_output.clone().unwrap());
    let Value::List(calls) = field(&last.delta, "tool_calls") else {
        panic!()
    };
    for call in calls {
        context.input_items.push(dict([
            ("type", s("function_call_output")),
            ("call_id", field(call, "id").clone()),
            ("output", s("{\"ok\":true}")),
        ]));
    }
    let second = stream(
        &mut client,
        &Turn {
            context,
            tools,
            ..Default::default()
        },
    );
    assert_eq!(content(&second, "content"), "All good.");
    assert_eq!(
        *field(&bodies(&provider, "chat/completions")[1], "messages"),
        load(
            r#"[{"role": "system", "content": "Kassiber prompt"}, {"role": "user", "content": "earlier"}, {"role": "assistant", "content": "earlier answer"}, {"role": "user", "content": "status?"}, {"role": "assistant", "content": "Checking.", "reasoning_content": "Need the status.", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "status", "arguments": "{}"}}, {"id": "call_2", "type": "function", "function": {"name": "health", "arguments": "{}"}}]}, {"role": "tool", "tool_call_id": "call_1", "content": "{\"ok\":true}"}, {"role": "tool", "tool_call_id": "call_2", "content": "{\"ok\":true}"}]"#
        )
    );
}

fn deepseek_round_two(
    provider: &ScriptedProvider,
    rest: Vec<Reply>,
) -> (HttpClient, RequestContext) {
    let mut replies = vec![
        error_reply(
            400,
            "Unsupported parameter: 'reasoning_effort' is not supported with this model.",
            r#"{"param": "reasoning_effort"}"#,
        ),
        chat_stream(
            &[
                chunk(r#"{"reasoning_content": "Need the status."}"#, None),
                chunk(
                    r#"{"tool_calls": [{"index": 0, "id": "call_1", "function": {"name": "status", "arguments": "{}"}}]}"#,
                    None,
                ),
                chunk("{}", Some("tool_calls")),
            ],
            true,
        ),
    ];
    replies.extend(rest);
    provider.queue("POST", "chat/completions", replies);
    let mut client = chat_client(provider);
    let mut context = user("status?", &[]);
    let first = stream(
        &mut client,
        &Turn {
            context: context.clone(),
            tools: Some(vec![load(r#"{"type": "function", "name": "status"}"#)]),
            options: load(r#"{"reasoning_effort": "high"}"#),
            ..Default::default()
        },
    );
    context
        .input_items
        .extend(terminal(&first).response_output.clone().unwrap());
    context.input_items.push(load(
        r#"{"type": "function_call_output", "call_id": "call_1", "output": "{\"ok\":true}"}"#,
    ));
    (client, context)
}

#[test]
fn a_refused_effort_keeps_the_reasoning_replay_and_a_refused_replay_drops_only_it() {
    let provider = ScriptedProvider::new().unwrap();
    let (mut client, context) = deepseek_round_two(&provider, vec![answer("All good.")]);
    let turn = Turn {
        context,
        tools: Some(vec![load(r#"{"type": "function", "name": "status"}"#)]),
        options: load(r#"{"reasoning_effort": "high"}"#),
        ..Default::default()
    };
    assert_eq!(content(&stream(&mut client, &turn), "content"), "All good.");
    let sent = bodies(&provider, "chat/completions");
    assert_eq!(*field(&sent[0], "reasoning_effort"), s("high"));
    assert!(
        get(&sent[1], "reasoning_effort").is_none() && get(&sent[2], "reasoning_effort").is_none()
    );
    assert!(dumps(&sent[2]).contains(r#""reasoning_content": "Need the status.""#));

    let provider = ScriptedProvider::new().unwrap();
    let (mut client, context) = deepseek_round_two(
        &provider,
        vec![
            error_reply(
                400,
                "Extra field 'reasoning_content' is not allowed in messages.",
                "",
            ),
            answer("All good."),
        ],
    );
    let turn = Turn { context, ..turn };
    assert_eq!(content(&stream(&mut client, &turn), "content"), "All good.");
    let sent = bodies(&provider, "chat/completions");
    assert_eq!(sent.len(), 4);
    assert!(
        dumps(&sent[2]).contains("reasoning_content")
            && !dumps(&sent[3]).contains("reasoning_content")
    );
}

#[test]
fn openrouter_reasoning_details_are_merged_and_replayed() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "chat/completions",
        vec![
            chat_stream(
                &[
                    chunk(r#"{"reasoning": "Plan", "reasoning_details": [{"type": "reasoning.text", "text": "Plan", "format": "anthropic-claude-v1", "index": 0}]}"#, None),
                    chunk(r#"{"reasoning": " it", "reasoning_details": [{"type": "reasoning.text", "text": " it", "signature": "sig-1", "index": 0}]}"#, None),
                    chunk(r#"{"tool_calls": [{"index": 0, "id": "call_1", "function": {"name": "status", "arguments": "{}"}}]}"#, None),
                    chunk("{}", Some("tool_calls")),
                ],
                true,
            ),
            answer("ok"),
        ],
    );
    let mut client = chat_client(&provider);
    let mut context = user("hello", &[]);
    let tools = Some(vec![load(r#"{"type": "function", "name": "status"}"#)]);
    let first = stream(
        &mut client,
        &Turn {
            context: context.clone(),
            tools: tools.clone(),
            ..Default::default()
        },
    );
    context
        .input_items
        .extend(terminal(&first).response_output.clone().unwrap());
    context.input_items.push(load(
        r#"{"type": "function_call_output", "call_id": "call_1", "output": "{}"}"#,
    ));
    stream(
        &mut client,
        &Turn {
            context,
            tools,
            ..Default::default()
        },
    );
    let Value::List(messages) =
        field(&bodies(&provider, "chat/completions")[1], "messages").clone()
    else {
        panic!()
    };
    assert_eq!(
        *field(&messages[1], "reasoning_details"),
        load(
            r#"[{"type": "reasoning.text", "text": "Plan it", "format": "anthropic-claude-v1", "index": 0, "signature": "sig-1"}]"#
        )
    );
    assert!(get(&messages[1], "reasoning").is_none());
}

#[test]
fn chat_bodies_clamp_effort_pass_known_options_and_keep_store() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue("POST", "chat/completions", vec![answer("ok"), answer("ok")]);
    stream(
        &mut chat_client(&provider),
        &options(r#"{"reasoning_effort": "ultra", "max_tokens": 64}"#),
    );
    stream(
        &mut chat_client(&provider),
        &options(
            r#"{"temperature": 0, "response_format": {"type": "json_object"}, "fast_mode": true, "provider_session_id": "cursor", "store": true, "include": ["x"], "stateless_session": true}"#,
        ),
    );
    let sent = bodies(&provider, "chat/completions");
    assert_eq!(
        (
            field(&sent[0], "reasoning_effort"),
            field(&sent[0], "max_tokens")
        ),
        (&s("high"), &Value::from(64))
    );
    assert!(get(&sent[0], "reasoning").is_none());
    assert_eq!(*field(&sent[1], "store"), Value::Bool(false));
    for absent in [
        "fast_mode",
        "provider_session_id",
        "include",
        "tools",
        "tool_choice",
        "stateless_session",
    ] {
        assert!(get(&sent[1], absent).is_none(), "{absent}");
    }
    assert_eq!(
        *field(&sent[1], "response_format"),
        load(r#"{"type": "json_object"}"#)
    );
    let context = user("hello", &[]);
    let openrouter = |effort: Value| {
        kassiber_ai::transport::wire::chat_completions_body(
            &context,
            "anthropic/claude",
            true,
            kassiber_ai::transport::wire::ChatBodyOptions {
                options: &dict([("reasoning_effort", effort)]),
                tools: None,
                tool_choice: &Value::Null,
                omit_store: false,
                replay_reasoning: true,
                max_completion_tokens: false,
                effort: true,
                base_url: "https://openrouter.ai/api/v1",
            },
        )
    };
    assert_eq!(
        *field(&openrouter(s("xhigh")), "reasoning"),
        load(r#"{"effort": "high"}"#)
    );
    assert!(get(&openrouter(s("auto")), "reasoning").is_none());
    assert!(get(&openrouter(Value::Null), "reasoning").is_none());
}

#[test]
fn store_false_is_never_dropped_by_an_error() {
    // Refusals about something else keep store and send once.
    for refusal in [
        error_reply(400, "Invalid 'messages': expected an array.", ""),
        error_reply(400, "The datastore is unavailable, unknown error.", ""),
    ] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue("POST", "chat/completions", vec![refusal]);
        until_error(&mut chat_client(&provider), &Turn::default());
        assert_eq!(bodies(&provider, "chat/completions").len(), 1);
    }
    // An echoed store in another field's error: the effort goes, store stays.
    for refusal in [
        Reply::json(
            422,
            r#"{"detail": [{"type": "value_error", "loc": ["body", "reasoning_effort"], "msg": "Value error, unsupported reasoning_effort", "input": {"store": false, "reasoning_effort": "high"}}]}"#,
        ),
        Reply::json(
            422,
            r#"{"detail": [{"msg": "unsupported reasoning_effort", "input": {"store": false, "reasoning_effort": "high"}}]}"#,
        ),
    ] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue("POST", "chat/completions", vec![refusal, answer("ok")]);
        stream(
            &mut chat_client(&provider),
            &options(r#"{"reasoning_effort": "high"}"#),
        );
        let sent = bodies(&provider, "chat/completions");
        assert_eq!(*field(&sent[0], "reasoning_effort"), s("high"));
        assert!(get(&sent[1], "reasoning_effort").is_none());
        assert_eq!(*field(&sent[1], "store"), Value::Bool(false));
    }
    // Store refused after another part was dropped: the turn fails.
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "chat/completions",
        vec![
            Reply::json(422, r#"{"detail": [{"loc": ["body", "reasoning_effort"], "msg": "unsupported reasoning_effort", "input": {"store": false}}]}"#),
            error_reply(400, "Unrecognized request argument supplied: store", ""),
            answer("ok"),
        ],
    );
    until_error(
        &mut chat_client(&provider),
        &options(r#"{"reasoning_effort": "high"}"#),
    );
    let sent = bodies(&provider, "chat/completions");
    assert_eq!(sent.len(), 2);
    assert!(sent
        .iter()
        .all(|b| *field(b, "store") == Value::Bool(false)));
    // Every refusal naming store fails the turn without a retry.
    for refusal in [
        Reply::json(
            422,
            r#"{"detail": [{"type": "extra_forbidden", "loc": ["body", "store"], "msg": "Extra inputs are not permitted"}]}"#,
        ),
        error_reply(
            400,
            "Unknown parameter: 'store'.",
            r#"{"param": "store", "code": "unknown_parameter"}"#,
        ),
        error_reply(400, "Unrecognized request argument supplied: store", ""),
        error_reply(400, "property 'store' is unsupported", ""),
        error_reply(
            400,
            "store is not supported with value false",
            r#"{"param": "store"}"#,
        ),
        error_reply(400, "store: false is not allowed for this organization", ""),
        error_reply(
            400,
            "Invalid value for 'store': must be true.",
            r#"{"param": "store", "code": "invalid_value"}"#,
        ),
        Reply::json(
            422,
            r#"{"detail": [{"type": "extra_forbidden", "loc": ["body", "store"], "msg": "x"}, {"type": "extra_forbidden", "loc": ["body", "reasoning_effort"], "msg": "x"}]}"#,
        ),
        error_reply(
            400,
            "Unrecognized request arguments supplied: reasoning_effort, store",
            "",
        ),
    ] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue("POST", "chat/completions", vec![refusal, answer("ok")]);
        let (_, error) = until_error(
            &mut chat_client(&provider),
            &options(r#"{"reasoning_effort": "high"}"#),
        );
        assert_eq!(error.code, "ai_store_refused");
        assert!(error.message.contains("store: false"));
        assert!(error.hint.as_deref().unwrap().contains("Test connection"));
        assert_eq!(bodies(&provider, "chat/completions").len(), 1);
    }
    // An OpenAI error naming another param keeps store.
    let provider = ScriptedProvider::new().unwrap();
    provider.queue("POST", "chat/completions", vec![error_reply(400, "Unsupported value: 'temperature' does not support 0 with this model when store is false.", r#"{"param": "temperature", "code": "unsupported_value"}"#)]);
    let (_, error) = until_error(
        &mut chat_client(&provider),
        &options(r#"{"temperature": 0}"#),
    );
    assert_eq!(error.code, "ai_request_invalid");
    // An error inside an accepted stream drops nothing.
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "chat/completions",
        vec![Reply::sse(&[r#"{"id": "chatcmpl-1", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {}, "finish_reason": "error"}], "error": {"code": 400, "message": "Unrecognized request argument supplied: store", "param": "store"}}"#], true), answer("ok")],
    );
    let (_, error) = until_error(
        &mut chat_client(&provider),
        &options(r#"{"reasoning_effort": "high"}"#),
    );
    assert_eq!(
        *field(error.details.as_ref().unwrap(), "status"),
        Value::from(400)
    );
    let sent = bodies(&provider, "chat/completions");
    assert_eq!(sent.len(), 1);
    assert_eq!(*field(&sent[0], "reasoning_effort"), s("high"));
    // Responses never drops store: false either.
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![error_reply(
            400,
            "Unknown parameter: 'store'.",
            r#"{"param": "store"}"#,
        )],
    );
    let (_, error) = until_error(
        &mut client(&provider, |c| c.api_format = "responses".into()),
        &Turn::default(),
    );
    assert_eq!(error.code, "ai_store_refused");
}

#[test]
fn store_is_left_out_only_by_the_providers_explicit_choice() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue("POST", "chat/completions", vec![answer("ok")]);
    provider.queue(
        "POST",
        "responses",
        vec![responses_stream(&[message_item("ok")], Some("ok"))],
    );
    let chunks = stream(
        &mut client(&provider, |c| {
            c.api_format = "chat_completions".into();
            c.omit_store_field = true;
        }),
        &Turn::default(),
    );
    assert_eq!(content(&chunks, "content"), "ok");
    assert!(get(&bodies(&provider, "chat/completions")[0], "store").is_none());
    stream(
        &mut client(&provider, |c| {
            c.api_format = "responses".into();
            c.omit_store_field = true;
        }),
        &Turn::default(),
    );
    assert_eq!(
        *field(&bodies(&provider, "responses")[0], "store"),
        Value::Bool(false)
    );
}

#[test]
fn echoed_chat_requests_drop_nothing_and_max_tokens_is_renamed_once() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "chat/completions",
        vec![
            error_reply(400, "Invalid value for 'temperature': must be at most 2.", r#"{"param": "temperature", "code": "invalid_value", "request": {"reasoning_effort": "high", "messages": [{"role": "assistant", "reasoning_content": "x"}], "hint": "use max_completion_tokens"}}"#),
            answer("ok"),
        ],
    );
    until_error(
        &mut chat_client(&provider),
        &options(r#"{"reasoning_effort": "high", "max_tokens": 16, "temperature": 9}"#),
    );
    let sent = bodies(&provider, "chat/completions");
    assert_eq!(sent.len(), 1);

    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "chat/completions",
        vec![
            error_reply(400, "Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.", r#"{"param": "max_tokens"}"#),
            answer("ok"),
            answer("ok"),
        ],
    );
    let mut client = chat_client(&provider);
    stream(&mut client, &options(r#"{"max_tokens": 16}"#));
    stream(&mut client, &options(r#"{"max_tokens": 16}"#));
    let sent = bodies(&provider, "chat/completions");
    assert_eq!(*field(&sent[0], "max_tokens"), Value::from(16));
    assert_eq!(*field(&sent[1], "max_completion_tokens"), Value::from(16));
    assert!(get(&sent[1], "max_tokens").is_none());
    assert_eq!(*field(&sent[2], "max_completion_tokens"), Value::from(16));

    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "chat/completions",
        vec![
            error_reply(
                400,
                "Unrecognized request argument supplied: reasoning_effort",
                "",
            ),
            answer("ok"),
        ],
    );
    let chunks = stream(
        &mut chat_client(&provider),
        &options(r#"{"reasoning_effort": "medium"}"#),
    );
    assert_eq!(notices(&chunks), ["reasoning_effort_unavailable"]);
}

#[test]
fn blocking_chat_converts_images_and_reads_tool_calls() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "chat/completions",
        vec![
            Reply::json(200, r#"{"choices": [{"index": 0, "message": {"role": "assistant", "content": "{\"rows\": []}", "reasoning_content": "Looked at the image."}, "finish_reason": "stop"}], "usage": {"total_tokens": 3}}"#),
            Reply::json(200, r#"{"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [{"id": "call_9", "type": "function", "function": {"name": "status", "arguments": "{}"}}]}, "finish_reason": "tool_calls"}]}"#),
        ],
    );
    let context = RequestContext::from_messages(&[
        load(r#"{"role": "system", "content": "Extract JSON."}"#),
        load(
            r#"{"role": "user", "content": [{"type": "text", "text": "Read this."}, {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA", "detail": "high"}}]}"#,
        ),
    ]);
    let opts = load(r#"{"temperature": 0, "response_format": {"type": "json_object"}}"#);
    let mut client = chat_client(&provider);
    let result = client
        .chat(
            &ChatRequest {
                context: &context,
                model: "vision",
                options: &opts,
                tools: None,
                tool_choice: &Value::Null,
            },
            None,
        )
        .unwrap();
    assert_eq!(*field(&result, "content"), s("{\"rows\": []}"));
    assert_eq!(*field(&result, "reasoning"), s("Looked at the image."));
    assert_eq!(*field(&result, "usage"), load(r#"{"total_tokens": 3}"#));
    let body = &bodies(&provider, "chat/completions")[0];
    assert_eq!(*field(body, "stream"), Value::Bool(false));
    assert_eq!(
        *field(body, "messages"),
        load(
            r#"[{"role": "system", "content": "Extract JSON."}, {"role": "user", "content": [{"type": "text", "text": "Read this."}, {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA", "detail": "high"}}]}]"#
        )
    );
    let none = dict::<&str>([]);
    let result = client
        .chat(
            &ChatRequest {
                context: &context,
                model: "m",
                options: &none,
                tools: None,
                tool_choice: &Value::Null,
            },
            None,
        )
        .unwrap();
    assert_eq!(*field(&result, "finish_reason"), s("tool_calls"));
    assert_eq!(*field(&result, "content"), s(""));
    let Value::List(calls) = field(&result, "tool_calls") else {
        panic!()
    };
    assert_eq!(*field(&calls[0], "id"), s("call_9"));
}

#[test]
fn the_ledger_records_each_request_and_the_bearer_key_goes_to_the_provider_only() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "chat/completions",
        vec![
            error_reply(400, "Unsupported parameter: 'reasoning_effort'.", ""),
            answer("ok"),
        ],
    );
    let ledger = Ledger::default();
    let gate = Gate::new(|| Offline::Online, ledger.clone());
    let mut config = ClientConfig::new(provider.base_url());
    config.api_format = "chat_completions".into();
    config.api_key = Some("sk-test-secret".into());
    let mut client = HttpClient::new(config, gate);
    stream(&mut client, &options(r#"{"reasoning_effort": "low"}"#));
    let writes = format!("{:?}", ledger.snapshot(0, 100));
    assert!(!writes.contains("sk-test-secret"));
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|r| r.authorization.as_deref() == Some("Bearer sk-test-secret")));
    assert!(requests
        .iter()
        .all(|r| r.accept.as_deref() == Some("text/event-stream")));
}

// --- AutoFallbackTest -----------------------------------------------------

#[test]
fn missing_responses_path_falls_back_once_and_reports_detection() {
    for missing in [
        Reply::json(404, r#"{"detail": "Not Found"}"#),
        Reply::raw(405, "text/plain", b"Method Not Allowed"),
        Reply::raw(501, "text/plain", b""),
        error_reply(501, "Responses API unavailable on this server", ""),
        error_reply(
            400,
            "Unexpected endpoint or method. (POST /v1/responses)",
            "",
        ),
    ] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue("POST", "responses", vec![missing]);
        provider.queue(
            "POST",
            "chat/completions",
            vec![answer("ok"), answer("again")],
        );
        let mut client = client_for(&provider);
        assert_eq!(
            content(&stream(&mut client, &Turn::default()), "content"),
            "ok"
        );
        assert_eq!(client.newly_detected_api_format, Some("chat_completions"));
        assert_eq!(
            content(&stream(&mut client, &Turn::default()), "content"),
            "again"
        );
        assert_eq!(
            paths(&provider),
            [
                "/v1/responses",
                "/v1/chat/completions",
                "/v1/chat/completions"
            ]
        );
    }
}

#[test]
fn no_fallback_without_a_missing_path_or_for_a_known_format() {
    for failure in [
        Reply::raw(501, "text/plain", b""),
        error_reply(501, "Tool calling is not implemented for this model", ""),
        error_reply(400, "File not found.", ""),
        error_reply(400, "Unexpected value for 'input'.", ""),
        error_reply(400, "Unknown endpoint for model gpt-x.", ""),
        error_reply(401, "Incorrect API key provided.", ""),
        error_reply(429, "Too many requests.", ""),
        error_reply(500, "Internal error.", ""),
        error_reply(
            404,
            "The model `gpt-x` does not exist.",
            r#"{"code": "model_not_found"}"#,
        ),
        error_reply(404, "No endpoints found that support tool use.", ""),
    ] {
        let explicit = failure.status == 501 && failure.body.is_empty();
        let provider = ScriptedProvider::new().unwrap();
        provider.queue("POST", "responses", vec![failure]);
        let mut client = if explicit {
            client(&provider, |c| c.api_format = "responses".into())
        } else {
            client_for(&provider)
        };
        until_error(&mut client, &Turn::default());
        assert_eq!(paths(&provider), ["/v1/responses"]);
    }
    for configure in [
        (|c: &mut ClientConfig| c.api_format = "responses".into()) as fn(&mut ClientConfig),
        |c: &mut ClientConfig| c.detected_api_format = Some("responses".into()),
    ] {
        let provider = ScriptedProvider::new().unwrap();
        let mut client = client(&provider, configure);
        let (_, error) = until_error(&mut client, &Turn::default());
        assert_eq!(error.code, "ai_request_invalid");
        assert_eq!(paths(&provider), ["/v1/responses"]);
        assert_eq!(client.newly_detected_api_format, None);
    }
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![responses_stream(&[message_item("ok")], Some("ok"))],
    );
    let mut client = client_for(&provider);
    stream(&mut client, &Turn::default());
    until_error(&mut client, &Turn::default());
    assert!(!paths(&provider).contains(&"/v1/chat/completions".to_owned()));
}

#[test]
fn both_paths_missing_names_the_base_url_and_blocking_chat_falls_back() {
    let provider = ScriptedProvider::new().unwrap();
    let (_, error) = until_error(&mut client_for(&provider), &Turn::default());
    assert!(error
        .message
        .contains("neither a Responses nor a Chat Completions"));
    assert!(error.hint.as_deref().unwrap().contains("/v1"));
    assert_eq!(
        *field(error.details.as_ref().unwrap(), "status"),
        Value::from(404)
    );

    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "chat/completions",
        vec![Reply::json(
            200,
            r#"{"choices": [{"message": {"content": "ok"}, "finish_reason": "stop"}]}"#,
        )],
    );
    let mut client = client_for(&provider);
    let context = RequestContext::from_messages(&[load(r#"{"role": "user", "content": "x"}"#)]);
    let none = dict::<&str>([]);
    let result = client
        .chat(
            &ChatRequest {
                context: &context,
                model: "m",
                options: &none,
                tools: None,
                tool_choice: &Value::Null,
            },
            None,
        )
        .unwrap();
    assert_eq!(*field(&result, "content"), s("ok"));
    assert_eq!(client.newly_detected_api_format, Some("chat_completions"));
}

// --- ListModelsStrictModeTest, HTTP errors and routes (test_ai_core.py) ---

#[test]
fn list_models_is_forgiving_unless_strict() {
    for (reply, strict_code) in [
        (
            Reply::json(404, r#"{"detail": "Not Found"}"#),
            Some("ai_request_invalid"),
        ),
        (
            Reply::raw(200, "text/html", b"<html>"),
            Some("ai_request_invalid"),
        ),
        (
            Reply::json(200, r#"{"object": "list"}"#),
            Some("ai_request_invalid"),
        ),
        (Reply::json(200, r#"[1, 2]"#), Some("ai_request_invalid")),
        (Reply::json(401, r#"{"error": "nope"}"#), None),
    ] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue("GET", "models", vec![reply.clone(), reply]);
        let lenient = client_for(&provider).list_models(false);
        match strict_code {
            Some(code) => {
                assert_eq!(lenient.unwrap(), Vec::<Value>::new());
                assert_eq!(
                    client_for(&provider).list_models(true).unwrap_err().code,
                    code
                );
            }
            None => assert_eq!(lenient.unwrap_err().code, "ai_auth_failed"),
        }
    }
    let provider = ScriptedProvider::new().unwrap();
    provider.queue("GET", "models", vec![Reply::json(200, r#"{"data": [{"id": "a", "owned_by": "me", "supported_parameters": ["tools"]}, {"id": 5}, "x", {"name": "no id"}, {"id": "b"}]}"#)]);
    assert_eq!(
        Value::List(client_for(&provider).list_models(true).unwrap()),
        load(
            r#"[{"id": "a", "owned_by": "me", "supported_parameters": ["tools"], "supports_tools": true}, {"id": "b", "supports_tools": null}]"#
        )
    );
    let provider = ScriptedProvider::new().unwrap();
    provider.queue("GET", "models", vec![Reply::json(200, r#"[]"#)]);
    let error = client_for(&provider).list_models(true).unwrap_err();
    assert_eq!(
        *error.details.as_ref().unwrap(),
        dict([("shape", s("list"))])
    );
}

#[test]
fn http_errors_map_status_and_redact_the_body() {
    for (status, code, retryable) in [
        (401, "ai_auth_failed", false),
        (403, "ai_auth_failed", false),
        (429, "ai_rate_limited", true),
        (503, "ai_unavailable", true),
        (400, "ai_request_invalid", false),
    ] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue(
            "GET",
            "models",
            vec![Reply::json(
                status,
                r#"{"error": "Bearer sk-abcdefghijklmnopqrstuvwxyz0123456789"}"#,
            )],
        );
        let error = client_for(&provider).list_models(true).unwrap_err();
        assert_eq!((error.code.as_str(), error.retryable), (code, retryable));
        let details = error.details.unwrap();
        assert_eq!(*field(&details, "status"), Value::from(status));
        assert!(!dumps(&details).contains("sk-abcdefghijklmnop"));
    }
    // Nothing listens: unreachable, retryable.
    let port = ScriptedProvider::new().unwrap().port();
    let mut config = ClientConfig::new(format!("http://127.0.0.1:{port}/v1"));
    config.timeout = std::time::Duration::from_secs(2);
    let error = HttpClient::new(config, online())
        .list_models(true)
        .unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.retryable),
        ("ai_unavailable", true)
    );
}

#[test]
fn redirects_stay_on_origin_and_offline_keeps_only_local_loopback() {
    let provider = ScriptedProvider::new().unwrap();
    let mut redirect = Reply::raw(302, "text/plain", b"");
    redirect.content_type = "text/plain\r\nlocation: http://192.0.2.1/v1/models".into();
    provider.queue("GET", "models", vec![redirect]);
    let error = client_for(&provider).list_models(true).unwrap_err();
    assert_eq!(error.code, "ai_request_invalid");
    assert_eq!(
        error.message,
        "AI provider attempted an off-origin redirect"
    );
    assert_eq!(provider.requests().len(), 1);

    let offline = || Gate::new(|| Offline::Offline, Ledger::default());
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "GET",
        "models",
        vec![Reply::json(200, r#"{"data": [{"id": "m"}]}"#)],
    );
    let mut remote = HttpClient::new(ClientConfig::new(provider.base_url()), offline());
    let error = remote.list_models(true).unwrap_err();
    assert_eq!(error.code, "network_egress_disabled");
    assert_eq!(
        error.message,
        "Remote AI providers are disabled: Kassiber is in offline mode"
    );
    assert!(provider.requests().is_empty());
    let mut config = ClientConfig::new(provider.base_url());
    config.on_device = true;
    assert_eq!(
        HttpClient::new(config, offline())
            .list_models(true)
            .unwrap()
            .len(),
        1
    );
    // A remote-looking host marked local still counts as off-device.
    let mut config = ClientConfig::new("http://192.0.2.1:9/v1");
    config.on_device = true;
    assert_eq!(
        HttpClient::new(config, offline())
            .list_models(true)
            .unwrap_err()
            .code,
        "network_egress_disabled"
    );
}

#[test]
fn cancel_before_and_during_a_stream() {
    let provider = ScriptedProvider::new().unwrap();
    let client = client_for(&provider);
    client.canceller().cancel();
    let mut client = client;
    let (_, error) = until_error(&mut client, &Turn::default());
    assert_eq!(error.code, "ai_cancelled");
    assert!(provider.requests().is_empty());

    let provider = ScriptedProvider::new().unwrap();
    let events = format!(
        "data: {}\n\n",
        r#"{"type": "response.output_text.delta", "output_index": 0, "delta": "Hel"}"#
    );
    provider.queue(
        "POST",
        "responses",
        vec![Reply::raw(200, "text/event-stream", events.as_bytes())
            .chunked(64)
            .held()],
    );
    let mut client = client_for(&provider);
    let canceller = client.canceller();
    let mut seen = Vec::new();
    let turn = Turn::default();
    let request = ChatRequest {
        context: &turn.context,
        model: "m",
        options: &turn.options,
        tools: None,
        tool_choice: &Value::Null,
    };
    let started = std::time::Instant::now();
    let result = client.stream_chat(&request, &mut |chunk| {
        seen.push(chunk);
        canceller.cancel();
        Ok(())
    });
    // A cancelled turn ends quietly with what arrived.
    result.unwrap();
    assert_eq!(content(&seen, "content"), "Hel");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

// --- The connection test (run_connection_test) ----------------------------

#[test]
fn connection_test_detects_the_format_tools_and_store() {
    // auto: Responses missing, Chat Completions answers with tools.
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "GET",
        "models",
        vec![Reply::json(200, r#"{"data": [{"id": "only"}]}"#)],
    );
    provider.queue("POST", "chat/completions", vec![answer("OK")]);
    let mut auto = client_for(&provider);
    let result = run_connection_test(&mut auto, None).unwrap();
    assert_eq!(
        result,
        load(
            r#"{"model_count": 1, "models": [{"id": "only", "supports_tools": null}], "check_kind": "models", "api_format": "auto", "detected_api_format": "chat_completions", "tools_accepted": true, "store_accepted": true, "probe": {"status": "ok", "reason": null, "model": "only", "finish_reason": "stop"}}"#
        )
    );
    let probe = &bodies(&provider, "chat/completions")[0];
    assert_eq!(
        dumps(probe),
        r#"{"model": "only", "messages": [{"role": "user", "content": "Reply with OK"}], "stream": true, "store": false, "max_tokens": 16, "tools": [{"type": "function", "function": {"name": "connection_check", "description": "Not needed for the answer; offered only to check that tools are accepted.", "parameters": {"type": "object", "properties": {}, "additionalProperties": false}}}], "tool_choice": "auto"}"#
    );

    // Tools refused, then store refused: three requests at most.
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "GET",
        "models",
        vec![Reply::json(200, r#"{"data": [{"id": "a"}, {"id": "b"}]}"#)],
    );
    provider.queue(
        "POST",
        "chat/completions",
        vec![
            error_reply(400, "llama3 does not support tools", ""),
            error_reply(400, "Unrecognized request argument supplied: store", ""),
            answer("OK"),
        ],
    );
    let mut client = client(&provider, |c| c.api_format = "chat_completions".into());
    let result = run_connection_test(&mut client, Some(" b ")).unwrap();
    assert_eq!(*field(&result, "tools_accepted"), Value::Bool(false));
    assert_eq!(*field(&result, "store_accepted"), Value::Bool(false));
    let sent = bodies(&provider, "chat/completions");
    assert_eq!(sent.len(), 3);
    assert!(get(&sent[2], "store").is_none() && get(&sent[2], "tools").is_none());
    assert_eq!(*field(&sent[2], "model"), s("b"));

    // Several models and none chosen: no probe.
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "GET",
        "models",
        vec![Reply::json(200, r#"{"data": [{"id": "a"}, {"id": "b"}]}"#)],
    );
    let result = run_connection_test(&mut client_for(&provider), None).unwrap();
    assert_eq!(
        *field(&result, "probe"),
        load(
            r#"{"status": "skipped", "reason": "no_model", "model": null, "finish_reason": null}"#
        )
    );
    assert_eq!(provider.requests().len(), 1);

    // A Responses stream that answers in another shape falls through.
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "GET",
        "models",
        vec![Reply::json(200, r#"{"data": [{"id": "m"}]}"#)],
    );
    provider.queue(
        "POST",
        "responses",
        vec![Reply::raw(200, "text/plain", b"{}")],
    );
    provider.queue("POST", "chat/completions", vec![answer("OK")]);
    let result = run_connection_test(&mut client_for(&provider), None).unwrap();
    assert_eq!(
        *field(&result, "detected_api_format"),
        s("chat_completions")
    );
}
