//! The gates every path that contacts a provider passes, before any
//! request: enabled, acknowledged for off-device use, and still at the
//! endpoint the caller means.

use kassiber_envelope::{dict, truthy, AppError};
use kassiber_pyjson::Value;

use super::{is_loopback_http_url, normalize_base_url};
use crate::pyval::{field, s, str_or_none};

/// `require_ai_provider_acknowledged(provider)`.
pub fn require_acknowledged(provider: &Value) -> Result<(), AppError> {
    if *field(provider, "kind") == s("local") || truthy(field(provider, "acknowledged_at")) {
        return Ok(());
    }
    let name = str_or_none(field(provider, "name")).unwrap_or_else(|| "<unknown>".into());
    let kind = str_or_none(field(provider, "kind")).unwrap_or_else(|| "remote".into());
    Err(AppError::new(
        "ai_remote_ack_required",
        format!("AI provider '{name}' requires explicit off-device acknowledgement before chat"),
    )
    .with_hint(format!(
        "Run `kassiber ai providers update {name} --acknowledge`, or confirm the provider in Settings → AI providers."
    ))
    .with_details(dict([("provider", s(&name)), ("kind", s(kind))])))
}

/// `require_ai_test_target_acknowledged(...)`: a connection test sends a
/// prompt, so an off-device target needs the acknowledgement chat needs.
pub fn require_test_target_acknowledged(
    base_url: &str,
    kind: Option<&str>,
    acknowledged: bool,
    saved: Option<&Value>,
) -> Result<(), AppError> {
    if kind == Some("local") && is_loopback_http_url(base_url) {
        return Ok(());
    }
    if acknowledged {
        return Ok(());
    }
    if let Some(saved) = saved {
        let saved_kind = field(saved, "kind");
        if (*saved_kind == s("remote") || *saved_kind == s("tee"))
            && truthy(field(saved, "acknowledged_at"))
        {
            return Ok(());
        }
    }
    let name = saved.and_then(|saved| str_or_none(field(saved, "name")));
    let hint = format!(
        "Tick the acknowledgement in the provider form, or acknowledge the saved provider {}",
        match &name {
            Some(name) => format!("with `kassiber ai providers update {name} --acknowledge`."),
            None => "first.".into(),
        }
    );
    let mut details = vec![("kind", s(kind.unwrap_or("remote")))];
    if let Some(name) = &name {
        details.push(("provider", s(name)));
    }
    Err(AppError::new(
        "ai_remote_ack_required",
        "Testing this AI provider sends a prompt off this device and needs explicit acknowledgement first",
    )
    .with_hint(hint)
    .with_details(dict(details)))
}

/// `require_ai_provider_enabled(provider)`.
pub fn require_enabled(provider: &Value) -> Result<(), AppError> {
    // `provider.get("enabled", True)`.
    let enabled = kassiber_envelope::get(provider, "enabled").is_none_or(truthy);
    if enabled {
        return Ok(());
    }
    let name = str_or_none(field(provider, "name")).unwrap_or_else(|| "<unknown>".into());
    Err(
        AppError::new("ai_provider_disabled", format!("AI provider '{name}' is disabled"))
            .with_hint(format!(
                "Enable it with `kassiber ai providers update {name} --enable`, or switch it on in Settings → AI providers."
            ))
            .with_details(dict([("provider", s(name))])),
    )
}

/// `require_ai_provider_contactable(provider)`: enabled first, then
/// acknowledged.
pub fn require_contactable(provider: &Value) -> Result<(), AppError> {
    require_enabled(provider)?;
    require_acknowledged(provider)
}

/// `require_ai_provider_endpoint(provider, expected_base_url)`: a request
/// bound to an endpoint the row no longer has is refused. `None` passes.
pub fn require_endpoint(provider: &Value, expected: &Value) -> Result<(), AppError> {
    if *expected == Value::Null {
        return Ok(());
    }
    if !matches!(expected, Value::Str(_)) {
        return Err(AppError::new(
            "validation",
            "AI provider base_url must be a string",
        ));
    }
    let name = str_or_none(field(provider, "name")).unwrap_or_else(|| "<unknown>".into());
    if normalize_base_url(expected)? == normalize_base_url(field(provider, "base_url"))? {
        return Ok(());
    }
    Err(AppError::new(
        "ai_provider_changed",
        format!("AI provider '{name}' points at a different endpoint than the one requested"),
    )
    .with_hint("The provider was edited meanwhile. Review it and check it again.")
    .with_details(dict([("provider", s(name))])))
}
