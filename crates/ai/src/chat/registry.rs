//! `ActiveAiChats` (`kassiber/daemon.py:878`): the daemon's RAM-only
//! registry of running chats, their cancel handles and consent queues, early
//! cancels, and provider session cursors.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use kassiber_envelope::py_str;
use kassiber_pyjson::Value;

use crate::consent::ConsentState;

/// `PENDING_AI_CANCEL_TTL_SECONDS`: how long a cancel that arrived before
/// its chat is held.
pub const PENDING_CANCEL_TTL: Duration = Duration::from_secs(30);

/// `MAX_PENDING_AI_CANCELS`.
pub const MAX_PENDING_CANCELS: usize = 128;

/// Stops a turn's provider client from another thread: closes the socket
/// or ends the broker, and revokes the turn's network consent.
pub type CancelHandler = Arc<dyn Fn() + Send + Sync>;

/// `ActiveAiChat`: one running chat.
pub struct ActiveChat {
    pub cancelled: AtomicBool,
    pub consent: ConsentState,
    handler: Mutex<Option<CancelHandler>>,
}

impl ActiveChat {
    fn new() -> Self {
        ActiveChat {
            cancelled: AtomicBool::new(false),
            consent: ConsentState::default(),
            handler: Mutex::new(None),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
struct State {
    chats: HashMap<String, Arc<ActiveChat>>,
    pending: HashMap<String, Instant>,
    /// `(chat session id, provider name)` → `(cursor, history fingerprint)`.
    provider_sessions: HashMap<(String, String), (String, String)>,
}

/// The registry. One per daemon; clones share it.
#[derive(Clone, Default)]
pub struct ActiveChats {
    state: Arc<Mutex<State>>,
}

/// `_request_id_registry_key(request_id)`: `None` for a missing or null id
/// and the empty string; `str(request_id)` otherwise.
pub fn registry_key(request_id: Option<&Value>) -> Option<String> {
    match request_id {
        None | Some(Value::Null) => None,
        Some(Value::Str(text)) if text.code_points().next().is_none() => None,
        Some(value) => Some(py_str(value)),
    }
}

impl ActiveChats {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// `register(request_id)`: a held cancel for this key starts the chat
    /// cancelled.
    pub fn register(&self, key: Option<&str>) -> Arc<ActiveChat> {
        let chat = Arc::new(ActiveChat::new());
        let Some(key) = key else {
            return chat;
        };
        let now = Instant::now();
        let mut state = self.lock();
        prune(&mut state, now);
        if let Some(deadline) = state.pending.remove(key) {
            if deadline >= now {
                chat.cancelled.store(true, Ordering::SeqCst);
            }
        }
        state.chats.insert(key.to_owned(), Arc::clone(&chat));
        chat
    }

    /// `unregister(key, chat)`: only this chat, not a later one under the
    /// same key.
    pub fn unregister(&self, key: Option<&str>, chat: &Arc<ActiveChat>) {
        let Some(key) = key else {
            return;
        };
        let mut state = self.lock();
        if state
            .chats
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(current, chat))
        {
            state.chats.remove(key);
        }
    }

    /// `cancel(target_request_id)`: `(cancelled, queued)`.
    pub fn cancel(&self, target: &str) -> (bool, bool) {
        let now = Instant::now();
        let mut handler = None;
        let chat = {
            let mut state = self.lock();
            prune(&mut state, now);
            match state.chats.get(target).cloned() {
                Some(chat) => {
                    chat.cancelled.store(true, Ordering::SeqCst);
                    chat.consent.notify_cancelled();
                    handler = chat
                        .handler
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone();
                    Some(chat)
                }
                None => {
                    state
                        .pending
                        .insert(target.to_owned(), now + PENDING_CANCEL_TTL);
                    trim(&mut state);
                    None
                }
            }
        };
        if let Some(handler) = handler {
            handler();
        }
        if chat.is_some() {
            (true, false)
        } else {
            (false, true)
        }
    }

    /// `set_cancel_handler(chat, handler)`: a chat cancelled before its
    /// client existed is stopped as soon as the handler arrives.
    pub fn set_cancel_handler(&self, chat: &ActiveChat, handler: Option<CancelHandler>) {
        let already = {
            let _state = self.lock();
            *chat.handler.lock().unwrap_or_else(|p| p.into_inner()) = handler.clone();
            chat.is_cancelled()
        };
        if already {
            if let Some(handler) = handler {
                handler();
            }
        }
    }

    /// `provider_session(...)`: the cursor recorded for this chat session
    /// and provider, only while the visible history is the one it saw. A
    /// request with no resumable branch (`None`) resumes nothing.
    pub fn provider_session(
        &self,
        chat_session_id: Option<&str>,
        provider: &str,
        fingerprint: Option<&str>,
    ) -> Option<String> {
        let chat_session_id = chat_session_id.filter(|id| !id.is_empty())?;
        let fingerprint = fingerprint?;
        let state = self.lock();
        let (cursor, recorded) = state
            .provider_sessions
            .get(&(chat_session_id.to_owned(), provider.to_owned()))?;
        (recorded.as_str() == fingerprint).then(|| cursor.clone())
    }

    /// `remember_provider_session(...)`.
    pub fn remember_provider_session(
        &self,
        chat_session_id: Option<&str>,
        provider: &str,
        cursor: Option<&str>,
        fingerprint: &str,
    ) {
        let (Some(chat_session_id), Some(cursor)) = (
            chat_session_id.filter(|id| !id.is_empty()),
            cursor.filter(|cursor| !cursor.is_empty()),
        ) else {
            return;
        };
        self.lock().provider_sessions.insert(
            (chat_session_id.to_owned(), provider.to_owned()),
            (cursor.to_owned(), fingerprint.to_owned()),
        );
    }

    /// `record_consent(target_request_id, call_id, decision)`.
    pub fn record_consent(&self, target: &str, call_id: &str, decision: &str) -> bool {
        let chat = self.lock().chats.get(target).cloned();
        chat.is_some_and(|chat| chat.consent.record(call_id, decision))
    }

    /// Whether any chat is registered (the daemon polls its task queue
    /// while one is).
    pub fn is_empty(&self) -> bool {
        self.lock().chats.is_empty()
    }
}

fn prune(state: &mut State, now: Instant) {
    state.pending.retain(|_, deadline| *deadline > now);
}

/// `_trim_pending_locked`: drop the earliest deadlines past the cap.
fn trim(state: &mut State) {
    while state.pending.len() > MAX_PENDING_CANCELS {
        let Some(oldest) = state
            .pending
            .iter()
            .min_by_key(|(_, deadline)| **deadline)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        state.pending.remove(&oldest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn an_early_cancel_is_held_and_starts_the_chat_cancelled() {
        let chats = ActiveChats::default();
        assert_eq!(chats.cancel("chat-early"), (false, true));
        let chat = chats.register(Some("chat-early"));
        assert!(chat.is_cancelled());
        // Held once: a second chat under the key starts normally.
        let again = chats.register(Some("chat-early"));
        assert!(!again.is_cancelled());
        // A chat without a key is never registered.
        let anonymous = chats.register(None);
        assert!(!anonymous.is_cancelled());
        assert_eq!(chats.cancel("other"), (false, true));
    }

    #[test]
    fn held_cancels_are_capped_at_128() {
        let chats = ActiveChats::default();
        for index in 0..200 {
            chats.cancel(&format!("chat-{index}"));
        }
        assert_eq!(chats.lock().pending.len(), MAX_PENDING_CANCELS);
        // The earliest went first.
        assert!(!chats.register(Some("chat-0")).is_cancelled());
        assert!(chats.register(Some("chat-199")).is_cancelled());
    }

    #[test]
    fn cancel_runs_the_handler_and_late_handlers_run_at_once() {
        let chats = ActiveChats::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let chat = chats.register(Some("chat-1"));
        let counter = Arc::clone(&calls);
        chats.set_cancel_handler(
            &chat,
            Some(Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            })),
        );
        assert_eq!(chats.cancel("chat-1"), (true, false));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let counter = Arc::clone(&calls);
        chats.set_cancel_handler(
            &chat,
            Some(Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            })),
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        chats.unregister(Some("chat-1"), &chat);
        assert!(chats.is_empty());
    }

    #[test]
    fn provider_resume_cursor_is_bound_to_the_visible_chat_branch() {
        let chats = ActiveChats::default();
        chats.remember_provider_session(Some("s1"), "codex", Some("cursor-1"), "fp-a");
        assert_eq!(
            chats
                .provider_session(Some("s1"), "codex", Some("fp-a"))
                .as_deref(),
            Some("cursor-1")
        );
        assert_eq!(chats.provider_session(Some("s1"), "codex", Some("fp-b")), None);
        assert_eq!(chats.provider_session(Some("s1"), "claude", Some("fp-a")), None);
        assert_eq!(chats.provider_session(None, "codex", Some("fp-a")), None);
        assert_eq!(chats.provider_session(Some("s1"), "codex", None), None);
        chats.remember_provider_session(None, "codex", Some("cursor-2"), "fp-a");
        chats.remember_provider_session(Some("s1"), "codex", None, "fp-c");
        assert_eq!(
            chats
                .provider_session(Some("s1"), "codex", Some("fp-a"))
                .as_deref(),
            Some("cursor-1")
        );
    }

    #[test]
    fn registry_keys_follow_python_str() {
        assert_eq!(registry_key(None), None);
        assert_eq!(registry_key(Some(&Value::Null)), None);
        assert_eq!(registry_key(Some(&crate::pyval::s(""))), None);
        assert_eq!(
            registry_key(Some(&crate::pyval::s("chat-1"))).as_deref(),
            Some("chat-1")
        );
        assert_eq!(
            registry_key(Some(&Value::Int(7.into()))).as_deref(),
            Some("7")
        );
        assert_eq!(
            registry_key(Some(&Value::Bool(true))).as_deref(),
            Some("True")
        );
    }
}
