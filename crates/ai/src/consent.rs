//! `AiToolConsentState` (`kassiber/daemon.py:806`): one chat's consent queue.
//! The chat worker waits on a call id; `ai.tool_call.consent` records the
//! user's decision from the request loop. "Allow for this chat" is
//! remembered per tool, except for the once-only tools, where it is
//! downgraded to "allow once" and never remembered.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
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

#[derive(Default)]
struct State {
    decisions: HashMap<String, String>,
    pending: HashSet<String>,
    allow_session: HashSet<String>,
}

/// One chat's consent state.
#[derive(Default)]
pub struct ConsentState {
    state: Mutex<State>,
    changed: Condvar,
}

impl ConsentState {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
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

    /// `has_session_allow(tool_name)`.
    pub fn has_session_allow(&self, tool: &str) -> bool {
        !once_only(tool) && self.lock().allow_session.contains(tool)
    }

    /// `wait(call_id=..., tool_name=..., cancel_event=..., timeout=...)`:
    /// the decision, `"cancelled"`, or `"consent_timeout"`.
    pub fn wait(
        &self,
        call_id: &str,
        tool: &str,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> String {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        if state.allow_session.contains(tool) {
            return "allow_session".to_owned();
        }
        state.pending.insert(call_id.to_owned());
        let outcome = loop {
            if cancelled.load(Ordering::SeqCst) {
                break "cancelled".to_owned();
            }
            if let Some(decision) = state.decisions.remove(call_id) {
                if decision == "allow_session" {
                    if once_only(tool) {
                        break "allow_once".to_owned();
                    }
                    state.allow_session.insert(tool.to_owned());
                }
                break decision;
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
        state.decisions.remove(call_id);
        outcome
    }
}
