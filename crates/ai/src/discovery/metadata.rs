//! Bounded model-capability metadata (`kassiber/ai/model_metadata.py`):
//! what a provider's model listing may contribute to Kassiber's output,
//! sanitised to short string lists and booleans.

use kassiber_envelope::{dict, set};
use kassiber_pyjson::Value;

use crate::pyval::{field, s};

pub const LIST_LIMIT: usize = 32;
pub const STRING_LIMIT: usize = 96;
const HTTP_EFFORTS: &[&str] = &["low", "medium", "high"];
const REASONING_PARAMETERS: &[&str] = &["reasoning", "include_reasoning", "reasoning_effort"];

fn bounded(text: &str) -> String {
    crate::pyval::prefix(text, STRING_LIMIT)
}

/// `safe_string_list(value)`: stripped non-empty strings, at most 32 of at
/// most 96 code points; `None` for anything else or an empty result.
pub fn safe_string_list(value: &Value) -> Option<Vec<String>> {
    let Value::List(items) = value else {
        return None;
    };
    let mut out = Vec::new();
    for item in items {
        let Value::Str(text) = item else {
            continue;
        };
        let text = crate::pyval::lossy(text);
        let text = kassiber_pytext::strip(&text);
        if text.is_empty() {
            continue;
        }
        out.push(bounded(text));
        if out.len() >= LIST_LIMIT {
            break;
        }
    }
    (!out.is_empty()).then_some(out)
}

fn list(values: &[String]) -> Value {
    Value::List(values.iter().map(s).collect())
}

fn safe_capability_value(value: &Value) -> Option<Value> {
    match value {
        Value::Bool(_) => Some(value.clone()),
        Value::Str(text) => {
            let text = crate::pyval::lossy(text);
            let text = kassiber_pytext::strip(&text);
            (!text.is_empty()).then(|| s(bounded(text)))
        }
        other => safe_string_list(other).map(|values| list(&values)),
    }
}

/// `safe_model_capabilities(item)`.
pub fn safe_model_capabilities(item: &Value) -> Value {
    let mut metadata = dict::<&str>([]);
    if let value @ Value::Bool(_) = field(item, "supports_reasoning_effort") {
        set(&mut metadata, "supports_reasoning_effort", value.clone());
    }
    if let Some(values) = safe_string_list(field(item, "supported_parameters")) {
        set(&mut metadata, "supported_parameters", list(&values));
    }
    if let Some(values) = safe_string_list(field(item, "reasoning_efforts")) {
        set(&mut metadata, "reasoning_efforts", list(&values));
    }
    if let Value::Bool(fast) = field(item, "supports_fast_mode") {
        set(&mut metadata, "supports_fast_mode", Value::Bool(*fast));
        if let Value::Str(description) = field(item, "fast_mode_description") {
            let description = crate::pyval::lossy(description);
            let description = kassiber_pytext::strip(&description);
            if *fast && !description.is_empty() {
                set(
                    &mut metadata,
                    "fast_mode_description",
                    s(bounded(description)),
                );
            }
        }
    }
    let capabilities = field(item, "capabilities");
    if matches!(capabilities, Value::Dict(_)) {
        let mut safe = dict::<&str>([]);
        for key in [
            "reasoning_effort",
            "reasoning_efforts",
            "supported_parameters",
        ] {
            if let Some(value) = safe_capability_value(field(capabilities, key)) {
                set(&mut safe, key, value);
            }
        }
        if kassiber_envelope::truthy(&safe) {
            set(&mut metadata, "capabilities", safe);
        }
    }
    metadata
}

fn lowered(value: &Value) -> Vec<String> {
    match value {
        Value::List(items) => items
            .iter()
            .map(|item| kassiber_pytext::lower(&kassiber_envelope::py_str(item)))
            .collect(),
        _ => Vec::new(),
    }
}

/// `http_model_capabilities(item)`: the bounded metadata plus what Kassiber
/// derives for HTTP chat (efforts, tools, modalities).
pub fn http_model_capabilities(item: &Value) -> Value {
    let mut metadata = safe_model_capabilities(item);
    let capabilities = field(&metadata, "capabilities").clone();
    let capability_parameters = field(&capabilities, "supported_parameters");
    let mut parameters = lowered(field(&metadata, "supported_parameters"));
    if matches!(capability_parameters, Value::List(_)) {
        parameters.extend(lowered(capability_parameters));
    }
    // Known means the listing carried the list at all, even empty.
    let raw = field(item, "capabilities");
    let parameters_known = matches!(field(item, "supported_parameters"), Value::List(_))
        || (matches!(raw, Value::Dict(_))
            && matches!(field(raw, "supported_parameters"), Value::List(_)));
    let mut advertised = lowered(field(&metadata, "reasoning_efforts"));
    if let efforts @ Value::List(_) = field(&capabilities, "reasoning_efforts") {
        advertised.extend(lowered(efforts));
    }
    let reasoning = field(item, "reasoning");
    if matches!(reasoning, Value::Dict(_)) {
        if let Some(efforts) = safe_string_list(field(reasoning, "supported_efforts")) {
            advertised.extend(efforts.iter().map(|e| kassiber_pytext::lower(e)));
        }
    }
    let effort_capable = match field(item, "supports_reasoning_effort") {
        Value::Bool(explicit) => *explicit,
        _ => {
            parameters
                .iter()
                .any(|p| REASONING_PARAMETERS.contains(&p.as_str()))
                || !advertised.is_empty()
                || [Value::Bool(true), s("true"), s("supported")]
                    .contains(field(&capabilities, "reasoning_effort"))
        }
    };
    if effort_capable {
        let accepted: Vec<&str> = HTTP_EFFORTS
            .iter()
            .copied()
            .filter(|level| advertised.iter().any(|a| a == level))
            .collect();
        set(
            &mut metadata,
            "supports_reasoning_effort",
            Value::Bool(true),
        );
        let levels = if accepted.is_empty() {
            HTTP_EFFORTS.to_vec()
        } else {
            accepted
        };
        set(
            &mut metadata,
            "reasoning_efforts",
            Value::List(levels.into_iter().map(s).collect()),
        );
    }
    match field(item, "supports_tools") {
        explicit @ Value::Bool(_) => set(&mut metadata, "supports_tools", explicit.clone()),
        _ if parameters_known => set(
            &mut metadata,
            "supports_tools",
            Value::Bool(parameters.iter().any(|p| p == "tools")),
        ),
        _ => set(&mut metadata, "supports_tools", Value::Null),
    }
    let architecture = field(item, "architecture");
    for key in ["input_modalities", "output_modalities"] {
        let mut source = if matches!(architecture, Value::Dict(_)) {
            field(architecture, key)
        } else {
            &Value::Null
        };
        if *source == Value::Null {
            source = field(item, key);
        }
        if let Some(modalities) = safe_string_list(source) {
            let mut unique: Vec<String> = Vec::new();
            for modality in modalities {
                let modality = kassiber_pytext::lower(&modality);
                if !unique.contains(&modality) {
                    unique.push(modality);
                }
            }
            set(&mut metadata, key, list(&unique));
        }
    }
    metadata
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(raw: &str) -> Value {
        kassiber_pytext::loads(raw).unwrap()
    }

    // `ModelMetadataFastModeTest` and the HTTP listing cases.
    #[test]
    fn listings_are_bounded_and_derived_like_python() {
        let long = "x".repeat(200);
        let item = load(&format!(
            r#"{{"id":"m","supported_parameters":["tools"," Reasoning ","",3,"{long}"],"supports_fast_mode":true,"fast_mode_description":"  Faster  ","architecture":{{"input_modalities":["Text","image","text"]}},"output_modalities":["text"]}}"#
        ));
        let metadata = http_model_capabilities(&item);
        assert_eq!(
            metadata,
            load(&format!(
                r#"{{"supported_parameters":["tools","Reasoning","{}"],"supports_fast_mode":true,"fast_mode_description":"Faster","supports_reasoning_effort":true,"reasoning_efforts":["low","medium","high"],"supports_tools":true,"input_modalities":["text","image"],"output_modalities":["text"]}}"#,
                "x".repeat(96)
            ))
        );
        assert_eq!(
            http_model_capabilities(&load(r#"{"id":"gpt"}"#)),
            load(r#"{"supports_tools":null}"#)
        );
        assert_eq!(
            http_model_capabilities(&load(
                r#"{"supported_parameters":[],"supports_fast_mode":false,"fast_mode_description":"x"}"#
            )),
            load(r#"{"supports_fast_mode":false,"supports_tools":false}"#)
        );
        assert_eq!(
            http_model_capabilities(&load(
                r#"{"reasoning":{"supported_efforts":["HIGH","xhigh"]},"capabilities":{"reasoning_effort":"  ","supported_parameters":["tools"]}}"#
            )),
            load(
                r#"{"capabilities":{"supported_parameters":["tools"]},"supports_reasoning_effort":true,"reasoning_efforts":["high"],"supports_tools":true}"#
            )
        );
        assert_eq!(
            http_model_capabilities(&load(
                r#"{"supports_reasoning_effort":false,"reasoning_efforts":["low"],"supports_tools":false,"capabilities":{"reasoning_effort":true}}"#
            )),
            load(
                r#"{"supports_reasoning_effort":false,"reasoning_efforts":["low"],"capabilities":{"reasoning_effort":true},"supports_tools":false}"#
            )
        );
        let many: Vec<String> = (0..40).map(|i| format!("\"p{i}\"")).collect();
        let listed = safe_string_list(&load(&format!("[{}]", many.join(",")))).unwrap();
        assert_eq!(listed.len(), 32);
    }
}
