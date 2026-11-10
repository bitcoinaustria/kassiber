//! CLI proxy providers (docs/reference/ai-cli-proxy.md) against a loopback
//! fake CLIProxyAPI: the model list with mixed families, each model spoken
//! in its family's format with streaming and tool calls, auth on and off,
//! errors with the credential withheld, the saved model inventory, and no
//! request before the user starts a test or a refresh. The fake is
//! CLIProxyAPI's behaviour as a handler on `kassiber_net`'s loopback
//! [`FakeService`]; this crate opens no socket of its own.

use std::sync::atomic::Ordering;
use std::time::Duration;

use kassiber_ai::chat::{ChatClient, ProviderSnapshot};
use kassiber_ai::discovery::Cache;
use kassiber_ai::service::{daemon, Host, CHAT_PROVIDER};
use kassiber_ai::transport::{Receipt, RequestContext};
use kassiber_envelope::{dict, get, AppError};
use kassiber_net::test_support::{FakeService, HandledRequest, Reply};
use kassiber_net::{Gate, Ledger, Offline};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;

const KEY: &str = "sk-proxy-0123456789abcdef";

fn s(text: &str) -> Value {
    kassiber_envelope::s(text)
}

fn load(raw: &str) -> Value {
    kassiber_pytext::loads(raw).unwrap()
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap_or(&Value::Null)
}

fn text(value: &Value, key: &str) -> Option<String> {
    match field(value, key) {
        Value::Str(text) => text.as_str().map(str::to_owned),
        _ => None,
    }
}

fn dumps(value: &Value) -> String {
    kassiber_pyjson::dumps(value, &Default::default()).unwrap()
}

/// The models the fake lists, as CLIProxyAPI's OpenAI view does: `id`,
/// `object`, `created`, `owned_by`. A credential with `prefix: "work"` serves
/// its models again as `work/<model>`, the same entry with only the id
/// changed (`applyModelPrefixes` in `sdk/cliproxy/service_models.go`), so
/// `owned_by` is kept.
const MODELS: &str = r#"{"object": "list", "data": [
  {"id": "claude-sonnet-5-5", "object": "model", "created": 1, "owned_by": "anthropic"},
  {"id": "gpt-6.1-sol", "object": "model", "created": 1, "owned_by": "openai"},
  {"id": "gemini-3-pro-preview", "object": "model", "created": 1, "owned_by": "google"},
  {"id": "gpt-image-2", "object": "model", "created": 1, "owned_by": "openai"},
  {"id": "mystery-1", "object": "model", "created": 1, "owned_by": "someproxy"},
  {"id": "work/claude-sonnet-5-5", "object": "model", "created": 1, "owned_by": "anthropic"},
  {"id": "work/gpt-6.1-sol", "object": "model", "created": 1, "owned_by": "openai"}
]}"#;

/// A listing that gives no `owned_by` for its prefixed ids (another proxy,
/// or an older one): `home/<model>` can only take the family the listing
/// gives `<model>`, and `solo/<model>` (a `force-model-prefix` credential,
/// whose base is not listed) has none.
const BARE_PREFIXED_MODELS: &str = r#"{"object": "list", "data": [
  {"id": "claude-sonnet-5-5", "object": "model", "created": 1, "owned_by": "anthropic"},
  {"id": "home/claude-sonnet-5-5", "object": "model", "created": 1},
  {"id": "solo/claude-opus-5-5", "object": "model", "created": 1}
]}"#;

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Value,
}

#[derive(Clone, Default)]
struct Behaviour {
    /// The client keys the proxy accepts; none means it takes any request,
    /// as CLIProxyAPI does with no `api-keys` configured.
    key: Option<String>,
    /// Repeat the offered key in the 401 body, as a careless proxy might.
    echo_key: bool,
    /// Answer the model list with this status instead.
    models_status: Option<u16>,
    /// List `BARE_PREFIXED_MODELS` instead of `MODELS`.
    bare_prefixed: bool,
}

/// A loopback CLIProxyAPI stand-in. Codex models answer only on
/// `/v1/responses` and every other family only on `/v1/chat/completions`,
/// so a request in the wrong format fails the test loudly.
struct FakeProxy {
    service: FakeService,
}

impl FakeProxy {
    fn start(behaviour: Behaviour) -> Self {
        let service =
            FakeService::start(move |request| answer(&behaviour, &seen(request))).unwrap();
        FakeProxy { service }
    }

    /// Down and back at the same address, now behaving as `behaviour`.
    fn restart(&mut self, behaviour: Behaviour) {
        self.service
            .restart(move |request| answer(&behaviour, &seen(request)))
            .unwrap();
    }

    fn base_url(&self) -> String {
        self.service.base_url()
    }

    fn seen(&self) -> Vec<Seen> {
        self.service.requests().iter().map(seen).collect()
    }

    fn paths(&self) -> Vec<String> {
        self.service.paths()
    }
}

/// A request as the CLIProxyAPI behaviour reads it: its JSON body parsed.
fn seen(request: &HandledRequest) -> Seen {
    let body = if request.body.is_empty() {
        Value::Null
    } else {
        kassiber_pytext::loads(&String::from_utf8_lossy(&request.body)).unwrap_or(Value::Null)
    };
    Seen {
        method: request.method.clone(),
        path: request.target.clone(),
        authorization: request.header("authorization").map(str::to_owned),
        body,
    }
}

fn sse(events: &[String]) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(&format!("data: {event}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    body
}

fn first_tool(body: &Value) -> Option<String> {
    let Value::List(tools) = field(body, "tools") else {
        return None;
    };
    let tool = tools.first()?;
    text(field(tool, "function"), "name")
        .or_else(|| text(tool, "name"))
        .filter(|name| name != "connection_check")
}

fn answer(behaviour: &Behaviour, request: &Seen) -> Reply {
    let (status, content_type, body) = answer_parts(behaviour, request);
    Reply::raw(status, content_type, body.as_bytes())
}

fn answer_parts(behaviour: &Behaviour, request: &Seen) -> (u16, &'static str, String) {
    const JSON: &str = "application/json";
    if let Some(key) = &behaviour.key {
        match request.authorization.as_deref() {
            None => return (401, JSON, r#"{"error":"Missing API key"}"#.into()),
            Some(given) if given != format!("Bearer {key}") => {
                let message = if behaviour.echo_key {
                    format!("Invalid API key: {given}")
                } else {
                    "Invalid API key".into()
                };
                return (401, JSON, dumps(&dict([("error", s(&message))])));
            }
            Some(_) => {}
        }
    }
    let model = text(&request.body, "model").unwrap_or_default();
    // A prefixed id routes as its model does.
    let codex = model.rsplit('/').next().unwrap_or("").starts_with("gpt-");
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/v1/models") => match behaviour.models_status {
            Some(status) => (status, JSON, r#"{"error":"upstream down"}"#.into()),
            None if behaviour.bare_prefixed => (200, JSON, BARE_PREFIXED_MODELS.into()),
            None => (200, JSON, MODELS.into()),
        },
        ("POST", "/v1/chat/completions") if !codex => {
            let messages = match field(&request.body, "messages") {
                Value::List(items) => items.clone(),
                _ => Vec::new(),
            };
            let answered_tool = messages
                .iter()
                .any(|m| text(m, "role").as_deref() == Some("tool"));
            let events = match first_tool(&request.body).filter(|_| !answered_tool) {
                Some(tool) => vec![
                    format!(
                        r#"{{"id":"c1","object":"chat.completion.chunk","model":"{model}","choices":[{{"index":0,"delta":{{"role":"assistant","tool_calls":[{{"index":0,"id":"call_proxy_1","type":"function","function":{{"name":"{tool}","arguments":"{{}}"}}}}]}},"finish_reason":null}}]}}"#
                    ),
                    r#"{"id":"c1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#.into(),
                ],
                None => vec![
                    format!(
                        r#"{{"id":"c1","object":"chat.completion.chunk","model":"{model}","choices":[{{"index":0,"delta":{{"role":"assistant","content":"OK from {model}"}},"finish_reason":null}}]}}"#
                    ),
                    r#"{"id":"c1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#.into(),
                ],
            };
            (200, "text/event-stream", sse(&events))
        }
        ("POST", "/v1/responses") if codex => {
            let input = match field(&request.body, "input") {
                Value::List(items) => items.clone(),
                _ => Vec::new(),
            };
            let answered_tool = input
                .iter()
                .any(|item| text(item, "type").as_deref() == Some("function_call_output"));
            let output = match first_tool(&request.body).filter(|_| !answered_tool) {
                Some(tool) => format!(
                    r#"{{"type": "function_call", "id": "fc_1", "call_id": "call_proxy_1", "name": "{tool}", "arguments": "{{}}", "status": "completed"}}"#
                ),
                None => format!(
                    r#"{{"type": "message", "role": "assistant", "status": "completed", "content": [{{"type": "output_text", "text": "OK from {model}"}}]}}"#
                ),
            };
            let events = vec![format!(
                r#"{{"type": "response.completed", "response": {{"id": "resp_1", "status": "completed", "output": [{output}]}}}}"#
            )];
            (200, "text/event-stream", sse(&events))
        }
        ("POST", path) => (
            400,
            JSON,
            format!(r#"{{"error":{{"message":"fake proxy: {model} is not served on {path}"}}}}"#),
        ),
        _ => (404, JSON, r#"{"error":"not found"}"#.into()),
    }
}

/// A disposable book with Kassiber's creation schema.
struct Book {
    conn: Connection,
    root: std::path::PathBuf,
}

impl Drop for Book {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn book() -> Book {
    use std::sync::atomic::AtomicUsize;
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "kassiber-cli-proxy-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let conn = Connection::open(data.join("kassiber.sqlite3")).unwrap();
    conn.execute_batch("PRAGMA synchronous = OFF; PRAGMA journal_mode = MEMORY; BEGIN")
        .unwrap();
    let schema = load(include_str!("../../store/fixtures/creation.json"));
    let Some(Value::List(statements)) = get(&schema, "statements") else {
        panic!()
    };
    for statement in statements {
        conn.execute_batch(&kassiber_envelope::py_str(statement))
            .unwrap();
    }
    conn.execute_batch("COMMIT").unwrap();
    Book { conn, root }
}

struct TestHost {
    cache: Cache,
    gate: Gate,
}

impl TestHost {
    fn new() -> Self {
        TestHost {
            cache: Cache::default(),
            gate: Gate::new(|| Offline::Online, Ledger::default()),
        }
    }
}

impl Host for TestHost {
    fn secret_store(
        &mut self,
        _op: &str,
        _provider_name: &str,
        _store_id: &str,
        _service: &str,
        _account: &str,
        _secret: Option<&str>,
    ) -> Result<Value, AppError> {
        Err(AppError::new("secret_store_unavailable", "not in tests"))
    }
    fn discovery(&self) -> &Cache {
        &self.cache
    }
    fn gate(&self) -> Gate {
        self.gate.clone()
    }
}

fn call(book: &Book, host: &mut TestHost, kind: &str, args: &str) -> Result<Value, AppError> {
    daemon(&book.conn, host, kind, Some(&load(args)))
}

fn create_proxy(book: &Book, host: &mut TestHost, proxy: &FakeProxy, acknowledged: bool) {
    let args = dict([
        ("name", s("cli-proxy")),
        ("display_name", s("CLI proxy")),
        ("base_url", s(&proxy.base_url())),
        ("kind", s("cli_proxy")),
        ("acknowledged", Value::Bool(acknowledged)),
    ]);
    daemon(&book.conn, host, "ai.providers.create", Some(&args)).unwrap();
}

fn set_key(book: &Book, host: &mut TestHost, key: &str) {
    let args = dict([("name", s("cli-proxy")), ("api_key", s(key))]);
    daemon(&book.conn, host, "ai.providers.set_api_key", Some(&args)).unwrap();
}

fn chat_provider(book: &Book, host: &mut TestHost, model: &str) -> Result<Value, AppError> {
    let args = dict([("provider", s("cli-proxy")), ("model", s(model))]);
    daemon(&book.conn, host, CHAT_PROVIDER, Some(&args))
}

fn model_formats(models: &Value) -> Vec<(String, Value)> {
    let Value::List(rows) = models else {
        panic!("{models:?}")
    };
    rows.iter()
        .map(|row| (text(row, "id").unwrap(), field(row, "api_format").clone()))
        .collect()
}

fn expected_formats() -> Vec<(String, Value)> {
    vec![
        ("claude-sonnet-5-5".into(), s("chat_completions")),
        ("gpt-6.1-sol".into(), s("responses")),
        ("gemini-3-pro-preview".into(), s("chat_completions")),
        ("gpt-image-2".into(), Value::Null),
        ("mystery-1".into(), Value::Null),
        ("work/claude-sonnet-5-5".into(), s("chat_completions")),
        ("work/gpt-6.1-sol".into(), s("responses")),
    ]
}

#[test]
fn nothing_contacts_the_proxy_before_the_user_tests_or_refreshes() {
    let proxy = FakeProxy::start(Behaviour {
        key: Some(KEY.into()),
        ..Default::default()
    });
    let book = book();
    let mut host = TestHost::new();
    create_proxy(&book, &mut host, &proxy, false);
    set_key(&book, &mut host, KEY);
    // Display reads, edits and the provider list contact nothing.
    call(&book, &mut host, "ai.providers.list", "{}").unwrap();
    call(
        &book,
        &mut host,
        "ai.providers.get",
        r#"{"name": "cli-proxy"}"#,
    )
    .unwrap();
    let cached = call(
        &book,
        &mut host,
        "ai.list_models",
        r#"{"provider": "cli-proxy", "cached_only": true}"#,
    )
    .unwrap();
    assert_eq!(*field(&cached, "models"), Value::List(Vec::new()));
    assert_eq!(*field(&cached, "checked_at"), Value::Null);
    call(
        &book,
        &mut host,
        "ai.providers.update",
        r#"{"name": "cli-proxy", "notes": "tailnet"}"#,
    )
    .unwrap();
    // Unacknowledged: a refresh and a chat are refused before any request.
    let refused = call(
        &book,
        &mut host,
        "ai.list_models",
        r#"{"provider": "cli-proxy", "refresh": true}"#,
    )
    .unwrap_err();
    assert_eq!(refused.code, "ai_remote_ack_required");
    assert_eq!(
        chat_provider(&book, &mut host, "claude-sonnet-5-5")
            .unwrap_err()
            .code,
        "ai_remote_ack_required"
    );
    // A test of unsaved values needs the acknowledgement too.
    let args = dict([
        ("base_url", s(&proxy.base_url())),
        ("kind", s("cli_proxy")),
        ("api_key", s(KEY)),
    ]);
    assert_eq!(
        daemon(&book.conn, &mut host, "ai.test_connection", Some(&args))
            .unwrap_err()
            .code,
        "ai_remote_ack_required"
    );
    call(
        &book,
        &mut host,
        "ai.providers.acknowledge",
        r#"{"name": "cli-proxy"}"#,
    )
    .unwrap();
    // Acknowledged but never checked: a chat is refused from the missing
    // list, still without a request.
    let unchecked = chat_provider(&book, &mut host, "claude-sonnet-5-5").unwrap_err();
    assert_eq!(unchecked.code, "ai_cli_proxy_models_unchecked");
    assert!(proxy.seen().is_empty(), "{:?}", proxy.paths());
}

#[test]
fn test_and_discover_lists_every_family_and_saves_the_inventory() {
    let proxy = FakeProxy::start(Behaviour {
        key: Some(KEY.into()),
        ..Default::default()
    });
    let book = book();
    let mut host = TestHost::new();
    // The wizard's test of unsaved values: the list, no reply checked.
    let args = dict([
        ("base_url", s(&proxy.base_url())),
        ("kind", s("cli_proxy")),
        ("api_key", s(KEY)),
        ("acknowledged", Value::Bool(true)),
    ]);
    let unsaved = daemon(&book.conn, &mut host, "ai.test_connection", Some(&args)).unwrap();
    assert_eq!(model_formats(field(&unsaved, "models")), expected_formats());
    assert_eq!(*field(field(&unsaved, "probe"), "reason"), s("no_model"));
    assert_eq!(*field(&unsaved, "models_saved"), Value::Bool(false));
    assert_eq!(*field(&unsaved, "detected_api_format"), Value::Null);
    let mystery = match field(&unsaved, "models") {
        Value::List(rows) => rows[4].clone(),
        _ => unreachable!(),
    };
    assert_eq!(*field(&mystery, "unsupported_reason"), s("unknown_family"));
    assert_eq!(proxy.paths(), ["GET /v1/models"]);
    assert_eq!(
        proxy.seen()[0].authorization.as_deref(),
        Some(format!("Bearer {KEY}").as_str())
    );

    // Saved, then refreshed: the list becomes the provider's inventory.
    create_proxy(&book, &mut host, &proxy, true);
    set_key(&book, &mut host, KEY);
    let refreshed = call(
        &book,
        &mut host,
        "ai.list_models",
        r#"{"provider": "cli-proxy", "refresh": true}"#,
    )
    .unwrap();
    assert_eq!(
        model_formats(field(&refreshed, "models")),
        expected_formats()
    );
    let requests = proxy.seen().len();
    // A later launch reads it back without a request.
    let mut relaunched = TestHost::new();
    let cached = call(
        &book,
        &mut relaunched,
        "ai.list_models",
        r#"{"provider": "cli-proxy", "cached_only": true}"#,
    )
    .unwrap();
    assert_eq!(model_formats(field(&cached, "models")), expected_formats());

    // Chats take each model's format from it, without contacting the proxy.
    let claude = chat_provider(&book, &mut relaunched, "claude-sonnet-5-5").unwrap();
    assert_eq!(*field(&claude, "api_format"), s("chat_completions"));
    assert_eq!(*field(&claude, "cli_proxy"), Value::Bool(true));
    assert_eq!(*field(&claude, "api_key"), s(KEY));
    let codex = chat_provider(&book, &mut relaunched, "gpt-6.1-sol").unwrap();
    assert_eq!(*field(&codex, "api_format"), s("responses"));
    assert_eq!(*field(&codex, "detected_api_format"), Value::Null);
    for (model, code, reason) in [
        (
            "gpt-image-2",
            "ai_cli_proxy_format_unknown",
            Some("not_a_chat_model"),
        ),
        (
            "mystery-1",
            "ai_cli_proxy_format_unknown",
            Some("unknown_family"),
        ),
        ("claude-unlisted", "ai_cli_proxy_model_unknown", None),
    ] {
        let error = chat_provider(&book, &mut relaunched, model).unwrap_err();
        assert_eq!(error.code, code, "{model}");
        let details = error.details.clone().unwrap_or(Value::Null);
        assert_eq!(*field(&details, "model"), s(model));
        assert_eq!(text(&details, "reason").as_deref(), reason, "{model}");
    }
    assert_eq!(proxy.seen().len(), requests);

    // A new address forgets the inventory along with the key.
    let moved = dict([
        ("name", s("cli-proxy")),
        ("base_url", s("http://127.0.0.1:9/v1")),
    ]);
    daemon(
        &book.conn,
        &mut relaunched,
        "ai.providers.update",
        Some(&moved),
    )
    .unwrap();
    assert_eq!(
        chat_provider(&book, &mut relaunched, "claude-sonnet-5-5")
            .unwrap_err()
            .code,
        "ai_cli_proxy_models_unchecked"
    );
}

#[test]
fn testing_the_saved_provider_saves_its_models_and_probes_in_the_models_format() {
    let proxy = FakeProxy::start(Behaviour {
        key: Some(KEY.into()),
        ..Default::default()
    });
    let book = book();
    let mut host = TestHost::new();
    create_proxy(&book, &mut host, &proxy, true);
    set_key(&book, &mut host, KEY);
    for (model, path, format) in [
        (
            "claude-sonnet-5-5",
            "/v1/chat/completions",
            "chat_completions",
        ),
        ("gpt-6.1-sol", "/v1/responses", "responses"),
    ] {
        let args = dict([
            ("provider", s("cli-proxy")),
            ("base_url", s(&proxy.base_url())),
            ("model", s(model)),
        ]);
        let tested = daemon(&book.conn, &mut host, "ai.test_connection", Some(&args)).unwrap();
        let probe = field(&tested, "probe");
        assert_eq!(*field(probe, "status"), s("ok"), "{tested:?}");
        assert_eq!(*field(probe, "api_format"), s(format));
        assert_eq!(*field(&tested, "tools_accepted"), Value::Bool(true));
        assert_eq!(*field(&tested, "models_saved"), Value::Bool(true));
        assert_eq!(*field(&tested, "detection_saved"), Value::Bool(false));
        let last = proxy.seen().pop().unwrap();
        assert_eq!(last.path, path);
        // The no-retention flag goes with every proxied request.
        assert_eq!(*field(&last.body, "store"), Value::Bool(false));
    }
    // The saved test fed the inventory: a chat resolves without a refresh.
    assert_eq!(
        *field(
            &chat_provider(&book, &mut host, "gemini-3-pro-preview").unwrap(),
            "api_format"
        ),
        s("chat_completions")
    );
    // A model the list lacks, or that has no format, is reported, not asked.
    let before = proxy.seen().len();
    for (model, reason) in [
        ("nope", "model_not_listed"),
        ("gpt-image-2", "not_a_chat_model"),
    ] {
        let args = dict([("provider", s("cli-proxy")), ("model", s(model))]);
        let args = {
            let mut args = args;
            kassiber_envelope::set(&mut args, "base_url", s(&proxy.base_url()));
            args
        };
        let tested = daemon(&book.conn, &mut host, "ai.test_connection", Some(&args)).unwrap();
        assert_eq!(*field(field(&tested, "probe"), "status"), s("skipped"));
        assert_eq!(*field(field(&tested, "probe"), "reason"), s(reason));
    }
    // Only the two listings, no generation request.
    assert_eq!(proxy.seen().len(), before + 2);
    assert!(proxy.seen()[before..]
        .iter()
        .all(|r| r.path == "/v1/models"));
}

#[test]
fn a_proxy_without_keys_takes_requests_without_authorization() {
    let proxy = FakeProxy::start(Behaviour::default());
    let book = book();
    let mut host = TestHost::new();
    create_proxy(&book, &mut host, &proxy, true);
    let listed = call(
        &book,
        &mut host,
        "ai.list_models",
        r#"{"provider": "cli-proxy", "refresh": true}"#,
    )
    .unwrap();
    assert_eq!(model_formats(field(&listed, "models")), expected_formats());
    assert_eq!(proxy.seen()[0].authorization, None);
    let resolved = chat_provider(&book, &mut host, "claude-sonnet-5-5").unwrap();
    assert_eq!(*field(&resolved, "api_key"), Value::Null);
}

#[test]
fn auth_and_listing_errors_are_clear_and_never_repeat_the_key() {
    let wrong = "sk-proxy-wrong-key-9876543210";
    let proxy = FakeProxy::start(Behaviour {
        key: Some(KEY.into()),
        echo_key: true,
        ..Default::default()
    });
    let book = book();
    let mut host = TestHost::new();
    create_proxy(&book, &mut host, &proxy, true);
    set_key(&book, &mut host, wrong);
    let error = call(
        &book,
        &mut host,
        "ai.list_models",
        r#"{"provider": "cli-proxy", "refresh": true}"#,
    )
    .unwrap_err();
    assert_eq!(error.code, "ai_auth_failed");
    let shown = dumps(&error.envelope(None));
    assert!(!shown.contains(wrong), "{shown}");
    assert!(!shown.contains("wrong-key"), "{shown}");
    // The proxy's own words stay visible, minus the key.
    assert!(shown.contains("Invalid API key"), "{shown}");
    // Nothing was saved: a chat still asks for a refresh.
    assert_eq!(
        chat_provider(&book, &mut host, "claude-sonnet-5-5")
            .unwrap_err()
            .code,
        "ai_cli_proxy_models_unchecked"
    );
    // No key at all: the proxy's 401 is an auth failure, not an empty list.
    let keyless = dict([
        ("base_url", s(&proxy.base_url())),
        ("kind", s("cli_proxy")),
        ("acknowledged", Value::Bool(true)),
    ]);
    assert_eq!(
        daemon(&book.conn, &mut host, "ai.test_connection", Some(&keyless))
            .unwrap_err()
            .code,
        "ai_auth_failed"
    );
}

#[test]
fn a_failed_refresh_keeps_the_last_good_list_marked_stale() {
    let good = FakeProxy::start(Behaviour::default());
    let book = book();
    let mut host = TestHost::new();
    create_proxy(&book, &mut host, &good, true);
    call(
        &book,
        &mut host,
        "ai.list_models",
        r#"{"provider": "cli-proxy", "refresh": true}"#,
    )
    .unwrap();
    // The same address now failing: a 5xx, as an upstream outage.
    let mut failing = good;
    failing.restart(Behaviour {
        models_status: Some(503),
        ..Default::default()
    });
    let stale = call(
        &book,
        &mut host,
        "ai.list_models",
        r#"{"provider": "cli-proxy", "refresh": true}"#,
    )
    .unwrap();
    assert_eq!(*field(&stale, "stale"), Value::Bool(true));
    assert_eq!(*field(field(&stale, "error"), "code"), s("ai_unavailable"));
    assert_eq!(model_formats(field(&stale, "models")), expected_formats());
    assert_eq!(failing.paths(), ["GET /v1/models"]);
    // The stale list still answers chats.
    assert_eq!(
        *field(
            &chat_provider(&book, &mut host, "gpt-6.1-sol").unwrap(),
            "api_format"
        ),
        s("responses")
    );
}

fn proxied_client(proxy: &FakeProxy, format: &str) -> ChatClient {
    let snapshot = ProviderSnapshot::from_value(&dict([
        ("name", s("cli-proxy")),
        ("base_url", s(&proxy.base_url())),
        ("api_key", s(KEY)),
        ("kind", s("cli_proxy")),
        ("api_format", s(format)),
        ("cli_proxy", Value::Bool(true)),
        ("full_access", Value::Bool(true)),
    ]));
    ChatClient::for_provider(
        &snapshot,
        Duration::from_secs(10),
        Gate::new(|| Offline::Online, Ledger::default()),
        kassiber_ai::broker::default_locate(),
        Receipt::default(),
    )
    .unwrap()
}

fn user(text: &str) -> RequestContext {
    RequestContext {
        instructions: Some("Use tools.".into()),
        input_items: vec![dict([
            ("type", s("message")),
            ("role", s("user")),
            ("content", s(text)),
        ])],
    }
}

fn status_tool() -> Vec<Value> {
    vec![load(
        r#"{"type":"function","name":"ui_status","description":"Book status","parameters":{"type":"object","properties":{}}}"#,
    )]
}

#[test]
fn streamed_tool_calls_and_efforts_go_out_in_each_models_format() {
    let proxy = FakeProxy::start(Behaviour {
        key: Some(KEY.into()),
        ..Default::default()
    });
    for (format, model, effort, path, sent) in [
        (
            "chat_completions",
            "claude-sonnet-5-5",
            "xhigh",
            "/v1/chat/completions",
            "xhigh",
        ),
        (
            "chat_completions",
            "gemini-3-pro-preview",
            "ultra",
            "/v1/chat/completions",
            "max",
        ),
        ("responses", "gpt-6.1-sol", "max", "/v1/responses", "max"),
    ] {
        let mut client = proxied_client(&proxy, format);
        let mut calls = Vec::new();
        let options = dict([("reasoning_effort", s(effort))]);
        client
            .stream_tools(
                &user("status?"),
                model,
                &options,
                &status_tool(),
                &mut |chunk| {
                    if let Value::List(items) = field(&chunk.delta, "tool_calls") {
                        calls.extend(items.iter().cloned());
                    }
                    if let Some(output) = &chunk.response_output {
                        calls.extend(
                            output
                                .iter()
                                .filter(|item| {
                                    text(item, "type").as_deref() == Some("function_call")
                                })
                                .cloned(),
                        );
                    }
                    Ok(())
                },
            )
            .unwrap();
        assert!(!calls.is_empty(), "{model}: no tool call streamed");
        let request = proxy.seen().pop().unwrap();
        assert_eq!(request.path, path, "{model}");
        let effort_sent = if format == "responses" {
            field(field(&request.body, "reasoning"), "effort").clone()
        } else {
            field(&request.body, "reasoning_effort").clone()
        };
        assert_eq!(effort_sent, s(sent), "{model}");
        assert_eq!(*field(&request.body, "stream"), Value::Bool(true));
    }
}

#[test]
fn prefixed_credentials_keep_their_family_and_reach_the_proxy_under_their_own_id() {
    let proxy = FakeProxy::start(Behaviour {
        key: Some(KEY.into()),
        ..Default::default()
    });
    let book = book();
    let mut host = TestHost::new();
    create_proxy(&book, &mut host, &proxy, true);
    set_key(&book, &mut host, KEY);
    let listed = call(
        &book,
        &mut host,
        "ai.list_models",
        r#"{"provider": "cli-proxy", "refresh": true}"#,
    )
    .unwrap();
    let Value::List(rows) = field(&listed, "models") else {
        panic!()
    };
    let work = rows
        .iter()
        .find(|row| text(row, "id").as_deref() == Some("work/gpt-6.1-sol"))
        .unwrap();
    assert_eq!(*field(work, "prefix"), s("work"));
    assert_eq!(*field(work, "base_model"), s("gpt-6.1-sol"));
    assert_eq!(*field(work, "owned_by"), s("openai"));
    assert_eq!(*field(&rows[0], "prefix"), Value::Null);
    for (model, format, path) in [
        ("work/gpt-6.1-sol", "responses", "/v1/responses"),
        (
            "work/claude-sonnet-5-5",
            "chat_completions",
            "/v1/chat/completions",
        ),
    ] {
        let resolved = chat_provider(&book, &mut host, model).unwrap();
        assert_eq!(*field(&resolved, "api_format"), s(format), "{model}");
        let mut client = proxied_client(&proxy, format);
        client
            .stream_tools(
                &user("hi"),
                model,
                &dict::<&str>([]),
                &status_tool(),
                &mut |_| Ok(()),
            )
            .unwrap();
        let request = proxy.seen().pop().unwrap();
        assert_eq!(request.path, path, "{model}");
        // The proxy routes by the id: the prefix goes out as listed.
        assert_eq!(*field(&request.body, "model"), s(model));
    }
}

#[test]
fn without_owned_by_a_prefixed_id_takes_only_its_listed_base_family() {
    let proxy = FakeProxy::start(Behaviour {
        bare_prefixed: true,
        ..Default::default()
    });
    let book = book();
    let mut host = TestHost::new();
    create_proxy(&book, &mut host, &proxy, true);
    let listed = call(
        &book,
        &mut host,
        "ai.list_models",
        r#"{"provider": "cli-proxy", "refresh": true}"#,
    )
    .unwrap();
    assert_eq!(
        model_formats(field(&listed, "models")),
        vec![
            ("claude-sonnet-5-5".into(), s("chat_completions")),
            ("home/claude-sonnet-5-5".into(), s("chat_completions")),
            ("solo/claude-opus-5-5".into(), Value::Null),
        ]
    );
    assert_eq!(
        *field(
            &chat_provider(&book, &mut host, "home/claude-sonnet-5-5").unwrap(),
            "api_format"
        ),
        s("chat_completions")
    );
    // No listed base: nothing is guessed from the name.
    let refused = chat_provider(&book, &mut host, "solo/claude-opus-5-5").unwrap_err();
    assert_eq!(refused.code, "ai_cli_proxy_format_unknown");
    let details = refused.details.clone().unwrap();
    assert_eq!(*field(&details, "reason"), s("unknown_family"));
}
