//! Custody components (`routes/transfers/CustodyComponentResolver.tsx` and
//! `routes/transfers-custody/**`): the developer-gated advanced tools' "Components" tab. The
//! book's custody components (versioned interpretations of histories a simple link can't
//! describe), each with its legs, allocations, audit and validation on demand, and the actions
//! its state allows (edit or revise, activate, retire, supersede, undo). New components and
//! revisions are authored in the guided builder (`form`), whose daemon dry run is shown in full
//! before the one confirmation that applies it.
//!
//! - `model`: the guided form's model and the local validator, with their tests.
//! - `form`: the guided builder (fields, live validation, server preview, plan → apply).

pub mod form;
pub mod model;

use std::collections::HashSet;

use rune::{Color, Cx, El, TextEdit, Tx, col, el, row, text};
use serde_json::{Value, json};

use crate::app::{App, Reply};
use crate::custody::kit::{self, Var};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

const PAGE: usize = 25;

/// Tailwind v4 colours this tab uses beyond `kit::tw`'s set.
pub fn tw(name: &str) -> rune::Color {
    let (l, c, h) = match name {
        "orange-300" => (0.837, 0.128, 66.29),
        "orange-400" => (0.75, 0.183, 55.934),
        "emerald-100" => (0.95, 0.052, 163.051),
        "emerald-200" => (0.905, 0.093, 164.15),
        "emerald-500" => (0.696, 0.17, 162.48),
        "emerald-950" => (0.262, 0.051, 172.552),
        "violet-300" => (0.811, 0.111, 293.571),
        "violet-500" => (0.606, 0.25, 292.717),
        "amber-100" => (0.962, 0.059, 95.617),
        "amber-950" => (0.279, 0.077, 45.635),
        "cyan-500" => (0.715, 0.143, 215.221),
        _ => return crate::custody::kit::tw(name),
    };
    rune::oklch(l, c, h)
}

/// A select of the guided form, by what it picks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sel {
    Type,
    Mode,
    LegRole(String),
    LegLocation(String),
    AllocSource(String),
    AllocSink(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Popup {
    Select(Sel),
}

/// A component action (`mutateComponent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Activate,
    Retire,
    Supersede,
    Undo,
}

/// A component action's server plan, shown until the owner confirms or cancels it.
pub struct Confirm {
    pub id: String,
    pub action: Action,
    /// The plan request's arguments; confirming sends them again pinned to the plan.
    pub args: Value,
    /// The daemon's dry run (`ui.transfers.components.plan`), rendered as returned.
    pub plan: Value,
    pub input_version: i64,
    /// The book the plan was made in (`expected_scope`).
    pub scope: String,
    /// The apply is out (the dialog stays open, busy, until it answers).
    pub applying: bool,
}

#[derive(Default)]
pub struct State {
    pub components: Option<Vec<Value>>,
    pub has_more: bool,
    pub limit: Option<u64>,
    /// The first read is out (`isLoading`).
    pub loading: bool,
    /// The last read failed: its error, as the web words it.
    pub load_error: Option<String>,
    pub visible: usize,
    pub expanded: HashSet<String>,
    pub action_error: Option<String>,
    /// The component an action is running for, and its plan args while the plan is out.
    pub pending: Option<(String, Action, Value)>,
    /// The book the plan in flight was asked in.
    plan_scope: String,
    /// The action's plan, awaiting the owner's explicit confirmation.
    pub confirm: Option<Confirm>,
    /// The revision editor's component (`editingComponent`).
    pub editing: Option<Value>,
    pub creating: bool,
    pub form: Option<form::FormUi>,
    pub popup: Option<Popup>,
    /// A field that was removed while its input still had focus edits this instead.
    pub scratch: TextEdit,
    mounted: bool,
}

fn st(app: &App) -> &State {
    &app.custody.comps
}

/// Starts the reads the tab needs (called when it shows).
pub fn mount(app: &mut App) {
    if !app.custody.comps.mounted || app.custody.comps.components.is_none() {
        app.custody.comps.mounted = true;
        read(app);
    }
}

fn read(app: &mut App) {
    let s = &mut app.custody.comps;
    if s.components.is_none() {
        s.loading = true;
    }
    app.send(
        "ui.transfers.components.list",
        json!({"limit": 1000}),
        "custody.comps.list",
    );
}

/// The journals or the book changed: read again (only while the tab has been shown).
pub fn reload(app: &mut App) {
    if app.custody.comps.mounted {
        read(app);
    }
}

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    // A plan is only confirmable in the book it was made in.
    let mut changed = false;
    let scope = app.expected_scope().to_string();
    if app
        .custody
        .comps
        .confirm
        .as_ref()
        .is_some_and(|c| !c.applying && c.scope != scope)
    {
        app.custody.comps.confirm = None;
        changed = true;
    }
    if let Some(f) = app.custody.comps.form.as_mut()
        && f.reveal_review
        && f.reviewed.is_some()
    {
        f.reveal_review = false;
        tx.scroll_into_view("cc-review");
    }
    form::poll(app) || changed
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "custody.comps.list" => {
            let s = &mut app.custody.comps;
            s.loading = false;
            match &r.error {
                Some(_) => s.load_error = Some(model::mutation_error(&r.error_detail)),
                None => {
                    s.load_error = None;
                    s.components =
                        Some(r.data["components"].as_array().cloned().unwrap_or_default());
                    s.has_more = r.data["has_more"].as_bool().unwrap_or(false);
                    s.limit = r.data["limit"].as_u64();
                }
            }
        }
        // The plan is only shown: nothing applies until the owner confirms it (`confirm`).
        "custody.comps.act_plan" => {
            let scope = app.expected_scope().to_string();
            let s = &mut app.custody.comps;
            let Some((id, action, args)) = s.pending.take() else {
                return;
            };
            if r.error.is_some() {
                s.action_error = Some(model::mutation_error(&r.error_detail));
                return;
            }
            if s.plan_scope != scope {
                s.action_error = Some(t("review:swap.components.form.previewChanged"));
                return;
            }
            let Some(input_version) = plan_version(&args, &id, &r.data) else {
                s.action_error = Some(t("review:swap.components.backendError.unexpected"));
                return;
            };
            s.confirm = Some(Confirm {
                id,
                action,
                args,
                plan: r.data.clone(),
                input_version,
                scope,
                applying: false,
            });
        }
        "custody.comps.act_apply" => {
            app.custody.comps.confirm = None;
            let action = app.custody.comps.pending.take().map(|p| p.1);
            if r.error.is_some() {
                app.custody.comps.action_error = Some(model::mutation_error(&r.error_detail));
                return;
            }
            // Undo brings the prior revision back as a draft: open it in the editor.
            if action == Some(Action::Undo) {
                let c = r.data["component"].clone();
                if c.is_object()
                    && model::is_guided_editable(c["component_type"].as_str().unwrap_or(""))
                {
                    open_editor(app, &c);
                }
            }
            app.refresh_reads();
            crate::custody::invalidate(app);
        }
        t if t.starts_with("custody.comps.form_") => form::on_reply(app, t, r),
        _ => {}
    }
}

/// Escape closes the innermost overlay: a select, an action's confirmation, then the editor
/// (neither mid-submit).
pub fn escape(app: &mut App) -> bool {
    let s = &mut app.custody.comps;
    if s.popup.take().is_some() {
        return true;
    }
    if s.confirm.is_some() {
        cancel_action(app);
        return true;
    }
    if s.form.is_some() {
        close_form_guarded(app);
        return true;
    }
    false
}

pub fn toggle(app: &mut App, p: Popup) {
    let s = &mut app.custody.comps;
    s.popup = if s.popup.as_ref() == Some(&p) {
        None
    } else {
        Some(p)
    };
}

pub fn close_popup(app: &mut App) {
    app.custody.comps.popup = None;
}

/// Closes the editor or creator (`closeRevisionEditor` / `setCreating(false)`).
pub fn close_form(app: &mut App) {
    let s = &mut app.custody.comps;
    s.editing = None;
    s.creating = false;
    s.form = None;
    s.popup = None;
}

/// The dialogs refuse to close while their form submits.
fn close_form_guarded(app: &mut App) {
    let busy = app.custody.comps.form.as_ref().is_some_and(|f| f.pending())
        || app.custody.comps.pending.is_some();
    if !busy {
        close_form(app);
    }
}

/// `openRevisionEditor`.
pub fn open_editor(app: &mut App, c: &Value) {
    match model::component_to_form(c) {
        Ok(f) => {
            let state = if c["state"] == "active" {
                "active"
            } else {
                "draft"
            };
            let s = &mut app.custody.comps;
            s.form = Some(form::FormUi::new(
                f,
                Some((c["id"].as_str().unwrap_or("").to_owned(), state.to_owned())),
                true,
            ));
            s.editing = Some(c.clone());
            s.creating = false;
            s.action_error = None;
        }
        Err(()) => {
            app.custody.comps.action_error = Some(t("review:swap.components.form.unsupportedShape"))
        }
    }
}

/// "New component".
pub fn open_creator(app: &mut App) {
    let s = &mut app.custody.comps;
    s.form = Some(form::FormUi::new(model::initial_form(), None, false));
    s.creating = true;
    s.editing = None;
}

/// `mutateComponent`: ask the daemon for the action's plan. Its reply is shown for explicit
/// confirmation (`confirm_action`); only that applies it, at the planned input version.
fn mutate(app: &mut App, c: &Value, action: Action) {
    if app.custody.comps.pending.is_some() || app.custody.comps.confirm.is_some() {
        return;
    }
    let id = c["id"].as_str().unwrap_or("").to_owned();
    let args = match action {
        Action::Activate => json!({"action": "activate", "component_id": id}),
        Action::Retire => {
            json!({"action": "supersede", "component_id": id, "reason": "desktop_custody_review_retire"})
        }
        Action::Supersede => {
            json!({"action": "supersede", "component_id": id, "reason": "desktop_custody_review_supersede"})
        }
        Action::Undo => {
            json!({"action": "undo", "component_id": id, "reason": "desktop_custody_review_undo"})
        }
    };
    app.custody.comps.action_error = None;
    let mut send = args.clone();
    send["expected_scope"] = app.expected_scope();
    app.custody.comps.plan_scope = app.expected_scope().to_string();
    if app
        .send(
            "ui.transfers.components.plan",
            send,
            "custody.comps.act_plan",
        )
        .is_some()
    {
        app.custody.comps.pending = Some((id, action, args));
    }
}

/// The plan's input version, when the plan answers the action that was asked for this component.
fn plan_version(args: &Value, id: &str, plan: &Value) -> Option<i64> {
    let planned_id = if args["action"] == "undo" {
        &plan["source_component_id"]
    } else {
        &plan["component_id"]
    };
    (plan["action"] == args["action"]
        && planned_id.as_str() == Some(id)
        && plan["component"].is_object())
    .then(|| plan["input_version"].as_i64())
    .flatten()
    .filter(|v| (0..=9_007_199_254_740_991).contains(v))
}

/// The apply of a shown plan: its own request, pinned to the plan's input version and the book.
fn apply_args(c: &Confirm, expected_scope: Value) -> Value {
    let mut apply = c.args.clone();
    apply["expected_input_version"] = json!(c.input_version);
    apply["expected_scope"] = expected_scope;
    apply
}

/// The owner confirmed the shown plan: apply exactly it, pinned to its input version and book.
/// The daemon recomputes the effects and refuses if the book or its input changed meanwhile.
pub fn confirm_action(app: &mut App) {
    let expected_scope = app.expected_scope();
    let scope = expected_scope.to_string();
    let s = &mut app.custody.comps;
    let Some(c) = s.confirm.as_mut() else { return };
    if c.applying || s.pending.is_some() {
        return;
    }
    if c.scope != scope {
        s.confirm = None;
        s.action_error = Some(t("review:swap.components.form.previewChanged"));
        return;
    }
    let apply = apply_args(c, expected_scope);
    c.applying = true;
    let pending = (c.id.clone(), c.action, c.args.clone());
    app.custody.comps.pending = Some(pending);
    app.send(
        "ui.transfers.components.apply",
        apply,
        "custody.comps.act_apply",
    );
}

/// Cancel (or Escape, or the backdrop) discards the plan; not while it applies.
pub fn cancel_action(app: &mut App) {
    let s = &mut app.custody.comps;
    if s.confirm.as_ref().is_some_and(|c| !c.applying) {
        s.confirm = None;
    }
}

fn compact_id(v: &str) -> String {
    if v.is_empty() {
        return "—".into();
    }
    if v.chars().count() <= 22 {
        return v.to_owned();
    }
    let h: String = v.chars().take(12).collect();
    let tl: String = v.chars().skip(v.chars().count() - 6).collect();
    format!("{h}…{tl}")
}

/// A label (`crate::ui::tag`): the text ink for `default`, muted otherwise.
fn badge(tk: &Tk, label: String, variant: &str) -> El {
    let fg = if variant == "default" {
        tk.fg
    } else {
        tk.muted_fg
    };
    crate::ui::tag(tk, fg, None, label)
}

/// The amber box of backend validation issues (`validationIssues`, the review's issues).
pub fn issue_box(tk: &Tk, title: &str, lines: Vec<String>) -> El {
    let (fg, bg, border) = if tk.dark {
        (
            tw("amber-100"),
            tw("amber-950").alpha(0.3),
            tw("amber-400").alpha(0.3),
        )
    } else {
        (tw("amber-950"), tw("amber-50"), tw("amber-300").alpha(0.6))
    };
    let mut list = col().w_full().gap(tk.r(0.25)).pl(tk.r(1.0)).mt(tk.r(0.25));
    for l in lines {
        list = list.child(
            row()
                .w_full()
                .gap(tk.r(0.375))
                .items_start()
                .child(text("•").font(tk.txs(400.0)).color(fg).a11y_hidden())
                .child(
                    text(l)
                        .font(tk.txs(400.0))
                        .color(fg)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }
    col()
        .w_full()
        .padding(tk.r(0.75))
        .radius(tk.r(0.375))
        .border(1.0, border)
        .fill(bg)
        .child(text(title.to_owned()).font(tk.txs(500.0)).color(fg))
        .child(list)
}

/// A destructive box (`rounded-md border-destructive/40 bg-destructive/10 p-3 text-sm`).
fn error_box(tk: &Tk, key: &str, s: String) -> El {
    col()
        .key(key.to_owned())
        .w_full()
        .padding(tk.r(0.75))
        .radius(tk.r(0.375))
        .border(1.0, tk.destructive.alpha(0.4))
        .fill(tk.destructive.alpha(0.1))
        .role("alert")
        .child(text(s).font(tk.tsm(400.0)).color(tk.destructive).wrap())
}

/// One component's row (`rounded-lg border p-4`).
fn component_row(app: &App, tk: &Tk, c: &Value, wide: bool) -> El {
    let id = c["id"].as_str().unwrap_or("").to_owned();
    let ctype = c["component_type"].as_str().unwrap_or("");
    let state = c["state"].as_str().unwrap_or("");
    let effective = c["effective_state"].as_str().unwrap_or(state);
    let expanded = st(app).expanded.contains(&id);
    let busy_any =
        st(app).pending.is_some() || app.custody.comps.form.as_ref().is_some_and(|f| f.pending());
    let pending_here = st(app).pending.as_ref().is_some_and(|p| p.0 == id);
    let legs = c["legs"].as_array().cloned().unwrap_or_default();
    let allocs = c["allocations"].as_array().cloned().unwrap_or_default();
    let issues = c["validation"]["issues"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let variant = if effective == "active" {
        "default"
    } else if state == "superseded" {
        "outline"
    } else {
        "secondary"
    };
    let title = row()
        .gap(tk.r(0.5))
        .center_y()
        .flex_wrap()
        .child(
            text(model::t_or(
                &format!("review:swap.components.type.{ctype}"),
                ctype,
            ))
            .font(tk.tsm(500.0))
            .color(tk.fg),
        )
        .child(badge(
            tk,
            t(&format!("review:swap.components.state.{effective}")),
            variant,
        ))
        .child(
            text(tc(
                "review:swap.components.revision",
                c["revision"].as_u64().unwrap_or(0) as usize,
                &[],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        );
    let idk = id.clone();
    let toggle_btn = row()
        .key(format!("cc-expand-{id}"))
        .mt(tk.r(0.25))
        .gap(tk.r(0.25))
        .center_y()
        .role("button")
        .expanded(expanded)
        .label(tc("review:swap.components.legCount", legs.len(), &[]))
        .focusable(true)
        .hover_group()
        .on_press(move |a: &mut App| {
            let e = &mut a.custody.comps.expanded;
            if !e.remove(&idk) {
                e.insert(idk.clone());
            }
        })
        .child(
            ic(
                if expanded {
                    &I::CHEVRON_DOWN
                } else {
                    &I::CHEVRON_RIGHT
                },
                tk.r(0.875),
                tk.muted_fg,
            )
            .group_color(tk.fg),
        )
        .child(
            text(tc("review:swap.components.legCount", legs.len(), &[]))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .group_color(tk.fg),
        );
    let mut left = col()
        .min_w(0.0)
        .grow(1.0)
        .child(title)
        .child(
            text(compact_id(&id))
                .font(tk.txs(400.0).mono())
                .color(tk.muted_fg)
                .mt(tk.r(0.25)),
        )
        .child(row().child(toggle_btn));
    if expanded {
        let mode = c["conservation_mode"].as_str().unwrap_or("");
        let source = c["authored_source"].as_str().unwrap_or("");
        let mut audit = row()
            .mt(tk.r(0.5))
            .gap(tk.r(0.375))
            .flex_wrap()
            .child(badge(
                tk,
                tr(
                    "review:swap.components.audit.mode",
                    &[("mode", &t(&format!("review:swap.components.mode.{mode}")))],
                ),
                "outline",
            ))
            .child(badge(
                tk,
                tr(
                    "review:swap.components.audit.authoredSource",
                    &[(
                        "source",
                        &model::t_or(
                            &format!("review:swap.components.authoredSource.{source}"),
                            source,
                        ),
                    )],
                ),
                "outline",
            ));
        if c["evidence_kind"].is_string() || c["evidence_grade"].is_string() {
            audit = audit.child(badge(
                tk,
                tr(
                    "review:swap.components.audit.evidence",
                    &[
                        ("kind", c["evidence_kind"].as_str().unwrap_or("—")),
                        ("grade", c["evidence_grade"].as_str().unwrap_or("—")),
                    ],
                ),
                "outline",
            ));
        }
        if mode == "conversion" {
            let policy = c["conversion_policy"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| t("review:swap.components.audit.missing"));
            let reviewed = c["conversion_reviewed"].as_bool().unwrap_or(false);
            audit = audit
                .child(badge(
                    tk,
                    tr(
                        "review:swap.components.audit.policy",
                        &[("policy", &policy)],
                    ),
                    "outline",
                ))
                .child(badge(
                    tk,
                    tr(
                        "review:swap.components.audit.reviewed",
                        &[(
                            "value",
                            &t(if reviewed {
                                "review:swap.components.audit.yes"
                            } else {
                                "review:swap.components.audit.no"
                            }),
                        )],
                    ),
                    if reviewed { "default" } else { "secondary" },
                ));
        }
        left = left.child(audit);
    }
    if let Some(n) = c["notes"].as_str().filter(|n| !n.is_empty()) {
        left = left.child(
            text(n.to_owned())
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .mt(tk.r(0.5)),
        );
    }
    // Actions by state.
    let mut actions = row().gap(tk.r(0.5)).flex_wrap();
    let spin = |glyph: &'static rune::Icon| if pending_here { None } else { Some(glyph) };
    let lead_spinner = |b: El, fg: Color| {
        if pending_here {
            b.child(kit::spinner(tk, tk.r(1.0), fg))
        } else {
            b
        }
    };
    let _ = lead_spinner;
    if (state == "draft" || state == "active") && model::is_guided_editable(ctype) {
        let cc = c.clone();
        actions = actions.child(
            kit::btn(
                tk,
                format!("cc-edit-{id}"),
                Var::Outline,
                Some(&I::PENCIL),
                &t(if state == "draft" {
                    "review:swap.components.editDraft"
                } else {
                    "review:swap.components.revise"
                }),
                !busy_any,
            )
            .on_press(move |a: &mut App| open_editor(a, &cc)),
        );
    }
    let act_btn = |key: String,
                   variant: Var,
                   glyph: &'static rune::Icon,
                   label: String,
                   enabled: bool,
                   c: Value,
                   action: Action| {
        kit::btn_busy(tk, key, variant, pending_here, spin(glyph), &label, enabled)
            .on_press(move |a: &mut App| mutate(a, &c, action))
    };
    match state {
        "draft" => {
            let activatable = c["validation"]["activatable"].as_bool() != Some(false);
            actions = actions
                .child(act_btn(
                    format!("cc-activate-{id}"),
                    Var::Default,
                    &I::CHECK,
                    t("review:swap.components.activate"),
                    !busy_any && activatable,
                    c.clone(),
                    Action::Activate,
                ))
                .child(act_btn(
                    format!("cc-retire-{id}"),
                    Var::Outline,
                    &I::TRASH_2,
                    t("review:swap.components.retireDraft"),
                    !busy_any,
                    c.clone(),
                    Action::Retire,
                ));
        }
        "active" => {
            actions = actions.child(act_btn(
                format!("cc-supersede-{id}"),
                Var::Outline,
                &I::UNLINK,
                t("review:swap.components.supersede"),
                !busy_any,
                c.clone(),
                Action::Supersede,
            ))
        }
        "superseded" => {
            actions = actions.child(act_btn(
                format!("cc-undo-{id}"),
                Var::Outline,
                &I::UNDO_2,
                t("review:swap.components.undo"),
                !busy_any,
                c.clone(),
                Action::Undo,
            ))
        }
        _ => {}
    }
    let mut card = col()
        .key(format!("cc-component-{id}"))
        .w_full()
        .padding(tk.r(1.0))
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .role("group")
        .label(model::t_or(
            &format!("review:swap.components.type.{ctype}"),
            ctype,
        ))
        .child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .items_start()
                .flex_wrap()
                .child(left)
                .child(actions),
        );
    if expanded {
        // Legs in `md:grid-cols-2 xl:grid-cols-3`.
        let cols = if wide { 3 } else { 2 };
        let mut grid = col().w_full().mt(tk.r(0.75)).gap(tk.r(0.5));
        for chunk in legs.chunks(cols) {
            let mut line = row().w_full().gap(tk.r(0.5)).items_start();
            for leg in chunk {
                let role = leg["role"].as_str().unwrap_or("");
                let mut b = col()
                    .grow(1.0)
                    .w(0.0)
                    .min_w(0.0)
                    .padding(tk.r(0.5))
                    .radius(tk.r(0.375))
                    .fill(tk.muted.alpha(0.5))
                    .child(
                        row()
                            .w_full()
                            .gap(tk.r(0.5))
                            .center_y()
                            .justify_between()
                            .child(badge(tk, model::role_label(role), "outline"))
                            .child(
                                text(tr(
                                    "review:swap.components.audit.amountMsat",
                                    &[(
                                        "amount",
                                        &model::format_exact_integer(&leg["amount_msat"]),
                                    )],
                                ))
                                .font(tk.txs(400.0).mono())
                                .color(tk.fg),
                            ),
                    )
                    .child(
                        text(format!(
                            "{} · {} · {} / {}",
                            leg["rail"].as_str().unwrap_or(""),
                            leg["asset"].as_str().unwrap_or(""),
                            leg["exposure"].as_str().unwrap_or("—"),
                            leg["conservation_unit"].as_str().unwrap_or("—")
                        ))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .mt(tk.r(0.25))
                        .wrap(),
                    );
                if let (Some(unit), false) = (
                    leg["valuation_unit"].as_str().filter(|u| !u.is_empty()),
                    leg["valuation_amount"].is_null(),
                ) {
                    b = b.child(
                        text(tr(
                            "review:swap.components.audit.valuation",
                            &[
                                (
                                    "amount",
                                    &model::format_exact_integer(&leg["valuation_amount"]),
                                ),
                                ("unit", unit),
                            ],
                        ))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .mt(tk.r(0.25)),
                    );
                }
                let reference = leg["transaction_id"]
                    .as_str()
                    .or(leg["wallet_id"].as_str())
                    .unwrap_or("");
                b = b.child(
                    text(compact_id(reference))
                        .font(tk.txs(400.0).mono())
                        .color(tk.muted_fg)
                        .mt(tk.r(0.25))
                        .truncate(),
                );
                line = line.child(b);
            }
            for _ in chunk.len()..cols {
                line = line.child(el().grow(1.0).w(0.0));
            }
            grid = grid.child(line);
        }
        card = card.child(grid);
        if !allocs.is_empty() {
            let mut list = col().w_full().mt(tk.r(0.5)).gap(tk.r(0.375));
            for a in &allocs {
                let label_of = |leg_id: &Value| match legs.iter().find(|l| &l["id"] == leg_id) {
                    Some(l) => format!(
                        "{} ({})",
                        l["ordinal"].as_u64().unwrap_or(0) + 1,
                        model::role_label(l["role"].as_str().unwrap_or(""))
                    ),
                    None => compact_id(leg_id.as_str().unwrap_or("")),
                };
                list = list.child(
                    row()
                        .w_full()
                        .px(tk.r(0.5))
                        .py(tk.r(0.375))
                        .radius(tk.r(0.375))
                        .fill(tk.muted.alpha(0.5))
                        .child(
                            text(tr(
                                "review:swap.components.audit.allocationEdge",
                                &[
                                    ("source", &label_of(&a["source_leg_id"])),
                                    (
                                        "sourceAmount",
                                        &model::format_exact_integer(&a["source_amount_msat"]),
                                    ),
                                    ("sink", &label_of(&a["sink_leg_id"])),
                                    (
                                        "sinkAmount",
                                        &model::format_exact_integer(&a["sink_amount_msat"]),
                                    ),
                                ],
                            ))
                            .font(tk.txs(400.0).mono())
                            .color(tk.fg)
                            .wrap(),
                        ),
                );
            }
            card = card.child(
                col()
                    .w_full()
                    .mt(tk.r(0.75))
                    .padding(tk.r(0.75))
                    .radius(tk.r(0.375))
                    .border(1.0, tk.border)
                    .child(
                        text(t("review:swap.components.audit.allocations"))
                            .font(tk.txs(500.0))
                            .color(tk.fg),
                    )
                    .child(list),
            );
        }
        if !issues.is_empty() {
            card = card.child(
                issue_box(
                    tk,
                    &t("review:swap.components.validationIssues"),
                    issues.iter().map(model::backend_issue_text).collect(),
                )
                .mt(tk.r(0.75)),
            );
        }
    }
    card
}

/// `CustodyComponentResolver`.
pub fn resolver(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let s = st(app);
    let page_w = kit::page_w(app, cx, tk);
    let win_w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    // `xl:grid-cols-3` and `sm:flex-row` are viewport breakpoints (80 rem and 40 rem).
    let xl = win_w >= tk.r(80.0);
    let sm = win_w >= tk.r(40.0);
    let _ = page_w;
    let new_btn = kit::btn(
        tk,
        "cc-new",
        Var::Default,
        Some(&I::PLUS),
        &t("review:swap.components.new"),
        true,
    )
    .on_press(open_creator);
    let desc = text(t("review:swap.components.description"))
        .font(tk.tsm(400.0))
        .color(tk.muted_fg)
        .wrap()
        .max_w(tk.r(48.0));
    let top = if sm {
        row()
            .w_full()
            .gap(tk.r(0.75))
            .items_start()
            .justify_between()
            .child(col().min_w(0.0).shrink().child(desc))
            .child(new_btn)
    } else {
        col()
            .w_full()
            .gap(tk.r(0.75))
            .child(desc)
            .child(row().child(new_btn))
    };
    let mut page = col()
        .key("custody-components")
        .w_full()
        .gap(tk.r(0.75))
        .child(top);
    if let Some(e) = &s.action_error {
        page = page.child(error_box(tk, "cc-action-error", e.clone()));
    }
    // The list card (`Card`: kb-surface gap-4 py-5; header and content px-5).
    let header = col()
        .w_full()
        .px(tk.card_pad())
        .gap(tk.r(0.375))
        .child(
            text(t("review:swap.components.listTitle"))
                .font(tk.tsm(600.0).lh(tk.r(0.875)))
                .color(tk.fg)
                .role("heading"),
        )
        .child(
            text(t("review:swap.components.listDescription"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    let mut content = col().w_full().px(tk.card_pad());
    if s.has_more {
        let n = s.limit.unwrap_or(1000) as usize;
        let (fg, bg, border) = if tk.dark {
            (
                tw("amber-100"),
                tw("amber-950").alpha(0.3),
                tw("amber-400").alpha(0.3),
            )
        } else {
            (tw("amber-950"), tw("amber-50"), tw("amber-300"))
        };
        content = content.child(
            col()
                .w_full()
                .mb(tk.r(0.75))
                .padding(tk.r(0.75))
                .radius(tk.r(0.375))
                .border(1.0, border)
                .fill(bg)
                .child(
                    text(tc("review:swap.components.moreAvailable", n, &[]))
                        .font(tk.tsm(400.0))
                        .color(fg)
                        .wrap(),
                ),
        );
    }
    if s.loading {
        content = content.child(
            row()
                .w_full()
                .py(tk.r(2.0))
                .gap(tk.r(0.5))
                .center_y()
                .role("status")
                .child(kit::spinner(tk, tk.r(1.0), tk.muted_fg))
                .child(
                    text(t("review:swap.components.loading"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                ),
        );
    } else if let Some(e) = &s.load_error {
        content = content.child(error_box(
            tk,
            "cc-load-error",
            tr("review:swap.components.loadFailed", &[("error", e)]),
        ));
    } else {
        let comps = s.components.clone().unwrap_or_default();
        if comps.is_empty() {
            content = content.child(
                row().w_full().py(tk.r(2.0)).justify_center().child(
                    text(t("review:swap.components.empty"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                ),
            );
        } else {
            let visible = if s.visible == 0 { PAGE } else { s.visible };
            let mut list = col().w_full().gap(tk.r(0.75));
            for c in comps.iter().take(visible) {
                list = list.child(component_row(app, tk, c, xl));
            }
            if comps.len() > visible {
                list = list.child(
                    row().w_full().pt(tk.r(0.25)).justify_center().child(
                        kit::btn(
                            tk,
                            "cc-load-more",
                            Var::Outline,
                            None,
                            &t("review:swap.loadMore"),
                            true,
                        )
                        .on_press(|a: &mut App| {
                            let s = &mut a.custody.comps;
                            s.visible = (if s.visible == 0 { PAGE } else { s.visible }) + PAGE;
                        }),
                    ),
                );
            }
            content = content.child(list);
        }
    }
    page = page.child(
        kit::surface(tk)
            .key("cc-list")
            .w_full()
            .py(tk.card_pad())
            .gap(tk.r(1.0))
            .child(header)
            .child(content),
    );
    if st(app).form.is_some() {
        editor_dialog(app, cx, tk, layer);
    }
    if st(app).confirm.is_some() {
        confirm_dialog(app, tk, layer);
    }
    page
}

/// An action's key in `review:swap.components.confirmAction.*`.
fn action_key(a: Action) -> &'static str {
    match a {
        Action::Activate => "activate",
        Action::Retire => "retire",
        Action::Supersede => "supersede",
        Action::Undo => "undo",
    }
}

fn state_label(state: &str) -> String {
    model::t_or(&format!("review:swap.components.state.{state}"), state)
}

/// The confirmation of a component action: the daemon's plan as it returned it (the state
/// change, the book version it was checked against, and the component with its exact legs,
/// allocations and open issues), then Cancel or the one button that applies exactly that plan.
fn confirm_dialog(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let Some(c) = st(app).confirm.as_ref() else {
        return;
    };
    let key = action_key(c.action);
    let title = t(&format!("review:swap.components.confirmAction.title.{key}"));
    let p = &c.plan;
    let comp = &p["component"];
    let mono = |s: String| {
        text(s)
            .font(tk.txs(400.0).mono())
            .color(tk.fg)
            .wrap_anywhere()
    };
    let plain = |s: String| text(s).font(tk.txs(400.0)).color(tk.fg).wrap();
    let revision = |v: &Value| {
        tc(
            "review:swap.components.revision",
            v.as_u64().unwrap_or(0) as usize,
            &[],
        )
    };
    let mut facts = col().w_full().gap(tk.r(0.375));
    if c.action == Action::Undo {
        facts = facts
            .child(form::fact(
                tk,
                t("review:swap.components.confirmAction.restoredFrom"),
                mono(p["source_component_id"].as_str().unwrap_or("—").to_owned()),
            ))
            .child(form::fact(
                tk,
                t("review:swap.components.confirmAction.newRevision"),
                mono(comp["id"].as_str().unwrap_or("—").to_owned()),
            ))
            .child(form::fact(
                tk,
                t("review:swap.components.confirmAction.state"),
                plain(format!(
                    "{} · {}",
                    revision(&comp["revision"]),
                    state_label(comp["state"].as_str().unwrap_or(""))
                )),
            ));
    } else {
        facts = facts
            .child(form::fact(
                tk,
                t("review:swap.components.confirmAction.component"),
                mono(p["component_id"].as_str().unwrap_or("—").to_owned()),
            ))
            .child(form::fact(
                tk,
                t("review:swap.components.confirmAction.state"),
                plain(format!(
                    "{} · {} → {}",
                    revision(&comp["revision"]),
                    state_label(p["current_state"].as_str().unwrap_or("")),
                    state_label(p["resulting_state"].as_str().unwrap_or(""))
                )),
            ));
    }
    let facts = facts.child(
        text(tr(
            "review:swap.components.form.inputVersion",
            &[("version", &p["input_version"].to_string())],
        ))
        .font(tk.txs(400.0))
        .color(tk.muted_fg),
    );
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    // The dialog's height less its header, facts and footer: the component scrolls.
    let body_h = (win_h - crate::shell::dialog_top(tk, win_h) - tk.r(18.0)).max(tk.r(8.0));
    let body = col()
        .key("cc-confirm-body")
        .w_full()
        .max_h(body_h)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(form::component_preview(app, tk, comp));
    let confirm_label = t(match c.action {
        Action::Activate => "review:swap.components.form.confirmActivation",
        Action::Retire => "review:swap.components.confirmAction.button.retire",
        Action::Supersede => "review:swap.components.confirmAction.button.supersede",
        Action::Undo => "review:swap.components.form.confirmDraft",
    });
    let mut confirm = kit::btn_busy(
        tk,
        "cc-confirm-apply",
        Var::Default,
        c.applying,
        None,
        &confirm_label,
        !c.applying,
    )
    .h(tk.r(2.25))
    .px(tk.r(1.0));
    if !c.applying {
        confirm = confirm.on_press(confirm_action);
    }
    let footer = kit::dialog_footer(tk)
        .child(
            kit::btn_md(
                tk,
                "cc-confirm-cancel",
                Var::Outline,
                None,
                &t("common:actions.cancel"),
                !c.applying,
            )
            .on_press(cancel_action),
        )
        .child(confirm);
    let content = col()
        .w_full()
        .gap(tk.r(1.0))
        .label(title.clone())
        .child(kit::dialog_header(
            tk,
            title,
            Some(t(&format!(
                "review:swap.components.confirmAction.description.{key}"
            ))),
        ))
        .child(facts)
        .child(body)
        .child(footer);
    kit::dialog(app, tk, layer, "cc-confirm", 40.0, content, cancel_action);
}

/// The editor or creator dialog: header, the form's scrolling body, its pinned footer
/// (`grid-rows-[auto_minmax(0,1fr)_auto] p-0 sm:max-w-4xl xl:max-w-5xl`).
fn editor_dialog(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) {
    let s = st(app);
    let (title, desc) = if s.creating {
        (
            t("review:swap.components.newTitle"),
            t("review:swap.components.newDescription"),
        )
    } else {
        let active = s.editing.as_ref().is_some_and(|c| c["state"] == "active");
        (
            t(if active {
                "review:swap.components.revisionDialog.reviseTitle"
            } else {
                "review:swap.components.revisionDialog.editTitle"
            }),
            t("review:swap.components.revisionDialog.description"),
        )
    };
    let win_w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    let max_w = if win_w >= tk.r(80.0) {
        tk.r(64.0)
    } else {
        tk.r(56.0)
    };
    let width = max_w.min(win_w - tk.r(2.0));
    // `lg` (64 rem viewport) lays the fields out in their 12-column grid.
    let wide = win_w >= tk.r(64.0);
    // `top: calc(var(--kb-window-top-inset) + max(1rem, 8vh))`, `max-h` to 1 rem above the edge;
    // the window's title bar is 40 pt.
    let top = crate::shell::dialog_top(tk, win_h);
    let height = win_h - top - tk.r(1.0);
    // The form's selects anchor to its fields, so they mount after the dialog that holds them.
    let mut over: Vec<El> = vec![];
    let header = col()
        .w_full()
        .px(tk.r(1.25))
        .py(tk.r(1.0))
        .pr(tk.r(3.0))
        .gap(tk.r(0.5))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .child(
            text(title.clone())
                .font(tk.tlg(600.0).lh(tk.r(1.125)))
                .color(tk.fg)
                .role("heading")
                .wrap(),
        )
        .child(
            text(desc)
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .max_w(tk.r(48.0)),
        );
    let body = col()
        .key("cc-dialog-body")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .padding(tk.r(1.25))
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(form::body(app, cx, tk, &mut over, wide));
    let footer = col()
        .w_full()
        .px(tk.r(1.25))
        .py(tk.r(0.75))
        .fill(tk.background.alpha(0.8))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout()
        .child(form::footer(app, tk, win_w >= tk.r(40.0)));
    let content = col()
        .w_full()
        .h(height)
        .child(header)
        .child(body)
        .child(footer);
    // `kit::dialog`'s frame, without its padding: the header, body and footer carry their own.
    let wash = if tk.dark {
        Color::BLACK.alpha(0.22)
    } else {
        tk.fg.alpha(0.06)
    };
    layer.push(
        crate::shell::window_backdrop("cc-dialog-scrim", wash)
            .on_press(close_form_guarded)
            .role("none"),
    );
    let (border, shadows) = if tk.dark {
        (
            Color::WHITE.alpha(0.08),
            vec![
                rune::Shadow {
                    y: 1.0,
                    color: Color::WHITE.alpha(0.04),
                    inset: true,
                    ..Default::default()
                },
                rune::Shadow {
                    y: 24.0,
                    blur: 72.0,
                    spread: -20.0,
                    color: Color::BLACK.alpha(0.9),
                    ..Default::default()
                },
            ],
        )
    } else {
        (
            tk.fg.alpha(0.1),
            vec![rune::Shadow {
                y: 24.0,
                blur: 64.0,
                spread: -24.0,
                color: Color::BLACK.alpha(0.65),
                ..Default::default()
            }],
        )
    };
    let close_btn = kit::icon_btn(
        tk,
        "cc-dialog-close",
        Var::Ghost,
        tk.r(1.75),
        &I::X,
        tk.r(1.0),
        t("common:actions.close"),
    )
    .pin(None, Some(tk.r(0.75)), Some(tk.r(0.75)), None)
    .on_press(close_form_guarded);
    let mut surface = col()
        .key("cc-dialog")
        .role("dialog")
        .label(title)
        .w(width)
        .h(height)
        .radius(tk.r(1.0))
        .fill(tk.background.alpha(0.8))
        .backdrop_blur(6.0)
        .transition(crate::shell::dialog_enter())
        .border(1.0, border)
        .clip_rounded(tk.r(1.0))
        .modal(true, close_form_guarded)
        .child(content)
        .child(close_btn);
    for sh in shadows {
        surface = surface.box_shadow(sh);
    }
    layer.push(crate::shell::centered(surface, win_w, width, top));
    layer.extend(over);
}

// ---- Scenes -------------------------------------------------------------------------------------

fn tab(a: &mut App) {
    a.prefs.developer_tools = true;
    a.navigate("/transfers?section=advanced&tab=components");
    crate::custody::prime(a);
    mount(a);
    crate::custody::settle(a);
}

fn first(a: &App) -> Option<Value> {
    a.custody
        .comps
        .components
        .as_ref()
        .and_then(|c| c.first().cloned())
}

/// Blocks on the form's plan reply (headless scenes).
fn settle_form(a: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while a
        .pending
        .values()
        .any(|(t, _, _)| t.starts_with("custody.comps."))
        && std::time::Instant::now() < deadline
    {
        a.pump();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// The first component's action as its state allows, asked for its plan (a dry run: a read):
/// the confirmation shows, and nothing is applied.
fn confirm_scene(a: &mut App) {
    tab(a);
    let Some(c) = first(a) else { return };
    let action = match c["state"].as_str().unwrap_or("") {
        "draft" if c["validation"]["activatable"].as_bool() != Some(false) => Action::Activate,
        "draft" => Action::Retire,
        "active" => Action::Supersede,
        _ => Action::Undo,
    };
    mutate(a, &c, action);
    settle_form(a);
}

fn revise(a: &mut App) {
    tab(a);
    if let Some(c) = first(a) {
        open_editor(a, &c);
    }
}

pub const SCENES: &[crate::app::Scene] = &[
    ("custody-comps", tab),
    ("custody-comps-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        tab(a);
    }),
    ("custody-comps-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        tab(a);
    }),
    // The first component unfolded: legs, allocations and audit.
    ("custody-comps-expanded", |a| {
        tab(a);
        if let Some(c) = first(a) {
            a.custody
                .comps
                .expanded
                .insert(c["id"].as_str().unwrap_or("").to_owned());
        }
    }),
    // The guided builder, pristine.
    ("custody-comps-new", |a| {
        tab(a);
        open_creator(a);
    }),
    // The builder with an unbalanced movement typed: live validation and the summary.
    ("custody-comps-new-invalid", |a| {
        tab(a);
        open_creator(a);
        if let Some(f) = a.custody.comps.form.as_mut() {
            f.legs[0].amount = TextEdit::new("1");
            f.legs[0].transaction_ref = TextEdit::new("tx-out");
            f.legs[1].amount = TextEdit::new("0.98");
            f.legs[2].amount = TextEdit::new("0.01");
        }
    }),
    // Revising an active component (conversion mode).
    ("custody-comps-revise", revise),
    ("custody-comps-revise-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        revise(a);
    }),
    // A select open in the builder.
    ("custody-comps-select", |a| {
        revise(a);
        let key = a
            .custody
            .comps
            .form
            .as_ref()
            .map(|f| f.legs[0].key.clone())
            .unwrap_or_default();
        a.custody.comps.popup = Some(Popup::Select(Sel::LegRole(key)));
    }),
    // The daemon's dry run of the revision as a draft (a read: `plan` is a dry run).
    ("custody-comps-preview", |a| {
        revise(a);
        form::submit(a, false);
        settle_form(a);
    }),
    // An action's server plan, awaiting explicit confirmation.
    ("custody-comps-confirm", confirm_scene),
    ("custody-comps-confirm-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        confirm_scene(a);
    }),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::quiet_app;

    fn sent(app: &App, kind: &str) -> usize {
        app.pending.values().filter(|(_, k, _)| k == kind).count()
    }

    fn reply(data: Value) -> Reply {
        Reply {
            kind: "ui.transfers.components.plan".into(),
            data,
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        }
    }

    fn supersede_plan(id: &str) -> Value {
        json!({
            "schema_version": 1,
            "input_version": 42,
            "action": "supersede",
            "component_id": id,
            "current_state": "active",
            "resulting_state": "superseded",
            "reason": "desktop_custody_review_supersede",
            "component": {"id": id, "revision": 2, "state": "active", "legs": [], "allocations": []},
            "dry_run": true,
            "requires_explicit_confirmation": true,
        })
    }

    /// The plan is shown, never applied on its own: only the explicit confirmation sends the
    /// apply, pinned to the plan's input version and the book.
    #[test]
    fn a_plan_waits_for_explicit_confirmation() {
        let mut app = quiet_app();
        let c = json!({"id": "comp-1", "state": "active"});
        mutate(&mut app, &c, Action::Supersede);
        assert_eq!(sent(&app, "ui.transfers.components.plan"), 1);
        on_reply(
            &mut app,
            "custody.comps.act_plan",
            &reply(supersede_plan("comp-1")),
        );
        assert_eq!(
            sent(&app, "ui.transfers.components.apply"),
            0,
            "a plan reply does not apply"
        );
        let shown = app
            .custody
            .comps
            .confirm
            .as_ref()
            .expect("the plan is shown");
        assert_eq!(shown.input_version, 42);
        assert!(app.custody.comps.pending.is_none());
        let apply = apply_args(shown, app.expected_scope());
        assert_eq!(apply["expected_input_version"], 42);
        assert_eq!(apply["action"], "supersede");
        assert_eq!(apply["component_id"], "comp-1");
        assert_eq!(apply["expected_scope"], app.expected_scope());

        confirm_action(&mut app);
        assert_eq!(
            sent(&app, "ui.transfers.components.apply"),
            1,
            "confirm applies"
        );
        assert!(
            app.custody
                .comps
                .confirm
                .as_ref()
                .is_some_and(|c| c.applying)
        );
        escape(&mut app);
        assert!(
            app.custody.comps.confirm.is_some(),
            "Escape does not drop a plan while it applies"
        );
        // A second press while it applies sends nothing more.
        confirm_action(&mut app);
        assert_eq!(sent(&app, "ui.transfers.components.apply"), 1);
    }

    #[test]
    fn cancel_discards_the_plan_without_applying() {
        let mut app = quiet_app();
        mutate(&mut app, &json!({"id": "comp-1"}), Action::Supersede);
        on_reply(
            &mut app,
            "custody.comps.act_plan",
            &reply(supersede_plan("comp-1")),
        );
        assert!(escape(&mut app), "Escape closes the confirmation");
        assert!(app.custody.comps.confirm.is_none());
        confirm_action(&mut app);
        assert_eq!(sent(&app, "ui.transfers.components.apply"), 0);
    }

    /// A plan for another component or action is not confirmable.
    #[test]
    fn a_mismatched_plan_is_refused() {
        let mut app = quiet_app();
        mutate(&mut app, &json!({"id": "comp-1"}), Action::Supersede);
        on_reply(
            &mut app,
            "custody.comps.act_plan",
            &reply(supersede_plan("comp-2")),
        );
        assert!(app.custody.comps.confirm.is_none());
        assert!(app.custody.comps.action_error.is_some());
        assert_eq!(sent(&app, "ui.transfers.components.apply"), 0);
    }
}
