//! What the Review screen reads and decides: the attention pages (`useDaemonInfinite`), the
//! waiting page, the transfer list, the journal refresh after decisions
//! (`useJournalRefreshAfterDecisions`), each decision and its undo, and the reviewed steps
//! (`ui.review.cases` → `ui.review.plan` → `ui.review.apply`) behind every dialog.

use std::collections::{HashMap, HashSet, VecDeque};

use rune::{TextEdit, Tx};
use serde_json::{Value, json};

use super::model::{self, Item, Snap, s};
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tc, tr};

/// The daemon serves at most this many rows per request.
pub const PAGE: usize = 100;
/// Decisions settle this long before the journals recalculate (`SETTLE_MS`).
const SETTLE: f64 = 0.8;
/// How long a decision can be undone from the list.
const UNDO_SECS: f64 = 20.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AttKind {
    First,
    More,
    Refetch,
}

/// Who waits for a refetch of the attention pages.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Waiter {
    Fix,
    /// The detail panel's "Save & next" (`review.sheet_next` holds what it needs).
    SheetNext,
}

struct Refetch {
    /// Pages to re-read (as many as were loaded).
    target: usize,
    fresh: Vec<Value>,
    failed: bool,
    att_done: bool,
    waiting_done: bool,
    waiting_failed: bool,
    waiters: Vec<Waiter>,
}

/// The quarantine pages the screen shows.
#[derive(Default)]
pub struct Quarantine {
    pub pages: Vec<Value>,
    pub loading: bool,
    pub error: Option<String>,
    pub more_loading: bool,
    /// Set when a later page failed; "" when there is no message.
    pub more_error: Option<String>,
    queue: VecDeque<(u64, AttKind)>,
    generation: u64,
    refetch: Option<Refetch>,
    pub waiting_shown: bool,
    pub offset: usize,
    pub waiting: Option<Value>,
    pub waiting_loading: bool,
    /// Set when the waiting page failed; "" when there is no message.
    pub waiting_error: Option<String>,
    wqueue: VecDeque<(u64, bool)>,
    wgen: u64,
}

impl Quarantine {
    /// The loaded attention pages as one snapshot.
    pub fn attention(&self) -> Option<Snap> {
        Snap::merge(&self.pages)
    }
    pub fn has_more(&self) -> bool {
        self.pages.last().and_then(model::next_offset).is_some()
    }
    pub fn waiting_snap(&self) -> Option<Snap> {
        if self.waiting_shown {
            self.waiting.as_ref().map(Snap::parse)
        } else {
            None
        }
    }
}

/// One cause card's own state (folding, typed prices, ticked pairs).
#[derive(Default)]
pub struct Card {
    pub expanded: bool,
    pub entering: bool,
    pub entries: HashMap<String, TextEdit>,
    pub rate: bool,
    pub checked: bool,
    /// Ticked pairs, as they read when ticked, in tick order.
    pub picked: Vec<Item>,
}

impl Card {
    pub fn typed(&self) -> HashMap<String, String> {
        self.entries
            .iter()
            .map(|(k, v)| (k.clone(), v.text.clone()))
            .collect()
    }
}

#[derive(Clone, Debug)]
pub enum Decision {
    Pair { id: String, keys: Vec<String> },
    Dismiss { id: String, keys: Vec<String> },
}

impl Decision {
    fn keys(&self) -> &[String] {
        match self {
            Decision::Pair { keys, .. } | Decision::Dismiss { keys, .. } => keys,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Undo {
    pub decisions: Vec<Decision>,
    pub label: String,
    pub deadline: f64,
}

struct Pending {
    pair: bool,
    keys: Vec<String>,
    label: String,
}

struct UndoRun {
    current: Undo,
    undone: Vec<Decision>,
}

/// The transfer list (`ReviewTransfers`).
#[derive(Default)]
pub struct Transfers {
    pub live: Option<Vec<Value>>,
    pub loading: bool,
    pub failed: bool,
    pub frozen: Option<Vec<Value>>,
    pub hidden: HashSet<String>,
    decided: HashMap<String, Value>,
    pub restored: Vec<Value>,
    pub expanded: bool,
    pub saving: usize,
    pub undo: Option<Undo>,
    pub error: Option<String>,
    run: u64,
    was_refreshing: bool,
    /// Suggest reads in flight: the run a post-recalculation read belongs to.
    sq: VecDeque<Option<u64>>,
    pending: VecDeque<Pending>,
    undo_run: Option<UndoRun>,
    /// Suggestions ticked to set aside together, by key.
    pub selected: HashSet<String>,
    /// "Set N aside": the ticked suggestions still to send, one at a time.
    dismiss_run: Option<DismissRun>,
    /// After a decision: the row whose place focus goes to (`focusRow`).
    focus_row: Option<usize>,
}

/// `dismissSelected` in flight: what is left to send, the keys hidden for it, and what was set
/// aside so far.
struct DismissRun {
    queue: VecDeque<Value>,
    keys: Vec<String>,
    sent: usize,
    done: Vec<Decision>,
}

impl Transfers {
    /// Each candidate once: an undone one first, then the list as read, less the decided.
    pub fn candidates(&self) -> Vec<Value> {
        let mut seen = HashSet::new();
        let mut out = vec![];
        let base = self.frozen.as_ref().or(self.live.as_ref());
        for c in self.restored.iter().chain(base.into_iter().flatten()) {
            let k = model::candidate_key(c);
            if !self.hidden.contains(&k) && seen.insert(k) {
                out.push(c.clone());
            }
        }
        out
    }
}

/// A reviewed step's phase (`QuarantineFixDialog` / `ReviewProposalDialog`).
#[derive(Clone, Debug)]
pub enum Phase {
    Checking,
    Ready(Value, String),
    Applying(Value, String),
    Stale,
    /// Some picks no longer read as picked.
    Changed(usize),
    /// Every pick cleared meanwhile.
    Nothing,
    Failed(String),
    /// Apply failed without saying whether it ran: retry the same proposal and key.
    Uncertain(Value, String, String),
}

impl Phase {
    pub fn artifact(&self) -> Option<&Value> {
        match self {
            Phase::Ready(a, _) | Phase::Applying(a, _) => Some(a),
            _ => None,
        }
    }
}

/// The unpair step for picked pairs.
pub struct Fix {
    pub items: Vec<Item>,
    pub scope: Option<Value>,
    pub phase: Phase,
    pub targets: Vec<Item>,
    pub left: usize,
    token: u64,
}

#[derive(Clone, Debug)]
pub enum PropKind {
    /// Accept every approximate price the book holds for review.
    Accept,
    /// Prices typed on a card: each line (label, exact value, asset).
    Prices {
        card: String,
        lines: Vec<(String, String, String)>,
        rate: bool,
        currency: Option<String>,
    },
}

/// A review proposal decided on a card (accept prices, entered prices).
pub struct Proposal {
    pub kind: PropKind,
    pub scope: Value,
    pub ops: Vec<Value>,
    pub phase: Phase,
    token: u64,
}

/// The bulk pairing of exact matches.
pub struct Bulk {
    pub ops: Vec<Value>,
    pub count: usize,
    pub preview: Option<(Value, String)>,
    /// cases / plan / apply in flight.
    pub busy: bool,
    pub applying: bool,
    token: u64,
}

#[derive(Default)]
pub struct Review {
    pub q: Quarantine,
    pub tr: Transfers,
    pub cards: HashMap<String, Card>,
    pub all_causes: bool,
    pub locked_gap: Option<String>,
    pub fix: Option<Fix>,
    /// The detail panel's "Save & next" waiting for the re-read: (saved row, the queue it was
    /// opened with, its source).
    pub sheet_next: Option<(String, Vec<String>, crate::txdetail::ReviewSource)>,
    pub prop: Option<Proposal>,
    pub bulk: Option<Bulk>,
    /// A queued journal refresh (app-clock due time).
    refresh_due: Option<f64>,
    pub refresh_pending: bool,
    was_processing: bool,
    pub sync_pending: bool,
    sync_label: String,
    /// "Link it back" requests in flight, by dismissal id.
    linkback: VecDeque<String>,
    token: u64,
    tokens: HashMap<&'static str, VecDeque<u64>>,
}

impl Review {
    pub fn card(&self, key: &str) -> Option<&Card> {
        self.cards.get(key)
    }
    /// `isProcessingJournals || refreshPending`.
    pub fn refreshing(app: &App) -> bool {
        app.busy.processing || app.review.rv.refresh_pending
    }
}

fn rv(app: &mut App) -> &mut Review {
    &mut app.review.rv
}

pub fn card_mut<'a>(app: &'a mut App, key: &str) -> &'a mut Card {
    app.review.rv.cards.entry(key.to_owned()).or_default()
}

fn send_tok(app: &mut App, kind: &str, args: Value, tag: &'static str, token: u64) {
    if app.send(kind, args, tag).is_some() {
        app.review
            .rv
            .tokens
            .entry(tag)
            .or_default()
            .push_back(token);
    }
}

fn take_tok(app: &mut App, tag: &'static str) -> u64 {
    app.review
        .rv
        .tokens
        .get_mut(tag)
        .and_then(VecDeque::pop_front)
        .unwrap_or(u64::MAX)
}

fn is_stale(r: &Reply) -> bool {
    matches!(
        r.error_detail["code"].as_str(),
        Some(
            "review_plan_stale"
                | "custody_review_plan_stale"
                | "review_case_changed"
                | "stale_context"
        )
    )
}

fn same_scope(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            !s(a, "workspace_id").is_empty()
                && s(a, "workspace_id") == s(b, "workspace_id")
                && s(a, "profile_id") == s(b, "profile_id")
        }
        _ => false,
    }
}

// ---- Reads --------------------------------------------------------------------------------------

/// The screen mounts: what was read stays shown while everything is read again; the screen's own
/// choices (folds, ticks, typed prices, dialogs) start over, as a remount does.
pub fn mount(app: &mut App) {
    let old = std::mem::take(&mut app.review.rv);
    let processing = app.busy.processing;
    let r = rv(app);
    r.q.pages = old.q.pages;
    r.q.generation = old.q.generation + 1;
    r.q.wgen = old.q.wgen + 1;
    r.tr.live = old.tr.live;
    r.token = old.token + 1;
    r.was_processing = processing;
    r.tr.was_refreshing = processing;
    // Mounted while a run is already going, it shows nothing until the run is done rather than
    // a stale read.
    if processing {
        r.tr.frozen = Some(vec![]);
    }
    load_suggest(app, None);
    load_first(app);
}

fn load_suggest(app: &mut App, run: Option<u64>) {
    if app
        .send("ui.transfers.suggest", json!({}), "review.suggest")
        .is_some()
    {
        let r = rv(app);
        if r.tr.live.is_none() {
            r.tr.loading = true;
        }
        r.tr.sq.push_back(run);
    }
}

fn att_args(offset: usize) -> Value {
    json!({ "limit": PAGE, "offset": offset, "scope": "attention" })
}

fn load_first(app: &mut App) {
    if app
        .send("ui.journals.quarantine", att_args(0), "review.att")
        .is_some()
    {
        let r = rv(app);
        if r.q.pages.is_empty() {
            r.q.loading = true;
        }
        let g = r.q.generation;
        r.q.queue.push_back((g, AttKind::First));
    }
}

/// `fetchNextPage`.
pub fn load_more(app: &mut App) {
    let Some(next) = app.review.rv.q.pages.last().and_then(model::next_offset) else {
        return;
    };
    if app.review.rv.q.more_loading {
        return;
    }
    if app
        .send("ui.journals.quarantine", att_args(next), "review.att")
        .is_some()
    {
        let r = rv(app);
        r.q.more_loading = true;
        let g = r.q.generation;
        r.q.queue.push_back((g, AttKind::More));
    }
}

fn load_waiting(app: &mut App, refetch: bool) {
    let off = app.review.rv.q.offset;
    if app
        .send(
            "ui.journals.quarantine",
            json!({ "limit": PAGE, "offset": off, "scope": "waiting" }),
            "review.wait",
        )
        .is_some()
    {
        let r = rv(app);
        r.q.wgen += 1;
        if !refetch {
            r.q.waiting_loading = true;
        }
        let g = r.q.wgen;
        r.q.wqueue.push_back((g, refetch));
    }
}

/// Shows or hides the waiting list (back to its first page).
pub fn set_waiting_shown(app: &mut App, shown: bool) {
    let r = rv(app);
    r.q.waiting_shown = shown;
    r.q.offset = 0;
    if shown {
        r.q.waiting = None;
        r.q.waiting_error = None;
        load_waiting(app, false);
    }
}

pub fn set_offset(app: &mut App, offset: usize) {
    rv(app).q.offset = offset;
    load_waiting(app, false);
}

pub fn retry_waiting(app: &mut App) {
    rv(app).q.waiting_error = None;
    load_waiting(app, false);
}

/// Re-reads exactly the pages the screen shows (`refresh`); `waiter` continues once both lists
/// are back.
pub fn refetch(app: &mut App, waiter: Option<Waiter>) {
    if let Some(rf) = app.review.rv.q.refetch.as_mut() {
        rf.waiters.extend(waiter);
        return;
    }
    let target = app.review.rv.q.pages.len().max(1);
    let shown = app.review.rv.q.waiting_shown;
    rv(app).q.refetch = Some(Refetch {
        target,
        fresh: vec![],
        failed: false,
        att_done: false,
        waiting_done: !shown,
        waiting_failed: false,
        waiters: waiter.into_iter().collect(),
    });
    if app
        .send("ui.journals.quarantine", att_args(0), "review.att")
        .is_some()
    {
        let r = rv(app);
        let g = r.q.generation;
        r.q.queue.push_back((g, AttKind::Refetch));
    } else if let Some(rf) = rv(app).q.refetch.as_mut() {
        rf.failed = true;
        rf.att_done = true;
    }
    if shown {
        load_waiting(app, true);
    }
    finish_refetch(app);
}

/// Everything the screen reads, again (what a mutation's query invalidation does).
pub fn refetch_all(app: &mut App) {
    refetch(app, None);
    load_suggest(app, None);
    app.send("ui.review.badges", Value::Null, "app.badges");
}

fn finish_refetch(app: &mut App) {
    let done = app
        .review
        .rv
        .q
        .refetch
        .as_ref()
        .is_some_and(|r| r.att_done && r.waiting_done);
    if !done {
        return;
    }
    let rf = app.review.rv.q.refetch.take().unwrap();
    let ok = !rf.failed && !rf.waiting_failed && !rf.fresh.is_empty();
    // "Save & next" needs the attention pages; a failed waiting page only empties that list.
    let att_ok = !rf.failed && !rf.fresh.is_empty();
    if !rf.failed && !rf.fresh.is_empty() {
        let r = rv(app);
        r.q.pages = rf.fresh;
        r.q.error = None;
        r.q.more_error = None;
    }
    for w in rf.waiters {
        match w {
            Waiter::Fix => super::dialogs::fix_refreshed(app, ok),
            Waiter::SheetNext => super::links::sheet_next_refreshed(app, att_ok),
        }
    }
}

// ---- Poll ---------------------------------------------------------------------------------------

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    // The undo offer lapses.
    if let Some(u) = &app.review.rv.tr.undo {
        if app.clock >= u.deadline {
            app.review.rv.tr.undo = None;
            changed = true;
        } else {
            tx.wake_after(std::time::Duration::from_secs_f64(
                (u.deadline - app.clock).max(0.05),
            ));
        }
    }
    // A queued journal refresh starts once nothing else processes this book.
    if let Some(due) = app.review.rv.refresh_due {
        if app.clock >= due {
            if app.busy.processing || app.busy.syncing {
                app.review.rv.refresh_due = Some(app.clock + SETTLE);
                tx.wake_after(std::time::Duration::from_secs_f64(SETTLE));
            } else {
                app.review.rv.refresh_due = None;
                app.review.rv.refresh_pending = false;
                let scope = app.expected_scope();
                if app
                    .send(
                        "ui.journals.process",
                        json!({ "auto_pair": false, "expected_scope": scope }),
                        "review.journals",
                    )
                    .is_some()
                {
                    app.busy.processing = true;
                    app.busy.progress = Some(t("journals:processing.phase.fallback"));
                }
            }
            changed = true;
        } else {
            tx.wake_after(std::time::Duration::from_secs_f64(
                (due - app.clock).max(0.01),
            ));
        }
    }
    if app.route.path() != "/review" {
        return changed;
    }
    // Focus goes to the row now in the decided one's place (its first button, not its tick).
    if let Some(i) = rv(app).tr.focus_row.take() {
        let cands = app.review.rv.tr.candidates();
        if let Some(c) = cands.get(i.min(cands.len().saturating_sub(1))) {
            tx.focus(super::inbox::primary_key(c), true);
        }
        changed = true;
    }
    // The transfer list holds still while the journals recalculate, and is read again after.
    let refreshing = Review::refreshing(app);
    let r = rv(app);
    if refreshing && !r.tr.was_refreshing {
        r.tr.run += 1;
        if r.tr.frozen.is_none() {
            r.tr.frozen = Some(r.tr.live.clone().unwrap_or_default());
        }
        changed = true;
    }
    let finished = !refreshing && r.tr.was_refreshing;
    r.tr.was_refreshing = refreshing;
    if finished {
        let run = r.tr.run;
        load_suggest(app, Some(run));
        changed = true;
    }
    // A finished journal run (from anywhere) re-reads the lists, as its invalidation does.
    let processing = app.busy.processing;
    let was = std::mem::replace(&mut rv(app).was_processing, processing);
    if was && !processing {
        refetch(app, None);
        changed = true;
    }
    // A rebuild can shrink the waiting list below the current page.
    let q = &app.review.rv.q;
    if let Some(w) = q.waiting_snap() {
        let total = w.scope_count();
        if q.offset > 0 && total > 0 && q.offset >= total {
            set_offset(app, 0);
            changed = true;
        }
    }
    changed
}

// ---- Replies ------------------------------------------------------------------------------------

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "review.suggest" => {
            let run = rv(app).tr.sq.pop_front().flatten();
            let tr = &mut rv(app).tr;
            tr.loading = false;
            match &r.error {
                Some(_) => tr.failed = tr.live.is_none(),
                None => {
                    tr.failed = false;
                    tr.live = Some(r.data["candidates"].as_array().cloned().unwrap_or_default());
                }
            }
            // The read after a recalculation lists what is open now.
            if let Some(run) = run
                && run == tr.run
            {
                tr.frozen = None;
                tr.hidden.clear();
                tr.restored.clear();
            }
        }
        "review.att" => on_att(app, r),
        "review.wait" => {
            let Some((g, refetch)) = rv(app).q.wqueue.pop_front() else {
                return;
            };
            let q = &mut rv(app).q;
            if refetch && let Some(rf) = q.refetch.as_mut() {
                rf.waiting_done = true;
                rf.waiting_failed = r.error.is_some();
            }
            if g == q.wgen {
                q.waiting_loading = false;
                match &r.error {
                    Some(e) => q.waiting_error = Some(e.clone()),
                    None => {
                        q.waiting_error = None;
                        q.waiting = Some(r.data.clone());
                    }
                }
            }
            if refetch {
                finish_refetch(app);
            }
        }
        "review.journals" => {
            app.busy.processing = false;
            app.busy.progress = None;
            if let Some(e) = &r.error {
                app.toast(
                    ToastTone::Error,
                    &t("journals:quarantine.unavailable.title"),
                    Some(e.clone()),
                    Some("journals"),
                );
            }
            app.refresh_reads();
            refetch_all(app);
        }
        "review.pair" | "review.dismiss" => on_decision(app, r),
        "review.dismiss.sel" => on_dismiss_selected(app, r),
        "review.linkback" => {
            // One toast per restore (the web keys it by the dismissal).
            let _ = rv(app).linkback.pop_front();
            let key = "quarantine-link-back";
            if let Some(e) = &r.error {
                app.toast(
                    ToastTone::Error,
                    &t("journals:quarantine.panel.linkBackFailedTitle"),
                    Some(e.clone()).filter(|e| !e.is_empty()),
                    Some(key),
                );
                return;
            }
            app.toast(
                ToastTone::Success,
                &t("journals:quarantine.panel.linkedBackTitle"),
                Some(t("journals:quarantine.panel.linkedBackBody")),
                Some(key),
            );
            // The mutation's invalidation, then the journals recalculate.
            refetch_all(app);
            app.process_journals();
        }
        "review.unpair" | "review.undismiss" => on_undo_step(app, r),
        "review.sync" => {
            rv(app).sync_pending = false;
            let label = std::mem::take(&mut rv(app).sync_label);
            let result = &r.data["results"][0];
            let failed = r.error.is_some() || s(result, "status") == "error";
            if failed {
                app.toast(
                    ToastTone::Error,
                    &tc(
                        "chrome:notifications.condition.connectionSyncFailed.title",
                        1,
                        &[],
                    ),
                    Some(label),
                    Some("review-sync"),
                );
            } else {
                app.toast(
                    ToastTone::Success,
                    &t("chrome:notifications.result.connectionSynced.title"),
                    Some(label),
                    Some("review-sync"),
                );
                crate::shell::maint::record_journal_step(app, &r.data["journals"]);
            }
            app.refresh_reads();
            refetch_all(app);
        }
        "review.fix.cases" | "review.fix.plan" | "review.fix.apply" => {
            super::dialogs::on_fix_reply(app, tag, r)
        }
        "review.prop.cases" | "review.prop.plan" | "review.prop.apply" => {
            super::dialogs::on_prop_reply(app, tag, r)
        }
        "review.bulk.cases" | "review.bulk.plan" | "review.bulk.apply" => {
            on_bulk_reply(app, tag, r)
        }
        _ => {}
    }
}

fn on_att(app: &mut App, r: &Reply) {
    let Some((g, kind)) = rv(app).q.queue.pop_front() else {
        return;
    };
    if g != app.review.rv.q.generation {
        return;
    }
    match kind {
        AttKind::First => {
            let q = &mut rv(app).q;
            q.loading = false;
            match &r.error {
                Some(e) => {
                    if q.pages.is_empty() {
                        q.error = Some(e.clone());
                    }
                }
                None => {
                    q.error = None;
                    q.more_error = None;
                    q.pages = vec![r.data.clone()];
                }
            }
        }
        AttKind::More => {
            let q = &mut rv(app).q;
            q.more_loading = false;
            match &r.error {
                Some(e) => q.more_error = Some(e.clone()),
                None => {
                    q.more_error = None;
                    q.pages.push(r.data.clone());
                }
            }
        }
        AttKind::Refetch => {
            let Some(rf) = rv(app).q.refetch.as_mut() else {
                return;
            };
            match &r.error {
                Some(_) => {
                    rf.failed = true;
                    rf.att_done = true;
                }
                None => {
                    rf.fresh.push(r.data.clone());
                    let next = model::next_offset(&r.data);
                    match next.filter(|_| rf.fresh.len() < rf.target) {
                        Some(off) => {
                            if app
                                .send("ui.journals.quarantine", att_args(off), "review.att")
                                .is_some()
                            {
                                let q = &mut rv(app).q;
                                let g = q.generation;
                                q.queue.push_back((g, AttKind::Refetch));
                            } else if let Some(rf) = rv(app).q.refetch.as_mut() {
                                rf.att_done = true;
                            }
                        }
                        None => rf.att_done = true,
                    }
                }
            }
            finish_refetch(app);
        }
    }
}

// ---- Journal refresh after decisions ------------------------------------------------------------

/// `requestRefresh`: decisions settle, then the book's journals recalculate without auto pairing.
pub fn request_refresh(app: &mut App) {
    let scope = app.expected_scope();
    if scope["workspace_id"].is_null() || scope["profile_id"].is_null() {
        return;
    }
    rv(app).refresh_pending = true;
    app.send("ui.review.badges", Value::Null, "app.badges");
    rv(app).refresh_due = Some(app.clock + SETTLE);
}

// ---- Transfer decisions ---------------------------------------------------------------------------

fn route_label(c: &Value) -> String {
    tr(
        "review:inbox.transfers.route",
        &[
            ("from", &s(c, "out_wallet_label")),
            ("to", &s(c, "in_wallet_label")),
        ],
    )
}

fn hide(app: &mut App, keys: &[String], on: bool) {
    let tr = &mut rv(app).tr;
    if on {
        let base: Vec<Value> = tr
            .frozen
            .clone()
            .or_else(|| tr.live.clone())
            .unwrap_or_default();
        for c in base {
            let k = model::candidate_key(&c);
            if keys.contains(&k) {
                tr.decided.insert(k, c);
            }
        }
        for k in keys {
            tr.hidden.insert(k.clone());
        }
    } else {
        let back: Vec<Value> = keys
            .iter()
            .filter_map(|k| tr.decided.get(k).cloned())
            .collect();
        tr.restored.extend(back);
        for k in keys {
            tr.hidden.remove(k);
        }
    }
}

/// Pair or set aside one candidate (`decide`).
pub fn decide(app: &mut App, c: &Value, pair: bool) {
    rv(app).tr.error = None;
    let cands = app.review.rv.tr.candidates();
    let key = model::candidate_key(c);
    rv(app).tr.focus_row = cands.iter().position(|x| model::candidate_key(x) == key);
    let keys: Vec<String> = if pair {
        let (o, i) = (s(c, "out_id"), s(c, "in_id"));
        cands
            .iter()
            .filter(|x| {
                s(x, "out_id") == o
                    || s(x, "in_id") == i
                    || s(x, "out_id") == i
                    || s(x, "in_id") == o
            })
            .map(model::candidate_key)
            .collect()
    } else {
        vec![key]
    };
    hide(app, &keys, true);
    // A decided row is no longer ticked, so an undo does not bring it back selected.
    rv(app).tr.selected.retain(|k| !keys.contains(k));
    let label = if pair {
        tr(
            "review:inbox.transfers.paired",
            &[("route", &route_label(c))],
        )
    } else {
        tr(
            "review:inbox.transfers.dismissed",
            &[("route", &route_label(c))],
        )
    };
    let (kind, tag, args) = if pair {
        {
            let (kind, policy) = model::candidate_terms(c);
            (
                "ui.transfers.pair",
                "review.pair",
                json!({ "tx_out": s(c, "out_id"), "tx_in": s(c, "in_id"), "kind": kind, "policy": policy, "pair_source": "manual", "confidence_at_pair": c["confidence"] }),
            )
        }
    } else {
        (
            "ui.transfers.dismiss",
            "review.dismiss",
            json!({ "tx_out": s(c, "out_id"), "tx_in": s(c, "in_id"), "reason": model::DISMISS_REASON }),
        )
    };
    if app.send(kind, args, tag).is_some() {
        let tr = &mut rv(app).tr;
        tr.saving += 1;
        tr.pending.push_back(Pending { pair, keys, label });
    } else {
        hide(app, &keys, false);
    }
}

fn on_decision(app: &mut App, r: &Reply) {
    let Some(p) = rv(app).tr.pending.pop_front() else {
        return;
    };
    rv(app).tr.saving = app.review.rv.tr.saving.saturating_sub(1);
    match &r.error {
        Some(e) => {
            hide(app, &p.keys, false);
            rv(app).tr.error = Some(if e.is_empty() {
                t("review:inbox.transfers.failed")
            } else {
                e.clone()
            });
            load_suggest(app, None);
        }
        None => {
            let id = s(&r.data, "id");
            let deadline = app.clock + UNDO_SECS;
            rv(app).tr.undo = (!id.is_empty()).then(|| Undo {
                decisions: vec![if p.pair {
                    Decision::Pair { id, keys: p.keys }
                } else {
                    Decision::Dismiss { id, keys: p.keys }
                }],
                label: p.label,
                deadline,
            });
            request_refresh(app);
        }
    }
}

/// The ticked suggestions that are still listed (a decided one drops out on its own).
pub fn chosen(tr: &Transfers) -> Vec<Value> {
    tr.candidates()
        .into_iter()
        .filter(|c| tr.selected.contains(&model::candidate_key(c)))
        .collect()
}

pub fn toggle_selected(app: &mut App, key: &str) {
    let sel = &mut rv(app).tr.selected;
    if !sel.remove(key) {
        sel.insert(key.to_owned());
    }
}

/// `dismissSelected`: sets every ticked suggestion aside, one after another, with one Undo for
/// all of them.
pub fn dismiss_selected(app: &mut App) {
    let chosen = chosen(&app.review.rv.tr);
    if chosen.is_empty() || app.review.rv.tr.dismiss_run.is_some() {
        return;
    }
    rv(app).tr.error = None;
    let keys: Vec<String> = chosen.iter().map(model::candidate_key).collect();
    // Focus goes to where the first of them was.
    let place = app
        .review
        .rv
        .tr
        .candidates()
        .iter()
        .position(|c| model::candidate_key(c) == keys[0])
        .unwrap_or(0);
    rv(app).tr.focus_row = Some(place);
    hide(app, &keys, true);
    let tr = &mut rv(app).tr;
    tr.selected.clear();
    tr.saving += 1;
    tr.dismiss_run = Some(DismissRun {
        queue: chosen.into(),
        keys,
        sent: 0,
        done: vec![],
    });
    dismiss_next(app);
}

fn dismiss_next(app: &mut App) {
    let next = app
        .review
        .rv
        .tr
        .dismiss_run
        .as_ref()
        .and_then(|d| d.queue.front().cloned());
    let Some(c) = next else {
        return finish_dismiss_selected(app, None);
    };
    let args = json!({ "tx_out": s(&c, "out_id"), "tx_in": s(&c, "in_id"), "reason": model::DISMISS_REASON });
    if app
        .send("ui.transfers.dismiss", args, "review.dismiss.sel")
        .is_none()
    {
        finish_dismiss_selected(app, Some(t("review:inbox.transfers.failed")));
    }
}

fn on_dismiss_selected(app: &mut App, r: &Reply) {
    if let Some(e) = &r.error {
        return finish_dismiss_selected(
            app,
            Some(if e.is_empty() {
                t("review:inbox.transfers.failed")
            } else {
                e.clone()
            }),
        );
    }
    let Some(run) = rv(app).tr.dismiss_run.as_mut() else {
        return;
    };
    if let Some(c) = run.queue.pop_front() {
        run.sent += 1;
        let id = s(&r.data, "id");
        if !id.is_empty() {
            run.done.push(Decision::Dismiss {
                id,
                keys: vec![model::candidate_key(&c)],
            });
        }
    }
    dismiss_next(app);
}

fn finish_dismiss_selected(app: &mut App, error: Option<String>) {
    let Some(run) = rv(app).tr.dismiss_run.take() else {
        return;
    };
    if let Some(e) = error {
        // The ones not set aside come back to the list.
        let back: Vec<String> = run.keys[run.sent.min(run.keys.len())..].to_vec();
        hide(app, &back, false);
        rv(app).tr.error = Some(e);
        load_suggest(app, None);
    }
    if !run.done.is_empty() {
        let deadline = app.clock + UNDO_SECS;
        let label = tc("review:inbox.transfers.dismissedMany", run.done.len(), &[]);
        rv(app).tr.undo = Some(Undo {
            decisions: run.done,
            label,
            deadline,
        });
    }
    if run.sent > 0 {
        request_refresh(app);
    }
    let tr = &mut rv(app).tr;
    tr.saving = tr.saving.saturating_sub(1);
}

/// `runUndo`: each decision on its own; a failure keeps the rest undoable.
pub fn run_undo(app: &mut App) {
    let Some(u) = rv(app).tr.undo.take() else {
        return;
    };
    rv(app).tr.error = None;
    rv(app).tr.undo_run = Some(UndoRun {
        current: u,
        undone: vec![],
    });
    undo_next(app);
}

fn undo_next(app: &mut App) {
    let next = {
        let Some(run) = app.review.rv.tr.undo_run.as_ref() else {
            return;
        };
        run.current.decisions.get(run.undone.len()).cloned()
    };
    match next {
        Some(Decision::Pair { id, .. }) => {
            app.send(
                "ui.transfers.unpair",
                json!({ "pair_id": id }),
                "review.unpair",
            );
        }
        Some(Decision::Dismiss { id, .. }) => {
            app.send(
                "ui.transfers.dismissals.delete",
                json!({ "dismissal_id": id }),
                "review.undismiss",
            );
        }
        None => finish_undo(app, None),
    }
}

fn on_undo_step(app: &mut App, r: &Reply) {
    if let Some(e) = &r.error {
        finish_undo(
            app,
            Some(if e.is_empty() {
                t("review:inbox.transfers.failed")
            } else {
                e.clone()
            }),
        );
        return;
    }
    if let Some(run) = rv(app).tr.undo_run.as_mut() {
        let i = run.undone.len();
        if let Some(d) = run.current.decisions.get(i).cloned() {
            run.undone.push(d);
        }
    }
    undo_next(app);
}

fn finish_undo(app: &mut App, error: Option<String>) {
    let Some(run) = rv(app).tr.undo_run.take() else {
        return;
    };
    if let Some(e) = error {
        rv(app).tr.error = Some(e);
        let left: Vec<Decision> = run.current.decisions[run.undone.len()..].to_vec();
        let deadline = app.clock + UNDO_SECS;
        rv(app).tr.undo = Some(Undo {
            decisions: left,
            label: run.current.label.clone(),
            deadline,
        });
    }
    if !run.undone.is_empty() {
        let keys: Vec<String> = run.undone.iter().flat_map(|d| d.keys().to_vec()).collect();
        hide(app, &keys, false);
        request_refresh(app);
    }
}

// ---- Bulk pairing of exact matches ------------------------------------------------------------------

/// `prepareBulk`: capture the exact matches, then preview pairing them.
pub fn prepare_bulk(app: &mut App) {
    rv(app).tr.error = None;
    let exact = model::bulk_pairable(&app.review.rv.tr.candidates());
    let reason = t("review:inbox.transfers.bulk.reason");
    let ops: Vec<Value> = exact
        .iter()
        .map(|c| {
            let (kind, policy) = model::candidate_terms(c);
            json!({ "type": "pair", "out_transaction_id": s(c, "out_id"), "in_transaction_id": s(c, "in_id"), "pair_kind": kind, "policy": policy, "reason": reason })
        })
        .collect();
    let scope = app.expected_scope();
    let token = {
        let r = rv(app);
        r.token += 1;
        r.token
    };
    rv(app).bulk = Some(Bulk {
        count: ops.len(),
        ops,
        preview: None,
        busy: true,
        applying: false,
        token,
    });
    if scope["workspace_id"].is_null() || scope["profile_id"].is_null() {
        rv(app).bulk = None;
        rv(app).tr.error = Some(t("review:inbox.transfers.bulk.none"));
        return;
    }
    send_tok(
        app,
        "ui.review.cases",
        json!({ "limit": 1, "expected_scope": scope }),
        "review.bulk.cases",
        token,
    );
}

/// `pairAllExact`.
pub fn pair_all_exact(app: &mut App) {
    let Some(b) = rv(app).bulk.as_mut() else {
        return;
    };
    let Some((artifact, key)) = b.preview.clone() else {
        return;
    };
    b.busy = true;
    b.applying = true;
    let token = b.token;
    rv(app).tr.error = None;
    let scope =
        json!({ "workspace_id": artifact["workspace_id"], "profile_id": artifact["profile_id"] });
    send_tok(
        app,
        "ui.review.apply",
        json!({ "artifact": artifact, "idempotency_key": key, "expected_scope": scope }),
        "review.bulk.apply",
        token,
    );
}

pub fn close_bulk(app: &mut App) {
    if app.review.rv.bulk.as_ref().is_some_and(|b| b.busy) {
        return;
    }
    rv(app).bulk = None;
}

fn on_bulk_reply(app: &mut App, tag: &str, r: &Reply) {
    let tag: &'static str = match tag {
        "review.bulk.cases" => "review.bulk.cases",
        "review.bulk.plan" => "review.bulk.plan",
        _ => "review.bulk.apply",
    };
    let token = take_tok(app, tag);
    if app.review.rv.bulk.as_ref().map(|b| b.token) != Some(token) {
        return;
    }
    let fail = |app: &mut App, msg: String, close: bool| {
        if close {
            rv(app).bulk = None;
        } else if let Some(b) = rv(app).bulk.as_mut() {
            b.busy = false;
            b.applying = false;
        }
        rv(app).tr.error = Some(msg);
    };
    let msg = |r: &Reply| {
        r.error
            .clone()
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| t("review:inbox.transfers.failed"))
    };
    match tag {
        "review.bulk.cases" => {
            if r.error.is_some() {
                return fail(app, msg(r), true);
            }
            let scope = app.expected_scope();
            let ops = app
                .review
                .rv
                .bulk
                .as_ref()
                .map(|b| b.ops.clone())
                .unwrap_or_default();
            send_tok(
                app,
                "ui.review.plan",
                json!({ "operations": ops, "expected_input_version": r.data["input_version"], "expected_scope": scope }),
                "review.bulk.plan",
                token,
            );
        }
        "review.bulk.plan" => {
            if r.error.is_some() {
                return fail(app, msg(r), true);
            }
            if !model::artifact_ok(&r.data) {
                return fail(app, t("review:inbox.transfers.bulk.none"), true);
            }
            if let Some(b) = rv(app).bulk.as_mut() {
                b.preview = Some((r.data.clone(), model::uuid()));
                b.busy = false;
            }
        }
        _ => {
            if r.error.is_some() {
                // Keep the same artifact and idempotency key if the answer was lost.
                return fail(app, msg(r), false);
            }
            let applied = r.data["operations"].as_array().cloned().unwrap_or_default();
            rv(app).bulk = None;
            let decisions: Vec<Decision> = applied
                .iter()
                .map(|op| {
                    let res = &op["result"];
                    let ids = model::strs(&res["transaction_ids"]);
                    Decision::Pair {
                        id: s(res, "pair_id"),
                        keys: vec![format!(
                            "{}->{}",
                            ids.first().cloned().unwrap_or_default(),
                            ids.get(1).cloned().unwrap_or_default()
                        )],
                    }
                })
                .collect();
            let keys: Vec<String> = decisions.iter().flat_map(|d| d.keys().to_vec()).collect();
            hide(app, &keys, true);
            let deadline = app.clock + UNDO_SECS;
            rv(app).tr.undo = Some(Undo {
                label: tc("review:inbox.transfers.pairedMany", applied.len(), &[]),
                decisions,
                deadline,
            });
            request_refresh(app);
        }
    }
}

// ---- Quarantine actions ---------------------------------------------------------------------------

/// A cause's "Link it back": deletes the dismissal that undid the automatic link
/// (`ui.transfers.dismissals.delete`, local).
pub fn link_back(app: &mut App, dismissal_id: &str) {
    if app
        .send(
            "ui.transfers.dismissals.delete",
            json!({ "dismissal_id": dismissal_id }),
            "review.linkback",
        )
        .is_some()
    {
        rv(app).linkback.push_back(dismissal_id.to_owned());
    }
}

/// `runSync`: the same wallet sync job the wallet page starts. Network egress: only ever from the
/// owner's press.
pub fn run_sync(app: &mut App, wallet_id: &str, label: &str) {
    if app.review.rv.sync_pending {
        return;
    }
    if app
        .send(
            "ui.wallets.sync",
            json!({ "wallet": wallet_id }),
            "review.sync",
        )
        .is_some()
    {
        rv(app).sync_pending = true;
        rv(app).sync_label = label.to_owned();
    }
}

/// Starts a reviewed step for picked pairs (`setFixing`).
pub fn start_fix(app: &mut App, items: Vec<Item>, scope: Option<Value>) {
    let token = {
        let r = rv(app);
        r.token += 1;
        r.token
    };
    rv(app).fix = Some(Fix {
        items,
        scope,
        phase: Phase::Checking,
        targets: vec![],
        left: 0,
        token,
    });
    super::dialogs::fix_check(app);
}

/// Starts a review proposal (accept prices, entered prices).
pub fn start_proposal(app: &mut App, kind: PropKind, scope: Value, ops: Vec<Value>) {
    let token = {
        let r = rv(app);
        r.token += 1;
        r.token
    };
    rv(app).prop = Some(Proposal {
        kind,
        scope,
        ops,
        phase: Phase::Checking,
        token,
    });
    super::dialogs::prop_check(app);
}

// Token plumbing the dialogs share.
pub fn fix_token(app: &App) -> Option<u64> {
    app.review.rv.fix.as_ref().map(|f| f.token)
}
pub fn prop_token(app: &App) -> Option<u64> {
    app.review.rv.prop.as_ref().map(|p| p.token)
}
pub fn new_fix_token(app: &mut App) -> u64 {
    let r = rv(app);
    r.token += 1;
    let t = r.token;
    if let Some(f) = r.fix.as_mut() {
        f.token = t;
    }
    t
}
pub fn new_prop_token(app: &mut App) -> u64 {
    let r = rv(app);
    r.token += 1;
    let t = r.token;
    if let Some(p) = r.prop.as_mut() {
        p.token = t;
    }
    t
}
pub fn send_t(app: &mut App, kind: &str, args: Value, tag: &'static str, token: u64) {
    send_tok(app, kind, args, tag, token);
}
pub fn take_t(app: &mut App, tag: &'static str) -> u64 {
    take_tok(app, tag)
}
pub fn stale(r: &Reply) -> bool {
    is_stale(r)
}
pub fn scopes_match(a: Option<&Value>, b: Option<&Value>) -> bool {
    same_scope(a, b)
}
