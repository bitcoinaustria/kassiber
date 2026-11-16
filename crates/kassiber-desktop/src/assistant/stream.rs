//! The `ai.chat` stream reducers of `daemon/stream.ts`, as pure functions: each progress record
//! (`ai.chat.status`, `.delta`, `.tool_call`, `.tool_result`, `.tool_consent_required`) updates
//! the running assistant message. Tool cards are updates keyed by `call_id`: the same call shows
//! up before its approval and again when it starts running. A status label is a loading hint,
//! never reasoning; only `delta.reasoning` and inline `<think>` text feed the reasoning panes.

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

use super::model::{Consent, Msg, MsgStatus, ThinkingSegment, ToolCall, ToolStatus};
use super::think::ThinkParser;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// `makeId`: unique within the app run.
pub fn make_id(prefix: &str) -> String {
    format!("{prefix}-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

/// `AI_CHAT_NOTICE_PHASES`: status phases that describe the answer rather than progress.
pub const NOTICE_PHASES: [&str; 2] = ["fast_mode_unavailable", "reasoning_effort_unavailable"];

/// `AI_TOOL_ONCE_ONLY_CONSENT`.
const ONCE_ONLY: [&str; 6] = [
    "ui.journals.quarantine.resolve",
    "ui.transfers.components.apply",
    "ui.review.apply",
    "ui.wallets.import_attachment.apply",
    "ui.wallets.import_document.apply",
    "ui.imports.rollback",
];

const APPROVALS: [&str; 5] = ["required", "auto", "full_access", "session", "not_needed"];

/// `aiChatNotices`: the recognised notices in `v`, once each, in the canonical order.
pub fn notices(v: &Value) -> Vec<String> {
    let Some(list) = v.as_array() else {
        return vec![];
    };
    NOTICE_PHASES
        .iter()
        .filter(|p| list.iter().any(|x| x.as_str() == Some(p)))
        .map(|p| p.to_string())
        .collect()
}

/// `aiChatStatusLabel`.
pub fn status_label(data: &Value) -> String {
    if let Some(l) = data["label"]
        .as_str()
        .map(str::trim)
        .filter(|l| !l.is_empty())
    {
        return l.to_owned();
    }
    match data["phase"].as_str() {
        Some("preparing") => "Preparing chat",
        Some("connecting") => "Connecting",
        Some("waiting_for_model") => "Loading model",
        Some("thinking") => "Thinking",
        _ => "Generating",
    }
    .to_owned()
}

/// `beginThinkingSegment`: a fresh pane for the next provider round, unless the current one is
/// still empty and owns no tool calls.
pub fn begin_segment(m: &mut Msg) {
    if let Some(last) = m.thinking_segments.last() {
        let has_tools = m
            .tool_calls
            .iter()
            .any(|c| c.segment_id.as_deref() == Some(last.id.as_str()));
        if last.content.is_empty() && !has_tools {
            return;
        }
    }
    m.thinking_segments.push(ThinkingSegment {
        id: make_id("seg"),
        content: String::new(),
    });
}

fn active_segment(m: &Msg) -> Option<String> {
    m.thinking_segments.last().map(|s| s.id.clone())
}

pub fn append_thinking(m: &mut Msg, add: &str) {
    if add.is_empty() {
        return;
    }
    match m.thinking_segments.last_mut() {
        Some(last) => last.content.push_str(add),
        None => m.thinking_segments.push(ThinkingSegment {
            id: make_id("seg"),
            content: add.to_owned(),
        }),
    }
}

fn tool_result_status(data: &Value) -> ToolStatus {
    if data["ok"].as_bool() == Some(true) {
        return ToolStatus::Done;
    }
    match data["reason"].as_str() {
        Some("tool_not_allowed" | "user_denied" | "consent_timeout") => ToolStatus::Denied,
        _ => ToolStatus::Error,
    }
}

fn upsert(m: &mut Msg, next: ToolCall, keep_approval: bool) {
    match m.tool_calls.iter_mut().find(|c| c.call_id == next.call_id) {
        Some(c) => {
            let approval = if keep_approval && next.approval.is_none() {
                c.approval.clone()
            } else {
                next.approval.clone()
            };
            let (result, reason, preview) =
                (c.result.take(), c.reason.take(), c.review_preview.take());
            *c = next;
            c.approval = approval;
            // `{ ...toolCall, ...nextToolCall }`: what the new record does not name stays.
            if c.result.is_none() {
                c.result = result;
            }
            if c.reason.is_none() {
                c.reason = reason;
            }
            if c.review_preview.is_none() {
                c.review_preview = preview;
            }
        }
        None => m.tool_calls.push(next),
    }
}

/// `applyAiChatStreamRecordToMessage`.
pub fn apply_record(m: &mut Msg, record: &Value, parser: &mut ThinkParser, aborted: bool) {
    if aborted {
        return;
    }
    let d = &record["data"];
    match record["kind"].as_str() {
        Some("ai.chat.delta") => {
            let content = d["delta"]["content"].as_str().unwrap_or("");
            let reasoning = d["delta"]["reasoning"].as_str().unwrap_or("");
            if content.is_empty() && reasoning.is_empty() {
                return;
            }
            let mut visible = String::new();
            let mut thinking = String::new();
            if !content.is_empty() {
                let s = parser.feed(content);
                visible = s.content;
                thinking = s.thinking;
            }
            thinking.push_str(reasoning);
            if visible.is_empty() && thinking.is_empty() {
                return;
            }
            m.status = MsgStatus::Streaming;
            m.content.push_str(&visible);
            if visible.is_empty() && !thinking.is_empty() {
                m.activity_label = Some("Thinking".into());
            }
            append_thinking(m, &thinking);
        }
        Some("ai.chat.status") => {
            let phase = d["phase"].as_str().unwrap_or("");
            if NOTICE_PHASES.contains(&phase) {
                if !m.notices.iter().any(|n| n == phase) {
                    m.notices.push(phase.to_owned());
                }
                return;
            }
            if phase == "waiting_for_model" || phase == "thinking" {
                begin_segment(m);
            }
            m.status = MsgStatus::Streaming;
            m.activity_label = Some(status_label(d));
        }
        Some("ai.chat.tool_call") => {
            let (Some(id), Some(name)) = (d["call_id"].as_str(), d["name"].as_str()) else {
                return;
            };
            let needs = d["needs_consent"].as_bool().unwrap_or(false);
            let approval = d["approval"]
                .as_str()
                .filter(|a| APPROVALS.contains(a))
                .map(str::to_owned);
            let call = ToolCall {
                call_id: id.to_owned(),
                name: name.to_owned(),
                arguments: if d["arguments"].is_object() {
                    d["arguments"].clone()
                } else {
                    json!({})
                },
                kind_class: d["kind_class"].as_str().unwrap_or("unknown").to_owned(),
                needs_consent: needs,
                status: if needs {
                    ToolStatus::Pending
                } else {
                    ToolStatus::Running
                },
                approval,
                result: None,
                reason: None,
                review_preview: None,
                segment_id: active_segment(m),
            };
            m.status = MsgStatus::Streaming;
            upsert(m, call, true);
        }
        Some("ai.chat.tool_result") => {
            let Some(id) = d["call_id"].as_str() else {
                return;
            };
            let status = tool_result_status(d);
            let result = if !d["envelope"].is_null() {
                d["envelope"].clone()
            } else if !d["message"].is_null() {
                d["message"].clone()
            } else {
                Value::Null
            };
            let reason = d["reason"].as_str().map(str::to_owned);
            m.status = MsgStatus::Streaming;
            match m.tool_calls.iter_mut().find(|c| c.call_id == id) {
                Some(c) => {
                    c.status = status;
                    c.result = Some(result);
                    c.reason = reason;
                }
                None => {
                    let segment_id = active_segment(m);
                    m.tool_calls.push(ToolCall {
                        call_id: id.to_owned(),
                        name: "Tool".into(),
                        arguments: json!({}),
                        kind_class: "unknown".into(),
                        needs_consent: false,
                        status,
                        approval: None,
                        result: Some(result),
                        reason,
                        review_preview: None,
                        segment_id,
                    });
                }
            }
        }
        Some("ai.chat.tool_consent_required") => {
            let (Some(id), Some(name)) = (d["call_id"].as_str(), d["name"].as_str()) else {
                return;
            };
            let call = ToolCall {
                call_id: id.to_owned(),
                name: name.to_owned(),
                arguments: if d["arguments_preview"].is_object() {
                    d["arguments_preview"].clone()
                } else {
                    json!({})
                },
                kind_class: "mutating".into(),
                needs_consent: true,
                status: ToolStatus::AwaitingConsent,
                approval: Some("required".into()),
                result: None,
                reason: None,
                review_preview: (!d["review_preview"].is_null())
                    .then(|| d["review_preview"].clone()),
                segment_id: active_segment(m),
            };
            m.status = MsgStatus::Streaming;
            upsert(m, call, false);
        }
        _ => {}
    }
}

/// `buildAiToolConsentRequest`.
pub fn consent_request(d: &Value) -> Option<Consent> {
    let id = d["call_id"].as_str()?;
    let name = d["name"].as_str()?;
    Some(Consent {
        call_id: id.to_owned(),
        name: name.to_owned(),
        summary: d["summary"].as_str().unwrap_or(name).to_owned(),
        arguments_preview: if d["arguments_preview"].is_object() {
            d["arguments_preview"].clone()
        } else {
            json!({})
        },
        review_preview: (!d["review_preview"].is_null()).then(|| d["review_preview"].clone()),
        reversible: d["reversible"].as_bool() == Some(true),
        egresses: d["egresses"].as_bool() == Some(true),
        can_allow_session: d["can_allow_session"].as_bool() != Some(false)
            && !ONCE_ONLY.contains(&name),
    })
}

/// The call that left its approval with this record: a result, or released to run.
pub fn settled_call(record: &Value) -> Option<&str> {
    let d = &record["data"];
    match record["kind"].as_str() {
        Some("ai.chat.tool_result") => d["call_id"].as_str(),
        Some("ai.chat.tool_call") if d["needs_consent"].as_bool() != Some(true) => {
            d["call_id"].as_str()
        }
        _ => None,
    }
}

/// `applyToolConsentResponseToMessage`.
pub fn apply_consent_response(
    m: &mut Msg,
    call_id: &str,
    decision: &str,
    recorded: bool,
    reason: Option<&str>,
) {
    for c in m.tool_calls.iter_mut().filter(|c| c.call_id == call_id) {
        c.status = match (recorded, decision) {
            (true, "deny") => ToolStatus::Denied,
            (true, _) => ToolStatus::Running,
            (false, _) => ToolStatus::Error,
        };
        if recorded {
            if decision == "deny" {
                c.reason = Some("user_denied".into());
            }
        } else {
            c.reason = Some(reason.unwrap_or("not_found").to_owned());
        }
    }
}

/// `completedAiMutationKind`: a mutating call that finished successfully, once.
pub fn completed_mutation(
    observed: &mut std::collections::HashMap<String, (String, String)>,
    record: &Value,
) -> Option<String> {
    let d = &record["data"];
    match record["kind"].as_str() {
        Some("ai.chat.tool_call") => {
            if let (Some(id), Some(name)) = (d["call_id"].as_str(), d["name"].as_str()) {
                observed.insert(
                    id.to_owned(),
                    (
                        name.to_owned(),
                        d["kind_class"].as_str().unwrap_or("unknown").to_owned(),
                    ),
                );
            }
            None
        }
        Some("ai.chat.tool_result") => {
            let id = d["call_id"].as_str()?;
            let seen = observed.remove(id);
            (d["ok"].as_bool() == Some(true))
                .then_some(seen)
                .flatten()
                .filter(|(_, k)| k == "mutating")
                .map(|(n, _)| n)
        }
        _ => None,
    }
}

/// `terminalAiChatStatus`.
pub fn terminal_status(finish_reason: Option<&str>, aborted: bool) -> MsgStatus {
    if aborted || finish_reason == Some("cancelled") {
        MsgStatus::Cancelled
    } else {
        MsgStatus::Done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::model::Role;

    fn rec(kind: &str, data: Value) -> Value {
        json!({"kind": kind, "data": data, "request_id": "r"})
    }

    fn fresh() -> Msg {
        let mut m = Msg::new("a", Role::Assistant, "");
        m.status = MsgStatus::Pending;
        m
    }

    #[test]
    fn deltas_split_content_and_reasoning() {
        let mut m = fresh();
        let mut p = ThinkParser::default();
        apply_record(
            &mut m,
            &rec(
                "ai.chat.status",
                json!({"phase": "waiting_for_model", "label": "Thinking"}),
            ),
            &mut p,
            false,
        );
        apply_record(
            &mut m,
            &rec(
                "ai.chat.delta",
                json!({"delta": {"reasoning": "Let me see."}}),
            ),
            &mut p,
            false,
        );
        apply_record(
            &mut m,
            &rec(
                "ai.chat.delta",
                json!({"delta": {"content": "<think>more</think>Answer"}}),
            ),
            &mut p,
            false,
        );
        assert_eq!(m.content, "Answer");
        assert_eq!(m.thinking_segments.len(), 1);
        assert_eq!(m.thinking_segments[0].content, "Let me see.more");
        assert_eq!(m.status, MsgStatus::Streaming);
    }

    #[test]
    fn a_status_label_is_a_hint_never_reasoning() {
        let mut m = fresh();
        let mut p = ThinkParser::default();
        apply_record(
            &mut m,
            &rec("ai.chat.status", json!({"phase": "connecting"})),
            &mut p,
            false,
        );
        assert_eq!(m.activity_label.as_deref(), Some("Connecting"));
        assert!(m.thinking_segments.iter().all(|s| s.content.is_empty()));
        apply_record(
            &mut m,
            &rec("ai.chat.status", json!({"phase": "fast_mode_unavailable"})),
            &mut p,
            false,
        );
        assert_eq!(m.notices, vec!["fast_mode_unavailable".to_owned()]);
        assert_eq!(m.activity_label.as_deref(), Some("Connecting"));
    }

    #[test]
    fn a_consent_gated_call_is_one_card_keyed_by_call_id() {
        let mut m = fresh();
        let mut p = ThinkParser::default();
        let call = json!({"call_id": "c1", "name": "ui.transactions.metadata.update", "arguments": {"note": "x"}, "kind_class": "mutating", "needs_consent": true, "approval": "required"});
        apply_record(
            &mut m,
            &rec("ai.chat.tool_call", call.clone()),
            &mut p,
            false,
        );
        assert_eq!(m.tool_calls[0].status, ToolStatus::Pending);
        apply_record(
            &mut m,
            &rec(
                "ai.chat.tool_consent_required",
                json!({"call_id": "c1", "name": "ui.transactions.metadata.update", "arguments_preview": {"note": "x"}}),
            ),
            &mut p,
            false,
        );
        assert_eq!(m.tool_calls[0].status, ToolStatus::AwaitingConsent);
        let mut running = call;
        running["needs_consent"] = json!(false);
        running.as_object_mut().unwrap().remove("approval");
        apply_record(&mut m, &rec("ai.chat.tool_call", running), &mut p, false);
        assert_eq!(m.tool_calls.len(), 1);
        assert_eq!(m.tool_calls[0].status, ToolStatus::Running);
        // An older daemon's record without `approval` keeps what the earlier one said.
        assert_eq!(m.tool_calls[0].approval.as_deref(), Some("required"));
        apply_record(
            &mut m,
            &rec(
                "ai.chat.tool_result",
                json!({"call_id": "c1", "ok": true, "envelope": {"kind": "k"}}),
            ),
            &mut p,
            false,
        );
        assert_eq!(m.tool_calls[0].status, ToolStatus::Done);
        assert_eq!(m.tool_calls.len(), 1);
    }

    #[test]
    fn denials_and_errors_map_from_the_reason() {
        let mut m = fresh();
        let mut p = ThinkParser::default();
        apply_record(
            &mut m,
            &rec(
                "ai.chat.tool_result",
                json!({"call_id": "x", "ok": false, "reason": "user_denied"}),
            ),
            &mut p,
            false,
        );
        apply_record(
            &mut m,
            &rec(
                "ai.chat.tool_result",
                json!({"call_id": "y", "ok": false, "reason": "boom", "message": "failed"}),
            ),
            &mut p,
            false,
        );
        assert_eq!(m.tool_calls[0].status, ToolStatus::Denied);
        assert_eq!(m.tool_calls[0].name, "Tool");
        assert_eq!(m.tool_calls[1].status, ToolStatus::Error);
        assert_eq!(m.tool_calls[1].result, Some(json!("failed")));
    }

    #[test]
    fn aborted_records_are_ignored() {
        let mut m = fresh();
        let mut p = ThinkParser::default();
        apply_record(
            &mut m,
            &rec("ai.chat.delta", json!({"delta": {"content": "late"}})),
            &mut p,
            true,
        );
        assert!(m.content.is_empty());
    }

    #[test]
    fn rounds_open_new_segments_only_when_needed() {
        let mut m = fresh();
        begin_segment(&mut m);
        begin_segment(&mut m);
        assert_eq!(m.thinking_segments.len(), 1);
        let seg = m.thinking_segments[0].id.clone();
        let mut p = ThinkParser::default();
        apply_record(
            &mut m,
            &rec(
                "ai.chat.tool_call",
                json!({"call_id": "c", "name": "ui.overview.snapshot", "kind_class": "read_only"}),
            ),
            &mut p,
            false,
        );
        assert_eq!(m.tool_calls[0].segment_id.as_deref(), Some(seg.as_str()));
        begin_segment(&mut m);
        assert_eq!(m.thinking_segments.len(), 2);
    }

    #[test]
    fn consent_requests_respect_once_only_tools() {
        let r = consent_request(&json!({"call_id": "c", "name": "ui.review.apply"})).unwrap();
        assert!(!r.can_allow_session);
        assert!(!r.reversible && !r.egresses);
        assert_eq!(r.summary, "ui.review.apply");
        let s = consent_request(
            &json!({"call_id": "c", "name": "ui.saved_views.create", "reversible": true}),
        )
        .unwrap();
        assert!(s.can_allow_session && s.reversible);
    }

    #[test]
    fn mutations_complete_once_on_success() {
        let mut seen = std::collections::HashMap::new();
        assert_eq!(
            completed_mutation(
                &mut seen,
                &rec(
                    "ai.chat.tool_call",
                    json!({"call_id": "c", "name": "ui.x", "kind_class": "mutating"})
                )
            ),
            None
        );
        assert_eq!(
            completed_mutation(
                &mut seen,
                &rec("ai.chat.tool_result", json!({"call_id": "c", "ok": true}))
            )
            .as_deref(),
            Some("ui.x")
        );
        assert_eq!(
            completed_mutation(
                &mut seen,
                &rec("ai.chat.tool_result", json!({"call_id": "c", "ok": true}))
            ),
            None
        );
    }

    #[test]
    fn consent_responses_update_the_card() {
        let mut m = fresh();
        let mut p = ThinkParser::default();
        apply_record(
            &mut m,
            &rec(
                "ai.chat.tool_consent_required",
                json!({"call_id": "c", "name": "n"}),
            ),
            &mut p,
            false,
        );
        apply_consent_response(&mut m, "c", "deny", true, None);
        assert_eq!(m.tool_calls[0].status, ToolStatus::Denied);
        assert_eq!(m.tool_calls[0].reason.as_deref(), Some("user_denied"));
        apply_consent_response(&mut m, "c", "allow_once", false, Some("expired"));
        assert_eq!(m.tool_calls[0].status, ToolStatus::Error);
        assert_eq!(m.tool_calls[0].reason.as_deref(), Some("expired"));
    }
}
