//! Prompt assembly for the in-app assistant (`kassiber/ai/prompt.py`).
//!
//! `DEFAULT_KASSIBER_SYSTEM_PROMPT` and the tool-area index are embedded
//! from `src/data/catalog.json`, which `scripts/gen_catalog.py` writes from
//! Python, so the text the model reads cannot drift.

use std::sync::OnceLock;

use kassiber_envelope::{dict, get, AppError};
use kassiber_pyjson::Value;

use crate::catalog::catalog_document;
use crate::pyval::s;

/// `DEFAULT_KASSIBER_SYSTEM_PROMPT`.
pub fn default_system_prompt() -> &'static str {
    static PROMPT: OnceLock<String> = OnceLock::new();
    PROMPT.get_or_init(|| match get(&catalog_document(), "system_prompt") {
        Some(Value::Str(text)) => text.as_str().expect("system prompt is text").to_owned(),
        _ => panic!("embedded catalog has no system prompt"),
    })
}

fn catalog_text(key: &str) -> String {
    match get(&catalog_document(), key) {
        Some(Value::Str(text)) => text.as_str().expect("catalog text").to_owned(),
        _ => panic!("embedded catalog has no {key}"),
    }
}

/// `tool_area_index()`: the compact area list for small models.
pub fn tool_area_index() -> &'static str {
    static INDEX: OnceLock<String> = OnceLock::new();
    INDEX.get_or_init(|| catalog_text("tool_area_index"))
}

/// `KASSIBER_SYSTEM_PROMPT`: the product prompt with the area index. It is
/// static, so a provider's prompt cache keeps working across turns.
pub fn kassiber_system_prompt() -> &'static str {
    static PROMPT: OnceLock<String> = OnceLock::new();
    PROMPT.get_or_init(|| format!("{}\n{}\n", default_system_prompt(), tool_area_index()))
}

/// `SystemPromptKind`: `"kassiber"`, `"raw"`, or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptKind {
    Kassiber,
    Raw,
}

impl PromptKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PromptKind::Kassiber => "kassiber",
            PromptKind::Raw => "raw",
        }
    }
}

/// `normalize_system_prompt_kind(raw, tools_enabled=...)`.
pub fn normalize_system_prompt_kind(
    raw: Option<&Value>,
    tools_enabled: bool,
) -> Result<Option<PromptKind>, AppError> {
    match raw {
        None | Some(Value::Null) => Ok(tools_enabled.then_some(PromptKind::Kassiber)),
        Some(Value::Str(text)) if text.is("kassiber") => Ok(Some(PromptKind::Kassiber)),
        Some(Value::Str(text)) if text.is("raw") => Ok(Some(PromptKind::Raw)),
        Some(other) => Err(AppError::new(
            "validation",
            "ai.chat system_prompt_kind must be 'kassiber', 'raw', or null",
        )
        .with_details(dict([("system_prompt_kind", other.clone())]))),
    }
}

/// `build_chat_messages(messages, system_prompt_kind=..., system_prompt=...,
/// tools_enabled=...)`.
pub fn build_chat_messages(
    messages: &[Value],
    kind: Option<PromptKind>,
    system_prompt: Option<&Value>,
    tools_enabled: bool,
) -> Result<Vec<Value>, AppError> {
    let system_prompt = system_prompt.filter(|value| **value != Value::Null);
    if system_prompt.is_some() && kind != Some(PromptKind::Raw) {
        return Err(AppError::new(
            "validation",
            "ai.chat system_prompt is only accepted when system_prompt_kind is raw",
        ));
    }
    let system = match kind {
        Some(PromptKind::Raw) => match system_prompt {
            Some(text @ Value::Str(_)) => text.clone(),
            _ => {
                return Err(AppError::new(
                    "validation",
                    "ai.chat raw system_prompt must be a string",
                ))
            }
        },
        Some(PromptKind::Kassiber) if tools_enabled => s(kassiber_system_prompt()),
        Some(PromptKind::Kassiber) => s(default_system_prompt()),
        None => return Ok(messages.to_vec()),
    };
    let mut built = Vec::with_capacity(messages.len() + 1);
    built.push(dict([("role", s("system")), ("content", system)]));
    built.extend_from_slice(messages);
    Ok(built)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_kinds_follow_python() {
        assert_eq!(
            normalize_system_prompt_kind(None, true).unwrap(),
            Some(PromptKind::Kassiber)
        );
        assert_eq!(normalize_system_prompt_kind(None, false).unwrap(), None);
        assert_eq!(
            normalize_system_prompt_kind(Some(&s("raw")), false).unwrap(),
            Some(PromptKind::Raw)
        );
        let error = normalize_system_prompt_kind(Some(&Value::Bool(true)), true).unwrap_err();
        assert_eq!(error.code, "validation");
        assert!(default_system_prompt().starts_with("You are Kassiber's in-app"));
    }

    #[test]
    fn messages_gain_one_system_message() {
        let user = vec![dict([("role", s("user")), ("content", s("hi"))])];
        assert_eq!(build_chat_messages(&user, None, None, true).unwrap(), user);
        let raw =
            build_chat_messages(&user, Some(PromptKind::Raw), Some(&s("Be terse.")), true).unwrap();
        assert_eq!(raw.len(), 2);
        assert_eq!(get(&raw[0], "content"), Some(&s("Be terse.")));
        assert_eq!(
            build_chat_messages(&user, Some(PromptKind::Kassiber), Some(&s("x")), true)
                .unwrap_err()
                .code,
            "validation"
        );
        let with_tools =
            build_chat_messages(&user, Some(PromptKind::Kassiber), None, true).unwrap();
        assert_eq!(
            get(&with_tools[0], "content"),
            Some(&s(kassiber_system_prompt()))
        );
        assert!(kassiber_system_prompt().contains("\n- reports: "));
        let without = build_chat_messages(&user, Some(PromptKind::Kassiber), None, false).unwrap();
        assert_eq!(
            get(&without[0], "content"),
            Some(&s(default_system_prompt()))
        );
        assert_eq!(
            build_chat_messages(&user, Some(PromptKind::Raw), None, true)
                .unwrap_err()
                .message,
            "ai.chat raw system_prompt must be a string"
        );
    }
}
