//! The suggestions card (`components/review/ReviewTransfers.tsx`, `framed`): moves the matcher
//! thinks went between the owner's wallets, paired or set aside one at a time, exact ones in
//! bulk after the review core's plan, several set aside at once, each decision undoable for
//! 20 s. A suggestion never pairs on its own: every pair is the owner's press, and the bulk step
//! applies only the plan the core showed (bound to the book and its input version).
//!
//! - `model`: the candidate helpers (`transferModel.ts`, `suggestionReason.ts`, review artifacts).
//! - `view`: the card, its rows, the bulk confirmation and the tax change by year.

pub mod model;
mod view;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};

use rune::{Cx, El, Tx};
use serde_json::{Value, json};

use crate::app::{App, Reply};
use crate::i18n::{t, tc, tr};
use crate::theme::Tk;

use model::{candidate_key, s};

/// Candidates listed before the rest fold away.
pub const SHOWN: usize = 6;
/// How long a decision can be undone from here.
const UNDO_SECS: f64 = 20.0;

/// What a saved view selects among the suggestions (`SuggestionFilter`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub confidence: Option<String>,
    pub method: Option<String>,
    pub asset_pair: Option<String>,
    pub route_pair: Option<String>,
    pub candidate_type: Option<String>,
}

impl Filter {
    /// The read's args (`filter ?? {}`), keys in the web's order.
    pub fn args(&self) -> Value {
        let mut m = serde_json::Map::new();
        for (k, v) in [
            ("confidence", &self.confidence),
            ("method", &self.method),
            ("asset_pair", &self.asset_pair),
            ("route_pair", &self.route_pair),
            ("candidate_type", &self.candidate_type),
        ] {
            if let Some(v) = v {
                m.insert(k.into(), json!(v));
            }
        }
        Value::Object(m)
    }
}

/// A pair or dismissal made here, and the candidates it took off the list.
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

/// "Set N aside": one dismissal after another, one Undo for all.
struct DismissMany {
    queue: Vec<Value>,
    keys: Vec<String>,
    sent: usize,
    done: Vec<Decision>,
}

/// The bulk step for exact matches (`confirmBulk` + `bulkPreview`).
pub struct Bulk {
    pub count: usize,
    ops: Vec<Value>,
    /// The core's plan and the idempotency key it is applied with.
    pub preview: Option<(Value, String)>,
    /// `cases`/`plan`/`apply` in flight.
    pub busy: bool,
    pub applying: bool,
    token: u64,
}

/// An open popover or menu of the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Popup {
    Info,
    Row(String),
}

#[derive(Default)]
pub struct State {
    /// The filter the card was last drawn with (set while drawing; read by `poll`).
    want: RefCell<Option<Filter>>,
    /// The filter the reads are for; None until the card first shows.
    loaded_for: Option<Filter>,
    pub live: Option<Vec<Value>>,
    pub loading: bool,
    pub failed: bool,
    frozen: Option<Vec<Value>>,
    hidden: HashSet<String>,
    decided: HashMap<String, Value>,
    restored: Vec<Value>,
    pub expanded: bool,
    pub saving: usize,
    pub undo: Option<Undo>,
    pub error: Option<String>,
    pub selected: HashSet<String>,
    run: u64,
    was_refreshing: bool,
    /// Suggest reads in flight: the run a post-recalculation read belongs to.
    sq: VecDeque<Option<u64>>,
    pending: VecDeque<Pending>,
    undo_run: Option<UndoRun>,
    many: Option<DismissMany>,
    pub bulk: Option<Bulk>,
    token: u64,
    toks: HashMap<&'static str, VecDeque<u64>>,
    pub popup: Option<Popup>,
}

impl State {
    /// Each candidate once: an undone one first, then the list as read, less the decided.
    pub fn candidates(&self) -> Vec<Value> {
        let mut seen = HashSet::new();
        let mut out = vec![];
        let base = self.frozen.as_ref().or(self.live.as_ref());
        for c in self.restored.iter().chain(base.into_iter().flatten()) {
            let k = candidate_key(c);
            if !self.hidden.contains(&k) && seen.insert(k) {
                out.push(c.clone());
            }
        }
        out
    }
}

fn st(app: &mut App) -> &mut State {
    &mut app.custody.sugg
}

/// `refreshing`: the journals are recalculating, or about to.
fn refreshing(app: &App) -> bool {
    super::updating(app)
}

/// The framed card. `filter`/`filter_label`: the saved view in force (a removable chip).
pub fn card(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    filter: Option<&Filter>,
    filter_label: Option<&str>,
) -> El {
    *app.custody.sugg.want.borrow_mut() = Some(filter.cloned().unwrap_or_default());
    view::card(app, cx, tk, layer, filter_label)
}

/// Whether a decision's Undo is on offer (`onUndoChange`).
pub fn undo_showing(app: &App) -> bool {
    app.custody.sugg.undo.is_some()
}

fn load(app: &mut App, run: Option<u64>) {
    let args = st(app).loaded_for.clone().unwrap_or_default().args();
    if app
        .send("ui.transfers.suggest", args, "custody.sugg.suggest")
        .is_some()
    {
        let s = st(app);
        if s.live.is_none() {
            s.loading = true;
        }
        s.sq.push_back(run);
    }
}

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    // The card asked for a filter it has not read yet: a mount, or a saved view chosen.
    let want = app.custody.sugg.want.borrow_mut().take();
    if let Some(w) = want
        && app.custody.sugg.loaded_for.as_ref() != Some(&w)
    {
        let first = app.custody.sugg.loaded_for.is_none();
        let refreshing = refreshing(app);
        let s = st(app);
        if !first {
            // Another view: a fresh query, nothing decided carried over.
            s.live = None;
            s.frozen = None;
            s.hidden.clear();
            s.restored.clear();
            s.selected.clear();
            s.expanded = false;
        } else if refreshing {
            // Mounted while a run is already going: nothing until the run is done.
            s.frozen = Some(vec![]);
        }
        s.loaded_for = Some(w);
        s.was_refreshing = refreshing;
        load(app, None);
        changed = true;
    }
    if !super::owns(app.route.path()) {
        return changed;
    }
    // The undo offer lapses.
    if let Some(u) = &app.custody.sugg.undo {
        if app.clock >= u.deadline {
            st(app).undo = None;
            changed = true;
        } else {
            tx.wake_at(u.deadline);
        }
    }
    if app.custody.sugg.loaded_for.is_none() {
        return changed;
    }
    // The list holds still while the journals recalculate, and is read again after.
    let refreshing = refreshing(app);
    let s = st(app);
    if refreshing && !s.was_refreshing {
        s.run += 1;
        if s.frozen.is_none() {
            s.frozen = Some(s.live.clone().unwrap_or_default());
        }
        changed = true;
    }
    let finished = !refreshing && s.was_refreshing;
    s.was_refreshing = refreshing;
    if finished {
        let run = s.run;
        load(app, Some(run));
        changed = true;
    }
    changed
}

fn send_tok(app: &mut App, kind: &str, args: Value, tag: &'static str, token: u64) {
    if app.send(kind, args, tag).is_some() {
        st(app).toks.entry(tag).or_default().push_back(token);
    }
}

fn take_tok(app: &mut App, tag: &'static str) -> u64 {
    st(app)
        .toks
        .get_mut(tag)
        .and_then(VecDeque::pop_front)
        .unwrap_or(0)
}

fn failed_text(r: &Reply) -> String {
    r.error
        .clone()
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| t("review:inbox.transfers.failed"))
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "custody.sugg.suggest" => {
            let run = st(app).sq.pop_front().flatten();
            let s = st(app);
            s.loading = false;
            match &r.error {
                Some(_) => s.failed = true,
                None => {
                    s.failed = false;
                    s.live = Some(r.data["candidates"].as_array().cloned().unwrap_or_default());
                }
            }
            // A read after a finished run (and no later run) clears what was held for it.
            if let Some(run) = run
                && run == s.run
                && r.error.is_none()
            {
                s.frozen = None;
                s.hidden.clear();
                s.restored.clear();
            }
        }
        "custody.sugg.pair" | "custody.sugg.dismiss" => on_decision(app, r),
        "custody.sugg.dismiss_many" => on_dismiss_many(app, r),
        "custody.sugg.unpair" | "custody.sugg.undismiss" => on_undo_step(app, r),
        "custody.sugg.bulk.cases" | "custody.sugg.bulk.plan" | "custody.sugg.bulk.apply" => {
            on_bulk(app, tag, r)
        }
        _ => {}
    }
}

/// Escape closes this part's innermost overlay; true when it did.
pub fn escape(app: &mut App) -> bool {
    if st(app).popup.take().is_some() {
        return true;
    }
    if app.custody.sugg.bulk.is_some() {
        close_bulk(app);
        return true;
    }
    false
}

/// The journals were recalculated or a mutation refreshed the book: read again (a run that
/// just ended is read by `poll`).
pub fn reload(app: &mut App) {
    let s = &app.custody.sugg;
    if s.loaded_for.is_none() || s.was_refreshing {
        return;
    }
    load(app, None);
}

// ---- Decisions ---------------------------------------------------------------------------------

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
    let s = st(app);
    if on {
        let base: Vec<Value> = s
            .frozen
            .clone()
            .or_else(|| s.live.clone())
            .unwrap_or_default();
        for c in base {
            let k = candidate_key(&c);
            if keys.contains(&k) {
                s.decided.insert(k, c);
            }
        }
        for k in keys {
            s.hidden.insert(k.clone());
        }
    } else {
        let back: Vec<Value> = keys
            .iter()
            .filter_map(|k| s.decided.get(k).cloned())
            .collect();
        s.restored.extend(back);
        for k in keys {
            s.hidden.remove(k);
        }
    }
}

fn on_decided(app: &mut App) {
    super::page::on_suggestions_decided(app);
}

/// Pair or set aside one candidate (`decide`). A decision leaves the list at once and comes
/// back if it fails.
pub fn decide(app: &mut App, c: &Value, pair: bool) {
    st(app).error = None;
    let cands = app.custody.sugg.candidates();
    let keys: Vec<String> = if pair {
        // A paired leg is taken: the matches that competed for it go too.
        let (o, i) = (s(c, "out_id"), s(c, "in_id"));
        cands
            .iter()
            .filter(|x| {
                s(x, "out_id") == o
                    || s(x, "in_id") == i
                    || s(x, "out_id") == i
                    || s(x, "in_id") == o
            })
            .map(candidate_key)
            .collect()
    } else {
        vec![candidate_key(c)]
    };
    hide(app, &keys, true);
    // A decided row is no longer ticked, so an undo does not bring it back selected.
    st(app).selected.retain(|k| !keys.contains(k));
    let label = tr(
        if pair {
            "review:inbox.transfers.paired"
        } else {
            "review:inbox.transfers.dismissed"
        },
        &[("route", &route_label(c))],
    );
    let scope = app.expected_scope();
    let (kind, tag, args) = if pair {
        let (k, p) = model::candidate_terms(c);
        (
            "ui.transfers.pair",
            "custody.sugg.pair",
            json!({ "tx_out": s(c, "out_id"), "tx_in": s(c, "in_id"), "kind": k, "policy": p, "pair_source": "manual", "confidence_at_pair": c["confidence"], "expected_scope": scope }),
        )
    } else {
        (
            "ui.transfers.dismiss",
            "custody.sugg.dismiss",
            json!({ "tx_out": s(c, "out_id"), "tx_in": s(c, "in_id"), "reason": super::model::DISMISS_REASON, "expected_scope": scope }),
        )
    };
    if app.send(kind, args, tag).is_some() {
        let s = st(app);
        s.saving += 1;
        s.pending.push_back(Pending { pair, keys, label });
    } else {
        hide(app, &keys, false);
    }
}

fn on_decision(app: &mut App, r: &Reply) {
    let Some(p) = st(app).pending.pop_front() else {
        return;
    };
    let s = st(app);
    s.saving = s.saving.saturating_sub(1);
    match &r.error {
        Some(_) => {
            hide(app, &p.keys, false);
            st(app).error = Some(failed_text(r));
            load(app, None);
        }
        None => {
            let id = s_id(&r.data);
            let deadline = app.clock + UNDO_SECS;
            st(app).undo = (!id.is_empty()).then(|| Undo {
                decisions: vec![if p.pair {
                    Decision::Pair { id, keys: p.keys }
                } else {
                    Decision::Dismiss { id, keys: p.keys }
                }],
                label: p.label,
                deadline,
            });
            on_decided(app);
        }
    }
}

fn s_id(v: &Value) -> String {
    s(v, "id")
}

/// `dismissSelected`: every ticked suggestion still listed is set aside, with one Undo.
pub fn dismiss_selected(app: &mut App) {
    let chosen: Vec<Value> = app
        .custody
        .sugg
        .candidates()
        .into_iter()
        .filter(|c| app.custody.sugg.selected.contains(&candidate_key(c)))
        .collect();
    if chosen.is_empty() {
        return;
    }
    st(app).error = None;
    let keys: Vec<String> = chosen.iter().map(candidate_key).collect();
    hide(app, &keys, true);
    let s = st(app);
    s.selected.clear();
    s.saving += 1;
    s.many = Some(DismissMany {
        queue: chosen,
        keys,
        sent: 0,
        done: vec![],
    });
    dismiss_next(app);
}

fn dismiss_next(app: &mut App) {
    let next = app
        .custody
        .sugg
        .many
        .as_ref()
        .and_then(|m| m.queue.get(m.sent).cloned());
    match next {
        Some(c) => {
            let args = json!({ "tx_out": s(&c, "out_id"), "tx_in": s(&c, "in_id"), "reason": super::model::DISMISS_REASON, "expected_scope": app.expected_scope() });
            if app
                .send("ui.transfers.dismiss", args, "custody.sugg.dismiss_many")
                .is_none()
            {
                finish_many(app, Some(t("review:inbox.transfers.failed")));
            }
        }
        None => finish_many(app, None),
    }
}

fn on_dismiss_many(app: &mut App, r: &Reply) {
    if r.error.is_some() {
        return finish_many(app, Some(failed_text(r)));
    }
    if let Some(m) = st(app).many.as_mut() {
        let c = m.queue[m.sent].clone();
        m.sent += 1;
        let id = s(&r.data, "id");
        if !id.is_empty() {
            m.done.push(Decision::Dismiss {
                id,
                keys: vec![candidate_key(&c)],
            });
        }
    }
    dismiss_next(app);
}

fn finish_many(app: &mut App, error: Option<String>) {
    let Some(m) = st(app).many.take() else { return };
    if let Some(e) = error {
        // The ones not set aside come back to the list.
        hide(app, &m.keys[m.sent..], false);
        st(app).error = Some(e);
        load(app, None);
    }
    if !m.done.is_empty() {
        let n = m.done.len();
        let deadline = app.clock + UNDO_SECS;
        st(app).undo = Some(Undo {
            decisions: m.done,
            label: tc("review:inbox.transfers.dismissedMany", n, &[]),
            deadline,
        });
    }
    let s = st(app);
    s.saving = s.saving.saturating_sub(1);
    if m.sent > 0 {
        on_decided(app);
    }
}

/// `runUndo`: each decision on its own; a failure keeps the rest undoable.
pub fn run_undo(app: &mut App) {
    let Some(u) = st(app).undo.take() else { return };
    st(app).error = None;
    st(app).undo_run = Some(UndoRun {
        current: u,
        undone: vec![],
    });
    undo_next(app);
}

fn undo_next(app: &mut App) {
    let next = {
        let Some(run) = app.custody.sugg.undo_run.as_ref() else {
            return;
        };
        run.current.decisions.get(run.undone.len()).cloned()
    };
    let scope = app.expected_scope();
    match next {
        Some(Decision::Pair { id, .. }) => {
            app.send(
                "ui.transfers.unpair",
                json!({ "pair_id": id, "expected_scope": scope }),
                "custody.sugg.unpair",
            );
        }
        Some(Decision::Dismiss { id, .. }) => {
            app.send(
                "ui.transfers.dismissals.delete",
                json!({ "dismissal_id": id, "expected_scope": scope }),
                "custody.sugg.undismiss",
            );
        }
        None => finish_undo(app, None),
    }
}

fn on_undo_step(app: &mut App, r: &Reply) {
    if r.error.is_some() {
        return finish_undo(app, Some(failed_text(r)));
    }
    if let Some(run) = st(app).undo_run.as_mut() {
        let i = run.undone.len();
        if let Some(d) = run.current.decisions.get(i).cloned() {
            run.undone.push(d);
        }
    }
    undo_next(app);
}

fn finish_undo(app: &mut App, error: Option<String>) {
    let Some(run) = st(app).undo_run.take() else {
        return;
    };
    if let Some(e) = error {
        st(app).error = Some(e);
        let left: Vec<Decision> = run.current.decisions[run.undone.len()..].to_vec();
        let deadline = app.clock + UNDO_SECS;
        st(app).undo = Some(Undo {
            decisions: left,
            label: run.current.label.clone(),
            deadline,
        });
    }
    if !run.undone.is_empty() {
        let keys: Vec<String> = run.undone.iter().flat_map(|d| d.keys().to_vec()).collect();
        hide(app, &keys, false);
        on_decided(app);
    }
}

// ---- Bulk pairing of exact matches -------------------------------------------------------------

/// `prepareBulk`: capture the exact matches, then ask the core for its plan of pairing them,
/// bound to the book's input version.
pub fn prepare_bulk(app: &mut App) {
    st(app).error = None;
    let exact = model::bulk_pairable(&app.custody.sugg.candidates());
    let reason = t("review:inbox.transfers.bulk.reason");
    let ops: Vec<Value> = exact
        .iter()
        .map(|c| {
            let (k, p) = model::candidate_terms(c);
            json!({ "type": "pair", "out_transaction_id": s(c, "out_id"), "in_transaction_id": s(c, "in_id"), "pair_kind": k, "policy": p, "reason": reason })
        })
        .collect();
    let scope = app.expected_scope();
    let s = st(app);
    s.token += 1;
    let token = s.token;
    s.bulk = Some(Bulk {
        count: ops.len(),
        ops,
        preview: None,
        busy: true,
        applying: false,
        token,
    });
    if scope["workspace_id"].is_null() || scope["profile_id"].is_null() {
        let s = st(app);
        s.bulk = None;
        s.error = Some(t("review:inbox.transfers.bulk.none"));
        return;
    }
    send_tok(
        app,
        "ui.review.cases",
        json!({ "limit": 1, "expected_scope": scope }),
        "custody.sugg.bulk.cases",
        token,
    );
}

/// `pairAllExact`: applies exactly the plan shown, with its idempotency key.
pub fn pair_all_exact(app: &mut App) {
    let Some(b) = st(app).bulk.as_mut() else {
        return;
    };
    let Some((artifact, key)) = b.preview.clone() else {
        return;
    };
    b.busy = true;
    b.applying = true;
    let token = b.token;
    st(app).error = None;
    let scope =
        json!({ "workspace_id": artifact["workspace_id"], "profile_id": artifact["profile_id"] });
    send_tok(
        app,
        "ui.review.apply",
        json!({ "artifact": artifact, "idempotency_key": key, "expected_scope": scope }),
        "custody.sugg.bulk.apply",
        token,
    );
}

pub fn close_bulk(app: &mut App) {
    if app.custody.sugg.bulk.as_ref().is_some_and(|b| b.busy) {
        return;
    }
    st(app).bulk = None;
}

fn on_bulk(app: &mut App, tag: &str, r: &Reply) {
    let tag: &'static str = match tag {
        "custody.sugg.bulk.cases" => "custody.sugg.bulk.cases",
        "custody.sugg.bulk.plan" => "custody.sugg.bulk.plan",
        _ => "custody.sugg.bulk.apply",
    };
    let token = take_tok(app, tag);
    if app.custody.sugg.bulk.as_ref().map(|b| b.token) != Some(token) {
        return;
    }
    let fail = |app: &mut App, msg: String, close: bool| {
        let s = st(app);
        if close {
            s.bulk = None;
        } else if let Some(b) = s.bulk.as_mut() {
            b.busy = false;
            b.applying = false;
        }
        s.error = Some(msg);
    };
    match tag {
        "custody.sugg.bulk.cases" => {
            if r.error.is_some() {
                return fail(app, failed_text(r), true);
            }
            let scope = app.expected_scope();
            let ops = app
                .custody
                .sugg
                .bulk
                .as_ref()
                .map(|b| b.ops.clone())
                .unwrap_or_default();
            send_tok(
                app,
                "ui.review.plan",
                json!({ "operations": ops, "expected_input_version": r.data["input_version"], "expected_scope": scope }),
                "custody.sugg.bulk.plan",
                token,
            );
        }
        "custody.sugg.bulk.plan" => {
            if r.error.is_some() {
                return fail(app, failed_text(r), true);
            }
            if !model::artifact_ok(&r.data) {
                return fail(app, t("review:inbox.transfers.bulk.none"), true);
            }
            if let Some(b) = st(app).bulk.as_mut() {
                b.preview = Some((r.data.clone(), model::uuid()));
                b.busy = false;
            }
        }
        _ => {
            if r.error.is_some() {
                // Keep the same artifact and idempotency key if the answer was lost.
                return fail(app, failed_text(r), false);
            }
            let applied = r.data["operations"].as_array().cloned().unwrap_or_default();
            st(app).bulk = None;
            let decisions: Vec<Decision> = applied
                .iter()
                .map(|op| {
                    let res = &op["result"];
                    let ids: Vec<String> = res["transaction_ids"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .map(|x| x.as_str().unwrap_or("").to_owned())
                                .collect()
                        })
                        .unwrap_or_default();
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
            st(app).undo = Some(Undo {
                label: tc("review:inbox.transfers.pairedMany", applied.len(), &[]),
                decisions,
                deadline,
            });
            on_decided(app);
        }
    }
}

/// Scenes: takes candidates off the list as a decision would, without sending anything.
pub(crate) fn hide_for_scene(app: &mut App, keys: &[String]) {
    hide(app, keys, true);
}

// ---- Small state changes from the view ---------------------------------------------------------

pub fn toggle_selected(app: &mut App, key: &str) {
    let s = st(app);
    if !s.selected.remove(key) {
        s.selected.insert(key.to_owned());
    }
}

pub fn toggle_popup(app: &mut App, p: Popup) {
    let s = st(app);
    s.popup = if s.popup.as_ref() == Some(&p) {
        None
    } else {
        Some(p)
    };
}

pub fn close_popup(app: &mut App) {
    st(app).popup = None;
}

/// Named states for `--scene` (see `view::scenes`).
pub const SCENES: &[crate::app::Scene] = view::SCENES;
