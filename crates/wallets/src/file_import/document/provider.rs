use super::*;
use kassiber_net::{
    Consent, Endpoint, Gate, HttpOptions, HttpRequest, Offline, RedirectPolicy,
    Request as NetRequest, Subsystem, TlsTrust,
};
use std::{path::Path, time::Duration};

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
        files::source_path(&request.source_file)?;
        threshold(&request.confidence_threshold)?;
        preview::max_pages(&request.max_pages)?;
        let book_id = setting(conn, "database_instance_id")?.ok_or_else(consent_error)?;
        Ok(Self {
            request: request.clone(),
            book_id,
            provider: resolve_provider(conn, request.provider_name.as_deref())?,
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
fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), AppError> {
    conn.execute("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [key,value]).map_err(database_error)?;
    Ok(())
}
fn commit(conn: &Connection) -> Result<(), AppError> {
    if !conn.is_autocommit() {
        conn.execute_batch("COMMIT").map_err(database_error)?;
    }
    Ok(())
}
fn seeded(conn: &Connection) -> Result<(), AppError> {
    let Py::List(defaults) =
        kassiber_pytext::loads(include_str!("provider_defaults.json")).unwrap()
    else {
        unreachable!()
    };
    let exists = |sql: &str, values: &[&str]| -> Result<bool, AppError> {
        conn.query_row(sql, params_from_iter(values), |_| Ok(()))
            .optional()
            .map(|v| v.is_some())
            .map_err(database_error)
    };
    if setting(conn, "ai_providers_seeded")?.is_none_or(|s| s.is_empty()) {
        if !exists("SELECT 1 FROM ai_providers LIMIT 1", &[])? {
            let default = std::env::var("KASSIBER_DEFAULT_AI_PROVIDER")
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "ollama".into());
            let default = kassiber_pytext::lower(kassiber_pytext::strip(&default));
            if !defaults.iter().any(|v| field(v, "name") == &s(&default)) {
                return Err(AppError::new(
                    "validation",
                    format!("Unsupported default AI provider '{default}'"),
                )
                .with_hint("Choose one of: claude, codex, copilot, ollama, omlx, opencode"));
            }
            let timestamp = lifecycle::now(conn)?;
            for provider in &defaults {
                let name = rendered(field(provider, "name"));
                let base = std::env::var(format!(
                    "KASSIBER_{}_AI_BASE_URL",
                    name.to_uppercase().replace('-', "_")
                ))
                .ok()
                .filter(|v| !v.is_empty())
                .or_else(|| {
                    (name == default)
                        .then(|| {
                            std::env::var("KASSIBER_DEFAULT_AI_BASE_URL")
                                .ok()
                                .filter(|v| !v.is_empty())
                        })
                        .flatten()
                })
                .unwrap_or_else(|| rendered(field(provider, "base_url")));
                insert_provider(
                    conn,
                    provider,
                    &name,
                    &base,
                    &timestamp,
                    field(provider, "kind") == &s("local"),
                )?;
            }
            if setting(conn, "default_ai_provider")?.is_none_or(|v| v.is_empty()) {
                set_setting(conn, "default_ai_provider", &default)?;
            }
        }
        set_setting(conn, "ai_providers_seeded", "1")?;
        commit(conn)?;
    }
    for (sentinel, names) in [
        (
            "ai_native_broker_providers_seeded",
            &["codex", "claude", "opencode"][..],
        ),
        ("ai_acp_broker_providers_seeded", &["copilot"][..]),
    ] {
        if setting(conn, sentinel)?.is_some_and(|s| !s.is_empty()) {
            continue;
        }
        let timestamp = lifecycle::now(conn)?;
        for provider in &defaults {
            let name = rendered(field(provider, "name"));
            let base = rendered(field(provider, "base_url"));
            if !names.contains(&name.as_str())
                || exists(
                    "SELECT 1 FROM ai_providers WHERE lower(base_url)=lower(?) LIMIT 1",
                    &[&base],
                )?
            {
                continue;
            }
            let mut free = name.clone();
            let mut suffix = 2;
            while exists("SELECT 1 FROM ai_providers WHERE name=? LIMIT 1", &[&free])? {
                free = format!("{name}-{suffix}");
                suffix += 1;
            }
            insert_provider(conn, provider, &free, &base, &timestamp, false)?;
        }
        set_setting(conn, sentinel, "1")?;
        commit(conn)?;
    }
    Ok(())
}
fn insert_provider(
    conn: &Connection,
    provider: &Py,
    name: &str,
    base: &str,
    timestamp: &str,
    acknowledged: bool,
) -> Result<(), AppError> {
    conn.execute("INSERT INTO ai_providers(name,display_name,base_url,api_key,default_model,kind,notes,acknowledged_at,created_at,updated_at) VALUES(?,?,?,NULL,?,?,?,?,?,?)",
        params![name,rendered(field(provider,"display_name")),base, if field(provider,"default_model") == &Py::Null {None} else {Some(rendered(field(provider,"default_model")))},rendered(field(provider,"kind")),rendered(field(provider,"notes")),acknowledged.then_some(timestamp),timestamp,timestamp]).map_err(database_error)?;
    Ok(())
}
fn resolve_provider(conn: &Connection, name: Option<&str>) -> Result<Py, AppError> {
    seeded(conn)?;
    let name = name
        .map(str::to_owned)
        .or(setting(conn, "default_ai_provider")?);
    let Some(name) = name.filter(|v| !v.is_empty()) else {
        return Err(AppError::new("ai_provider_not_configured","No AI provider is configured").with_hint("Run `kassiber ai providers create` or set a default with `kassiber ai providers set-default`."));
    };
    let name = kassiber_pytext::lower(kassiber_pytext::strip(&name));
    if name.is_empty() {
        return Err(AppError::new("validation", "AI provider name is required"));
    }
    let mut stmt = conn.prepare("SELECT p.*,r.store_id AS secret_store_id,r.service AS secret_service,r.account AS secret_account,r.state AS secret_state,r.created_at AS secret_created_at,r.rotated_at AS secret_rotated_at FROM ai_providers p LEFT JOIN ai_provider_secret_refs r ON r.provider_name=p.name WHERE p.name=?").map_err(database_error)?;
    let names = stmt
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    let mut provider = stmt
        .query_row([&name], |row| {
            use kassiber_store::rusqlite::types::ValueRef;
            let values = names
                .iter()
                .enumerate()
                .map(|(i, key)| {
                    let value = match row.get_ref(i)? {
                        ValueRef::Null => Py::Null,
                        ValueRef::Integer(n) => Py::from(n),
                        ValueRef::Real(n) => Py::Float(n),
                        ValueRef::Text(t) => s(String::from_utf8_lossy(t)),
                        ValueRef::Blob(_) => Py::Null,
                    };
                    Ok((key.clone(), value))
                })
                .collect::<kassiber_store::rusqlite::Result<Vec<_>>>()?;
            Ok(dict(values))
        })
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| {
            AppError::new("not_found", format!("AI provider '{name}' not found"))
                .with_hint("Use `kassiber ai providers list` to see configured providers.")
        })?;
    if truthy(field(&provider, "secret_store_id")) {
        let store =
            kassiber_pytext::lower(&rendered(&cleaned(field(&provider, "secret_store_id"))));
        if ![
            "macos_keychain",
            "windows_dpapi",
            "linux_secret_service",
            "sqlcipher_inline",
        ]
        .contains(&store.as_str())
        {
            return Err(AppError::new("validation", format!("Unsupported AI provider secret store '{store}'")).with_hint("Choose one of: macos_keychain, windows_dpapi, linux_secret_service, sqlcipher_inline"));
        }
        set(&mut provider, "secret_store_id", s(store));
        if truthy(field(&provider, "secret_state")) {
            let state =
                kassiber_pytext::lower(&rendered(&cleaned(field(&provider, "secret_state"))));
            if !["ok", "missing", "needs_reauth", "unavailable"].contains(&state.as_str()) {
                return Err(AppError::new(
                    "validation",
                    format!("Unsupported AI provider secret state '{state}'"),
                )
                .with_hint("Choose one of: ok, missing, needs_reauth, unavailable"));
            }
            set(&mut provider, "secret_state", s(state));
        }
    }
    Ok(provider)
}
fn validate_local(provider: &Py) -> Result<(), AppError> {
    let name = rendered(field(provider, "name"));
    if field(provider, "enabled") != &Py::Null && !truthy(field(provider, "enabled")) {
        return Err(AppError::new("ai_provider_disabled",format!("AI provider '{name}' is disabled")).with_hint(format!("Enable it with `kassiber ai providers update {name} --enable`, or switch it on in Settings → AI providers.")).with_details(dict([("provider",s(name))])));
    }
    let kind = rendered(field(provider, "kind"));
    if kind != "local" && !truthy(field(provider, "acknowledged_at")) {
        return Err(AppError::new("ai_remote_ack_required",format!("AI provider '{name}' requires explicit off-device acknowledgement before chat")).with_hint(format!("Run `kassiber ai providers update {name} --acknowledge`, or confirm the provider in Settings → AI providers.")).with_details(dict([("provider",s(&name)),("kind",s(&kind))])));
    }
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
fn api_key(conn: &Connection, provider: &Py) -> Result<Option<String>, AppError> {
    let store = cleaned(field(provider, "secret_store_id"));
    if store == Py::Null || store == s("sqlcipher_inline") {
        return Ok((cleaned(field(provider, "api_key")) != Py::Null)
            .then(|| rendered(&cleaned(field(provider, "api_key")))));
    }
    let store = rendered(&store);
    if !["macos_keychain", "windows_dpapi", "linux_secret_service"].contains(&store.as_str()) {
        return Err(AppError::new(
            "validation",
            format!("Unsupported AI provider secret store '{store}'"),
        )
        .with_hint(
            "Choose one of: macos_keychain, windows_dpapi, linux_secret_service, sqlcipher_inline",
        ));
    }
    let name = rendered(field(provider, "name"));
    let root: String = conn
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?
        .unwrap_or_default();
    let root = if root.is_empty() {
        String::new()
    } else {
        kassiber_paths::canonicalize_allow_missing(Path::new(&root))
            .map_err(|e| AppError::new(e.code, e.message))?
            .parent()
            .unwrap_or(Path::new(""))
            .to_string_lossy()
            .into_owned()
    };
    let service = Sha256::digest(format!("at.bitcoinaustria.kassiber:{root}").as_bytes())
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect::<String>();
    let raw_state = cleaned(field(provider, "secret_state"));
    let mut state = if raw_state == Py::Null {
        if cleaned(field(provider, "api_key")) == Py::Null {
            "missing".into()
        } else {
            "ok".into()
        }
    } else {
        kassiber_pytext::lower(&rendered(&raw_state))
    };
    if !["ok", "missing", "needs_reauth", "unavailable"].contains(&state.as_str()) {
        return Err(AppError::new(
            "validation",
            format!("Unsupported AI provider secret state '{state}'"),
        )
        .with_hint("Choose one of: ok, missing, needs_reauth, unavailable"));
    }
    if [("secret_service", &service), ("secret_account", &name)]
        .iter()
        .any(|(k, expected)| truthy(field(provider, k)) && field(provider, k) != &s(expected))
    {
        state = "unavailable".into();
    }
    let was_ok = state == "ok";
    if was_ok {
        state = "unavailable".into();
    }
    let message = if was_ok {
        format!("AI provider '{name}' secret is not available in the OS credential store")
    } else {
        format!("AI provider '{name}' secret is not available")
    };
    Err(AppError::new("secret_ref_unavailable", message)
        .retryable(true)
        .with_hint("Open Settings -> AI providers and re-enter or repair the provider API key.")
        .with_details(dict([
            (
                "refs",
                Py::List(vec![dict([
                    ("provider_name", s(&name)),
                    ("store_id", s(store)),
                    ("service", s(service)),
                    ("account", s(name)),
                    ("state", s(&state)),
                ])]),
            ),
            ("state", s(state)),
        ])))
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
