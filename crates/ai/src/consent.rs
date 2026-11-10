//! `AiToolConsentState` (`kassiber/daemon.py`): one chat's consent queue and
//! permission mode. The chat worker waits on a call id; `ai.tool_call.consent`
//! records the user's decision and `ai.chat.permission_mode` switches the
//! mode from the request loop, releasing every waiting call the new mode
//! approves. "Allow for this chat" is remembered per tool for the whole chat
//! session, except for the once-only tools, where it is downgraded to "allow
//! once" and never remembered.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// `AI_TOOL_CONSENT_TIMEOUT_SECONDS`.
pub const CONSENT_TIMEOUT: Duration = Duration::from_secs(300);

/// The longest single wait before cancellation is checked again.
const POLL: Duration = Duration::from_millis(250);

/// `AI_TOOL_ONCE_ONLY_CONSENT`: tools whose "allow for this chat" means
/// "allow once".
pub const ONCE_ONLY_CONSENT: [&str; 13] = [
    "ui.chain_analysis.acquire.apply",
    "ui.chain_analysis.datasets.import",
    "ui.chain_analysis.datasets.import.start",
    "ui.chain_analysis.sources.authorize",
    "ui.chain_analysis.sources.run",
    "ui.custody.review.apply",
    // Each import and each rollback is its own plan-bound decision.
    "ui.imports.rollback",
    "ui.journals.quarantine.resolve",
    // Changes standing egress policy: one session grant must not keep
    // re-flipping it.
    "ui.maintenance.configure",
    "ui.review.apply",
    "ui.transfers.components.apply",
    "ui.wallets.import_attachment.apply",
    "ui.wallets.import_document.apply",
];

/// Whether a session grant for `tool` is never remembered.
pub fn once_only(tool: &str) -> bool {
    ONCE_ONLY_CONSENT.contains(&tool)
}

/// `AI_PERMISSION_MODES`: who approves a consented call. `approval` asks
/// every time; `auto` approves a reversible tool that stays on this machine;
/// `full_access` approves everything. An absent mode is `approval`.
pub const PERMISSION_MODES: [&str; 3] = ["approval", "auto", "full_access"];

/// `AI_SESSION_ALLOWS_MAX`: chat sessions whose "allow for this chat" the
/// registry keeps in memory.
pub const SESSION_ALLOWS_MAX: usize = 256;

/// The tools a chat session allowed, shared by its turns.
pub type SessionAllows = Arc<Mutex<HashSet<String>>>;

/// `_AI_MODE_DECISIONS`: the decision a mode records instead of a click.
fn mode_decision(mode: &str, reversible: bool, egresses: bool) -> Option<&'static str> {
    match mode {
        "full_access" => Some("mode_full_access"),
        "auto" if reversible && !egresses => Some("mode_auto"),
        _ => None,
    }
}

struct State {
    decisions: HashMap<String, String>,
    pending: HashSet<String>,
    permission_mode: String,
    /// call id → (reversible, egresses), in arrival order, for calls
    /// waiting on a decision.
    pending_risk: Vec<(String, (bool, bool))>,
}

/// One chat's consent state, with its permission mode.
pub struct ConsentState {
    state: Mutex<State>,
    /// "Allow for this chat": shared with the registry's entry for the chat
    /// session, so it outlives this one answer.
    allow_session: SessionAllows,
    changed: Condvar,
}

impl Default for ConsentState {
    fn default() -> Self {
        Self::new(SessionAllows::default())
    }
}

impl ConsentState {
    /// `AiToolConsentState(allow_session=...)`, in the `approval` mode.
    pub fn new(allow_session: SessionAllows) -> Self {
        ConsentState {
            state: Mutex::new(State {
                decisions: HashMap::new(),
                pending: HashSet::new(),
                permission_mode: "approval".into(),
                pending_risk: Vec::new(),
            }),
            allow_session,
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn allows(&self) -> MutexGuard<'_, HashSet<String>> {
        self.allow_session
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// The current permission mode.
    pub fn permission_mode(&self) -> String {
        self.lock().permission_mode.clone()
    }

    /// `preapproval(tool_name, reversible=..., egresses=...)`: the decision
    /// a call gets without asking, or `None` to ask.
    pub fn preapproval(
        &self,
        tool: &str,
        reversible: bool,
        egresses: bool,
    ) -> Option<&'static str> {
        let state = self.lock();
        if let Some(decision) = mode_decision(&state.permission_mode, reversible, egresses) {
            return Some(decision);
        }
        (!once_only(tool) && self.allows().contains(tool)).then_some("allow_session")
    }

    /// `set_permission_mode(mode)`: switch modes and return the waiting
    /// calls the new mode releases.
    pub fn set_permission_mode(&self, mode: &str) -> Vec<String> {
        let mut state = self.lock();
        state.permission_mode = mode.to_owned();
        let released = state
            .pending_risk
            .iter()
            .filter(|(call_id, (reversible, egresses))| {
                !state.decisions.contains_key(call_id)
                    && mode_decision(mode, *reversible, *egresses).is_some()
            })
            .map(|(call_id, _)| call_id.clone())
            .collect();
        self.changed.notify_all();
        released
    }

    /// How many calls wait on a decision (for tests).
    #[doc(hidden)]
    pub fn waiting(&self) -> usize {
        self.lock().pending_risk.len()
    }

    /// `expect(call_id)`: register a call before its consent record is
    /// written, so a fast decision is not lost.
    pub fn expect(&self, call_id: &str) {
        self.lock().pending.insert(call_id.to_owned());
    }

    /// `record(call_id, decision)`: `false` when nobody waits on the call.
    pub fn record(&self, call_id: &str, decision: &str) -> bool {
        let mut state = self.lock();
        if !state.pending.contains(call_id) {
            return false;
        }
        state
            .decisions
            .insert(call_id.to_owned(), decision.to_owned());
        self.changed.notify_all();
        true
    }

    /// `notify_cancelled()`: wake waiters so they see the cancel flag.
    pub fn notify_cancelled(&self) {
        let _state = self.lock();
        self.changed.notify_all();
    }

    /// `session_allows`: the tools allowed for the whole chat.
    pub fn session_allows(&self) -> HashSet<String> {
        self.allows().clone()
    }

    /// `has_session_allow(tool_name)`.
    pub fn has_session_allow(&self, tool: &str) -> bool {
        !once_only(tool) && self.allows().contains(tool)
    }

    /// `wait(call_id=..., tool_name=..., cancel_event=..., timeout=...,
    /// reversible=..., egresses=...)`: the decision, a mode's decision when
    /// the mode switches while the call waits, `"cancelled"`, or
    /// `"consent_timeout"`.
    pub fn wait(
        &self,
        call_id: &str,
        tool: &str,
        cancelled: &AtomicBool,
        timeout: Duration,
        reversible: bool,
        egresses: bool,
    ) -> String {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        if self.allows().contains(tool) {
            return "allow_session".to_owned();
        }
        state.pending.insert(call_id.to_owned());
        state
            .pending_risk
            .push((call_id.to_owned(), (reversible, egresses)));
        let outcome = loop {
            if cancelled.load(Ordering::SeqCst) {
                break "cancelled".to_owned();
            }
            if let Some(decision) = state.decisions.remove(call_id) {
                if decision == "allow_session" {
                    if once_only(tool) {
                        break "allow_once".to_owned();
                    }
                    self.allows().insert(tool.to_owned());
                }
                break decision;
            }
            if let Some(decision) = mode_decision(&state.permission_mode, reversible, egresses) {
                break decision.to_owned();
            }
            let now = Instant::now();
            if now >= deadline {
                break "consent_timeout".to_owned();
            }
            let wait = POLL.min(deadline - now);
            state = self
                .changed
                .wait_timeout(state, wait)
                .unwrap_or_else(|poison| poison.into_inner())
                .0;
        };
        state.pending.remove(call_id);
        state.pending_risk.retain(|(id, _)| id != call_id);
        state.decisions.remove(call_id);
        outcome
    }
}
