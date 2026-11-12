//! `errors.build_remediation`: the shared `details["remediation"]` shape.
//!
//! A blocking error names a condition; its remediation names what resolves
//! it, on which record, and with which tool. `target` holds full, resolvable
//! ids only, and `arguments_hint` is a starting point the caller must still
//! review.

use kassiber_pyjson::Value;

use crate::{dict, s};

/// The key under which `AppError.details` carries a remediation.
pub const REMEDIATION_KEY: &str = "remediation";

/// The optional parts of a remediation; `Default` is Python's defaults.
#[derive(Default)]
pub struct Remediation<'a> {
    pub target: Option<Value>,
    pub tool: Option<&'a str>,
    pub arguments_hint: Option<Value>,
    pub desktop_only: bool,
    pub message: Option<&'a str>,
    pub alternatives: Vec<Value>,
}

/// `build_remediation(action, ...)`: `action`, `target`, `tool`,
/// `arguments_hint` and `desktop_only` always, then `message` and
/// `alternatives` when given.
pub fn build_remediation(action: &str, parts: Remediation<'_>) -> Value {
    let empty = || dict::<&str>([]);
    let mut pairs = vec![
        ("action", s(action)),
        ("target", parts.target.unwrap_or_else(empty)),
        ("tool", parts.tool.map_or(Value::Null, s)),
        ("arguments_hint", parts.arguments_hint.unwrap_or_else(empty)),
        ("desktop_only", Value::Bool(parts.desktop_only)),
    ];
    if let Some(message) = parts.message.filter(|m| !m.is_empty()) {
        pairs.push(("message", s(message)));
    }
    if !parts.alternatives.is_empty() {
        pairs.push(("alternatives", Value::List(parts.alternatives)));
    }
    dict(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remediation_keeps_python_key_order_and_omits_empty_optionals() {
        let bare = build_remediation("replan", Remediation::default());
        let encoded = kassiber_pyjson::dumps(&bare, &kassiber_pyjson::Options::default()).unwrap();
        assert_eq!(
            encoded,
            r#"{"action": "replan", "target": {}, "tool": null, "arguments_hint": {}, "desktop_only": false}"#
        );
        let full = build_remediation(
            "revise",
            Remediation {
                tool: Some("ui_transfers_components_plan"),
                desktop_only: true,
                message: Some("m"),
                alternatives: vec![dict([("action", s("supersede"))])],
                ..Remediation::default()
            },
        );
        let encoded = kassiber_pyjson::dumps(&full, &kassiber_pyjson::Options::default()).unwrap();
        assert_eq!(
            encoded,
            r#"{"action": "revise", "target": {}, "tool": "ui_transfers_components_plan", "arguments_hint": {}, "desktop_only": true, "message": "m", "alternatives": [{"action": "supersede"}]}"#
        );
    }
}
