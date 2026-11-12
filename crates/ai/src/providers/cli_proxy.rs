//! CLI proxy providers (`kind = "cli_proxy"`, docs/reference/ai-cli-proxy.md):
//! a CLIProxyAPI server that serves the user's signed-in coding-agent
//! subscriptions (Claude Code, Codex, Gemini CLI, ...) as one OpenAI-compatible
//! API. Rust-only: the Python backend refuses the kind.
//!
//! The proxy lists every model it can serve at `GET /v1/models`, with
//! `owned_by` naming the upstream family. Each model is spoken in the format
//! its family is served best in: Codex models (`openai`) natively over the
//! Responses API, every other known family over Chat Completions, which the
//! proxy translates. A model whose family is unknown, or that is not a chat
//! model, has no format and is refused before anything is sent.
//!
//! A credential the proxy gives a `prefix` serves its models a second time
//! as `<prefix>/<model>`, the same entry with only the id changed, so its
//! `owned_by` still names the family. Only when a listing gives no
//! `owned_by` at all does a `<prefix>/<model>` id take the family the same
//! listing gives `<model>`; nothing is ever guessed from a name.
//!
//! The list is read only when the user tests or refreshes the provider, and
//! is kept as the provider's saved model check; a chat reads that saved list
//! and never probes. Nothing here opens a socket.

use kassiber_envelope::{dict, set, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;

use super::checks::{self, CHECK_KIND_MODELS};
use super::{API_FORMAT_CHAT_COMPLETIONS, API_FORMAT_RESPONSES};
use crate::pyval::{field, s, string};

/// The provider kind.
pub const KIND: &str = "cli_proxy";

/// The reasoning levels the proxy takes and maps onto each family's own
/// thinking control (levels for Codex, token budgets for Claude and Gemini),
/// clamped to what the model supports.
pub const REASONING_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// `owned_by` values of the families the proxy serves, and the format each
/// is spoken in. Codex models answer natively on `/v1/responses`; the proxy
/// translates Chat Completions for every other family.
const FAMILY_FORMATS: &[(&str, &str)] = &[
    ("openai", API_FORMAT_RESPONSES),
    ("anthropic", API_FORMAT_CHAT_COMPLETIONS),
    ("google", API_FORMAT_CHAT_COMPLETIONS),
    ("antigravity", API_FORMAT_CHAT_COMPLETIONS),
    ("xai", API_FORMAT_CHAT_COMPLETIONS),
    ("moonshot", API_FORMAT_CHAT_COMPLETIONS),
];

/// Model ids the proxy lists that make images, video or speech, not chat
/// answers (`gpt-image-2`, `grok-imagine-video`, ...).
const MEDIA_MARKERS: &[&str] = &["image", "imagine", "video", "tts", "speech", "audio"];

/// Why a listed model has no format.
pub const NOT_A_CHAT_MODEL: &str = "not_a_chat_model";
pub const UNKNOWN_FAMILY: &str = "unknown_family";

/// Whether a provider row is a CLI proxy.
pub fn is_cli_proxy(provider: &Value) -> bool {
    *field(provider, "kind") == s(KIND)
}

fn is_media_model(id: &str) -> bool {
    let id = kassiber_pytext::lower(id);
    id.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| MEDIA_MARKERS.contains(&part))
}

/// The format a listed model is spoken in, or why it has none.
pub fn model_api_format(id: &str, owned_by: Option<&str>) -> Result<&'static str, &'static str> {
    if is_media_model(id) {
        return Err(NOT_A_CHAT_MODEL);
    }
    let family = owned_by.map(|owned| kassiber_pytext::lower(kassiber_pytext::strip(owned)));
    FAMILY_FORMATS
        .iter()
        .find(|(name, _)| Some(*name) == family.as_deref())
        .map(|(_, format)| *format)
        .ok_or(UNKNOWN_FAMILY)
}

/// `<prefix>/<model>` split at its first slash: a credential's namespaced
/// ids (`work/claude-sonnet-5-5`), and the proxy's own (`devin/swe-2`).
pub fn route_prefix(id: &str) -> Option<(&str, &str)> {
    id.split_once('/')
        .filter(|(prefix, model)| !prefix.trim().is_empty() && !model.trim().is_empty())
}

fn given(owned_by: Option<String>) -> Option<String> {
    owned_by.filter(|owned| !kassiber_pytext::strip(owned).is_empty())
}

/// The family a listed row's format comes from: its own `owned_by`, or,
/// only when it gives none, the `owned_by` the same listing gives the id
/// without its prefix. `None` when neither says.
fn listed_family(row: &Value, models: &[Value]) -> Option<String> {
    if let Some(owned) = given(string(field(row, "owned_by"))) {
        return Some(owned);
    }
    let id = string(field(row, "id"))?;
    let (_, base) = route_prefix(&id)?;
    models
        .iter()
        .find(|other| string(field(other, "id")).as_deref() == Some(base))
        .and_then(|other| given(string(field(other, "owned_by"))))
}

/// The format a row of the proxy's listing is spoken in, or why it has
/// none (`model_api_format` over `listed_family`).
pub fn listed_api_format(row: &Value, models: &[Value]) -> Result<&'static str, &'static str> {
    let id = string(field(row, "id")).unwrap_or_default();
    model_api_format(&id, listed_family(row, models).as_deref())
}

/// The proxy's model listing as Kassiber keeps it: each row with its
/// `api_format` (`null` with an `unsupported_reason` when it has none), and
/// for a chat model the reasoning levels the proxy takes and tool support,
/// which the proxy translates for every family it serves. A namespaced id
/// also gets its `prefix` and `base_model`, for the picker to group by.
pub fn annotate_models(models: Vec<Value>) -> Vec<Value> {
    let listed = models.clone();
    models
        .into_iter()
        .map(|mut row| {
            let id = string(field(&row, "id")).unwrap_or_default();
            if let Some((prefix, base)) = route_prefix(&id) {
                set(&mut row, "prefix", s(prefix));
                set(&mut row, "base_model", s(base));
            }
            match listed_api_format(&row, &listed) {
                Ok(format) => {
                    set(&mut row, "api_format", s(format));
                    set(&mut row, "supports_tools", Value::Bool(true));
                    set(&mut row, "supports_reasoning_effort", Value::Bool(true));
                    set(
                        &mut row,
                        "reasoning_efforts",
                        Value::List(REASONING_EFFORTS.iter().map(s).collect()),
                    );
                }
                Err(reason) => {
                    set(&mut row, "api_format", Value::Null);
                    set(&mut row, "unsupported_reason", s(reason));
                }
            }
            row
        })
        .collect()
}

/// `reasoning_effort` as the proxy takes it: Kassiber's `ultra` is the
/// proxy's `max`, `auto` and anything else send nothing.
pub fn reasoning_effort(value: &Value) -> Option<&'static str> {
    let Value::Str(text) = value else {
        return None;
    };
    let effort = kassiber_pytext::lower(kassiber_pytext::strip(&crate::pyval::lossy(text)));
    match effort.as_str() {
        "ultra" => Some("max"),
        other => REASONING_EFFORTS
            .iter()
            .find(|level| **level == other)
            .copied(),
    }
}

fn refusal(code: &str, message: String, provider: &str, model: &str) -> AppError {
    AppError::new(code, message)
        .with_details(dict([("provider", s(provider)), ("model", s(model))]))
}

/// The format `model` is spoken in on a CLI proxy provider, from the model
/// list its last test or refresh saved at its current address. Refuses,
/// before anything is sent, a provider never checked there, a model the
/// list does not have, and a model without a known format.
pub fn chat_api_format(
    conn: &Connection,
    provider: &Value,
    model: &str,
) -> Result<&'static str, AppError> {
    let name = kassiber_envelope::py_str(field(provider, "name"));
    let base_url = kassiber_envelope::py_str(field(provider, "base_url"));
    let model = kassiber_pytext::strip(model);
    let hint = format!(
        "Refresh the models in Settings → Assistant, or run `kassiber ai models --provider {name}`."
    );
    let Some(saved) = checks::load(conn, &name, CHECK_KIND_MODELS, &base_url)? else {
        return Err(refusal(
            "ai_cli_proxy_models_unchecked",
            format!("CLI proxy '{name}' has no model list yet"),
            &name,
            model,
        )
        .with_hint(hint));
    };
    let Value::List(models) = &saved.value else {
        return Err(refusal(
            "ai_cli_proxy_models_unchecked",
            format!("CLI proxy '{name}' has no model list yet"),
            &name,
            model,
        )
        .with_hint(hint));
    };
    let Some(row) = models
        .iter()
        .find(|row| string(field(row, "id")).as_deref() == Some(model))
    else {
        return Err(refusal(
            "ai_cli_proxy_model_unknown",
            format!("CLI proxy '{name}' did not list the model '{model}'"),
            &name,
            model,
        )
        .with_hint(hint));
    };
    // Recomputed from the listing, never trusted from the stored row alone.
    let owned_by = string(field(row, "owned_by"));
    listed_api_format(row, models).map_err(|reason| {
        let message = if reason == NOT_A_CHAT_MODEL {
            format!("The CLI proxy model '{model}' is not a chat model")
        } else {
            format!(
                "Kassiber does not know which API format the CLI proxy model '{model}' needs (owned_by: {})",
                owned_by.as_deref().unwrap_or("none")
            )
        };
        let mut error = refusal("ai_cli_proxy_format_unknown", message, &name, model)
            .with_hint("Choose another model from the proxy's list.");
        if let Some(details) = error.details.as_mut() {
            set(details, "reason", s(reason));
            set(details, "owned_by", owned_by.map_or(Value::Null, s));
        }
        error
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_map_to_their_format_and_unknown_ones_fail_closed() {
        assert_eq!(
            model_api_format("gpt-6.1-sol", Some("openai")),
            Ok("responses")
        );
        assert_eq!(
            model_api_format("claude-opus-5-5", Some("Anthropic ")),
            Ok("chat_completions")
        );
        for family in ["google", "antigravity", "xai", "moonshot"] {
            assert_eq!(model_api_format("m", Some(family)), Ok("chat_completions"));
        }
        assert_eq!(
            model_api_format("m", Some("openrouter")),
            Err(UNKNOWN_FAMILY)
        );
        assert_eq!(model_api_format("m", None), Err(UNKNOWN_FAMILY));
        assert_eq!(
            model_api_format("gpt-image-2", Some("openai")),
            Err(NOT_A_CHAT_MODEL)
        );
        assert_eq!(
            model_api_format("grok-imagine-video", Some("xai")),
            Err(NOT_A_CHAT_MODEL)
        );
        // A marker must be a whole id segment.
        assert_eq!(
            model_api_format("imagined-7", Some("openai")),
            Ok("responses")
        );
    }

    #[test]
    fn a_prefixed_id_takes_its_own_family_and_only_falls_back_to_the_listed_base() {
        let listing = vec![
            dict([("id", s("claude-sonnet-5-5")), ("owned_by", s("anthropic"))]),
            dict([("id", s("gpt-6.1-sol")), ("owned_by", s("openai"))]),
            // As CLIProxyAPI lists a prefixed credential: owned_by kept.
            dict([("id", s("work/gpt-6.1-sol")), ("owned_by", s("openai"))]),
            // A listing that gives no family: the listed base decides.
            dict([("id", s("home/claude-sonnet-5-5"))]),
            dict([("id", s("home/claude-sonnet-5-5-x")), ("owned_by", s(" "))]),
            // Its own family wins, even an unknown one, over the base's.
            dict([
                ("id", s("lab/claude-sonnet-5-5")),
                ("owned_by", s("someproxy")),
            ]),
            // No listed base (force-model-prefix): nothing to go on.
            dict([("id", s("solo/claude-opus-5-5"))]),
            dict([("id", s("work/gpt-image-2"))]),
        ];
        let format = |index: usize| listed_api_format(&listing[index], &listing);
        assert_eq!(format(2), Ok("responses"));
        assert_eq!(format(3), Ok("chat_completions"));
        assert_eq!(format(4), Err(UNKNOWN_FAMILY));
        assert_eq!(format(5), Err(UNKNOWN_FAMILY));
        assert_eq!(format(6), Err(UNKNOWN_FAMILY));
        assert_eq!(format(7), Err(NOT_A_CHAT_MODEL));
        let rows = annotate_models(listing.clone());
        assert_eq!(*field(&rows[2], "prefix"), s("work"));
        assert_eq!(*field(&rows[2], "base_model"), s("gpt-6.1-sol"));
        assert_eq!(*field(&rows[3], "api_format"), s("chat_completions"));
        assert_eq!(*field(&rows[0], "prefix"), Value::Null);
        assert_eq!(route_prefix("/x"), None);
        assert_eq!(route_prefix("x/"), None);
    }

    #[test]
    fn efforts_reach_the_proxys_levels() {
        assert_eq!(reasoning_effort(&s("XHigh")), Some("xhigh"));
        assert_eq!(reasoning_effort(&s("max")), Some("max"));
        assert_eq!(reasoning_effort(&s("ultra")), Some("max"));
        assert_eq!(reasoning_effort(&s("low")), Some("low"));
        assert_eq!(reasoning_effort(&s("auto")), None);
        assert_eq!(reasoning_effort(&Value::Null), None);
    }

    #[test]
    fn annotation_marks_chat_models_and_explains_the_rest() {
        let rows = annotate_models(vec![
            dict([("id", s("claude-sonnet-5-5")), ("owned_by", s("anthropic"))]),
            dict([("id", s("mystery")), ("owned_by", s("someone"))]),
        ]);
        assert_eq!(*field(&rows[0], "api_format"), s("chat_completions"));
        assert_eq!(*field(&rows[0], "supports_tools"), Value::Bool(true));
        assert_eq!(
            *field(&rows[0], "reasoning_efforts"),
            Value::List(REASONING_EFFORTS.iter().map(s).collect())
        );
        assert_eq!(*field(&rows[1], "api_format"), Value::Null);
        assert_eq!(*field(&rows[1], "unsupported_reason"), s(UNKNOWN_FAMILY));
        assert_eq!(*field(&rows[1], "supports_tools"), Value::Null);
    }
}
