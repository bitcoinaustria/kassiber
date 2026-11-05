//! Streamed tool calls, assembled as `kassiber/ai/client.py` assembles them:
//! Responses `function_call` events by output index and item/call id, Chat
//! Completions `tool_calls` deltas strictly by index, and OpenRouter's
//! `reasoning_details` pieces.

use std::collections::BTreeMap;

use kassiber_envelope::{dict, set, AppError};
use kassiber_pyjson::Value;

use crate::pyval::{entries, field, s, text};

/// One call in the daemon's nested shape.
#[derive(Clone, Default)]
struct Call {
    id: Option<String>,
    name: String,
    arguments: String,
}

fn call_value(id: String, name: &str, arguments: &str) -> Value {
    dict([
        ("id", s(id)),
        ("type", s("function")),
        (
            "function",
            dict([("name", s(name)), ("arguments", s(arguments))]),
        ),
    ])
}

/// `ResponsesToolCallAccumulator`.
#[derive(Default)]
pub struct ResponsesCalls {
    calls: BTreeMap<usize, Call>,
    by_output_index: BTreeMap<i64, usize>,
    by_item_id: BTreeMap<String, Option<usize>>,
    by_call_id: BTreeMap<String, Option<usize>>,
    next_slot: usize,
}

/// `int(event["output_index"])`: `None` for a bool, a non-number or a
/// negative index.
fn output_index(event: &Value) -> Option<i64> {
    let index = match field(event, "output_index") {
        Value::Int(number) => i64::try_from(number).ok(),
        Value::Float(number) if number.is_finite() => Some(number.trunc() as i64),
        Value::Str(raw) => kassiber_pytext::parse_int(&crate::pyval::lossy(raw))
            .ok()
            .and_then(|number| i64::try_from(&number).ok()),
        _ => None,
    }?;
    (index >= 0).then_some(index)
}

fn identifiers(event: &Value, item: Option<&Value>) -> (Vec<String>, Vec<String>) {
    let mut item_ids = vec![field(event, "item_id")];
    let mut call_ids = vec![field(event, "call_id")];
    if let Some(item @ Value::Dict(_)) = item {
        item_ids.push(field(item, "id"));
        call_ids.push(field(item, "call_id"));
    }
    let unique = |values: Vec<&Value>| {
        let mut out: Vec<String> = Vec::new();
        for value in values.into_iter().filter_map(text) {
            if !out.contains(&value) {
                out.push(value);
            }
        }
        out
    };
    (unique(item_ids), unique(call_ids))
}

fn mapped(slots: &BTreeMap<String, Option<usize>>, identifiers: &[String]) -> Option<usize> {
    identifiers
        .iter()
        .find_map(|identifier| slots.get(identifier).copied().flatten())
}

fn register(slots: &mut BTreeMap<String, Option<usize>>, identifiers: &[String], slot: usize) {
    for identifier in identifiers {
        match slots.get(identifier) {
            None => {
                slots.insert(identifier.clone(), Some(slot));
            }
            Some(existing) if *existing != Some(slot) => {
                slots.insert(identifier.clone(), None);
            }
            Some(_) => {}
        }
    }
}

impl ResponsesCalls {
    fn slot(&mut self, event: &Value, item: Option<&Value>) -> usize {
        let (item_ids, call_ids) = identifiers(event, item);
        let index = output_index(event);
        let known = index.is_some_and(|index| self.by_output_index.contains_key(&index));
        let mut slot = index.and_then(|index| self.by_output_index.get(&index).copied());
        if slot.is_none() {
            slot = mapped(&self.by_item_id, &item_ids);
        }
        if slot.is_none() && (item_ids.is_empty() || index.is_some()) {
            slot = mapped(&self.by_call_id, &call_ids);
        }
        if let Some(found) = slot {
            if index.is_some() && !known && self.by_output_index.values().any(|v| *v == found) {
                // A valid output index is authoritative: a reused id must not
                // hide a second call from consent checks.
                slot = None;
            }
        }
        let slot = slot.unwrap_or_else(|| {
            self.next_slot += 1;
            self.next_slot - 1
        });
        if let Some(index) = index {
            self.by_output_index.insert(index, slot);
        }
        register(&mut self.by_item_id, &item_ids, slot);
        register(&mut self.by_call_id, &call_ids, slot);
        slot
    }

    fn merge_item(&mut self, slot: usize, item: &Value, replace: bool) {
        if !matches!(item, Value::Dict(_)) || *field(item, "type") != s("function_call") {
            return;
        }
        let current = self.calls.entry(slot).or_default();
        let call_id = if kassiber_envelope::truthy(field(item, "call_id")) {
            field(item, "call_id")
        } else {
            field(item, "id")
        };
        if let Some(id) = text(call_id) {
            current.id = Some(id);
        }
        if let Some(name) = text(field(item, "name")) {
            current.name = name;
        }
        if let Value::Str(arguments) = field(item, "arguments") {
            let arguments = crate::pyval::lossy(arguments);
            if replace {
                current.arguments = arguments;
            } else {
                current.arguments.push_str(&arguments);
            }
        }
    }

    /// `add_event(event)`: the calls so far.
    pub fn add_event(&mut self, event: &Value) -> Vec<Value> {
        if !matches!(event, Value::Dict(_)) {
            return self.snapshot();
        }
        let kind = field(event, "type");
        if *kind == s("response.output_item.added") || *kind == s("response.output_item.done") {
            let item = field(event, "item");
            let slot = self.slot(event, Some(item));
            self.merge_item(slot, item, *kind == s("response.output_item.done"));
        } else if *kind == s("response.function_call_arguments.delta")
            || *kind == s("response.function_call_arguments.done")
        {
            let done = *kind == s("response.function_call_arguments.done");
            let slot = self.slot(event, None);
            let current = self.calls.entry(slot).or_default();
            if let Some(id) = text(field(event, "call_id")) {
                current.id = Some(id);
            }
            if let Some(name) = text(field(event, "name")) {
                current.name = name;
            }
            let key = if done { "arguments" } else { "delta" };
            if let Value::Str(arguments) = field(event, key) {
                let arguments = crate::pyval::lossy(arguments);
                if done {
                    current.arguments = arguments;
                } else {
                    current.arguments.push_str(&arguments);
                }
            }
        }
        self.snapshot()
    }

    /// `replace_output_items(items)`: the terminal output replaces what
    /// streamed.
    pub fn replace_output_items(&mut self, items: &[Value]) -> Vec<Value> {
        *self = Self::default();
        for (index, item) in items.iter().enumerate() {
            if !matches!(item, Value::Dict(_)) || *field(item, "type") != s("function_call") {
                continue;
            }
            let event = dict([("output_index", Value::from(index))]);
            let slot = self.slot(&event, Some(item));
            self.merge_item(slot, item, true);
        }
        self.snapshot()
    }

    pub fn snapshot(&self) -> Vec<Value> {
        self.calls
            .iter()
            .map(|(slot, call)| {
                call_value(
                    call.id.clone().unwrap_or_else(|| format!("call_{slot}")),
                    &call.name,
                    &call.arguments,
                )
            })
            .collect()
    }
}

fn duplicate_id() -> AppError {
    AppError::new(
        "ai_request_invalid",
        "AI provider sent two tool calls with the same id",
    )
    .with_details(dict([("reason", s("duplicate_tool_call_id"))]))
    .with_hint("Nothing was run. Try again, or use another model or API format.")
}

/// `ChatCompletionsToolCallAccumulator`: keyed strictly by index; the same
/// id under two indices fails closed rather than merging two calls.
pub struct ChatCalls {
    prefix: String,
    calls: BTreeMap<usize, Call>,
    by_index: BTreeMap<i64, usize>,
    by_id: BTreeMap<String, usize>,
    last: Option<usize>,
    next_slot: usize,
}

impl ChatCalls {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            calls: BTreeMap::new(),
            by_index: BTreeMap::new(),
            by_id: BTreeMap::new(),
            last: None,
            next_slot: 0,
        }
    }

    pub fn add(&mut self, deltas: &Value) -> Result<Vec<Value>, AppError> {
        let Value::List(deltas) = deltas else {
            return Ok(self.snapshot());
        };
        for raw in deltas {
            if !matches!(raw, Value::Dict(_)) {
                continue;
            }
            let call_id = text(field(raw, "id"));
            let index = match field(raw, "index") {
                Value::Int(number) => i64::try_from(number).ok().filter(|index| *index >= 0),
                _ => None,
            };
            let mut slot;
            if let Some(index) = index {
                slot = self.by_index.get(&index).copied();
                if let (Some(found), Some(id)) = (slot, &call_id) {
                    if self.calls[&found]
                        .id
                        .as_ref()
                        .is_some_and(|known| known != id)
                    {
                        slot = None;
                    }
                }
                let known = call_id.as_ref().and_then(|id| self.by_id.get(id).copied());
                if slot.is_none()
                    && known.is_some_and(|known| !self.by_index.values().any(|v| *v == known))
                {
                    // The call began without an index; this is the same call.
                    slot = known;
                } else if known.is_some() && known != slot {
                    return Err(duplicate_id());
                }
            } else if let Some(id) = &call_id {
                slot = self.by_id.get(id).copied();
            } else {
                slot = self.last;
            }
            let slot = match slot {
                Some(slot) => slot,
                None => {
                    let slot = self.next_slot;
                    self.next_slot += 1;
                    self.calls.insert(slot, Call::default());
                    slot
                }
            };
            if let Some(index) = index {
                self.by_index.insert(index, slot);
            }
            if let Some(id) = &call_id {
                self.by_id.entry(id.clone()).or_insert(slot);
            }
            self.last = Some(slot);
            let current = self.calls.get_mut(&slot).expect("slot exists");
            if let Some(id) = call_id {
                current.id = Some(id);
            }
            let function = field(raw, "function");
            if !matches!(function, Value::Dict(_)) {
                continue;
            }
            if let Some(name) = text(field(function, "name")) {
                current.name = name;
            }
            match field(function, "arguments") {
                Value::Str(arguments) => {
                    current.arguments.push_str(&crate::pyval::lossy(arguments))
                }
                arguments @ Value::Dict(_) => {
                    current.arguments =
                        kassiber_pyjson::dumps(arguments, &Default::default()).unwrap_or_default();
                }
                _ => {}
            }
        }
        Ok(self.snapshot())
    }

    pub fn snapshot(&self) -> Vec<Value> {
        self.calls
            .iter()
            .map(|(slot, call)| {
                call_value(
                    call.id
                        .clone()
                        .unwrap_or_else(|| format!("{}{slot}", self.prefix)),
                    &call.name,
                    &call.arguments,
                )
            })
            .collect()
    }
}

/// `_normalized_tool_calls(raw_calls, id_prefix=...)`: the complete calls of
/// a non-streamed answer, indexed by position.
pub fn normalized_tool_calls(raw: &Value, prefix: &str) -> Result<Vec<Value>, AppError> {
    let mut calls = ChatCalls::new(prefix);
    if let Value::List(items) = raw {
        let indexed: Vec<Value> = items
            .iter()
            .enumerate()
            .filter(|(_, call)| matches!(call, Value::Dict(_)))
            .map(|(position, call)| {
                // `{**call, "index": position}`.
                let mut call = call.clone();
                set(&mut call, "index", Value::from(position));
                call
            })
            .collect();
        calls.add(&Value::List(indexed))?;
    }
    Ok(calls.snapshot())
}

/// `_ReasoningDetailsAccumulator`: OpenRouter's reasoning blocks merged by
/// index; `text`, `summary` and `data` continue across pieces, the other
/// fields take their latest value.
#[derive(Default)]
pub struct ReasoningDetails {
    details: Vec<(Value, Value)>,
}

impl ReasoningDetails {
    pub fn add(&mut self, pieces: &Value) {
        let Value::List(pieces) = pieces else {
            return;
        };
        for piece in pieces {
            if !matches!(piece, Value::Dict(_)) {
                continue;
            }
            let key = match field(piece, "index") {
                index @ Value::Int(_) => index.clone(),
                _ => s(format!("unindexed-{}", self.details.len())),
            };
            let position = match self.details.iter().position(|(k, _)| *k == key) {
                Some(position) => position,
                None => {
                    self.details.push((key, dict::<&str>([])));
                    self.details.len() - 1
                }
            };
            let current = &mut self.details[position].1;
            for (name, value) in entries(piece) {
                if ["text", "summary", "data"].contains(&name.as_str()) {
                    if let Value::Str(addition) = value {
                        let previous =
                            crate::pyval::string(field(current, &name)).unwrap_or_default();
                        set(current, &name, s(previous + &crate::pyval::lossy(addition)));
                        continue;
                    }
                }
                if *value != Value::Null {
                    set(current, &name, value.clone());
                }
            }
        }
    }

    pub fn result(&self) -> Vec<Value> {
        self.details
            .iter()
            .map(|(_, value)| value.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(raw: &str) -> Value {
        kassiber_pytext::loads(raw).unwrap()
    }

    fn ids(calls: &[Value]) -> Vec<String> {
        calls
            .iter()
            .map(|c| text(field(c, "id")).unwrap())
            .collect()
    }

    // `ResponsesToolCallAccumulatorTest` (tests/test_ai_core.py).
    #[test]
    fn responses_calls_merge_by_index_and_ids() {
        let mut calls = ResponsesCalls::default();
        calls.add_event(&load(r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"status","arguments":""}}"#));
        calls.add_event(&load(r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{\"a\""}"#));
        calls.add_event(&load(
            r#"{"type":"response.function_call_arguments.delta","output_index":0,"delta":":1}"}"#,
        ));
        let snapshot = calls.add_event(&load(r#"{"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"call_1","name":"dup","arguments":"{}"}}"#));
        // A valid new output index is authoritative even with a reused id.
        assert_eq!(ids(&snapshot), ["call_1", "call_1"]);
        assert_eq!(
            text(field(field(&snapshot[0], "function"), "arguments")).unwrap(),
            "{\"a\":1}"
        );
        let done = calls.add_event(&load(
            r#"{"type":"response.function_call_arguments.done","output_index":0,"arguments":"{}"}"#,
        ));
        assert_eq!(
            text(field(field(&done[0], "function"), "arguments")).unwrap(),
            "{}"
        );
        let replaced = calls.replace_output_items(&[
            load(r#"{"type":"message"}"#),
            load(r#"{"type":"function_call","name":"x","arguments":"[]"}"#),
        ]);
        assert_eq!(ids(&replaced), ["call_0"]);
        let mut odd = ResponsesCalls::default();
        let snapshot = odd.add_event(&load(
            r#"{"type":"response.function_call_arguments.delta","output_index":"2","delta":"x"}"#,
        ));
        odd.add_event(&load(
            r#"{"type":"response.function_call_arguments.delta","output_index":true,"delta":"y"}"#,
        ));
        assert_eq!(snapshot.len(), 1);
        assert_eq!(odd.snapshot().len(), 2);
    }

    #[test]
    fn chat_calls_split_by_index_and_fail_closed_on_duplicates() {
        let mut calls = ChatCalls::new("call_1_");
        calls
            .add(&load(
                r#"[{"id":"x1","function":{"name":"a","arguments":"{}"}}]"#,
            ))
            .unwrap();
        calls
            .add(&load(
                r#"[{"id":"x2","function":{"name":"b","arguments":"{"}}]"#,
            ))
            .unwrap();
        calls
            .add(&load(r#"[{"function":{"arguments":"}"}}]"#))
            .unwrap();
        let snapshot = calls
            .add(&load(
                r#"[{"index":0,"id":"x3","function":{"name":"c","arguments":{"k":1}}}]"#,
            ))
            .unwrap();
        assert_eq!(ids(&snapshot), ["x1", "x2", "x3"]);
        assert_eq!(
            text(field(field(&snapshot[2], "function"), "arguments")).unwrap(),
            "{\"k\": 1}"
        );
        let mut duplicate = ChatCalls::new("call_1_");
        duplicate
            .add(&load(
                r#"[{"index":0,"id":"dup","function":{"arguments":"{\"limit\":"}}]"#,
            ))
            .unwrap();
        let error = duplicate
            .add(&load(
                r#"[{"index":1,"id":"dup","function":{"arguments":"5}"}}]"#,
            ))
            .unwrap_err();
        assert_eq!(
            field(error.details.as_ref().unwrap(), "reason"),
            &s("duplicate_tool_call_id")
        );
        let mut unnamed = ChatCalls::new("call_2_");
        let snapshot = unnamed
            .add(&load(r#"[{"index":0,"function":{"name":"a"}}]"#))
            .unwrap();
        assert_eq!(ids(&snapshot), ["call_2_0"]);
        assert_eq!(
            ids(&normalized_tool_calls(
                &load(r#"[{"function":{"name":"a"}},5,{"index":9,"function":{"name":"b"}}]"#),
                "call_1_"
            )
            .unwrap()),
            ["call_1_0", "call_1_1"]
        );
    }

    #[test]
    fn reasoning_details_merge_text_by_index() {
        let mut details = ReasoningDetails::default();
        details.add(&load(
            r#"[{"index":0,"type":"reasoning.text","text":"a","signature":null}]"#,
        ));
        details.add(&load(r#"[{"index":0,"text":"b","signature":"sig"},{"type":"reasoning.encrypted","data":"x"}]"#));
        assert_eq!(
            Value::List(details.result()),
            load(
                r#"[{"index":0,"type":"reasoning.text","text":"ab","signature":"sig"},{"type":"reasoning.encrypted","data":"x"}]"#
            )
        );
    }
}
