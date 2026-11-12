//! `chats list|show|delete|clear|config` (R6-7): persisted chat sessions on
//! the open book (`list_chat_sessions_cli` and its neighbours in
//! `kassiber/cli/handlers.py`). Registered behind the chat gate.

use kassiber_ai::chat::store::Op;
use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::{PyStr, Value};
use kassiber_runtime::store::Book;

use crate::argparse::Namespace;

pub const COMMANDS: &[&str] = &[
    "chats.list",
    "chats.show",
    "chats.delete",
    "chats.clear",
    "chats.config",
];

pub fn execute(
    book: &mut dyn Book,
    command: &str,
    args: &Namespace,
    database_encrypted: &dyn Fn() -> Result<bool, AppError>,
) -> Result<Value, AppError> {
    if command == "chats.config" {
        let encrypted = database_encrypted()?;
        return book.ai_chat(Op::Configure {
            history: args.text("history"),
            encrypted,
        });
    }
    let (_, profile) = book.resolve_scope(args.get("workspace"), args.get("profile"))?;
    let session_id = || PyStr::from(args.text("session_id").unwrap_or_default());
    match command {
        "chats.list" => {
            let limit = match args.get("limit") {
                Some(Value::Int(limit)) => i64::try_from(limit).unwrap_or(i64::MAX),
                _ => 50,
            };
            let sessions = book.ai_chat(Op::ListSessions {
                profile_id: &profile,
                limit,
            })?;
            let mode = book.ai_chat(Op::HistoryMode)?;
            Ok(dict([("sessions", sessions), ("history_mode", mode)]))
        }
        "chats.show" => book.ai_chat(Op::GetSession {
            profile_id: &profile,
            session_id: &session_id(),
            include_messages: true,
        }),
        "chats.delete" => book.ai_chat(Op::DeleteSession {
            profile_id: &profile,
            session_id: &session_id(),
        }),
        _ => book.ai_chat(Op::ClearSessions {
            profile_id: &profile,
        }),
    }
}
