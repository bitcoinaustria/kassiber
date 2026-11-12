use super::*;
use kassiber_ai::transport::errors::{http_error, provider_error, route_missing, store_refused};
pub(super) use kassiber_ai::transport::errors::{names_store, transport_error};
use kassiber_credentials::{credential_forms, url_credentials, Forms};

fn response_error(error: &Py, responses: bool, withheld: &Forms) -> AppError {
    provider_error(
        error,
        if responses {
            "AI provider rejected the Responses request"
        } else {
            "AI provider rejected the request"
        },
        withheld,
    )
}

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
pub(super) fn content(payload: &Py, responses: bool, withheld: &Forms) -> Result<String, AppError> {
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
            withheld,
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
    book: Option<super::preview::Book<'a>>,
    provider: &'a Py,
    detected: std::cell::Cell<bool>,
}
impl<'a> Invocation<'a> {
    pub(super) fn new(
        client: &'a Providers,
        book: Option<super::preview::Book<'a>>,
        provider: &'a Py,
    ) -> Self {
        Self {
            client,
            book,
            provider,
            detected: std::cell::Cell::new(false),
        }
    }
    /// The client's `_withheld()`: the provider's API key and any credential
    /// in its base URL, which no provider error may repeat.
    fn withheld(&self) -> Forms {
        let mut credentials =
            vec![
                kassiber_ai::providers::api_key_for_use(self.provider, None, None)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
            ];
        credentials.extend(url_credentials(
            &rendered(field(self.provider, "base_url")),
            None,
        ));
        credential_forms(credentials)
    }

    /// `client.newly_detected_api_format`: the server answered only in Chat
    /// Completions.
    pub(super) fn detected(&self) -> Option<&'static str> {
        self.detected.get().then_some("chat_completions")
    }
    fn json(&self, path: &str, body: Option<Py>, timeout: u64) -> Result<Py, AppError> {
        let Self {
            client,
            book,
            provider,
            ..
        } = self;
        if let Some(book) = book {
            book.consent.verify(
                book.conn,
                book.request,
                &resolve_provider(book.conn, book.request.provider_name.as_deref())?,
            )?;
        }
        validate_local(provider)?;
        let key = kassiber_ai::providers::api_key_for_use(provider, None, None)?;
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
            .map_err(|error| transport_error(error, &self.withheld()))?;
        let text = String::from_utf8_lossy(&response.body);
        if response.status >= 400 {
            return Err(http_error(response.status, &text, &self.withheld()));
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
        let Self { book, provider, .. } = self;
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
            let text = content(&response, wire == "responses", &self.withheld())?;
            if fell_back {
                self.detected.set(true);
                if let Some(book) = book {
                    book.conn.execute("UPDATE ai_providers SET detected_api_format='chat_completions' WHERE name=? AND base_url=? AND (detected_api_format IS NULL OR detected_api_format!='chat_completions')",[rendered(field(provider,"name")),rendered(field(provider,"base_url"))]).map_err(database_error)?;
                    commit(book.conn)?;
                }
            }
            return Ok(text);
        }
    }
}

#[cfg(test)]
pub(super) fn request_body(model: &str, parts: &[Py], wire: &str, omit: bool) -> Py {
    body(model, parts, wire, omit)
}
