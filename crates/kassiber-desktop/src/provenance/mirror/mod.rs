//! Privacy Mirror (`/privacy-mirror`, routes/PrivacyMirror.tsx): the public-observer report of
//! the shared local analysis (`ui.reports.privacy_mirror`). It adds no detector, graph or score
//! of its own; every investigation opens the Chain analysis workbench on the finding's own
//! query, and the assistant receives only the daemon's projection.
//!
//! The read follows the web's `useDaemon` inside the page's `DaemonScopeContext`: it carries the
//! book's `expected_scope`, refetches on every mount (`refetchOnMount: "always"`) and on every
//! book refresh cycle (bookRefresh.ts refetches active queries), retries a failed read the way
//! `retryDaemonQuery` does before it shows an error, and never lets an answer for another book
//! or an older mount replace what the page shows.

pub mod scenes;
mod view;

use std::collections::{HashSet, VecDeque};

use rune::{KeyEvent, Tx};
use serde_json::{Value, json};

use crate::app::{App, Reply};
use crate::i18n::t;
use crate::provenance::chain::model::{Workspace, investigation_href};

pub use view::view;

pub const ROUTE: &str = "/privacy-mirror";

#[derive(Default)]
pub struct State {
    /// The book scope the page is mounted for (`None` while away from the route).
    pub mounted_for: Option<String>,
    /// The book scope the cached report and reads belong to (kept while away).
    pub book: Option<String>,
    pub payload: Option<Value>,
    /// The current read is in flight or waiting for its retry (`query.isFetching`).
    pub loading: bool,
    /// The read failed after its retries (`query.error`).
    pub error: Option<String>,
    pub action_error: Option<String>,
    /// The assistant projection is in flight (`projection.isPending`).
    pub asking: bool,
    /// The book the pending assistant question was asked from (`beginAssistantHandoff`).
    pub ask_scope: Option<String>,
    /// Open disclosures (`<details>`), keyed by section / `group-<id>` / `basis-<id>`.
    pub open: HashSet<String>,
    /// Bumped on every mount, book change and refetch: answers of older reads are dropped.
    pub epoch: u64,
    /// The epoch of each report read in flight, oldest first (the daemon answers in order).
    pub inflight: VecDeque<u64>,
    /// Failed attempts of the current read, and when the next retry goes out (`app.clock`).
    pub failures: u32,
    pub retry_at: Option<f64>,
    /// The book refresh cycle (`app.book_generation`) the payload belongs to.
    pub seen_generation: u64,
    /// An element to scroll into view on the next frame (a count tile's press, headless scenes).
    pub reveal: Option<String>,
    /// When the current read began (`app.clock`): the page counts the seconds while it waits.
    pub started: Option<f64>,
}

fn st(app: &mut App) -> &mut State {
    &mut app.provenance.mirror
}

fn scope_key(app: &App) -> String {
    app.expected_scope().to_string()
}

/// Starts a fresh read of the report (a mount, a refresh press, a refresh cycle). An older read
/// still in flight is superseded (`cancelRefetch`): its answer is dropped when it comes.
fn load(app: &mut App) {
    if app.locked {
        return;
    }
    let scope = app.expected_scope();
    let clock = app.clock;
    {
        let s = st(app);
        s.started = Some(clock);
        s.epoch += 1;
        s.failures = 0;
        s.retry_at = None;
    }
    send_read(app, scope);
}

fn send_read(app: &mut App, scope: Value) {
    let epoch = app.provenance.mirror.epoch;
    let args = json!({"expected_scope": scope});
    if app
        .send(
            "ui.reports.privacy_mirror",
            args,
            "provenance.mirror.report",
        )
        .is_some()
    {
        let s = st(app);
        s.inflight.push_back(epoch);
        s.loading = true;
    }
}

/// The refresh button (`query.refetch()`): disabled while the read is fetching.
fn refresh(app: &mut App) {
    if !app.provenance.mirror.loading {
        load(app);
    }
}

pub fn is_open(app: &App, k: &str) -> bool {
    app.provenance.mirror.open.contains(k)
}

pub fn toggle(app: &mut App, k: &str) {
    let o = &mut st(app).open;
    if !o.remove(k) {
        o.insert(k.to_owned());
    }
}

/// `retryDaemonQuery` with react-query's default `retryDelay`: an auth prompt never retries; a
/// daemon-flagged retryable error retries up to 5 times, any other error 3 times; the delays
/// double from 1 s up to 30 s. `failures` counts the attempts that failed so far (≥ 1).
pub fn retry_delay(failures: u32, error: &Value) -> Option<f64> {
    if error.get("code").and_then(Value::as_str) == Some("auth_required") {
        return None;
    }
    let max = if error.get("retryable").and_then(Value::as_bool) == Some(true) {
        5
    } else {
        3
    };
    if failures > max {
        return None;
    }
    Some((2f64.powi(failures as i32 - 1)).min(30.0))
}

/// `privacyHeadline`.
pub fn headline(summary: &Value, findings: &[Value]) -> &'static str {
    if findings.iter().any(personal) {
        return "personal_findings";
    }
    let n = |k: &str| summary.get(k).and_then(Value::as_u64).unwrap_or(0);
    if n("local_transaction_count") == 0 {
        return "no_local_evidence";
    }
    if summary.get("status").and_then(Value::as_str) == Some("unavailable") {
        return if n("owned_output_count") == 0 {
            "no_owned_outputs"
        } else {
            "unavailable"
        };
    }
    if n("owned_output_count") == 0 {
        return "no_owned_outputs";
    }
    if n("finding_count") > 0 {
        "context_only"
    } else {
        "nothing_found"
    }
}

/// `PERSONAL_RELEVANCE.has(finding.relevance)`.
pub fn personal(f: &Value) -> bool {
    matches!(
        f.get("relevance").and_then(Value::as_str),
        Some("own_spend" | "owned_output")
    )
}

/// `groupPrivacyFindings`: presentation groups by code, relevance and authority, in first-seen
/// order; evidence and counts stay per finding.
pub fn group(findings: &[Value]) -> Vec<Vec<Value>> {
    let mut groups: Vec<(String, Vec<Value>)> = vec![];
    for f in findings {
        let key = json!([f.get("code"), f.get("relevance"), f.get("authority")]).to_string();
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, g)) => g.push(f.clone()),
            None => groups.push((key, vec![f.clone()])),
        }
    }
    groups.into_iter().map(|(_, g)| g).collect()
}

/// The workbench tab a finding's investigation opens on.
pub fn finding_tab(category: &str) -> &'static str {
    match category {
        "attribution" => "exposure",
        "pattern" => "patterns",
        _ => "findings",
    }
}

/// Opens the workbench on an investigation (`analysisInvestigationSearch`).
fn investigate(app: &mut App, investigation: &Value, workspace: Workspace, tab: &str) {
    let href = investigation_href(
        investigation.get("query").unwrap_or(&Value::Null),
        workspace,
        tab,
    );
    crate::shell::go(app, &href);
}

/// `useChainAnalysisAssistant().ask`: an investigation goes through the daemon's projection
/// (`ui.chain_analysis.ai_context`, scoped to this book); the page's own question is the fixed
/// overview prompt. Only the projected identities ever reach the prompt.
fn ask(app: &mut App, investigation: Option<&Value>) {
    if app.provenance.mirror.asking || app.assistant.streaming || !app.prefs.ai_features {
        return;
    }
    st(app).action_error = None;
    match investigation {
        Some(inv) => {
            let scope = app.expected_scope();
            let args = json!({
                "query": inv.get("query").cloned().unwrap_or(Value::Null),
                "expected_snapshot_id": inv.get("snapshot_id").cloned().unwrap_or(Value::Null),
                "expected_scope": scope,
            });
            let key = scope.to_string();
            if app
                .send(
                    "ui.chain_analysis.ai_context",
                    args,
                    "provenance.mirror.ai_context",
                )
                .is_some()
            {
                let s = st(app);
                s.asking = true;
                s.ask_scope = Some(key);
            }
        }
        None => crate::assistant::hand_off(app, &t("privacyMirror:assistantPrompt")),
    }
}

/// Mounts on arrival (`refetchOnMount: "always"`), on a book change, and refetches on every
/// book refresh cycle; fires a pending retry.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    if app.route.path() != ROUTE {
        if app.provenance.mirror.mounted_for.is_some() {
            // Leaving keeps the cached report (the query cache), but a return reads again; the
            // page's own state (disclosures, the action error, a pending retry) goes with it.
            let s = st(app);
            s.mounted_for = None;
            s.action_error = None;
            s.open.clear();
            if s.retry_at.take().is_some() {
                s.loading = false;
            }
        }
        return false;
    }
    if let Some(k) = st(app).reveal.take() {
        tx.scroll_into_view(k);
    }
    if app.read("ui.profiles.snapshot").is_none() {
        return false;
    }
    let scope = scope_key(app);
    let generation = app.book_generation;
    let s = &app.provenance.mirror;
    if s.mounted_for.as_deref() != Some(scope.as_str()) {
        let same_book = s.book.as_deref() == Some(scope.as_str());
        let s = st(app);
        if !same_book {
            // Another book: nothing of the old one stays (the web remounts on a new key), and
            // its reads still in flight are dropped when they answer. A question still out
            // keeps its book, so its answer is dropped too rather than taken for a new one's.
            let (epoch, inflight) = (s.epoch + 1, std::mem::take(&mut s.inflight));
            let (asking, ask_scope) = (s.asking, s.ask_scope.take());
            *s = State {
                epoch,
                inflight,
                asking,
                ask_scope,
                ..State::default()
            };
        }
        let reuse =
            same_book && s.loading && s.retry_at.is_none() && s.seen_generation == generation;
        s.mounted_for = Some(scope.clone());
        s.book = Some(scope);
        s.seen_generation = generation;
        s.action_error = None;
        if !reuse {
            // A read of this book already in flight is shared, as react-query dedupes a mount.
            load(app);
        }
        return true;
    }
    if s.seen_generation != generation {
        // A book refresh cycle (a sync, a journal run, a write): re-read, keeping the report up.
        st(app).seen_generation = generation;
        load(app);
        return true;
    }
    if let Some(at) = s.retry_at {
        if app.clock >= at {
            st(app).retry_at = None;
            let scope = app.expected_scope();
            send_read(app, scope);
            return true;
        }
        tx.wake_after(std::time::Duration::from_secs_f64(
            (at - app.clock).max(0.05),
        ));
    }
    if s.loading {
        // The header counts the seconds of a read in flight.
        tx.wake_after(std::time::Duration::from_secs(1));
    }
    false
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "provenance.mirror.report" => {
            let clock = app.clock;
            let s = st(app);
            let Some(epoch) = s.inflight.pop_front() else {
                return;
            };
            if epoch != s.epoch {
                return;
            }
            match &r.error {
                Some(e) => {
                    s.failures += 1;
                    match retry_delay(s.failures, &r.error_detail) {
                        Some(delay) if s.mounted_for.is_some() => s.retry_at = Some(clock + delay),
                        _ => {
                            let s = st(app);
                            s.loading = false;
                            s.failures = 0;
                            s.error = Some(e.clone());
                        }
                    }
                }
                None => {
                    s.loading = false;
                    s.failures = 0;
                    s.error = None;
                    s.payload = Some(r.data.clone()).filter(|v| v.is_object());
                }
            }
        }
        "provenance.mirror.ai_context" => {
            let asked_from = st(app).ask_scope.take();
            st(app).asking = false;
            // A book opened meanwhile never receives the question (`boundary.isCurrent()`).
            if asked_from.as_deref() != Some(scope_key(app).as_str())
                || app.provenance.mirror.mounted_for.is_none()
            {
                return;
            }
            match &r.error {
                Some(e) => st(app).action_error = Some(e.clone()),
                None if r.data.is_object() => {
                    let prompt = crate::provenance::chain::handoff_prompt(&r.data);
                    crate::assistant::hand_off(app, &prompt);
                }
                None => {}
            }
        }
        _ => {}
    }
}

/// The page has no shortcuts of its own; `<details>` toggles with Enter/Space as buttons do.
pub fn on_key(_app: &mut App, _ev: &KeyEvent, _tx: &mut Tx) -> bool {
    false
}

/// Headless runs: mount and wait for the report (up to 30 s; the shared analysis is heavy).
pub fn prime(app: &mut App) {
    let mut tx = Tx::default();
    poll(app, &mut tx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        app.pump();
        let busy = app
            .pending
            .values()
            .any(|(tag, _, _)| tag.starts_with("provenance.mirror."));
        if !busy || std::time::Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An assistant question is out when another book opens: it stays the old book's, so a new
    /// question waits for it and its answer is never handed off as the new book's.
    #[test]
    fn a_question_from_the_old_book_stays_the_old_books() {
        let mut app = crate::app::tests::quiet_app();
        app.route = crate::app::Route(ROUTE.into());
        app.prefs.ai_features = true;
        let reply = |data: Value| Reply {
            kind: "ui.profiles.snapshot".into(),
            data,
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        let book = |app: &mut App, p: &str| {
            let snapshot = reply(json!({ "activeWorkspaceId": "w", "activeProfileId": p }));
            app.reads.insert("ui.profiles.snapshot".into(), snapshot);
            scope_key(app)
        };
        let a = book(&mut app, "a");
        let s = st(&mut app);
        s.mounted_for = Some(a.clone());
        s.book = Some(a.clone());
        s.asking = true;
        s.ask_scope = Some(a.clone());
        book(&mut app, "b");
        poll(&mut app, &mut Tx::default());
        ask(&mut app, Some(&json!({ "query": {} })));
        assert_eq!(app.provenance.mirror.ask_scope.as_deref(), Some(a.as_str()));
        on_reply(
            &mut app,
            "provenance.mirror.ai_context",
            &reply(json!({ "prompt": "a" })),
        );
        assert!(!app.provenance.mirror.asking);
        assert!(app.provenance.mirror.ask_scope.is_none());
    }

    #[test]
    fn headline_follows_personal_relevance_then_population() {
        let s = |status: &str, owned: u64, local: u64, findings: u64| json!({"status": status, "owned_output_count": owned, "local_transaction_count": local, "finding_count": findings});
        let own = vec![json!({"relevance": "owned_output"})];
        let ctx = vec![json!({"relevance": "nearby_context"})];
        assert_eq!(headline(&s("findings", 1, 1, 1), &own), "personal_findings");
        assert_eq!(headline(&s("findings", 0, 0, 1), &ctx), "no_local_evidence");
        assert_eq!(
            headline(&s("unavailable", 0, 3, 0), &ctx),
            "no_owned_outputs"
        );
        assert_eq!(headline(&s("unavailable", 2, 3, 0), &[]), "unavailable");
        assert_eq!(headline(&s("findings", 2, 3, 1), &ctx), "context_only");
        assert_eq!(
            headline(&s("no_observed_exposure", 2, 3, 0), &[]),
            "nothing_found"
        );
    }

    /// privacyMirror.test.ts: an unavailable snapshot with surrounding findings stays unknown.
    #[test]
    fn surrounding_findings_never_upgrade_an_unassessed_snapshot() {
        let ctx = vec![json!({"relevance": "received_context"})];
        let s = json!({"status": "unavailable", "owned_output_count": 4, "local_transaction_count": 9, "finding_count": 1});
        assert_eq!(headline(&s, &ctx), "unavailable");
        let own_spend = vec![json!({"relevance": "own_spend"})];
        assert_eq!(headline(&s, &own_spend), "personal_findings");
    }

    #[test]
    fn groups_fold_only_identical_code_relevance_authority() {
        let f = |id: &str, code: &str, rel: &str, auth: &str| json!({"id": id, "code": code, "relevance": rel, "authority": auth});
        let g = group(&[
            f("a", "script_reuse", "owned_output", "observed"),
            f("b", "script_reuse", "owned_output", "observed"),
            f("c", "script_reuse", "nearby_context", "observed"),
            f("d", "script_reuse", "owned_output", "hypothesis"),
        ]);
        assert_eq!(g.len(), 3);
        assert_eq!(
            g[0].iter()
                .map(|f| f["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(g[1][0]["id"], "c");
        assert_eq!(g[2][0]["id"], "d");
    }

    #[test]
    fn finding_tabs_follow_the_category() {
        assert_eq!(finding_tab("attribution"), "exposure");
        assert_eq!(finding_tab("pattern"), "patterns");
        assert_eq!(finding_tab("linkage"), "findings");
        assert_eq!(finding_tab("structure"), "findings");
    }

    #[test]
    fn retries_follow_retry_daemon_query() {
        let plain = json!({"code": "internal_error"});
        let busy = json!({"code": "database_busy", "retryable": true});
        let auth = json!({"code": "auth_required"});
        assert_eq!(retry_delay(1, &plain), Some(1.0));
        assert_eq!(retry_delay(2, &plain), Some(2.0));
        assert_eq!(retry_delay(3, &plain), Some(4.0));
        assert_eq!(retry_delay(4, &plain), None);
        assert_eq!(retry_delay(5, &busy), Some(16.0));
        assert_eq!(retry_delay(6, &busy), None);
        assert_eq!(retry_delay(1, &auth), None);
    }
}
