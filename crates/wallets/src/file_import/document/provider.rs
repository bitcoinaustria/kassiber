use super::*;
use kassiber_net::{
    Consent, Endpoint, Gate, HttpOptions, HttpRequest, Offline, RedirectPolicy,
    Request as NetRequest, Subsystem, TlsTrust,
};
use std::time::Duration;

mod http;
mod preview;
#[cfg(test)]
mod tests;

#[derive(Clone, PartialEq)]
pub struct DocumentPreview {
    pub source_file: String,
    pub provider_name: Option<String>,
    pub model: Option<String>,
    pub confidence_threshold: Py,
    pub max_pages: Py,
    pub pages: Py,
    pub expected_fiat_currency: Option<String>,
}

pub struct DocumentConsent {
    request: DocumentPreview,
    book_id: String,
    provider: Py,
    context: (Option<String>, Option<String>),
}
impl DocumentConsent {
    pub fn explicit(conn: &Connection, request: &DocumentPreview) -> Result<Self, AppError> {
        // `preview_document_import` (#698): the provider is resolved and
        // contactable before `read_document` checks its arguments.
        let provider = resolve_provider(conn, request.provider_name.as_deref())?;
        kassiber_ai::providers::require_contactable(&provider)?;
        files::source_path(&request.source_file)?;
        threshold(&request.confidence_threshold)?;
        preview::max_pages(&request.max_pages)?;
        let book_id = setting(conn, "database_instance_id")?.ok_or_else(consent_error)?;
        Ok(Self {
            request: request.clone(),
            book_id,
            provider,
            context: (
                setting(conn, "context_workspace")?,
                setting(conn, "context_profile")?,
            ),
        })
    }
    fn verify(
        &self,
        conn: &Connection,
        request: &DocumentPreview,
        provider: &Py,
    ) -> Result<(), AppError> {
        if self.request != *request
            || self.provider != *provider
            || setting(conn, "database_instance_id")?.as_deref() != Some(&self.book_id)
            || self.context
                != (
                    setting(conn, "context_workspace")?,
                    setting(conn, "context_profile")?,
                )
        {
            return Err(consent_error());
        }
        Ok(())
    }
}
fn consent_error() -> AppError {
    AppError::new(
        "interaction_required",
        "Document OCR requires current consent for this document and provider",
    )
}

pub struct Providers {
    gate: Gate,
}
impl Providers {
    pub fn new(gate: Gate) -> Self {
        Self { gate }
    }
    pub fn system() -> Self {
        Self::new(Gate::new(
            || match kassiber_paths::Context::from_env()
                .ok()
                .and_then(|c| kassiber_setup::offline::preference_path(&c).ok())
            {
                Some(path) if kassiber_setup::offline::enabled(&path) => Offline::Offline,
                Some(_) => Offline::Online,
                None => Offline::Unavailable,
            },
            Default::default(),
        ))
    }
}

fn setting(conn: &Connection, key: &str) -> Result<Option<String>, AppError> {
    conn.query_row("SELECT value FROM settings WHERE key=?", [key], |r| {
        r.get(0)
    })
    .optional()
    .map_err(database_error)
}
fn commit(conn: &Connection) -> Result<(), AppError> {
    if !conn.is_autocommit() {
        conn.execute_batch("COMMIT").map_err(database_error)?;
    }
    Ok(())
}
/// `resolve_ai_provider(conn, name)`, from the provider store chat shares.
fn resolve_provider(conn: &Connection, name: Option<&str>) -> Result<Py, AppError> {
    kassiber_ai::providers::resolve(conn, name)
}
fn validate_local(provider: &Py) -> Result<(), AppError> {
    kassiber_ai::providers::require_contactable(provider)?;
    let name = rendered(field(provider, "name"));
    let kind = rendered(field(provider, "kind"));
    let (scheme, host) = files::locator(&rendered(field(provider, "base_url")))?;
    if kind != "local" || !["http", "https"].contains(&scheme.as_str()) {
        return Err(AppError::new(
            "document_import_local_ai_required",
            "Document OCR requires a local AI provider",
        )
        .with_hint("Configure a local Ollama provider such as http://localhost:11434/v1."));
    }
    if !host.as_deref().is_some_and(|h| {
        h == "localhost"
            || h.parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    }) {
        return Err(AppError::new("document_import_remote_ai_disabled","Document OCR is hard-disabled for off-device AI providers").with_hint("Use a loopback Ollama endpoint. Photos, receipts, and statements are never sent to remote providers by this importer.").with_details(dict([("provider",s(name)),("base_url_host",host.map_or(Py::Null,s))])));
    }
    Ok(())
}
/// `get_ai_provider_api_key_for_use(provider)`: inline keys only, as
/// Python's document import resolves them.
fn api_key(_conn: &Connection, provider: &Py) -> Result<Option<String>, AppError> {
    kassiber_ai::providers::api_key_for_use(provider, None, None)
}

const RECOMMENDATIONS: &[(&str, &str)] = &[
    (
        "glm-ocr",
        "Fast document OCR and table extraction on modest hardware.",
    ),
    (
        "qwen3-vl:8b",
        "Best default for OCR plus table reasoning when memory allows it.",
    ),
    ("qwen3-vl:4b", "Smaller Qwen3-VL option for laptops."),
    (
        "llama3.2-vision:11b",
        "General image reasoning fallback with broad Ollama availability.",
    ),
    (
        "minicpm-v:8b",
        "Compact multimodal fallback for receipts and screenshots.",
    ),
];
fn recommendations() -> Py {
    Py::List(
        RECOMMENDATIONS
            .iter()
            .map(|(id, why)| {
                dict([
                    ("id", s(id)),
                    ("command", s(format!("ollama pull {id}"))),
                    ("use", s(why)),
                ])
            })
            .collect(),
    )
}
fn vision(model: &str) -> bool {
    let lower = kassiber_pytext::lower(kassiber_pytext::strip(model));
    [
        "vision",
        "qwen3-vl",
        "qwen2.5vl",
        "qwen2-vl",
        "glm-ocr",
        "deepseek-ocr",
        "minicpm-v",
        "llava",
        "bakllava",
        "moondream",
        "gemma3",
        "ocr",
        "-vl",
    ]
    .iter()
    .any(|v| lower.contains(v))
}
fn choose_model(
    provider: &Py,
    models: &[Py],
    requested: Option<&str>,
) -> Result<(String, Vec<String>), AppError> {
    let installed = models
        .iter()
        .map(|m| field(m, "id"))
        .map(|v| {
            if truthy(v) {
                rendered(&cleaned(v))
            } else {
                String::new()
            }
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    let find = |model: &str| {
        installed.iter().find(|s| {
            kassiber_pytext::lower(kassiber_pytext::strip(s))
                == kassiber_pytext::lower(kassiber_pytext::strip(model))
        })
    };
    let hint = "Install a local vision model with Ollama, for example: ollama pull glm-ocr, ollama pull qwen3-vl:8b, ollama pull qwen3-vl:4b.";
    if let Some(requested) = requested.filter(|s| !s.is_empty()) {
        let requested = kassiber_pytext::strip(requested);
        let found = find(requested);
        if !installed.is_empty() && found.is_none() {
            return Err(AppError::new(
                "document_import_model_missing",
                format!("Local vision model '{requested}' is not installed"),
            )
            .with_hint(hint)
            .with_details(dict([
                ("requested_model", s(requested)),
                (
                    "installed_models",
                    Py::List(installed.iter().map(s).collect()),
                ),
                ("recommendations", recommendations()),
            ])));
        }
        if !vision(requested) {
            return Err(AppError::new(
                "document_import_model_not_vision",
                format!("Model '{requested}' does not look like a vision/OCR model"),
            )
            .with_hint("Choose an installed model with vision/OCR capabilities.")
            .with_details(dict([
                ("requested_model", s(requested)),
                ("recommendations", recommendations()),
            ])));
        }
        return Ok((
            found.cloned().unwrap_or_else(|| requested.to_owned()),
            installed,
        ));
    }
    let default = if truthy(field(provider, "default_model")) {
        rendered(&cleaned(field(provider, "default_model")))
    } else {
        String::new()
    };
    if let Some(found) = find(&default).filter(|m| !default.is_empty() && vision(m)) {
        return Ok((found.clone(), installed));
    }
    for (model, _) in RECOMMENDATIONS {
        if let Some(found) = find(model) {
            return Ok((found.clone(), installed));
        }
    }
    if let Some(found) = installed.iter().find(|m| vision(m)) {
        return Ok((found.clone(), installed));
    }
    Err(AppError::new(
        "document_import_model_missing",
        "No local vision model is installed",
    )
    .with_hint(hint)
    .with_details(dict([
        (
            "installed_models",
            Py::List(installed.iter().map(s).collect()),
        ),
        ("recommendations", recommendations()),
    ])))
}
fn extract(text: &str) -> Result<Py, AppError> {
    let raw = kassiber_pytext::strip(text);
    if raw.is_empty() {
        return Err(AppError::new(
            "document_import_ai_response_invalid",
            "Local AI returned an empty OCR response",
        )
        .retryable(true));
    }
    let fenced = regex::Regex::new(r"(?s)```(?:json)?\s*(.*?)\s*```")
        .unwrap()
        .captures(raw);
    let raw = fenced.as_ref().map_or(raw, |c| c.get(1).unwrap().as_str());
    let raw = match (raw.find('{'), raw.rfind('}')) {
        (Some(start), Some(end)) if end > start => &raw[start..=end],
        _ => raw,
    };
    let payload = kassiber_pytext::loads(raw).map_err(|e| {
        AppError::new(
            "document_import_ai_response_invalid",
            "Local AI returned non-JSON OCR output",
        )
        .with_hint("Try a stronger local vision model or a clearer image.")
        .with_details(dict([
            ("reason", s(e.to_string())),
            ("response_length", Py::from(raw.chars().count() as i64)),
        ]))
        .retryable(true)
    })?;
    if !matches!(payload, Py::Dict(_)) {
        return Err(AppError::new(
            "document_import_ai_response_invalid",
            "Local AI OCR output must be a JSON object",
        )
        .retryable(true));
    }
    Ok(payload)
}
