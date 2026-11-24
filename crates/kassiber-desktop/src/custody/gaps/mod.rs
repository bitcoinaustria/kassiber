//! Missing wallets and scan coverage (`routes/CustodyGaps.tsx`, `custodyGapsModel.ts`): the
//! developer-gated advanced tools' "Missing wallets" and "Scan coverage" tabs.
//!
//! Missing wallets lists sends that may have come back through a wallet the book does not
//! know, verdict first. Each suggestion opens its guided review below it: why Kassiber
//! suggested it, its append-only history, and the actions the core allows (create a reviewed
//! bridge or set it aside; reopen and revise a bridge; classify a residual). Every action is
//! planned by the review core first (`ui.custody.review.plan`, a dry run) and applied only
//! after the owner confirms that exact plan (`ui.custody.review.apply` with the plan's
//! `expected_input_version`). Kassiber never closes a gap on its own. Scan coverage is a
//! diagnostic read that never decides anything. Every read is local.
//!
//! - `model`: the pure model (pages, action modes, plan/apply arguments, exact amounts).
//! - `view`: both tabs' rendering.

pub mod model;
mod view;

use rune::{Cx, El, TextEdit, Tx};
use serde_json::{Value, json};

use crate::app::{App, Reply};
use crate::custody::route::{Search, Tab, View};
use crate::i18n::t;
use crate::theme::Tk;

/// A read's state: the first fetch, then data kept through refetches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rd {
    #[default]
    Idle,
    Loading,
    Ready,
}

/// A plan the owner may confirm (`PendingReviewPlan`).
#[derive(Clone, Debug)]
pub struct Pending {
    /// `create`, `reopen`, `revise` or `classify_residual`.
    pub action: String,
    pub preview: Value,
    pub reason: String,
}

/// The guided review of one gap (`GapReviewDetails`).
pub struct Detail {
    pub gap_id: String,
    /// `ui.custody.gaps.review_context`: None while loading, Err with the message on failure.
    pub ctx: Option<Result<Value, String>>,
    /// `ui.custody.gaps.history`.
    pub history: Option<Result<Value, String>>,
    /// The plan in flight and the action it plans.
    pub planning: Option<String>,
    pub applying: bool,
    pub pending: Option<Pending>,
    /// The explicit confirmation checkbox of the shown plan.
    pub confirmed: bool,
    pub correction: TextEdit,
    pub residual_class: String,
    pub residual: TextEdit,
    pub error: Option<String>,
    /// A planned dismissal waiting for the owner's OK (`window.confirm`).
    pub dismiss: Option<Value>,
    pub select_open: bool,
    /// The reason the plan in flight was asked with.
    plan_reason: String,
}

impl Detail {
    pub(crate) fn new(gap_id: &str) -> Detail {
        Detail {
            gap_id: gap_id.to_owned(),
            ctx: None,
            history: None,
            planning: None,
            applying: false,
            pending: None,
            confirmed: false,
            correction: TextEdit::new(""),
            residual_class: "external_payment".into(),
            residual: TextEdit::new(""),
            error: None,
            dismiss: None,
            select_open: false,
            plan_reason: String::new(),
        }
    }
}

#[derive(Default)]
pub struct State {
    /// `ui.custody.gaps.list` pages read so far.
    pub pages: Vec<Value>,
    pub list_rd: Rd,
    pub list_failed: Option<String>,
    pub cursor: Option<String>,
    pub loading_more: bool,
    /// A refetch reads as many pages as were loaded.
    buf: Vec<Value>,
    refetch_left: usize,
    /// `ui.custody.coverage.snapshot`.
    pub coverage: Option<Value>,
    pub cov_rd: Rd,
    pub detail: Option<Detail>,
}

fn st(app: &mut App) -> &mut State {
    &mut app.custody.gaps
}

fn focus(app: &App) -> Option<String> {
    match crate::custody::current_view(app) {
        View::Advanced {
            tab: Tab::Gaps,
            gap,
        } => gap,
        _ => None,
    }
}

/// Starts the reads the gaps tab needs (called when the tab shows).
pub fn mount_gaps(app: &mut App) {
    if st(app).list_rd == Rd::Idle {
        read_list(app, false);
    }
    sync_detail(app);
}

/// Starts the reads the coverage tab needs.
pub fn mount_coverage(app: &mut App) {
    if st(app).cov_rd == Rd::Idle {
        read_coverage(app);
    }
}

fn read_list(app: &mut App, refetch: bool) {
    let s = st(app);
    if s.list_rd == Rd::Idle {
        s.list_rd = Rd::Loading;
    }
    s.buf.clear();
    s.refetch_left = if refetch {
        s.pages.len().saturating_sub(1)
    } else {
        0
    };
    app.send(
        "ui.custody.gaps.list",
        json!({"limit": 100}),
        "custody.gaps.list",
    );
}

fn read_coverage(app: &mut App) {
    if st(app).cov_rd == Rd::Idle {
        st(app).cov_rd = Rd::Loading;
    }
    app.send(
        "ui.custody.coverage.snapshot",
        Value::Null,
        "custody.gaps.coverage",
    );
}

/// `fetchNextPage`.
fn load_more(app: &mut App) {
    let Some(c) = st(app).cursor.clone() else {
        return;
    };
    st(app).loading_more = true;
    app.send(
        "ui.custody.gaps.list",
        json!({"limit": 100, "cursor": c}),
        "custody.gaps.more",
    );
}

/// The review details follow the gap the URL focuses (a fresh review per gap, as the web's
/// `key={gapId}` remount).
/// The gap under review on the page, once its review packet named it in the open book: its id
/// and the day it started (the assistant tray's open item).
pub fn open_gap(app: &App) -> Option<(String, Option<String>)> {
    if !crate::custody::owns(app.route.path()) {
        return None;
    }
    let id = focus(app)?;
    let d = app
        .custody
        .gaps
        .detail
        .as_ref()
        .filter(|d| d.gap_id == id)?;
    let Some(Ok(ctx)) = &d.ctx else {
        return None;
    };
    let g = ctx["gaps"].as_array()?.first()?;
    (g["gap_id"] == id.as_str()).then(|| (id, g["started_at"].as_str().map(str::to_owned)))
}

fn sync_detail(app: &mut App) {
    let f = focus(app);
    let current = st(app).detail.as_ref().map(|d| d.gap_id.clone());
    if f == current {
        return;
    }
    st(app).detail = f.as_deref().map(Detail::new);
    if f.is_some() {
        read_detail(app);
    }
}

fn read_detail(app: &mut App) {
    let Some(id) = st(app).detail.as_ref().map(|d| d.gap_id.clone()) else {
        return;
    };
    app.send(
        "ui.custody.gaps.review_context",
        json!({"gap_id": id}),
        "custody.gaps.ctx",
    );
    app.send(
        "ui.custody.gaps.history",
        json!({"gap_id": id, "limit": 100}),
        "custody.gaps.history",
    );
}

/// Opens or closes a gap's review (`onFocusGap`).
pub fn toggle_focus(app: &mut App, gap_id: &str) {
    let open = focus(app).as_deref() == Some(gap_id);
    crate::custody::select(
        app,
        Search {
            advanced: true,
            tab: Some(Tab::Gaps),
            gap: (!open).then(|| gap_id.to_owned()),
            ..Search::default()
        },
    );
}

fn msg(r: &Reply, fallback: &str) -> String {
    r.error
        .clone()
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| t(fallback))
}

pub fn poll(app: &mut App, _tx: &mut Tx) -> bool {
    if !crate::custody::owns(app.route.path()) {
        return false;
    }
    let before = st(app).detail.as_ref().map(|d| d.gap_id.clone());
    if matches!(
        crate::custody::current_view(app),
        View::Advanced { tab: Tab::Gaps, .. }
    ) {
        sync_detail(app);
    }
    before != st(app).detail.as_ref().map(|d| d.gap_id.clone())
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    let ok = r.error.is_none();
    match tag {
        "custody.gaps.list" => {
            let s = st(app);
            if !ok {
                s.list_failed = Some(msg(r, "custodyGaps:unavailable.body"));
                if s.list_rd == Rd::Loading {
                    s.list_rd = Rd::Ready;
                }
                return;
            }
            s.buf.push(r.data.clone());
            let cursor = r.data["next_cursor"]
                .as_str()
                .filter(|c| !c.is_empty())
                .map(str::to_owned);
            if s.refetch_left > 0 && cursor.is_some() {
                s.refetch_left -= 1;
                app.send(
                    "ui.custody.gaps.list",
                    json!({"limit": 100, "cursor": cursor}),
                    "custody.gaps.list",
                );
                return;
            }
            s.pages = std::mem::take(&mut s.buf);
            s.cursor = cursor;
            s.list_failed = None;
            s.list_rd = Rd::Ready;
        }
        "custody.gaps.more" => {
            let s = st(app);
            s.loading_more = false;
            if ok {
                s.cursor = r.data["next_cursor"]
                    .as_str()
                    .filter(|c| !c.is_empty())
                    .map(str::to_owned);
                s.pages.push(r.data.clone());
            }
        }
        "custody.gaps.coverage" => {
            let s = st(app);
            s.cov_rd = Rd::Ready;
            // A failed refetch keeps what was read (react-query keeps the data).
            if ok {
                s.coverage = r.data.is_object().then(|| r.data.clone());
            }
        }
        "custody.gaps.ctx" => {
            if let Some(d) = st(app).detail.as_mut() {
                d.ctx = Some(if ok && r.data.is_object() {
                    Ok(r.data.clone())
                } else {
                    Err(r
                        .error
                        .clone()
                        .filter(|e| !e.is_empty())
                        .unwrap_or_else(|| t("custodyGaps:review.unavailable")))
                });
            }
        }
        "custody.gaps.history" => {
            if let Some(d) = st(app).detail.as_mut() {
                d.history = Some(if ok && r.data.is_object() {
                    Ok(r.data.clone())
                } else {
                    Err(String::new())
                });
            }
        }
        "custody.gaps.plan" => {
            let Some(d) = st(app).detail.as_mut() else {
                return;
            };
            let action = d.planning.take().unwrap_or_default();
            if !ok {
                d.error = Some(msg(r, "custodyGaps:actions.failed"));
                return;
            }
            if r.data.is_null() {
                return;
            }
            if action == "dismiss" {
                d.dismiss = Some(r.data.clone());
            } else {
                d.pending = Some(Pending {
                    action,
                    preview: r.data.clone(),
                    reason: std::mem::take(&mut d.plan_reason),
                });
                d.confirmed = false;
            }
        }
        "custody.gaps.apply" => {
            // The panel may have been collapsed meanwhile; the review is applied either way.
            if let Some(d) = st(app).detail.as_mut() {
                d.applying = false;
                if !ok {
                    d.error = Some(msg(r, "custodyGaps:actions.failed"));
                } else {
                    if let Some(p) = d.pending.take() {
                        match p.action.as_str() {
                            "reopen" | "revise" => d.correction = TextEdit::new(""),
                            "classify_residual" => d.residual = TextEdit::new(""),
                            _ => {}
                        }
                    }
                    d.confirmed = false;
                }
            }
            if ok {
                // `ui.custody.review.apply` changed the gaps, their review context and history,
                // the lineage, components, journals and report blockers: every one re-reads,
                // whether or not the review panel is still open.
                app.refresh_reads();
                crate::custody::invalidate(app);
            }
        }
        _ => {}
    }
}

/// Plans an action with the review core (a dry run; nothing changes).
pub fn plan(app: &mut App, action: &str) {
    let scope = app.expected_scope();
    let Some(d) = st(app).detail.as_mut() else {
        return;
    };
    if d.planning.is_some() {
        return;
    }
    d.error = None;
    if action != "dismiss" {
        d.pending = None;
    }
    let reason = match action {
        "reopen" | "revise" => d.correction.text.trim().to_owned(),
        "classify_residual" => d.residual.text.trim().to_owned(),
        _ => String::new(),
    };
    let class = (action == "classify_residual").then(|| d.residual_class.clone());
    let mut args = model::plan_args(action, &d.gap_id, &reason, class.as_deref());
    args["expected_scope"] = scope;
    d.planning = Some(action.to_owned());
    d.plan_reason = reason;
    app.send("ui.custody.review.plan", args, "custody.gaps.plan");
}

/// Applies exactly the plan the owner confirmed.
pub fn confirm(app: &mut App) {
    let scope = app.expected_scope();
    let Some(d) = st(app).detail.as_mut() else {
        return;
    };
    let Some(p) = d.pending.as_ref() else { return };
    if d.applying || !d.confirmed || !model::confirmable(&p.action, &p.preview) {
        return;
    }
    let mut args = model::apply_args(&p.action, &p.preview, &p.reason);
    args["expected_scope"] = scope;
    d.error = None;
    d.applying = true;
    app.send("ui.custody.review.apply", args, "custody.gaps.apply");
}

/// The owner said OK to the planned dismissal.
pub fn confirm_dismiss(app: &mut App) {
    let scope = app.expected_scope();
    let Some(d) = st(app).detail.as_mut() else {
        return;
    };
    let Some(p) = d.dismiss.take() else { return };
    d.error = None;
    d.applying = true;
    let args = json!({"action": "dismiss", "gap_id": d.gap_id, "expected_input_version": p["input_version"], "expected_scope": scope});
    app.send("ui.custody.review.apply", args, "custody.gaps.apply");
}

/// Escape closes this part's innermost overlay; true when it did.
pub fn escape(app: &mut App) -> bool {
    let Some(d) = st(app).detail.as_mut() else {
        return false;
    };
    if d.select_open {
        d.select_open = false;
        return true;
    }
    d.dismiss.take().is_some()
}

/// The journals or the book changed: read again what was read.
pub fn reload(app: &mut App) {
    if st(app).list_rd != Rd::Idle {
        read_list(app, true);
    }
    if st(app).cov_rd != Rd::Idle {
        read_coverage(app);
    }
    if st(app).detail.is_some() {
        read_detail(app);
    }
}

/// `CustodyGapsContent`. `focus_gap`: the gap the URL names.
pub fn content(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    focus_gap: Option<&str>,
) -> El {
    let page = view::content(app, cx, tk, layer, focus_gap);
    view::dismiss_dialog(app, tk, layer);
    page
}

/// `CustodyCoverageSection`.
pub fn coverage(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let _ = (cx, layer);
    view::coverage(app, tk)
}

fn scene_gaps(a: &mut App, q: &str) {
    a.prefs.developer_tools = true;
    a.navigate(&format!("/transfers?section=advanced&tab=gaps{q}"));
    crate::custody::prime(a);
    // The review's reads start from the focus.
    sync_detail(a);
    crate::custody::settle(a);
}

fn first_gap(a: &App) -> Option<String> {
    model::collect_pages(&a.custody.gaps.pages)
        .into_iter()
        .find(|g| g.in_queue())
        .map(|g| g.gap_id)
}

fn focused(a: &mut App) {
    scene_gaps(a, "");
    if let Some(id) = first_gap(a) {
        scene_gaps(a, &format!("&gap={id}"));
    }
}

/// The plan of the focused gap's bridge (a dry run: the core changes nothing).
fn focused_plan(a: &mut App) {
    focused(a);
    plan(a, "create");
    crate::custody::settle(a);
}

/// Display-only fixtures for states the demo book cannot reach (every gap there is a weak,
/// non-activatable hint, so no bridge can be created and none is resolved): the focused gap's
/// review context is rewritten as a resolved bridge with a residual, and a plan as the core
/// returns one is shown. Nothing is sent to the core for them.
fn fixture(a: &mut App, action: &str) {
    focused(a);
    let Some(d) = a.custody.gaps.detail.as_mut() else {
        return;
    };
    let Some(Ok(ctx)) = d.ctx.as_mut() else {
        return;
    };
    let gap = &mut ctx["gaps"][0];
    gap["status"] = json!("resolved");
    gap["status_reason"] = json!("bridge_created");
    gap["residual_msat"] = json!(148604000);
    let preview = match action {
        "reopen" => {
            json!({"gap_id": gap["gap_id"], "input_version": 1, "dry_run": true, "requires_explicit_confirmation": true, "current_component_revision": 1, "filed_report_impacts": []})
        }
        _ => {
            json!({"gap_id": gap["gap_id"], "input_version": 1, "dry_run": true, "requires_explicit_confirmation": true, "activatable": true, "classification": "external_gift", "custody_state": "external_confirmed", "country_tax_meaning": "not_assigned", "residual_msat": 148604000, "current_component_revision": 1, "new_component_revision": 2, "filed_report_impacts": []})
        }
    };
    if action != "form" {
        d.residual_class = if action == "reopen" {
            "external_payment".into()
        } else {
            "external_gift".into()
        };
        d.pending = Some(Pending {
            action: action.to_owned(),
            preview,
            reason: String::new(),
        });
    }
}

pub const SCENES: &[crate::app::Scene] = &[
    ("custody-gaps-fixture-resolved", |a| fixture(a, "form")),
    ("custody-gaps-fixture-reopen", |a| fixture(a, "reopen")),
    ("custody-gaps-fixture-residual", |a| {
        fixture(a, "classify_residual")
    }),
    ("custody-gaps", |a| scene_gaps(a, "")),
    ("custody-gaps-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_gaps(a, "");
    }),
    ("custody-gaps-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        crate::i18n::set_lang(crate::i18n::Lang::De);
        scene_gaps(a, "");
    }),
    ("custody-gaps-review", focused),
    ("custody-gaps-plan", focused_plan),
    ("custody-gaps-plan-confirmed", |a| {
        focused_plan(a);
        if let Some(d) = a.custody.gaps.detail.as_mut() {
            d.confirmed = true;
        }
    }),
    ("custody-gaps-dismiss", |a| {
        focused(a);
        plan(a, "dismiss");
        crate::custody::settle(a);
    }),
    ("custody-gaps-review-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        crate::i18n::set_lang(crate::i18n::Lang::De);
        focused_plan(a);
    }),
    ("custody-gaps-coverage", |a| {
        a.prefs.developer_tools = true;
        a.navigate("/transfers?section=advanced&tab=coverage");
        crate::custody::prime(a);
    }),
    ("custody-gaps-coverage-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        a.prefs.developer_tools = true;
        a.navigate("/transfers?section=advanced&tab=coverage");
        crate::custody::prime(a);
    }),
];
