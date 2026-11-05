//! Terminal approval of the daemon's recomputed custody-review proposal
//! (`kassiber/cli/review_consent.py`). Only the daemon's separate
//! `review_preview` field is an approval surface; anything nested in the
//! model's arguments is not, and no blanket flag approves a review.

use std::io::{BufRead, Write};

use kassiber_envelope::get;
use kassiber_pyjson::{dumps, Options, Value};

pub const TOOL_NAME: &str = "ui.review.apply";

fn count(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Int(n)) if *n >= 0.into())
}

fn text(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Str(text)) if text.code_points().next().is_some())
}

fn hex64(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Str(text))
        if text.code_points().count() == 64
            && text.code_points().all(|c| matches!(c, 0x30..=0x39 | 0x61..=0x66)))
}

fn effects(value: Option<&Value>) -> bool {
    let Some(value @ Value::Dict(_)) = value else {
        return false;
    };
    count(get(value, "entries_count"))
        && count(get(value, "quarantine_count"))
        && matches!(get(value, "report_ready"), Some(Value::Bool(_)))
        && matches!(get(value, "quarantines"), Some(Value::List(_)))
        && matches!(get(value, "wallet_holdings"), Some(Value::List(_)))
        && hex64(get(value, "journal_digest"))
}

fn schema_one(value: &Value) -> bool {
    matches!(get(value, "schema_version"), Some(Value::Int(n)) if *n == 1.into())
}

/// `_validated_preview(data)`.
fn validated_preview(data: &Value) -> Option<&Value> {
    let (Some(preview @ Value::Dict(_)), Some(arguments @ Value::Dict(_))) =
        (get(data, "review_preview"), get(data, "arguments_preview"))
    else {
        return None;
    };
    let proposed = match get(arguments, "artifact") {
        Some(proposed @ Value::Dict(_)) => proposed,
        _ => return None,
    };
    if !["workspace_id", "profile_id", "digest"]
        .iter()
        .all(|key| text(get(proposed, key)))
        || !hex64(get(proposed, "digest"))
        || !text(get(arguments, "idempotency_key"))
    {
        return None;
    }
    let status = get(preview, "status");
    let valid = if matches!(status, Some(Value::Str(s)) if s.is("ready")) {
        match get(preview, "artifact") {
            Some(artifact @ Value::Dict(_)) if artifact == proposed => {
                let operations_ok = match get(artifact, "operations") {
                    Some(Value::List(operations)) => {
                        (1..=50).contains(&operations.len())
                            && operations.iter().all(|op| {
                                matches!(op, Value::Dict(_))
                                    && matches!(get(op, "type"), Some(Value::Str(kind))
                                        if kind.is("price_override") || kind.is("exclude") || kind.is("custody_component"))
                            })
                    }
                    _ => false,
                };
                schema_one(artifact)
                    && count(get(artifact, "base_input_version"))
                    && operations_ok
                    && effects(get(artifact, "before"))
                    && effects(get(artifact, "after"))
            }
            _ => false,
        }
    } else if matches!(status, Some(Value::Str(s)) if s.is("applied")) {
        match get(preview, "receipt") {
            Some(receipt @ Value::Dict(_)) => {
                schema_one(receipt)
                    && matches!(get(receipt, "status"), Some(Value::Str(s)) if s.is("verified"))
                    && ["id", "created_at", "artifact_digest"]
                        .iter()
                        .all(|key| text(get(receipt, key)))
                    && ["workspace_id", "profile_id"]
                        .iter()
                        .all(|key| get(receipt, key) == get(proposed, key))
                    && get(receipt, "artifact_digest") == get(proposed, "digest")
                    && get(receipt, "idempotency_key") == get(arguments, "idempotency_key")
                    && count(get(receipt, "result_input_version"))
                    && matches!(get(receipt, "operations"), Some(Value::List(_)))
                    && effects(get(receipt, "verification"))
            }
            _ => false,
        }
    } else {
        false
    };
    valid.then_some(preview)
}

/// `decide(data, interactive=..., stdin=..., out=...)`: a fresh human
/// answer, independent of `--yes`, `--allow-tool` and session grants.
pub fn decide(
    data: &Value,
    interactive: bool,
    stdin: &mut dyn BufRead,
    out: &mut dyn Write,
) -> String {
    let preview = validated_preview(data);
    let Some(preview) = preview.filter(|_| interactive) else {
        let _ = out.write_all(
            b"Review denied: interactive, once-only approval of a complete daemon preview is required.\n",
        );
        let _ = out.flush();
        return "deny".into();
    };
    let _ = out.write_all(b"\nDaemon-validated review (full proposal or historical receipt):\n");
    // JSON escapes terminal control characters in imported text.
    let shown = dumps(preview, &Options::new().indent(2).sort_keys(true)).unwrap_or_default();
    let _ = writeln!(out, "{shown}");
    loop {
        let _ = out.write_all(b"Allow this exact review? [y] once, [n] deny, [c] cancel: ");
        let _ = out.flush();
        let mut line = String::new();
        let _ = stdin.read_line(&mut line);
        let choice = kassiber_pytext::lower(kassiber_pytext::strip(&line));
        match choice.as_str() {
            "y" | "yes" => return "allow_once".into(),
            "" | "n" | "no" | "deny" => return "deny".into(),
            "c" | "cancel" => return "cancel".into(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_envelope::{dict, s};

    #[test]
    fn without_a_daemon_preview_a_review_is_denied() {
        let data = dict([(
            "arguments_preview",
            dict([("review_preview", dict([("status", s("ready"))]))]),
        )]);
        let mut out = Vec::new();
        let decision = decide(
            &data,
            true,
            &mut std::io::Cursor::new(b"y\n".to_vec()),
            &mut out,
        );
        assert_eq!(decision, "deny");
        assert!(String::from_utf8(out).unwrap().starts_with("Review denied"));
    }
}
