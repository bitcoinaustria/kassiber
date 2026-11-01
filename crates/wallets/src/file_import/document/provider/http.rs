use super::*;

fn json_type(v: &Py) -> &'static str {
    match v {
        Py::Dict(_) => "dict",
        Py::List(_) => "list",
        Py::Str(_) => "str",
        Py::Int(_) => "int",
        Py::Float(_) => "float",
        Py::Bool(_) => "bool",
        Py::Null => "NoneType",
        _ => "object",
    }
}
fn models(payload: &Py) -> Result<Vec<Py>, AppError> {
    let Py::List(data) = field(payload, "data") else {
        return Err(AppError::new(
            "ai_request_invalid",
            "AI provider models response did not contain a data list",
        )
        .with_hint("Check that the base URL points at an OpenAI-compatible /v1 endpoint.")
        .with_details(dict([("shape", s(json_type(payload)))])));
    };
    Ok(data
        .iter()
        .filter(|v| matches!(field(v, "id"), Py::Str(_)))
        .map(|v| dict([("id", field(v, "id").clone())]))
        .collect())
}
fn body(model: &str, parts: &[Py], wire: &str, omit: bool) -> Py {
    let mut content = vec![dict([("type",s("text")),("text",s("Extract a draft Kassiber transaction table from this local document. Return JSON only and include per-cell confidence plus source regions."))])];
    content.extend_from_slice(parts);
    if wire == "chat_completions" {
        let mut result = dict([
            ("model", s(model)),
            (
                "messages",
                Py::List(vec![
                    dict([
                        ("role", s("system")),
                        ("content", s(include_str!("../ocr_prompt.txt"))),
                    ]),
                    dict([("role", s("user")), ("content", Py::List(content))]),
                ]),
            ),
            ("stream", Py::Bool(false)),
        ]);
        if !omit {
            set(&mut result, "store", Py::Bool(false));
        }
        set(&mut result, "temperature", Py::from(0));
        set(
            &mut result,
            "response_format",
            dict([("type", s("json_object"))]),
        );
        result
    } else {
        let content = content
            .into_iter()
            .map(|v| {
                if field(&v, "type") == &s("image_url") {
                    dict([
                        ("type", s("input_image")),
                        ("image_url", field(field(&v, "image_url"), "url").clone()),
                    ])
                } else {
                    dict([
                        ("type", s("input_text")),
                        ("text", field(&v, "text").clone()),
                    ])
                }
            })
            .collect();
        dict([
            ("temperature", Py::from(0)),
            (
                "text",
                dict([("format", dict([("type", s("json_object"))]))]),
            ),
            ("model", s(model)),
            (
                "input",
                Py::List(vec![dict([
                    ("type", s("message")),
                    ("role", s("user")),
                    ("content", Py::List(content)),
                ])]),
            ),
            ("stream", Py::Bool(false)),
            ("store", Py::Bool(false)),
            ("instructions", s(include_str!("../ocr_prompt.txt"))),
        ])
    }
}
fn error_details(status: u16, text: &str) -> Py {
    let mut details = dict([("status", Py::from(i64::from(status)))]);
    if !text.is_empty() {
        let preview = text.chars().take(512).collect::<String>();
        set(
            &mut details,
            "body",
            s(kassiber_envelope::redaction::redact_secret_text(&preview)),
        );
        set(
            &mut details,
            "body_truncated",
            Py::Bool(text.chars().count() > 512),
        );
    }
    details
}
fn http_error(status: u16, text: &str) -> AppError {
    let (code, message, hint, retry) = match status {
        401 | 403 => (
            "ai_auth_failed",
            "AI provider rejected the request as unauthorized".into(),
            "Check the provider's API key in Settings → AI providers.",
            false,
        ),
        429 => (
            "ai_rate_limited",
            "AI provider is rate-limiting requests".into(),
            "Wait a few seconds and try again, or pick a different provider.",
            true,
        ),
        500..=599 => (
            "ai_unavailable",
            format!("AI provider returned a server error ({status})"),
            "The provider may be temporarily unavailable. Try again or switch providers.",
            true,
        ),
        _ => (
            "ai_request_invalid",
            format!("AI provider rejected the request ({status})"),
            "Check the model name and request shape.",
            false,
        ),
    };
    AppError::new(code, message)
        .with_hint(hint)
        .retryable(retry)
        .with_details(error_details(status, text))
}
fn route_missing(error: &AppError) -> bool {
    let details = error.details.as_ref().unwrap_or(&Py::Null);
    let status = rendered(field(details, "status")).parse::<u16>().ok();
    if !matches!(status, Some(400 | 404 | 405 | 501)) {
        return false;
    }
    let text = kassiber_pytext::lower(&format!(
        "{} {}",
        rendered(field(details, "body")),
        error.message
    ));
    if ["model", "tool", "endpoints found"]
        .iter()
        .any(|s| text.contains(s))
    {
        return false;
    }
    status != Some(400)
        || [
            "unexpected endpoint",
            "unknown endpoint",
            "unsupported endpoint",
            "endpoint not found",
            "no such endpoint",
            "route not found",
            "unknown route",
        ]
        .iter()
        .any(|s| text.contains(s))
}
pub(super) fn names_store(error: &AppError) -> bool {
    let details = error.details.as_ref().unwrap_or(&Py::Null);
    if ![Py::from(400), Py::from(422)].contains(field(details, "status")) {
        return false;
    }
    let raw = field(details, "body");
    let body = if matches!(raw, Py::Str(_)) && !kassiber_pytext::strip(&rendered(raw)).is_empty() {
        rendered(raw)
    } else {
        error.message.clone()
    };
    let parsed = kassiber_pytext::loads(&body);
    let parsed = match parsed {
        Ok(v) => v,
        Err(_) if truthy(field(details, "body_truncated")) => return false,
        Err(_) => return store_word(&body),
    };
    fn visit(node: &Py, depth: usize, fields: &mut Vec<String>, messages: &mut Vec<String>) {
        if depth > 6 {
            return;
        }
        match node {
            Py::Str(_) => messages.push(kassiber_pytext::lower(&rendered(node))),
            Py::List(v) => {
                for item in v {
                    visit(item, depth + 1, fields, messages)
                }
            }
            Py::Dict(v) => {
                for (key, value) in v {
                    let Key::Str(key) = key else { continue };
                    let key = kassiber_pytext::lower(&rendered(&Py::Str(key.clone())));
                    if ["param", "parameter", "loc", "path", "field", "property"]
                        .contains(&key.as_str())
                    {
                        let path = match value {
                            Py::List(v) => v
                                .iter()
                                .filter(|v| matches!(v, Py::Str(_) | Py::Int(_) | Py::Bool(_)))
                                .map(rendered)
                                .collect::<Vec<_>>(),
                            Py::Str(_) => rendered(value)
                                .split(['.', '[', ']', '/'])
                                .map(str::to_owned)
                                .filter(|v| !v.is_empty())
                                .collect(),
                            _ => vec![],
                        };
                        let path = path
                            .iter()
                            .map(|v| {
                                kassiber_pytext::lower(
                                    kassiber_pytext::strip(v).trim_matches(['\'', '"']),
                                )
                            })
                            .filter(|v| !v.is_empty())
                            .collect::<Vec<_>>();
                        if let Some(first) = path
                            .iter()
                            .find(|v| !["body", "query", "request", "json"].contains(&v.as_str()))
                        {
                            fields.push(first.clone());
                        }
                    } else if [
                        "message",
                        "msg",
                        "detail",
                        "description",
                        "reason",
                        "error",
                        "errors",
                        "details",
                    ]
                    .contains(&key.as_str())
                    {
                        visit(value, depth + 1, fields, messages)
                    }
                }
            }
            _ => (),
        }
    }
    let mut fields = vec![];
    let mut messages = vec![];
    visit(&parsed, 0, &mut fields, &mut messages);
    if !fields.is_empty() {
        fields.iter().any(|v| v == "store")
    } else {
        messages.iter().any(|v| store_word(v))
    }
}
fn store_word(text: &str) -> bool {
    let text = kassiber_pytext::lower(text);
    let letter = |c: char| c.is_ascii_lowercase() || c == '_';
    text.match_indices("store").any(|(index, _)| {
        !text[..index].chars().next_back().is_some_and(letter)
            && !text[index + 5..].chars().next().is_some_and(letter)
    })
}
pub(super) fn transport_error(error: kassiber_net::Error) -> AppError {
    match error {
        kassiber_net::Error::RedirectRefused => AppError::new(
            "ai_request_invalid",
            "AI provider attempted an off-origin redirect",
        ),
        kassiber_net::Error::Io(_) | kassiber_net::Error::Tls | kassiber_net::Error::InvalidTimeout => {
            AppError::new("ai_unavailable", "AI provider is unreachable")
                .with_hint("Is the provider running? For local Ollama, try `ollama serve`. For remote providers, check connectivity.")
                .with_details(dict([("reason", s(error.to_string()))]))
                .retryable(true)
        }
        error => error.app_error(),
    }
}
fn store_refused(error: AppError) -> AppError {
    let mut details = error.details.unwrap_or_else(|| dict::<&str>([]));
    set(&mut details, "reason", s("store_refused"));
    AppError::new("ai_store_refused","This endpoint refused Kassiber's no-retention request (store: false)").with_details(details).with_hint("Run Test connection in Settings → AI providers to see your options, or `kassiber ai providers test <name>`.")
}
fn response_error(error: &Py, responses: bool) -> AppError {
    let raw_code = field(error, "code");
    let status = match raw_code {
        Py::Int(v) => v.to_string().parse::<u16>().ok(),
        _ => None,
    };
    let provider_code = if *raw_code == Py::Null {
        String::new()
    } else {
        kassiber_pytext::lower(&rendered(raw_code))
    };
    let error_type = kassiber_pytext::lower(&rendered(field(error, "type")));
    let encoded = kassiber_pyjson::dumps(error, &Options::new().sort_keys(true)).unwrap();
    let mut details = dict([
        (
            "provider_code",
            if provider_code.is_empty() {
                Py::Null
            } else {
                s(&provider_code)
            },
        ),
        (
            "body",
            s(kassiber_envelope::redaction::redact_secret_text(
                &encoded.chars().take(512).collect::<String>(),
            )),
        ),
        ("body_truncated", Py::Bool(encoded.chars().count() > 512)),
        ("in_stream", Py::Bool(true)),
    ]);
    if let Some(status) = status {
        set(&mut details, "status", Py::from(i64::from(status)));
    }
    let (code, message, retry) = if status == Some(429)
        || provider_code.contains("rate")
        || error_type.contains("rate_limit")
    {
        (
            "ai_rate_limited",
            "AI provider is rate-limiting requests",
            true,
        )
    } else if matches!(status, Some(401 | 403))
        || ["api_key", "auth", "forbidden", "permission"]
            .iter()
            .any(|v| provider_code.contains(v))
    {
        (
            "ai_auth_failed",
            "AI provider rejected the request as unauthorized",
            false,
        )
    } else if status.is_some_and(|v| (400..500).contains(&v))
        || ["invalid_prompt", "invalid_request", "invalid_request_error"]
            .contains(&provider_code.as_str())
        || error_type == "invalid_request_error"
    {
        (
            "ai_request_invalid",
            if responses {
                "AI provider rejected the Responses request"
            } else {
                "AI provider rejected the request"
            },
            false,
        )
    } else {
        (
            "ai_unavailable",
            "AI provider failed while generating a response",
            true,
        )
    };
    AppError::new(code, message)
        .retryable(retry)
        .with_details(details)
}
pub(super) fn content(payload: &Py, responses: bool) -> Result<String, AppError> {
    if !matches!(payload, Py::Dict(_)) {
        return Err(AppError::new(
            "ai_request_invalid",
            "AI provider returned an invalid response payload",
        ));
    }
    let mut error = field(payload, "error").clone();
    if !responses && !truthy(&error) && field(payload, "object") == &s("error") {
        let Py::Dict(values) = payload else {
            unreachable!()
        };
        error = Py::Dict(
            values
                .iter()
                .filter(|(k, _)| *k != Key::Str("object".into()))
                .cloned()
                .collect(),
        );
    }
    if (responses && matches!(error, Py::Dict(_)))
        || (!responses && truthy(&error))
        || (responses && field(payload, "status") == &s("failed"))
    {
        let empty = dict::<&str>([]);
        let text = dict([("message", s(rendered(&error)))]);
        return Err(response_error(
            if matches!(error, Py::Dict(_)) {
                &error
            } else if !responses {
                &text
            } else {
                &empty
            },
            responses,
        ));
    }
    fn part(v: &Py, responses: bool) -> String {
        let kind = rendered(field(v, "type"));
        let value = if kind == "refusal" {
            field(v, "refusal")
        } else if kind == "output_text"
            || (!responses && ["text", "input_text"].contains(&kind.as_str()))
        {
            field(v, "text")
        } else {
            &Py::Null
        };
        match value {
            Py::Str(_) => rendered(value),
            _ => String::new(),
        }
    }
    if responses {
        let mut text = String::new();
        if let Py::List(output) = field(payload, "output") {
            for item in output {
                if field(item, "type") == &s("message") {
                    if let Py::List(parts) = field(item, "content") {
                        for v in parts {
                            text.push_str(&part(v, true));
                        }
                    }
                }
            }
        }
        Ok(text)
    } else {
        let choice = match field(payload, "choices") {
            Py::List(v) => v.first().filter(|v| matches!(v, Py::Dict(_))),
            _ => None,
        }
        .ok_or_else(|| {
            AppError::new(
                "ai_request_invalid",
                "AI provider returned a Chat Completions payload without choices",
            )
        })?;
        let message = field(choice, "message");
        let mut text = match field(message, "content") {
            Py::Str(_) => rendered(field(message, "content")),
            Py::List(v) => v.iter().map(|v| part(v, false)).collect(),
            _ => String::new(),
        };
        if matches!(field(message, "refusal"), Py::Str(_)) {
            text.push_str(&rendered(field(message, "refusal")));
        }
        Ok(text)
    }
}

pub(super) struct Invocation<'a> {
    client: &'a Providers,
    conn: &'a Connection,
    request: &'a DocumentPreview,
    consent: &'a DocumentConsent,
    provider: &'a Py,
}
impl<'a> Invocation<'a> {
    pub(super) fn new(
        client: &'a Providers,
        conn: &'a Connection,
        request: &'a DocumentPreview,
        consent: &'a DocumentConsent,
        provider: &'a Py,
    ) -> Self {
        Self {
            client,
            conn,
            request,
            consent,
            provider,
        }
    }
    fn json(&self, path: &str, body: Option<Py>, timeout: u64) -> Result<Py, AppError> {
        let Self {
            client,
            conn,
            request,
            consent,
            provider,
        } = self;
        consent.verify(
            conn,
            request,
            &resolve_provider(conn, request.provider_name.as_deref())?,
        )?;
        validate_local(provider)?;
        let key = api_key(conn, provider)?;
        let base = rendered(field(provider, "base_url"));
        let url = format!(
            "{}/{}",
            base.trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        let parsed = kassiber_net::HttpUrl::parse(&url)
            .map_err(|_| AppError::new("ai_request_invalid", "Invalid AI provider URL"))?;
        let tls = if parsed.scheme() == "https" {
            TlsTrust::python_default().map_err(|e| e.app_error())?
        } else {
            TlsTrust::Plaintext
        };
        let endpoint = Endpoint::new(
            parsed
                .host_str()
                .unwrap_or_default()
                .trim_matches(['[', ']']),
            parsed.port_or_known_default().unwrap_or(0),
        )
        .map_err(|e| e.app_error())?;
        let route = NetRequest::new(Subsystem::Ai, endpoint, tls).on_device_ai();
        let route = client
            .gate
            .authorize(route.clone(), Some(Consent::explicit_trigger(&route)))
            .map_err(|e| e.app_error())?;
        let mut http = HttpRequest::new(if body.is_some() { "POST" } else { "GET" }, &url)
            .map_err(|e| e.app_error())?
            .header("User-Agent", "kassiber/ai")
            .map_err(|e| e.app_error())?
            .header("Accept", "application/json")
            .map_err(|e| e.app_error())?;
        if let Some(key) = key {
            http = http
                .header("Authorization", &format!("Bearer {key}"))
                .map_err(|e| e.app_error())?;
        }
        if let Some(body) = body {
            http = http
                .header("Content-Type", "application/json")
                .map_err(|e| e.app_error())?
                .body(
                    kassiber_pyjson::dumps(&body, &Options::default())
                        .map_err(|e| AppError::new("internal_error", e.to_string()))?
                        .into_bytes(),
                );
        }
        let response = http
            .send(
                route,
                HttpOptions {
                    timeout: Duration::from_secs(timeout),
                    redirects: RedirectPolicy::SameOrigin,
                    ..Default::default()
                },
            )
            .map_err(transport_error)?;
        let text = String::from_utf8_lossy(&response.body);
        if response.status >= 400 {
            return Err(http_error(response.status, &text));
        }
        kassiber_pytext::loads(&text).map_err(|e| {
            if path == "models" {
                AppError::new(
                    "ai_request_invalid",
                    "AI provider returned a non-JSON models response",
                )
                .with_hint("Check that the base URL points at an OpenAI-compatible /v1 endpoint.")
            } else {
                AppError::new(
                    "ai_request_invalid",
                    "AI provider returned a non-JSON response",
                )
                .with_details(dict([("reason", s(e.to_string()))]))
            }
        })
    }
    pub(super) fn discover(&self) -> Result<Vec<Py>, AppError> {
        models(&self.json("models", None, 30)?)
    }
    pub(super) fn ocr(&self, model: &str, parts: &[Py]) -> Result<String, AppError> {
        let Self { conn, provider, .. } = self;
        let configured = rendered(field(provider, "api_format"));
        let detected = rendered(field(provider, "detected_api_format"));
        let known = |v: &str| ["responses", "chat_completions"].contains(&v);
        let mut wire = if known(&configured) {
            configured.as_str()
        } else if known(&detected) {
            detected.as_str()
        } else {
            "responses"
        };
        let fallback = !known(&configured) && !known(&detected);
        let omit = truthy(field(provider, "omit_store_field"));
        let mut fell_back = false;
        loop {
            let data = body(model, parts, wire, omit);
            let sent_store = get(&data, "store").is_some();
            let response = self.json(
                if wire == "responses" {
                    "responses"
                } else {
                    "chat/completions"
                },
                Some(data),
                75,
            );
            let response=match response {
                Err(error) if sent_store && names_store(&error) => return Err(store_refused(error)),
                Err(error) if wire=="responses" && fallback && route_missing(&error) => {wire="chat_completions";fell_back=true;continue;},
                Err(error) if fell_back && route_missing(&error) => return Err(AppError::new("ai_request_invalid","AI provider has neither a Responses nor a Chat Completions endpoint at this base URL").with_hint("Check that the base URL is the API root, usually ending in /v1.").with_details(error.details.unwrap_or(Py::Null))),
                result => result?,
            };
            let text = content(&response, wire == "responses")?;
            if fell_back {
                conn.execute("UPDATE ai_providers SET detected_api_format='chat_completions' WHERE name=? AND base_url=? AND (detected_api_format IS NULL OR detected_api_format!='chat_completions')",[rendered(field(provider,"name")),rendered(field(provider,"base_url"))]).map_err(database_error)?;
                commit(conn)?;
            }
            return Ok(text);
        }
    }
}

#[cfg(test)]
pub(super) fn request_body(model: &str, parts: &[Py], wire: &str, omit: bool) -> Py {
    body(model, parts, wire, omit)
}
