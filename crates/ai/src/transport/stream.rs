//! `_responses_deltas` and `_chat_completions_deltas`: one streamed answer
//! read into [`ChatDelta`]s. A stream that ends before its terminal event
//! is never an answer, and a call it may have cut off never runs.

use kassiber_envelope::{dict, set, AppError};
use kassiber_net::Cancel;
use kassiber_pyjson::Value;

use super::accumulate::{ChatCalls, ReasoningDetails, ResponsesCalls};
use super::errors::provider_error;
use super::sse::Event;
use super::{wire, ChatDelta};
use crate::pyval::{field, s, string, text};

type Events<'a> = dyn FnMut() -> Result<Option<Event>, AppError> + 'a;
type Emit<'a> = dyn FnMut(ChatDelta) -> Result<(), AppError> + 'a;

fn no_answer() -> AppError {
    AppError::new(
        "ai_request_invalid",
        "AI provider ended the stream without an answer",
    )
    .with_details(dict([("reason", s("no_answer"))]))
    .with_hint("Check the base URL and the provider's API format.")
}

fn stream_incomplete() -> AppError {
    AppError::new(
        "ai_unavailable",
        "AI provider's stream ended before the answer was complete",
    )
    .with_details(dict([("reason", s("stream_incomplete"))]))
    .with_hint("The connection may have dropped. Try again.")
    .retryable(true)
}

fn no_stream_events() -> AppError {
    AppError::new(
        "ai_request_invalid",
        "AI provider answered without any stream events",
    )
    .with_details(dict([("reason", s("no_stream_events"))]))
    .with_hint("Check the base URL and the provider's API format.")
}

fn withdraw_calls() -> ChatDelta {
    ChatDelta::new(
        dict([("tool_calls", Value::List(Vec::new()))]),
        None,
        dict::<&str>([]),
    )
}

/// The Responses API stream.
pub(super) fn responses(
    cancel: &Cancel,
    events: &mut Events<'_>,
    emit: &mut Emit<'_>,
) -> Result<(), AppError> {
    let mut calls = ResponsesCalls::default();
    let (mut emitted_content, mut emitted_reasoning) = (false, false);
    let (mut received, mut ended, mut finished, mut spoke_format) = (false, false, false, false);
    while let Some(event) = events()? {
        let Event::Payload(event) = event else {
            ended = true;
            break;
        };
        received = true;
        if cancel.is_cancelled() {
            return Ok(());
        }
        let kind = string(field(&event, "type")).unwrap_or_default();
        let typed = matches!(field(&event, "type"), Value::Str(_));
        if typed && kind.starts_with("response.") {
            spoke_format = true;
        }
        if typed && (kind == "error" || kind == "response.failed") {
            return Err(wire::response_stream_error(&event));
        }
        let mut delta = dict::<&str>([]);
        match kind.as_str() {
            "response.output_text.delta" | "response.refusal.delta" if typed => {
                if let Some(piece) = text(field(&event, "delta")) {
                    set(&mut delta, "content", s(piece));
                    emitted_content = true;
                }
            }
            "response.reasoning_summary_text.delta"
            | "response.reasoning_text.delta"
            | "response.reasoning.delta"
                if typed =>
            {
                if let Some(piece) = text(field(&event, "delta")) {
                    set(&mut delta, "reasoning", s(piece));
                    emitted_reasoning = true;
                }
            }
            _ => {}
        }
        if typed
            && matches!(
                kind.as_str(),
                "response.output_item.added"
                    | "response.output_item.done"
                    | "response.function_call_arguments.delta"
                    | "response.function_call_arguments.done"
            )
        {
            let snapshot = calls.add_event(&event);
            if !snapshot.is_empty() {
                set(&mut delta, "tool_calls", Value::List(snapshot));
            }
        }
        let completed = (typed && (kind == "response.completed" || kind == "response.incomplete"))
            .then(|| field(&event, "response").clone())
            .filter(|response| *response != Value::Null);
        let mut output = None;
        let mut finish = None;
        if let Some(completed) = &completed {
            let items = wire::response_output(completed);
            finish = wire::response_finish_reason(completed);
            let streamed = !calls.snapshot().is_empty();
            let snapshot = calls.replace_output_items(&items);
            if !snapshot.is_empty() || streamed {
                set(&mut delta, "tool_calls", Value::List(snapshot));
            }
            if !emitted_content {
                let content = wire::response_text(completed);
                if !content.is_empty() {
                    set(&mut delta, "content", s(content));
                }
            }
            if !emitted_reasoning {
                if let Some(reasoning) = wire::response_reasoning_summary(completed) {
                    set(&mut delta, "reasoning", s(reasoning));
                }
            }
            if kind == "response.completed"
                && items.is_empty()
                && !emitted_content
                && !emitted_reasoning
                && !streamed
                && !kassiber_envelope::truthy(&delta)
            {
                // "Completed" with nothing in it: no model replied.
                return Err(no_answer());
            }
            finished = true;
            output = Some(items);
        }
        if !kassiber_envelope::truthy(&delta) && completed.is_none() {
            continue;
        }
        let mut chunk = ChatDelta::new(delta, finish, event);
        chunk.response_output = output;
        emit(chunk)?;
    }
    if cancel.is_cancelled() {
        return Ok(());
    }
    if !received && !ended {
        return Err(no_stream_events());
    }
    if !finished {
        if !calls.snapshot().is_empty() {
            emit(withdraw_calls())?;
        }
        if !spoke_format {
            return Err(no_stream_events());
        }
        return Err(stream_incomplete());
    }
    Ok(())
}

/// The Chat Completions stream.
pub(super) fn chat_completions(
    cancel: &Cancel,
    prefix: &str,
    events: &mut Events<'_>,
    emit: &mut Emit<'_>,
) -> Result<(), AppError> {
    let mut calls = ChatCalls::new(prefix);
    let mut details = ReasoningDetails::default();
    let (mut content, mut reasoning) = (String::new(), String::new());
    let mut reasoning_field: Option<&'static str> = None;
    let mut finish: Option<String> = None;
    let mut usage = Value::Null;
    let (mut streamed_calls, mut answered, mut received, mut ended, mut spoke_format) =
        (false, false, false, false, false);
    while let Some(event) = events()? {
        let Event::Payload(event) = event else {
            ended = true;
            break;
        };
        received = true;
        if cancel.is_cancelled() {
            return Ok(());
        }
        if let Some(error) = wire::chat_completion_stream_error(&event) {
            return Err(error);
        }
        if let value @ Value::Dict(_) = field(&event, "usage") {
            usage = value.clone();
        }
        let choices = field(&event, "choices");
        if matches!(choices, Value::List(_)) {
            spoke_format = true;
        }
        let choice = match choices {
            Value::List(choices) => choices.first().filter(|c| matches!(c, Value::Dict(_))),
            _ => None,
        };
        let Some(choice) = choice else {
            // OpenAI's usage chunk carries no choices.
            continue;
        };
        let piece = match field(choice, "delta") {
            piece @ Value::Dict(_) => piece.clone(),
            _ => dict::<&str>([]),
        };
        let mut delta = dict::<&str>([]);
        let text_piece: String = ["content", "refusal"]
            .iter()
            .filter_map(|key| string(field(&piece, key)))
            .collect();
        if !text_piece.is_empty() {
            set(&mut delta, "content", s(&text_piece));
            content.push_str(&text_piece);
            answered = true;
        }
        for name in wire::CHAT_REASONING_FIELDS {
            if let Some(value) = text(field(&piece, name)) {
                set(&mut delta, "reasoning", s(&value));
                reasoning.push_str(&value);
                reasoning_field = reasoning_field.or(Some(name));
                break;
            }
        }
        details.add(field(&piece, "reasoning_details"));
        if *field(&piece, "tool_calls") != Value::Null {
            let snapshot = calls.add(field(&piece, "tool_calls"))?;
            if !snapshot.is_empty() {
                set(&mut delta, "tool_calls", Value::List(snapshot));
                streamed_calls = true;
                answered = true;
            }
        }
        if let Some(reason) = text(field(choice, "finish_reason")) {
            if reason == "error" {
                // OpenRouter ends a failed stream this way.
                return Err(provider_error(
                    &dict([("message", s("finish_reason error"))]),
                    "AI provider rejected the request",
                ));
            }
            finish = Some(reason);
            answered = true;
        }
        if kassiber_envelope::truthy(&delta) {
            emit(ChatDelta::new(delta, None, event))?;
        }
    }
    if cancel.is_cancelled() {
        return Ok(());
    }
    if !received && !ended {
        return Err(no_stream_events());
    }
    if !ended && finish.is_none() {
        // A bare end of the connection never marks an answer complete.
        if streamed_calls {
            emit(withdraw_calls())?;
        }
        if !spoke_format {
            return Err(no_stream_events());
        }
        return Err(stream_incomplete());
    }
    if !answered {
        return Err(no_answer());
    }
    if finish.is_none() && streamed_calls {
        emit(withdraw_calls())?;
        return Err(AppError::new(
            "ai_unavailable",
            "AI provider ended a stream of tool calls without a finish reason",
        )
        .with_details(dict([("reason", s("no_finish_reason"))]))
        .with_hint("The answer may have been cut off. Try again, or pick a different model.")
        .retryable(true));
    }
    let mut final_calls = calls.snapshot();
    let finish_reason =
        wire::chat_finish_reason(&finish.map_or(Value::Null, s), !final_calls.is_empty());
    if finish_reason == "length" || finish_reason == "content_filter" {
        // The message was cut off, so its last call may be too.
        final_calls.clear();
    }
    let mut terminal = dict::<&str>([]);
    if !final_calls.is_empty() || streamed_calls {
        set(
            &mut terminal,
            "tool_calls",
            Value::List(final_calls.clone()),
        );
    }
    let raw = if usage == Value::Null {
        dict::<&str>([])
    } else {
        dict([("usage", usage)])
    };
    let mut chunk = ChatDelta::new(terminal, Some(finish_reason), raw);
    chunk.response_output = Some(wire::chat_completion_output(
        &content,
        &reasoning,
        reasoning_field,
        details.result(),
        &final_calls,
    ));
    emit(chunk)
}
