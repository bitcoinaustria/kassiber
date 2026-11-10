//! CLI proxy providers end to end on the daemon and CLI against a loopback
//! fake CLIProxyAPI (docs/reference/ai-cli-proxy.md). Rust only: Python
//! refuses the kind. Replaces `tests/test_rust_cli_proxy_daemon.py`.

use kassiber_net::test_support::{Reply, ScriptedProvider};
use serde_json::{json, Value};

use crate::support::{Live, Sandbox};

const KEY: &str = "sk-proxy-0123456789abcdef";

fn models() -> Reply {
    Reply::json(
        200,
        &json!({"object": "list", "data": [
            {"id": "claude-sonnet-5-5", "object": "model", "created": 1, "owned_by": "anthropic"},
            {"id": "gpt-6.1-sol", "object": "model", "created": 1, "owned_by": "openai"},
            {"id": "gpt-image-2", "object": "model", "created": 1, "owned_by": "openai"},
        ]})
        .to_string(),
    )
}

/// One Chat Completions stream chunk with a single choice.
fn chunk(delta: Value, finish: Option<&str>) -> String {
    json!({"id": "chatcmpl-1", "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
    .to_string()
}

fn chat_stream(chunks: &[String]) -> Reply {
    let events: Vec<&str> = chunks.iter().map(String::as_str).collect();
    Reply::sse(&events, true)
}

fn book() -> Sandbox {
    let mut sandbox = Sandbox::empty();
    // AI keeps its own online gate; the observer admits loopback.
    sandbox.set("KASSIBER_NO_EGRESS", "1");
    for argv in [
        &["init"][..],
        &["workspaces", "create", "Demo"],
        &["profiles", "create", "Main"],
    ] {
        assert_eq!(sandbox.machine(argv).code, 0, "{argv:?}");
    }
    sandbox
}

fn chat(model: &str, extra: Value) -> Value {
    let mut args = json!({"provider": "proxy", "model": model,
        "messages": [{"role": "user", "content": "hello"}], "persist": false});
    for (key, value) in extra.as_object().unwrap() {
        args[key] = value.clone();
    }
    json!({"kind": "ai.chat", "args": args})
}

/// One daemon session; each request's records, consents allowed.
fn session(sandbox: &Sandbox, requests: Vec<Value>) -> Vec<Vec<Value>> {
    let mut live = Live::start(sandbox);
    let mut results = Vec::new();
    for (index, mut request) in requests
        .into_iter()
        .chain([json!({"kind": "daemon.shutdown"})])
        .enumerate()
    {
        let rid = format!("r{index}");
        request["request_id"] = json!(rid);
        let streaming = request["kind"] == "ai.chat";
        live.write(&request);
        let mut records = Vec::new();
        let mut waiting = vec![rid.clone()];
        while !waiting.is_empty() {
            let record: Value = serde_json::from_str(&live.read()).unwrap();
            if record["request_id"] == rid.as_str() {
                let kind = record["kind"].as_str().unwrap_or_default().to_owned();
                records.push(record.clone());
                if !streaming || kind == "ai.chat" || kind == "error" {
                    waiting.retain(|w| *w != rid);
                } else if kind == "ai.chat.tool_consent_required" {
                    let control = format!("{rid}c{}", waiting.len());
                    live.write(
                        &json!({"request_id": control, "kind": "ai.tool_call.consent",
                        "args": {"target_request_id": rid, "call_id": record["data"]["call_id"],
                            "decision": "allow_once"}}),
                    );
                    waiting.push(control);
                }
            } else if let Some(id) = record["request_id"].as_str() {
                waiting.retain(|w| w != id);
            }
        }
        results.push(records);
    }
    live.finish();
    results
}

fn paths(proxy: &ScriptedProvider) -> Vec<(String, String)> {
    proxy
        .requests()
        .into_iter()
        .map(|r| (r.method, r.path))
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(m, p)| ((*m).to_owned(), (*p).to_owned()))
        .collect()
}

#[test]
fn test_a_cli_proxy_is_set_up_and_chats_in_each_models_format() {
    let sandbox = book();
    let proxy = ScriptedProvider::new().unwrap();
    let base = proxy.base_url();
    proxy.queue("GET", "models", vec![models()]);
    proxy.queue(
        "POST",
        "chat/completions",
        vec![
            chat_stream(&[
                chunk(json!({"role": "assistant", "content": "from claude"}), None),
                chunk(json!({}), Some("stop")),
            ]),
            // The tool turn: one call, then the answer after its result.
            chat_stream(&[
                chunk(
                    json!({"role": "assistant", "tool_calls": [{"index": 0, "id": "call_proxy",
                        "type": "function",
                        "function": {"name": "ui_journals_process", "arguments": "{}"}}]}),
                    None,
                ),
                chunk(json!({}), Some("tool_calls")),
            ]),
            chat_stream(&[
                chunk(json!({"content": "processed"}), None),
                chunk(json!({}), Some("stop")),
            ]),
        ],
    );
    let completed = json!({"type": "response.completed", "response": {"id": "resp_1",
        "status": "completed", "output": [{"type": "message", "role": "assistant",
            "status": "completed", "content": [{"type": "output_text", "text": "from codex"}]}]}})
    .to_string();
    let delta = json!({"type": "response.output_text.delta", "output_index": 0,
        "delta": "from codex"})
    .to_string();
    proxy.queue(
        "POST",
        "responses",
        vec![Reply::sse(&[&delta, &completed], true)],
    );
    let records = session(
        &sandbox,
        vec![
            json!({"kind": "ai.providers.create", "args": {"name": "proxy", "base_url": base,
                "kind": "cli_proxy", "acknowledged": true}}),
            json!({"kind": "ai.providers.set_api_key", "args": {"name": "proxy", "api_key": KEY}}),
            json!({"kind": "ai.providers.list"}),
            json!({"kind": "ai.list_models", "args": {"provider": "proxy", "cached_only": true}}),
            chat("claude-sonnet-5-5", json!({})),
            json!({"kind": "ai.test_connection", "args": {"provider": "proxy", "base_url": base}}),
            chat(
                "claude-sonnet-5-5",
                json!({"options": {"reasoning_effort": "xhigh"}}),
            ),
            chat("gpt-6.1-sol", json!({})),
            chat(
                "claude-sonnet-5-5",
                json!({"tools_enabled": true, "tool_profile": "full"}),
            ),
            chat("claude-unlisted", json!({})),
            chat("gpt-image-2", json!({})),
        ],
    );
    let last = |i: usize| records[i].last().unwrap().clone();
    assert_eq!(last(0)["data"]["kind"], "cli_proxy");
    assert_eq!(last(0)["data"]["effective_api_format"], Value::Null);
    assert_eq!(last(1)["data"]["has_api_key"], true);
    assert_eq!(last(3)["data"]["models"], json!([]));
    assert_eq!(last(4)["kind"], "error");
    assert_eq!(last(4)["error"]["code"], "ai_cli_proxy_models_unchecked");
    assert!(!serde_json::to_string(&records[2]).unwrap().contains(KEY));
    assert_eq!(last(5)["data"]["models_saved"], true);
    let formats: Vec<(Value, Value)> = last(5)["data"]["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| (row["id"].clone(), row["api_format"].clone()))
        .collect();
    assert_eq!(
        formats,
        [
            (json!("claude-sonnet-5-5"), json!("chat_completions")),
            (json!("gpt-6.1-sol"), json!("responses")),
            (json!("gpt-image-2"), Value::Null),
        ]
    );
    let answer = |chat: &[Value]| -> String {
        chat.iter()
            .filter(|r| r["kind"] == "ai.chat.delta")
            .map(|r| {
                r["data"]["delta"]["content"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect()
    };
    assert_eq!(last(6)["kind"], "ai.chat", "{}", last(6));
    assert_eq!(answer(&records[6]), "from claude");
    assert_eq!(last(7)["kind"], "ai.chat", "{}", last(7));
    assert_eq!(answer(&records[7]), "from codex");
    let tools = &records[8];
    assert!(tools
        .iter()
        .any(|r| r["kind"] == "ai.chat.tool_consent_required"));
    let result = tools
        .iter()
        .find(|r| r["kind"] == "ai.chat.tool_result")
        .unwrap();
    assert_eq!(result["data"]["ok"], true, "{result}");
    assert_eq!(last(8)["kind"], "ai.chat", "{}", last(8));
    assert_eq!(last(9)["error"]["code"], "ai_cli_proxy_model_unknown");
    assert_eq!(last(10)["error"]["code"], "ai_cli_proxy_format_unknown");
    assert_eq!(last(10)["error"]["details"]["reason"], "not_a_chat_model");
    // Nothing before Test & discover; then the listing, two Claude legs plus
    // the tool turn's two over Chat Completions, Codex over the Responses
    // API, and nothing for the refused models.
    assert_eq!(
        paths(&proxy),
        pairs(&[
            ("GET", "/v1/models"),
            ("POST", "/v1/chat/completions"),
            ("POST", "/v1/responses"),
            ("POST", "/v1/chat/completions"),
            ("POST", "/v1/chat/completions"),
        ])
    );
    let requests = proxy.requests();
    let bearer = format!("Bearer {KEY}");
    assert!(requests
        .iter()
        .all(|r| r.authorization.as_deref() == Some(bearer.as_str())));
    let effort: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(effort["reasoning_effort"], "xhigh");
    assert_eq!(effort["store"], false);
    let codex: Value = serde_json::from_slice(&requests[2].body).unwrap();
    assert_eq!(codex["store"], false);
}

/// One `--machine` CLI run; the key, if any, on stdin (`--api-key-fd 0`).
fn cli(sandbox: &Sandbox, argv: &[&str], key: Option<&str>) -> (i32, Value) {
    let root = sandbox.root.to_string_lossy().into_owned();
    let mut args: Vec<String> = ["--machine", "--data-root", &root]
        .iter()
        .chain(argv)
        .map(|s| (*s).to_owned())
        .collect();
    if key.is_some() {
        args.extend(["--api-key-fd".to_owned(), "0".to_owned()]);
    }
    let out = sandbox.raw(&args, key.map(str::as_bytes));
    let both = format!("{}{}", out.text(), String::from_utf8_lossy(&out.stderr));
    assert!(!both.contains(KEY), "the key never prints");
    (out.code, out.json())
}

#[test]
fn test_the_cli_creates_a_cli_proxy_and_contacts_it_only_to_list_or_test() {
    let sandbox = book();
    let proxy = ScriptedProvider::new().unwrap();
    let base = proxy.base_url();
    let create = [
        "ai",
        "providers",
        "create",
        "proxy",
        "--kind",
        "cli_proxy",
        "--base-url",
        &base,
    ];
    // Like the desktop wizard, it is saved only with the acknowledgement.
    let (code, refused) = cli(&sandbox, &create, Some(KEY));
    assert_ne!(code, 0);
    assert_eq!(refused["error"]["code"], "validation");
    assert_eq!(
        refused["error"]["message"],
        "A CLI proxy provider needs --acknowledge"
    );
    // It takes no format, no store choice and no CLI locator.
    for (extra, message) in [
        (
            &["--api-format", "responses"][..],
            "CLI proxy providers do not take an API format",
        ),
        (
            &["--omit-store-field"],
            "CLI proxy providers do not take omit_store_field",
        ),
    ] {
        let argv = [&create[..], &["--acknowledge"], extra].concat();
        let (_, envelope) = cli(&sandbox, &argv, None);
        assert_eq!(envelope["error"]["message"], message);
    }
    let (_, envelope) = cli(
        &sandbox,
        &[
            "ai",
            "providers",
            "create",
            "proxy",
            "--kind",
            "cli_proxy",
            "--base-url",
            "claude-cli://default",
            "--acknowledge",
        ],
        None,
    );
    assert_eq!(
        envelope["error"]["message"],
        "A CLI proxy provider needs the proxy's HTTP address"
    );
    let argv = [
        &create[..],
        &["--acknowledge", "--default-model", "claude-sonnet-5-5"],
    ]
    .concat();
    let (code, created) = cli(&sandbox, &argv, Some(KEY));
    assert_eq!(code, 0, "{created}");
    assert_eq!(created["data"]["kind"], "cli_proxy");
    assert_eq!(created["data"]["has_api_key"], true);
    assert_ne!(created["data"]["acknowledged_at"], Value::Null);
    assert_eq!(created["data"]["supports_reasoning_effort"], true);
    cli(&sandbox, &["ai", "providers", "list"], None);
    cli(&sandbox, &["ai", "providers", "get", "proxy"], None);
    assert!(paths(&proxy).is_empty());
    proxy.queue("GET", "models", vec![models(), models()]);
    proxy.queue(
        "POST",
        "chat/completions",
        vec![chat_stream(&[
            chunk(json!({"role": "assistant", "content": "OK"}), None),
            chunk(json!({}), Some("stop")),
        ])],
    );
    let (code, listed) = cli(&sandbox, &["ai", "models", "--provider", "proxy"], None);
    assert_eq!(code, 0, "{listed}");
    let format = |id: &str| {
        listed["data"]["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == id)
            .unwrap()["api_format"]
            .clone()
    };
    assert_eq!(format("claude-sonnet-5-5"), "chat_completions");
    assert_eq!(format("gpt-6.1-sol"), "responses");
    let (code, tested) = cli(&sandbox, &["ai", "providers", "test", "proxy"], None);
    assert_eq!(code, 0, "{tested}");
    assert_eq!(tested["data"]["probe"]["api_format"], "chat_completions");
    assert_eq!(tested["data"]["models_saved"], true);
    assert_eq!(
        paths(&proxy),
        pairs(&[
            ("GET", "/v1/models"),
            ("GET", "/v1/models"),
            ("POST", "/v1/chat/completions"),
        ])
    );
    let bearer = format!("Bearer {KEY}");
    assert!(proxy
        .requests()
        .iter()
        .all(|r| r.authorization.as_deref() == Some(bearer.as_str())));
    // The grammar names the kind.
    let help = sandbox.raw(
        &[
            "ai".to_owned(),
            "providers".to_owned(),
            "create".to_owned(),
            "--help".to_owned(),
        ],
        None,
    );
    assert!(help.text().contains("{local,remote,tee,cli_proxy}"));
}
