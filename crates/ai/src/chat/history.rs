//! Persisted chat sessions (`kassiber/core/chat_history.py`): two
//! profile-scoped tables inside the book's SQLCipher boundary, and the
//! `ai_chat_history` policy. `auto` (the default) persists only when the
//! database file is encrypted; `on` always; `off` never. The daemon writes
//! only for a request that carried a persistence intent.
//!
//! Callers own the connection; nothing here opens a socket.

use kassiber_envelope::{dict, get, AppError};
use kassiber_pyjson::{dumps, Key, Options, PyStr, Value};
use kassiber_store::rusqlite::{params, Connection, OptionalExtension, Row};

use crate::clock::now_iso;
use crate::providers::{atomically, commit, database_error, set_setting, setting, value_of};
use crate::pyval::{lossy, s};

/// `AI_CHAT_HISTORY_SETTING`.
pub const HISTORY_SETTING: &str = "ai_chat_history";
/// `AI_CHAT_HISTORY_MODES`.
pub const HISTORY_MODES: [&str; 3] = ["auto", "on", "off"];
/// `SESSION_TITLE_MAX_CHARS`.
pub const SESSION_TITLE_MAX_CHARS: usize = 80;

/// `uuid.uuid4()`, from SQLite's CSPRNG so no new dependency is needed.
fn uuid4(conn: &Connection) -> Result<String, AppError> {
    let mut bytes: Vec<u8> = conn
        .query_row("SELECT randomblob(16)", [], |row| row.get(0))
        .map_err(database_error)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

/// `history_mode(conn)`: the stored mode, else `auto`.
pub fn history_mode(conn: &Connection) -> Result<&'static str, AppError> {
    Ok(match setting(conn, HISTORY_SETTING)? {
        Some(Value::Str(text)) => HISTORY_MODES
            .iter()
            .copied()
            .find(|mode| text.is(mode))
            .unwrap_or("auto"),
        _ => "auto",
    })
}

/// `set_history_mode(conn, value)`.
pub fn set_history_mode(conn: &Connection, value: &str) -> Result<&'static str, AppError> {
    let Some(mode) = HISTORY_MODES.iter().copied().find(|mode| *mode == value) else {
        return Err(AppError::new(
            "validation",
            "chat history mode must be one of auto, on, off",
        )
        .with_details(dict([("value", s(value))])));
    };
    atomically(conn, || set_setting(conn, HISTORY_SETTING, mode))?;
    Ok(mode)
}

/// `history_enabled(conn, database_encrypted=...)`: `auto` persists only
/// into an encrypted database.
pub fn history_enabled(conn: &Connection, database_encrypted: bool) -> Result<bool, AppError> {
    Ok(match history_mode(conn)? {
        "on" => true,
        "off" => false,
        _ => database_encrypted,
    })
}

/// `session_title_from_prompt(prompt)`: whitespace collapsed, at most 80
/// code points with an ellipsis.
pub fn session_title_from_prompt(prompt: &PyStr) -> PyStr {
    let space = kassiber_pytext::is_space;
    let points: Vec<u32> = prompt.code_points().collect();
    let mut collapsed: Vec<u32> = Vec::new();
    for word in points
        .split(|point| space(*point))
        .filter(|word| !word.is_empty())
    {
        if !collapsed.is_empty() {
            collapsed.push(u32::from(' '));
        }
        collapsed.extend_from_slice(word);
    }
    // The points came from a PyStr, so each is a valid code point.
    let text = |points: Vec<u32>| PyStr::from_code_points(points).unwrap_or_default();
    if collapsed.len() <= SESSION_TITLE_MAX_CHARS {
        if collapsed.is_empty() {
            return PyStr::from("Chat");
        }
        return text(collapsed);
    }
    collapsed.truncate(SESSION_TITLE_MAX_CHARS - 1);
    while collapsed.last().is_some_and(|point| space(*point)) {
        collapsed.pop();
    }
    collapsed.push(u32::from('…'));
    text(collapsed)
}

fn text_param(text: &PyStr) -> String {
    lossy(text)
}

fn session_row(conn: &Connection, profile_id: &str, session_id: &PyStr) -> Result<Value, AppError> {
    conn.query_row(
        "SELECT * FROM ai_chat_sessions WHERE id = ? AND profile_id = ?",
        params![text_param(session_id), profile_id],
        |row| Ok(session_dict(row, false)),
    )
    .optional()
    .map_err(database_error)?
    .ok_or_else(|| {
        AppError::new("not_found", "chat session not found for the active profile")
            .with_details(dict([("session_id", Value::Str(session_id.clone()))]))
    })
}

fn column(row: &Row<'_>, name: &str) -> Value {
    row.get_ref(name).map(value_of).unwrap_or(Value::Null)
}

/// `_session_to_dict(row)`.
fn session_dict(row: &Row<'_>, with_count: bool) -> Value {
    let mut payload: Vec<(Key, Value)> = [
        "id",
        "workspace_id",
        "profile_id",
        "title",
        "provider",
        "model",
        "created_at",
        "updated_at",
    ]
    .iter()
    .map(|name| (Key::from(*name), column(row, name)))
    .collect();
    if with_count {
        payload.push((Key::from("message_count"), column(row, "message_count")));
    }
    Value::Dict(payload)
}

fn decode_json(value: Value) -> Result<Value, AppError> {
    match value {
        Value::Str(text) if text.code_points().next().is_some() => {
            kassiber_pytext::loads(&lossy(&text))
                .map_err(|error| AppError::new("internal_error", error.to_string()))
        }
        _ => Ok(Value::Null),
    }
}

/// `_message_to_dict(row)`.
fn message_dict(row: &Row<'_>) -> Result<Value, AppError> {
    Ok(dict([
        ("id", column(row, "id")),
        ("seq", column(row, "seq")),
        ("role", column(row, "role")),
        ("content", column(row, "content")),
        ("tool_calls", decode_json(column(row, "tool_calls_json"))?),
        ("provenance", decode_json(column(row, "provenance_json"))?),
        ("finish_reason", column(row, "finish_reason")),
        ("created_at", column(row, "created_at")),
    ]))
}

/// `json.dumps(value, sort_keys=True)`.
fn stored_json(value: &Value) -> Result<String, AppError> {
    dumps(value, &Options::new().sort_keys(true))
        .map_err(|error| AppError::new("internal_error", error.to_string()))
}

/// `create_session(...)` without committing; returns the new id.
pub fn create_session(
    conn: &Connection,
    workspace_id: &str,
    profile_id: &str,
    title: &PyStr,
    provider: Option<&str>,
    model: Option<&PyStr>,
) -> Result<String, AppError> {
    let id = uuid4(conn)?;
    let timestamp = now_iso();
    conn.execute(
        "INSERT INTO ai_chat_sessions(id, workspace_id, profile_id, title, provider, model, created_at, updated_at) VALUES(?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            id,
            workspace_id,
            profile_id,
            text_param(title),
            provider,
            model.map(text_param),
            timestamp,
            timestamp
        ],
    )
    .map_err(database_error)?;
    Ok(id)
}

fn next_seq(conn: &Connection, session_id: &str) -> Result<i64, AppError> {
    conn.query_row(
        "SELECT COALESCE(MAX(seq), -1) + 1 FROM ai_chat_messages WHERE session_id = ?",
        [session_id],
        |row| row.get(0),
    )
    .map_err(database_error)
}

/// One finished exchange.
pub struct Exchange<'a> {
    pub user_content: &'a PyStr,
    pub assistant_content: &'a PyStr,
    pub provenance: Option<&'a Value>,
    pub finish_reason: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub model: Option<&'a PyStr>,
}

/// `append_exchange(...)` without committing.
pub fn append_exchange(
    conn: &Connection,
    profile_id: &str,
    session_id: &PyStr,
    exchange: &Exchange<'_>,
) -> Result<Value, AppError> {
    let session = session_row(conn, profile_id, session_id)?;
    let session_id = text_param(session_id);
    let timestamp = now_iso();
    let seq = next_seq(conn, &session_id)?;
    conn.execute(
        "INSERT INTO ai_chat_messages(id, session_id, seq, role, content, tool_calls_json, provenance_json, finish_reason, created_at) VALUES(?, ?, ?, 'user', ?, NULL, NULL, NULL, ?)",
        params![uuid4(conn)?, session_id, seq, text_param(exchange.user_content), timestamp],
    )
    .map_err(database_error)?;
    let provenance = exchange
        .provenance
        .filter(|value| kassiber_envelope::truthy(value))
        .map(stored_json)
        .transpose()?;
    conn.execute(
        "INSERT INTO ai_chat_messages(id, session_id, seq, role, content, tool_calls_json, provenance_json, finish_reason, created_at) VALUES(?, ?, ?, 'assistant', ?, NULL, ?, ?, ?)",
        params![
            uuid4(conn)?,
            session_id,
            seq + 1,
            text_param(exchange.assistant_content),
            provenance,
            exchange.finish_reason,
            timestamp
        ],
    )
    .map_err(database_error)?;
    conn.execute(
        "UPDATE ai_chat_sessions SET updated_at = ?, provider = COALESCE(?, provider), model = COALESCE(?, model) WHERE id = ?",
        params![
            timestamp,
            exchange.provider,
            exchange.model.map(text_param),
            session_id
        ],
    )
    .map_err(database_error)?;
    Ok(dict([
        (
            "session_id",
            get(&session, "id").cloned().unwrap_or(Value::Null),
        ),
        ("appended", Value::Int(2.into())),
    ]))
}

/// `append_messages(...)` without committing: prior user/assistant turns of
/// a branched or edited chat, with the notices of the answers they copy.
pub fn append_messages(
    conn: &Connection,
    session_id: &str,
    messages: &[Value],
    notices: &[(usize, Vec<&'static str>)],
) -> Result<usize, AppError> {
    let mut seq = next_seq(conn, session_id)?;
    let timestamp = now_iso();
    let mut appended = 0;
    for (index, message) in messages.iter().enumerate() {
        let role = match get(message, "role") {
            Some(Value::Str(role)) if role.is("user") || role.is("assistant") => role.clone(),
            _ => continue,
        };
        let content = match get(message, "content") {
            Some(Value::Str(content)) if content.code_points().next().is_some() => content,
            _ => continue,
        };
        let phases = if role.is("assistant") {
            notices
                .iter()
                .find(|(at, _)| *at == index)
                .map(|(_, phases)| phases)
                .filter(|phases| !phases.is_empty())
        } else {
            None
        };
        let provenance = phases
            .map(|phases| {
                stored_json(&dict([(
                    "notices",
                    Value::List(phases.iter().map(s).collect()),
                )]))
            })
            .transpose()?;
        conn.execute(
            "INSERT INTO ai_chat_messages(id, session_id, seq, role, content, tool_calls_json, provenance_json, finish_reason, created_at) VALUES(?, ?, ?, ?, ?, NULL, ?, NULL, ?)",
            params![
                uuid4(conn)?,
                session_id,
                seq,
                text_param(&role),
                text_param(content),
                provenance,
                timestamp
            ],
        )
        .map_err(database_error)?;
        seq += 1;
        appended += 1;
    }
    Ok(appended)
}

/// `list_sessions(conn, profile_id, limit=...)`.
pub fn list_sessions(
    conn: &Connection,
    profile_id: &str,
    limit: i64,
) -> Result<Vec<Value>, AppError> {
    let mut statement = conn
        .prepare(
            "SELECT s.*, COUNT(m.id) AS message_count FROM ai_chat_sessions s LEFT JOIN ai_chat_messages m ON m.session_id = s.id WHERE s.profile_id = ? GROUP BY s.id ORDER BY s.updated_at DESC, s.id LIMIT ?",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map(params![profile_id, limit.max(1)], |row| {
            Ok(session_dict(row, true))
        })
        .map_err(database_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(database_error)
}

/// `get_session(conn, profile_id, session_id, include_messages=...)`.
pub fn get_session(
    conn: &Connection,
    profile_id: &str,
    session_id: &PyStr,
    include_messages: bool,
) -> Result<Value, AppError> {
    let mut session = session_row(conn, profile_id, session_id)?;
    if include_messages {
        let mut statement = conn
            .prepare("SELECT * FROM ai_chat_messages WHERE session_id = ? ORDER BY seq")
            .map_err(database_error)?;
        let mut rows = statement
            .query([text_param(session_id)])
            .map_err(database_error)?;
        let mut messages = Vec::new();
        while let Some(row) = rows.next().map_err(database_error)? {
            messages.push(message_dict(row)?);
        }
        let count = messages.len();
        kassiber_envelope::set(&mut session, "messages", Value::List(messages));
        kassiber_envelope::set(&mut session, "message_count", Value::from(count));
    }
    Ok(session)
}

/// `delete_session(conn, profile_id, session_id)`.
pub fn delete_session(
    conn: &Connection,
    profile_id: &str,
    session_id: &PyStr,
) -> Result<Value, AppError> {
    atomically(conn, || {
        let session = session_row(conn, profile_id, session_id)?;
        let id = text_param(session_id);
        conn.execute("DELETE FROM ai_chat_messages WHERE session_id = ?", [&id])
            .map_err(database_error)?;
        conn.execute("DELETE FROM ai_chat_sessions WHERE id = ?", [&id])
            .map_err(database_error)?;
        Ok(dict([
            (
                "deleted",
                get(&session, "id").cloned().unwrap_or(Value::Null),
            ),
            (
                "title",
                get(&session, "title").cloned().unwrap_or(Value::Null),
            ),
        ]))
    })
}

/// `clear_sessions(conn, profile_id)`.
pub fn clear_sessions(conn: &Connection, profile_id: &str) -> Result<Value, AppError> {
    atomically(conn, || {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ai_chat_sessions WHERE profile_id = ?",
                [profile_id],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        conn.execute(
            "DELETE FROM ai_chat_messages WHERE session_id IN (SELECT id FROM ai_chat_sessions WHERE profile_id = ?)",
            [profile_id],
        )
        .map_err(database_error)?;
        conn.execute(
            "DELETE FROM ai_chat_sessions WHERE profile_id = ?",
            [profile_id],
        )
        .map_err(database_error)?;
        Ok(dict([("deleted", Value::from(count))]))
    })
}

/// `history.configure` / `chats config`: the policy and what it means for
/// this database.
pub fn configure(
    conn: &Connection,
    history: Option<&str>,
    database_encrypted: bool,
) -> Result<Value, AppError> {
    if let Some(history) = history {
        set_history_mode(conn, history)?;
    }
    Ok(dict([
        ("history", s(history_mode(conn)?)),
        (
            "history_enabled",
            Value::Bool(history_enabled(conn, database_encrypted)?),
        ),
        ("database_encrypted", Value::Bool(database_encrypted)),
    ]))
}

/// The exchange `_persist_ai_chat_exchange` writes, frozen at the turn's
/// end.
pub struct Persist {
    pub encrypted: bool,
    /// `scope_workspace_id` / `scope_profile_id`, when both are strings.
    pub scope: Option<(String, String)>,
    pub session_id: Option<PyStr>,
    pub user_content: PyStr,
    pub assistant_content: PyStr,
    pub provenance: Value,
    pub finish_reason: Option<String>,
    pub provider: String,
    pub model: PyStr,
    /// `_ai_chat_seed_prefix(messages)` when `seed_history`, else empty.
    pub seed_prefix: Vec<Value>,
    pub seed_notices: Vec<(usize, Vec<&'static str>)>,
}

/// `_persist`: the session the exchange went to, or `None` when the policy
/// refuses. A stale book is `stale_context`.
pub fn persist(
    conn: &Connection,
    request: &Persist,
    resolve_scope: &mut dyn FnMut() -> Result<(String, String), AppError>,
) -> Result<Option<PyStr>, AppError> {
    // The stored policy is authoritative even for continuations and
    // explicit requests: `off` never writes, `auto` only when encrypted.
    if !history_enabled(conn, request.encrypted)? {
        return Ok(None);
    }
    let (workspace_id, profile_id) = match &request.scope {
        Some((workspace, profile)) => {
            let workspace_found = conn
                .query_row("SELECT 1 FROM workspaces WHERE id = ?", [workspace], |_| {
                    Ok(())
                })
                .optional()
                .map_err(database_error)?
                .is_some();
            let profile_found = conn
                .query_row(
                    "SELECT 1 FROM profiles WHERE id = ? AND workspace_id = ?",
                    [profile, workspace],
                    |_| Ok(()),
                )
                .optional()
                .map_err(database_error)?
                .is_some();
            if !workspace_found || !profile_found {
                return Err(AppError::new(
                    "stale_context",
                    "The chat's original book is no longer available",
                ));
            }
            (workspace.clone(), profile.clone())
        }
        None => resolve_scope()?,
    };
    atomically(conn, || {
        let session_id = match &request.session_id {
            Some(id) => id.clone(),
            None => {
                let title = session_title_from_prompt(&request.user_content);
                let id = create_session(
                    conn,
                    &workspace_id,
                    &profile_id,
                    &title,
                    Some(&request.provider),
                    Some(&request.model),
                )?;
                if !request.seed_prefix.is_empty() {
                    append_messages(conn, &id, &request.seed_prefix, &request.seed_notices)?;
                }
                PyStr::from(id)
            }
        };
        append_exchange(
            conn,
            &profile_id,
            &session_id,
            &Exchange {
                user_content: &request.user_content,
                assistant_content: &request.assistant_content,
                provenance: Some(&request.provenance),
                finish_reason: request.finish_reason.as_deref(),
                provider: Some(&request.provider),
                model: Some(&request.model),
            },
        )?;
        Ok(Some(session_id))
    })
}

/// Commits what a caller left open; a no-op outside a transaction.
pub fn finish(conn: &Connection) -> Result<(), AppError> {
    commit(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE workspaces(id TEXT PRIMARY KEY, label TEXT);
             CREATE TABLE profiles(id TEXT PRIMARY KEY, workspace_id TEXT, label TEXT);
             CREATE TABLE ai_chat_sessions(id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, profile_id TEXT NOT NULL, title TEXT NOT NULL, provider TEXT, model TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
             CREATE TABLE ai_chat_messages(id TEXT PRIMARY KEY, session_id TEXT NOT NULL, seq INTEGER NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, tool_calls_json TEXT, provenance_json TEXT, finish_reason TEXT, created_at TEXT NOT NULL);
             INSERT INTO workspaces VALUES('w1','Demo');
             INSERT INTO profiles VALUES('p1','w1','Main');",
        )
        .unwrap();
        conn
    }

    fn text(value: &str) -> PyStr {
        PyStr::from(value)
    }

    #[test]
    fn titles_collapse_whitespace_and_cut_at_80() {
        assert_eq!(
            session_title_from_prompt(&text("  What\n changed\tthis week? ")),
            text("What changed this week?")
        );
        assert_eq!(session_title_from_prompt(&text(" \n ")), text("Chat"));
        let long = "word ".repeat(30);
        let title = session_title_from_prompt(&text(&long));
        let title = title.as_str().unwrap();
        assert_eq!(title.chars().count(), 80);
        assert!(title.ends_with("word…"));
        // rstrip before the ellipsis.
        let spaced = format!("{}  tail", "a".repeat(78));
        assert_eq!(
            session_title_from_prompt(&text(&spaced)).as_str().unwrap(),
            format!("{}…", "a".repeat(78))
        );
    }

    #[test]
    fn auto_policy_persists_only_into_encrypted_books() {
        let conn = book();
        assert_eq!(history_mode(&conn).unwrap(), "auto");
        assert!(!history_enabled(&conn, false).unwrap());
        assert!(history_enabled(&conn, true).unwrap());
        set_history_mode(&conn, "off").unwrap();
        assert!(!history_enabled(&conn, true).unwrap());
        set_history_mode(&conn, "on").unwrap();
        assert!(history_enabled(&conn, false).unwrap());
        let error = set_history_mode(&conn, "bogus").unwrap_err();
        assert_eq!(error.code, "validation");
        conn.execute(
            "UPDATE settings SET value = 'weird' WHERE key = ?",
            [HISTORY_SETTING],
        )
        .unwrap();
        assert_eq!(history_mode(&conn).unwrap(), "auto");
    }

    /// `ChatHistorySeedTest.test_seeded_prefix_round_trips_via_get_session`.
    #[test]
    fn seeded_prefix_round_trips_via_get_session() {
        let conn = book();
        let id = create_session(
            &conn,
            "w1",
            "p1",
            &text("branched chat"),
            Some("ollama"),
            Some(&text("qwen")),
        )
        .unwrap();
        let message =
            |role: &str, content: &str| dict([("role", s(role)), ("content", s(content))]);
        append_messages(
            &conn,
            &id,
            &[
                message("user", "seed question"),
                message("assistant", "seed answer"),
                message("system", "ignored"),
                message("assistant", ""),
            ],
            &[(1, vec!["fast_mode_unavailable"])],
        )
        .unwrap();
        append_exchange(
            &conn,
            "p1",
            &text(&id),
            &Exchange {
                user_content: &text("follow-up"),
                assistant_content: &text("reply"),
                provenance: None,
                finish_reason: None,
                provider: None,
                model: None,
            },
        )
        .unwrap();
        let stored = get_session(&conn, "p1", &text(&id), true).unwrap();
        assert_eq!(get(&stored, "message_count"), Some(&Value::from(4usize)));
        let Some(Value::List(messages)) = get(&stored, "messages") else {
            panic!()
        };
        let pairs: Vec<(String, String)> = messages
            .iter()
            .map(|m| {
                (
                    kassiber_envelope::py_str(get(m, "role").unwrap()),
                    kassiber_envelope::py_str(get(m, "content").unwrap()),
                )
            })
            .collect();
        assert_eq!(
            pairs,
            [
                ("user", "seed question"),
                ("assistant", "seed answer"),
                ("user", "follow-up"),
                ("assistant", "reply"),
            ]
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
        );
        assert_eq!(
            get(&messages[1], "provenance"),
            Some(&dict([(
                "notices",
                Value::List(vec![s("fast_mode_unavailable")])
            )]))
        );
        let listed = list_sessions(&conn, "p1", 0).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(get(&listed[0], "message_count"), Some(&Value::from(4i64)));
        assert_eq!(
            get_session(&conn, "p1", &text(&id), false)
                .map(|v| get(&v, "message_count").is_none())
                .unwrap(),
            true
        );
        let missing = get_session(&conn, "p2", &text(&id), false).unwrap_err();
        assert_eq!(missing.code, "not_found");
        assert_eq!(
            get(&delete_session(&conn, "p1", &text(&id)).unwrap(), "deleted"),
            Some(&s(&id))
        );
        assert_eq!(
            clear_sessions(&conn, "p1").unwrap(),
            dict([("deleted", Value::from(0i64))])
        );
    }

    #[test]
    fn persist_follows_the_stored_policy_and_the_frozen_book() {
        let conn = book();
        let request = |session: Option<&str>, scope: Option<(&str, &str)>| Persist {
            encrypted: false,
            scope: scope.map(|(w, p)| (w.to_owned(), p.to_owned())),
            session_id: session.map(text),
            user_content: text("hello"),
            assistant_content: text("hi"),
            provenance: dict([("notices", Value::List(vec![]))]),
            finish_reason: Some("stop".into()),
            provider: "local".into(),
            model: text("m"),
            seed_prefix: Vec::new(),
            seed_notices: Vec::new(),
        };
        let mut unused = || -> Result<(String, String), AppError> { unreachable!() };
        assert_eq!(
            persist(&conn, &request(None, Some(("w1", "p1"))), &mut unused).unwrap(),
            None
        );
        set_history_mode(&conn, "on").unwrap();
        let id = persist(&conn, &request(None, Some(("w1", "p1"))), &mut unused)
            .unwrap()
            .unwrap();
        let again = persist(
            &conn,
            &request(id.as_str(), Some(("w1", "p1"))),
            &mut unused,
        )
        .unwrap()
        .unwrap();
        assert_eq!(id, again);
        let stale = persist(&conn, &request(None, Some(("w1", "gone"))), &mut unused).unwrap_err();
        assert_eq!(stale.code, "stale_context");
        let stored: String = conn
            .query_row(
                "SELECT provenance_json FROM ai_chat_messages WHERE role = 'assistant' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, "{\"notices\": []}");
        set_history_mode(&conn, "off").unwrap();
        assert_eq!(
            persist(
                &conn,
                &request(id.as_str(), Some(("w1", "p1"))),
                &mut unused
            )
            .unwrap(),
            None
        );
    }
}
