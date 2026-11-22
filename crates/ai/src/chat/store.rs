//! The book work of chat (R6-7), as one seam the storage layer serves:
//! history for `ui.chat.*` and `chats`, and the request-loop steps of
//! `ai.chat` (screen binding, persistence, the detected API format).

use kassiber_envelope::{get, AppError};
use kassiber_pyjson::{Key, PyStr, Value};
use kassiber_store::rusqlite::Connection;

use super::history::{self, Persist};
use crate::providers::record_detected_api_format;
use crate::redact::redact_ai_tool_result;

/// One operation on the open book.
pub enum Op<'a> {
    /// `list_sessions(conn, profile_id, limit)`.
    ListSessions {
        profile_id: &'a str,
        limit: i64,
    },
    /// `get_session(...)`.
    GetSession {
        profile_id: &'a str,
        session_id: &'a PyStr,
        include_messages: bool,
    },
    DeleteSession {
        profile_id: &'a str,
        session_id: &'a PyStr,
    },
    ClearSessions {
        profile_id: &'a str,
    },
    /// `history_mode(conn)`.
    HistoryMode,
    /// `history_enabled(conn, database_encrypted=...)`.
    HistoryEnabled {
        encrypted: bool,
    },
    /// `ui.chat.history.configure` / `chats config`.
    Configure {
        history: Option<&'a str>,
        encrypted: bool,
    },
    /// `_persist`; the session id or `None`.
    Persist(&'a Persist),
    /// `_bind_screen_context_entity` then `_bind_screen_context_metrics`.
    BindScreen {
        screen_context: &'a Value,
        workspace_id: &'a str,
        profile_id: &'a str,
        /// The request asked to read the entity for the model
        /// (`read_pinned_entity`): a custody gap is bound to the book only
        /// then, since that check is a gap search.
        pinned: bool,
    },
    /// `record_detected_api_format(conn, name, base_url, detected)`.
    RecordDetectedFormat {
        provider: &'a str,
        base_url: &'a str,
        detected: &'a str,
    },
}

/// `transaction_in_profile(conn, profile_id, ref)`.
pub type InProfile<'a> = &'a dyn Fn(&Connection, &str, &str) -> Result<bool, AppError>;

/// `custody_gap_in_profile(conn, workspace_id, profile_id, gap_id)`:
/// whether the book's gap search finds the gap.
pub type GapInProfile<'a> = &'a dyn Fn(&Connection, &str, &str, &str) -> Result<bool, AppError>;

/// `bind_screen_metrics_scope(conn, scope, workspace_id, profile_id)`.
pub type BindMetricsScope<'a> =
    &'a dyn Fn(&Connection, &Value, &str, &str) -> Result<Option<Value>, AppError>;

/// The book-side helpers chat binding borrows from the transactions
/// family (`kassiber/core/ui_snapshot.py`).
pub struct ScreenBinding<'a> {
    pub transaction_in_profile: InProfile<'a>,
    pub custody_gap_in_profile: GapInProfile<'a>,
    pub bind_metrics_scope: BindMetricsScope<'a>,
}

fn without(value: &Value, keys: &[&str]) -> Value {
    match value {
        Value::Dict(pairs) => Value::Dict(
            pairs
                .iter()
                .filter(|(key, _)| !keys.iter().any(|drop| *key == Key::from(*drop)))
                .cloned()
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `_bind_screen_context_entity` and `_bind_screen_context_metrics`: a
/// transaction from another book is not named to the model, nor a custody
/// gap the book's gap search does not find when the request asked to read
/// it (`pinned`), and page counts reach it only while they still describe
/// the active book. Without `pinned` a gap is kept as before.
fn bind_screen(
    conn: &Connection,
    context: &Value,
    workspace_id: &str,
    profile_id: &str,
    pinned: bool,
    binding: &ScreenBinding<'_>,
) -> Result<Value, AppError> {
    let mut context = context.clone();
    let kind = match get(&context, "entity_type") {
        Some(Value::Str(kind)) if matches!(&context, Value::Dict(_)) => {
            if kind.is("transaction") {
                Some(true)
            } else if kind.is("custody_gap") && pinned {
                Some(false)
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(transaction) = kind {
        let entity = match get(&context, "entity_id") {
            Some(Value::Str(id)) => Some(crate::pyval::lossy(id)),
            _ => None,
        };
        let in_book = match entity {
            Some(entity) if transaction => {
                (binding.transaction_in_profile)(conn, profile_id, &entity)?
            }
            Some(entity) => {
                (binding.custody_gap_in_profile)(conn, workspace_id, profile_id, &entity)?
            }
            None => false,
        };
        if !in_book {
            context = without(&context, &["entity_type", "entity_id"]);
        }
    }
    if !matches!(&context, Value::Dict(_)) || get(&context, "metrics").is_none() {
        return Ok(context);
    }
    let mut bound = without(&context, &["metrics", "metrics_scope"]);
    let empty = Value::Dict(Vec::new());
    let scope = match get(&context, "metrics_scope") {
        Some(scope) if kassiber_envelope::truthy(scope) => scope,
        _ => &empty,
    };
    let Some(model_scope) = (binding.bind_metrics_scope)(conn, scope, workspace_id, profile_id)?
    else {
        return Ok(bound);
    };
    kassiber_envelope::set(
        &mut bound,
        "metrics",
        get(&context, "metrics").cloned().unwrap_or(Value::Null),
    );
    if kassiber_envelope::truthy(&model_scope) {
        // The wallet label is the user's own text: it reaches the model
        // through the AI result redaction, as the AI wallet list shows it.
        kassiber_envelope::set(
            &mut bound,
            "metrics_scope",
            redact_ai_tool_result(&model_scope),
        );
    }
    Ok(bound)
}

/// Runs `op` on `conn`. `resolve_scope` is `resolve_scope(conn, None, None)`.
pub fn run(
    conn: &Connection,
    op: Op<'_>,
    resolve_scope: &mut dyn FnMut() -> Result<(String, String), AppError>,
    binding: &ScreenBinding<'_>,
) -> Result<Value, AppError> {
    match op {
        Op::ListSessions { profile_id, limit } => Ok(Value::List(history::list_sessions(
            conn, profile_id, limit,
        )?)),
        Op::GetSession {
            profile_id,
            session_id,
            include_messages,
        } => history::get_session(conn, profile_id, session_id, include_messages),
        Op::DeleteSession {
            profile_id,
            session_id,
        } => history::delete_session(conn, profile_id, session_id),
        Op::ClearSessions { profile_id } => history::clear_sessions(conn, profile_id),
        Op::HistoryMode => Ok(crate::pyval::s(history::history_mode(conn)?)),
        Op::HistoryEnabled { encrypted } => {
            Ok(Value::Bool(history::history_enabled(conn, encrypted)?))
        }
        Op::Configure { history, encrypted } => history::configure(conn, history, encrypted),
        Op::Persist(request) => {
            Ok(history::persist(conn, request, resolve_scope)?.map_or(Value::Null, Value::Str))
        }
        Op::BindScreen {
            screen_context,
            workspace_id,
            profile_id,
            pinned,
        } => bind_screen(
            conn,
            screen_context,
            workspace_id,
            profile_id,
            pinned,
            binding,
        ),
        Op::RecordDetectedFormat {
            provider,
            base_url,
            detected,
        } => Ok(Value::Bool(record_detected_api_format(
            conn, provider, base_url, detected,
        )?)),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use kassiber_envelope::{dict, s};

    use super::*;

    fn screen(kind: &str, id: &str) -> Value {
        dict([
            ("route", s("/transfers")),
            ("entity_type", s(kind)),
            ("entity_id", s(id)),
        ])
    }

    /// A custody gap is bound to the book when the request reads it: one the
    /// gap search does not find is dropped like a transaction of another
    /// book. Without the read the gap is kept and nothing is searched.
    #[test]
    fn a_pinned_custody_gap_is_bound_to_the_book() {
        let conn = Connection::open_in_memory().unwrap();
        let searched = Cell::new(0);
        let tx = |_: &Connection, _: &str, id: &str| Ok(id == "t-1");
        let gap = |_: &Connection, w: &str, p: &str, id: &str| {
            searched.set(searched.get() + 1);
            assert_eq!((w, p), ("w", "p"));
            Ok(id == "cg_here")
        };
        let metrics = |_: &Connection, _: &Value, _: &str, _: &str| Ok(None);
        let binding = ScreenBinding {
            transaction_in_profile: &tx,
            custody_gap_in_profile: &gap,
            bind_metrics_scope: &metrics,
        };
        let bind = |context: &Value, pinned: bool| {
            bind_screen(&conn, context, "w", "p", pinned, &binding).unwrap()
        };
        let here = screen("custody_gap", "cg_here");
        assert_eq!(bind(&here, true), here);
        let elsewhere = screen("custody_gap", "cg_elsewhere");
        assert_eq!(bind(&elsewhere, true), dict([("route", s("/transfers"))]));
        assert_eq!(searched.get(), 2);
        assert_eq!(bind(&elsewhere, false), elsewhere);
        assert_eq!(searched.get(), 2, "no gap search without the read");
        // Transactions are bound either way, and never searched as gaps.
        for pinned in [true, false] {
            assert_eq!(
                bind(&screen("transaction", "t-2"), pinned),
                dict([("route", s("/transfers"))])
            );
            let kept = screen("transaction", "t-1");
            assert_eq!(bind(&kept, pinned), kept);
        }
        assert_eq!(searched.get(), 2);
    }
}
