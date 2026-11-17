//! The guided custody-component builder (`transfers-custody/GuidedComponentForm.tsx`,
//! `GuidedComponentFields.tsx`, `GuidedComponentLayout.tsx`): interpretation, legs, allocations,
//! evidence, live validation, then the daemon's dry run (`ui.transfers.components.plan`) shown in
//! full before the one confirmation (`ui.transfers.components.apply`) that applies exactly that
//! reviewed plan — bound to the reviewed document, the book and the server's input version.

use rune::{Color, Cx, El, Icon, TextEdit, col, el, row, text};
use rune_kit::{Placement, TextArea, TextInput};
use serde_json::{Value, json};

use super::model::{self, Allocation, Commitment, Form, Leg, Origin};
use super::{Popup, Sel};
use crate::app::{App, Reply};
use crate::custody::kit::{self, Var};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::{ic, sensitive};

/// The text fields a leg edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegField {
    Amount,
    Asset,
    TransactionRef,
    WalletRef,
    Untracked,
    OccurredAt,
    ValuationUnit,
    ValuationAmount,
}

pub struct LegUi {
    pub key: String,
    pub origin_id: String,
    pub role: String,
    pub origin_mode: bool,
    pub origin: Option<Origin>,
    pub location_kind: String,
    pub original_occurred_at: Option<String>,
    pub original_occurred_at_input: Option<String>,
    pub notes: String,
    pub amount: TextEdit,
    pub asset: TextEdit,
    pub transaction_ref: TextEdit,
    pub wallet_ref: TextEdit,
    pub untracked: TextEdit,
    pub occurred_at: TextEdit,
    pub valuation_unit: TextEdit,
    pub valuation_amount: TextEdit,
}

impl LegUi {
    fn from(l: Leg) -> LegUi {
        LegUi {
            key: l.key,
            origin_id: l.origin_id,
            role: l.role,
            origin_mode: l.origin_mode,
            origin: l.origin,
            location_kind: l.location_kind,
            original_occurred_at: l.original_occurred_at,
            original_occurred_at_input: l.original_occurred_at_input,
            notes: l.notes,
            amount: TextEdit::new(l.amount_btc),
            asset: TextEdit::new(l.asset),
            transaction_ref: TextEdit::new(l.transaction_ref),
            wallet_ref: TextEdit::new(l.wallet_ref),
            untracked: TextEdit::new(l.untracked_wallet),
            occurred_at: TextEdit::new(l.occurred_at),
            valuation_unit: TextEdit::new(l.valuation_unit),
            valuation_amount: TextEdit::new(l.valuation_amount),
        }
    }
    fn model(&self) -> Leg {
        Leg {
            key: self.key.clone(),
            origin_id: self.origin_id.clone(),
            role: self.role.clone(),
            amount_btc: self.amount.text.clone(),
            origin_mode: self.origin_mode,
            origin: self.origin.clone(),
            location_kind: self.location_kind.clone(),
            transaction_ref: self.transaction_ref.text.clone(),
            wallet_ref: self.wallet_ref.text.clone(),
            untracked_wallet: self.untracked.text.clone(),
            occurred_at: self.occurred_at.text.clone(),
            original_occurred_at: self.original_occurred_at.clone(),
            original_occurred_at_input: self.original_occurred_at_input.clone(),
            asset: self.asset.text.clone(),
            valuation_unit: self.valuation_unit.text.clone(),
            valuation_amount: self.valuation_amount.text.clone(),
            notes: self.notes.clone(),
        }
    }
    fn edit(&mut self, f: LegField) -> &mut TextEdit {
        match f {
            LegField::Amount => &mut self.amount,
            LegField::Asset => &mut self.asset,
            LegField::TransactionRef => &mut self.transaction_ref,
            LegField::WalletRef => &mut self.wallet_ref,
            LegField::Untracked => &mut self.untracked,
            LegField::OccurredAt => &mut self.occurred_at,
            LegField::ValuationUnit => &mut self.valuation_unit,
            LegField::ValuationAmount => &mut self.valuation_amount,
        }
    }
}

pub struct AllocUi {
    pub key: String,
    pub source_key: String,
    pub sink_key: String,
    pub amount: TextEdit,
    pub sink_amount: TextEdit,
}

/// A plan the daemon returned, bound to what was reviewed (`ReviewedPlan`).
pub struct Reviewed {
    pub commitment: Commitment,
    pub args: Value,
    pub result: Value,
    pub activate: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InFlight {
    Draft,
    Activate,
    Confirm,
}

pub struct FormUi {
    /// Revising this component (`edit`): (id, "draft" | "active").
    pub edit: Option<(String, String)>,
    /// Loaded from an existing component (`initialForm`), so validation shows at once.
    pub initial: bool,
    pub component_type: String,
    pub conservation_mode: String,
    pub evidence_grade: String,
    pub conversion_reviewed: bool,
    pub conversion_policy: TextEdit,
    pub evidence_kind: TextEdit,
    pub notes: TextEdit,
    pub legs: Vec<LegUi>,
    pub allocs: Vec<AllocUi>,
    pub result: Option<Value>,
    pub action_error: Option<String>,
    pub reviewed: Option<Reviewed>,
    pub in_flight: Option<InFlight>,
    /// The plan request in flight: its document, scope, args and whether it activates.
    pending_plan: Option<(String, String, Value, bool)>,
    /// The document last seen, to notice the owner's edits (`patchForm` clears notes).
    last_doc: String,
    /// The apply in flight was reviewed in this scope.
    apply_scope: Option<String>,
    /// A review just arrived: bring it into view once (`reviewRef.scrollIntoView`).
    pub reveal_review: bool,
}

impl FormUi {
    pub fn new(form: Form, edit: Option<(String, String)>, initial: bool) -> FormUi {
        let mut f = FormUi {
            edit,
            initial,
            component_type: form.component_type.clone(),
            conservation_mode: form.conservation_mode.clone(),
            evidence_grade: form.evidence_grade.clone(),
            conversion_reviewed: form.conversion_reviewed,
            conversion_policy: TextEdit::new(form.conversion_policy.clone()),
            evidence_kind: TextEdit::new(form.evidence_kind.clone()),
            notes: TextEdit::new(form.notes.clone()),
            legs: form.legs.into_iter().map(LegUi::from).collect(),
            allocs: form
                .allocations
                .into_iter()
                .map(|a| AllocUi {
                    key: a.key,
                    source_key: a.source_key,
                    sink_key: a.sink_key,
                    amount: TextEdit::new(a.amount_btc),
                    sink_amount: TextEdit::new(a.sink_amount_btc),
                })
                .collect(),
            result: None,
            action_error: None,
            reviewed: None,
            in_flight: None,
            pending_plan: None,
            last_doc: String::new(),
            apply_scope: None,
            reveal_review: false,
        };
        f.last_doc = f.document();
        f
    }

    pub fn model(&self) -> Form {
        Form {
            component_type: self.component_type.clone(),
            conservation_mode: self.conservation_mode.clone(),
            evidence_kind: self.evidence_kind.text.clone(),
            evidence_grade: self.evidence_grade.clone(),
            conversion_policy: self.conversion_policy.text.clone(),
            conversion_reviewed: self.conversion_reviewed,
            notes: self.notes.text.clone(),
            legs: self.legs.iter().map(LegUi::model).collect(),
            allocations: self
                .allocs
                .iter()
                .map(|a| Allocation {
                    key: a.key.clone(),
                    source_key: a.source_key.clone(),
                    sink_key: a.sink_key.clone(),
                    amount_btc: a.amount.text.clone(),
                    sink_amount_btc: a.sink_amount.text.clone(),
                })
                .collect(),
        }
    }

    pub fn document(&self) -> String {
        model::form_to_document(&self.model())
    }

    /// `pending`: a plan or apply is out.
    pub fn pending(&self) -> bool {
        self.in_flight.is_some()
    }
}

// ---- Access -------------------------------------------------------------------------------------

fn form_mut(a: &mut App) -> Option<&mut FormUi> {
    a.custody.comps.form.as_mut()
}

/// The `TextEdit` a field edits; a removed field falls back to a scratch edit.
fn leg_edit<'a>(a: &'a mut App, key: &str, f: LegField) -> &'a mut TextEdit {
    let st = &mut a.custody.comps;
    match st
        .form
        .as_mut()
        .and_then(|fm| fm.legs.iter_mut().find(|l| l.key == key))
    {
        Some(l) => l.edit(f),
        None => &mut st.scratch,
    }
}

fn alloc_edit<'a>(a: &'a mut App, key: &str, sink: bool) -> &'a mut TextEdit {
    let st = &mut a.custody.comps;
    match st
        .form
        .as_mut()
        .and_then(|fm| fm.allocs.iter_mut().find(|l| l.key == key))
    {
        Some(x) => {
            if sink {
                &mut x.sink_amount
            } else {
                &mut x.amount
            }
        }
        None => &mut st.scratch,
    }
}

/// Every frame: an edit by the owner clears the last note and the reviewed plan (the plan is
/// only confirmable for the document it was made for, in the book it was made in).
pub fn poll(app: &mut App) -> bool {
    let scope = app.expected_scope().to_string();
    let Some(f) = app.custody.comps.form.as_mut() else {
        return false;
    };
    let mut changed = false;
    // Quantity mode mirrors the source amount into the sink amount (`AllocationRow`).
    if f.conservation_mode != "conversion" {
        for a in f.allocs.iter_mut() {
            if a.sink_amount.text != a.amount.text {
                a.sink_amount = TextEdit::new(a.amount.text.clone());
            }
        }
    }
    let doc = f.document();
    if doc != f.last_doc {
        f.last_doc = doc.clone();
        f.result = None;
        f.action_error = None;
        changed = true;
    }
    if let Some(r) = &f.reviewed
        && !model::can_confirm(Some(&r.commitment), &doc, &scope)
    {
        f.reviewed = None;
        changed = true;
    }
    changed
}

// ---- Actions -----------------------------------------------------------------------------------

fn patch(a: &mut App, f: impl FnOnce(&mut FormUi)) {
    if let Some(fm) = form_mut(a) {
        if fm.pending() {
            return;
        }
        f(fm);
        fm.result = None;
        fm.action_error = None;
    }
}

fn add_leg(a: &mut App) {
    patch(a, |f| {
        f.legs.push(LegUi::from(model::new_leg("destination")))
    });
}

fn remove_leg(a: &mut App, key: &str) {
    let key = key.to_owned();
    // Drop allocation edges that referenced the removed leg, so none is silently discarded.
    patch(a, move |f| {
        f.legs.retain(|l| l.key != key);
        f.allocs
            .retain(|x| x.source_key != key && x.sink_key != key);
    });
}

fn add_alloc(a: &mut App) {
    patch(a, |f| {
        let n = model::new_allocation();
        f.allocs.push(AllocUi {
            key: n.key,
            source_key: String::new(),
            sink_key: String::new(),
            amount: TextEdit::new(""),
            sink_amount: TextEdit::new(""),
        });
    });
}

fn remove_alloc(a: &mut App, key: &str) {
    let key = key.to_owned();
    patch(a, move |f| f.allocs.retain(|x| x.key != key));
}

/// "Start over".
fn reset(a: &mut App) {
    if let Some(f) = form_mut(a) {
        *f = FormUi::new(model::initial_form(), None, false);
    }
}

/// `submit`: the daemon's dry run of the form as a draft or an activation.
pub fn submit(a: &mut App, activate: bool) {
    let scope = a.expected_scope().to_string();
    let Some(f) = form_mut(a) else { return };
    if f.pending() {
        return;
    }
    f.result = None;
    f.action_error = None;
    let form = f.model();
    let preview = model::preview_form(&form);
    if !preview.structural.is_empty() || (activate && !preview.activation.is_empty()) {
        return;
    }
    let spec = model::form_to_spec(&form);
    let args = match &f.edit {
        Some((id, _)) => {
            json!({"action": "revise", "component_id": id, "spec": spec, "activate": activate})
        }
        None => model::bulk_request(&preview, activate),
    };
    let doc = f.document();
    f.reviewed = None;
    f.in_flight = Some(if activate {
        InFlight::Activate
    } else {
        InFlight::Draft
    });
    f.pending_plan = Some((doc, scope, args.clone(), activate));
    let mut send = args;
    send["expected_scope"] = a.expected_scope();
    if a.send(
        "ui.transfers.components.plan",
        send,
        "custody.comps.form_plan",
    )
    .is_none()
        && let Some(f) = form_mut(a)
    {
        f.in_flight = None;
    }
}

/// `confirm`: apply exactly the reviewed plan, pinned to its input version and scope.
pub fn confirm(a: &mut App) {
    let scope = a.expected_scope().to_string();
    let Some(f) = form_mut(a) else { return };
    let doc = f.document();
    let ok = !f.pending()
        && f.reviewed
            .as_ref()
            .is_some_and(|r| model::can_confirm(Some(&r.commitment), &doc, &scope));
    if !ok {
        f.action_error = Some(t("review:swap.components.form.previewChanged"));
        return;
    }
    let r = f.reviewed.as_ref().unwrap();
    let mut args = r.args.clone();
    args["expected_input_version"] = json!(r.commitment.input_version);
    args["expected_scope"] = a.expected_scope();
    let f = form_mut(a).unwrap();
    f.action_error = None;
    f.in_flight = Some(InFlight::Confirm);
    f.apply_scope = Some(scope);
    if a.send(
        "ui.transfers.components.apply",
        args,
        "custody.comps.form_apply",
    )
    .is_none()
        && let Some(f) = form_mut(a)
    {
        f.in_flight = None;
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    let scope = app.expected_scope().to_string();
    match tag {
        "custody.comps.form_plan" => {
            let Some(f) = form_mut(app) else { return };
            f.in_flight = None;
            let Some((doc, plan_scope, args, activate)) = f.pending_plan.take() else {
                return;
            };
            if r.error.is_some() {
                f.action_error = Some(model::mutation_error(&r.error_detail));
                return;
            }
            let d = &r.data;
            let version = d["input_version"]
                .as_i64()
                .filter(|v| (0..=9_007_199_254_740_991).contains(v));
            let has_component = d["component"].is_object()
                || d["components"].as_array().is_some_and(|c| !c.is_empty());
            let (Some(version), true) = (version, has_component) else {
                f.action_error = Some(t("review:swap.components.backendError.unexpected"));
                return;
            };
            let Some(scoped) = model::bind_plan_scope(&args, d) else {
                f.action_error = Some(t("review:swap.components.backendError.unexpected"));
                return;
            };
            if scope != plan_scope || f.document() != doc {
                f.action_error = Some(t("review:swap.components.form.previewChanged"));
                return;
            }
            f.reveal_review = true;
            f.reviewed = Some(Reviewed {
                commitment: Commitment {
                    document: doc,
                    scope: plan_scope,
                    input_version: version,
                },
                args: scoped,
                result: d.clone(),
                activate,
            });
        }
        "custody.comps.form_apply" => {
            let Some(f) = form_mut(app) else { return };
            f.in_flight = None;
            f.reviewed = None;
            let reviewed_scope = f.apply_scope.take();
            if r.error.is_some() {
                f.action_error = Some(model::mutation_error(&r.error_detail));
                return;
            }
            let editing = f.edit.is_some();
            if reviewed_scope.as_deref() == Some(scope.as_str()) {
                if editing {
                    super::close_form(app);
                } else {
                    let summary = r.data.get("summary").cloned().filter(|s| s.is_object());
                    let f = form_mut(app).unwrap();
                    *f = FormUi::new(model::initial_form(), None, false);
                    f.result = summary;
                }
            }
            // `ui.transfers.components.apply` refreshes the book's derived reads.
            app.refresh_reads();
            crate::custody::invalidate(app);
        }
        _ => {}
    }
}

// ---- View --------------------------------------------------------------------------------------

/// A `Label` above its field (`text-sm font-medium leading-none`, `space-y-1.5`).
fn labelled(tk: &Tk, label: &str, field: El) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.375))
        .child(
            text(label.to_owned())
                .font(tk.tsm(500.0).lh(tk.r(0.875)))
                .color(tk.fg),
        )
        .child(field)
}

/// shadcn `Input` (`h-9 rounded-md border-input px-3 md:text-sm`, `dark:bg-input/30`).
#[allow(clippy::too_many_arguments)]
fn input<F>(
    tk: &Tk,
    edit: &TextEdit,
    key: String,
    label: &str,
    placeholder: &str,
    mono: bool,
    enabled: bool,
    on_edit: F,
) -> El
where
    F: for<'a> Fn(&'a mut App) -> &'a mut TextEdit + 'static,
{
    let k = crate::txdetail::widgets::form_kit(tk);
    let style = if mono {
        tk.tsm(400.0).mono()
    } else {
        tk.tsm(400.0)
    };
    let i = TextInput::new(edit)
        .label(label.to_owned())
        .placeholder(placeholder.to_owned())
        .height(tk.r(2.25))
        .width(None)
        .text_style(style)
        .disabled(!enabled)
        .on_edit(on_edit);
    let e = crate::txdetail::widgets::field(tk, i.build(&k).key(key).w_full());
    if enabled { e } else { e.opacity(0.5) }
}

/// shadcn `SelectTrigger` (`h-9 w-full rounded-md border-input px-3 text-sm`, chevron).
fn select_trigger(
    app: &App,
    tk: &Tk,
    key: String,
    label: &str,
    value: El,
    enabled: bool,
    sel: Sel,
) -> El {
    let open = app.custody.comps.popup == Some(Popup::Select(sel.clone()));
    let mut r = row()
        .key(key)
        .w_full()
        .h(tk.r(2.25))
        .px(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.375))
        .border(1.0, tk.input)
        .fill(if tk.dark {
            tk.input.alpha(tk.input.a * 0.3)
        } else {
            Color::TRANSPARENT
        })
        .role("combobox")
        .label(label.to_owned())
        .focusable(enabled)
        .disabled(!enabled)
        .child(
            row()
                .min_w(0.0)
                .grow(1.0)
                .gap(tk.r(0.5))
                .center_y()
                .child(value),
        )
        .child(ic(&I::CHEVRON_DOWN, tk.r(1.0), tk.muted_fg).opacity(0.5));
    if enabled {
        r = crate::ui::popup::trigger(
            r.hover(if tk.dark {
                tk.input.alpha(tk.input.a * 0.5)
            } else {
                tk.accent
            }),
            crate::ui::popup::Kind::Listbox,
            open,
            move |a: &mut App| super::toggle(a, Popup::Select(sel.clone())),
        );
    } else {
        r = r.opacity(0.5);
    }
    r
}

/// `SelectContent`: the options, the chosen one checked, a role dot where the web draws one.
fn select_menu(
    tk: &Tk,
    layer: &mut Vec<El>,
    anchor: &str,
    options: Vec<(String, String, Option<String>)>,
    current: &str,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) {
    let mut p = crate::ui::popup::Popup::listbox(anchor, super::close_popup);
    let list = col()
        .min_w(tk.r(8.0))
        .padding(tk.r(0.25))
        .items_stretch()
        .label(anchor.to_owned());
    for (v, label, dot) in options {
        let checked = v == current;
        let pick = choose.clone();
        let val = v.clone();
        let mut opt = row()
            .key(format!("{anchor}-opt-{v}"))
            .w_full()
            .py(tk.r(0.375))
            .pl(tk.r(0.5))
            .pr(tk.r(0.5))
            .gap(tk.r(0.5))
            .center_y()
            .radius(tk.r(0.25))
            .hover(tk.accent)
            .focus_highlight(tk.accent)
            .role("option")
            .checked(checked)
            .label(label.clone())
            .focusable(true)
            .focus_selected(checked)
            .on_press(move |a: &mut App| {
                super::close_popup(a);
                pick(a, &val);
            });
        if let Some(role) = dot {
            opt = opt.child(role_dot(tk, &role));
        }
        opt = opt.child(
            text(label.clone())
                .font(tk.tsm(400.0))
                .color(tk.fg)
                .grow(1.0),
        );
        opt = opt.child(row().size(tk.r(1.0), tk.r(1.0)).center().child(if checked {
            ic(&I::CHECK, tk.r(1.0), tk.fg)
        } else {
            el()
        }));
        p.item(opt, format!("{anchor}-opt-{v}"), label, true, checked);
    }
    p.mount(
        tk,
        layer,
        list,
        |l| kit::menu_surface(tk, col().min_h(0.0).child(l)).key(format!("{anchor}-content")),
        Placement::BottomStart,
        4.0,
    );
}

fn checkbox(
    tk: &Tk,
    key: &str,
    on: bool,
    label: &str,
    enabled: bool,
    toggle: impl Fn(&mut App) + 'static,
) -> El {
    let b = row()
        .key(key.to_owned())
        .size(tk.r(1.0), tk.r(1.0))
        .center()
        .radius(4.0)
        .border(1.0, if on { tk.primary } else { tk.input })
        .fill(if on {
            tk.primary
        } else if tk.dark {
            tk.input.alpha(tk.input.a * 0.3)
        } else {
            Color::TRANSPARENT
        })
        .role("checkbox")
        .checked(on)
        .label(label.to_owned())
        .focusable(enabled)
        .disabled(!enabled);
    let b = if enabled {
        b.on_press(toggle)
    } else {
        b.opacity(0.5)
    };
    if on {
        b.child(ic(&I::CHECK, tk.r(0.875), tk.primary_fg))
    } else {
        b
    }
}

/// `ROLE_ACCENT`: each role's stable colour.
pub fn role_accent(tk: &Tk, role: &str) -> Color {
    let tw = |light: &str, dark: &str, la: f32, da: f32| {
        if tk.dark {
            super::tw(dark).alpha(da)
        } else {
            super::tw(light).alpha(la)
        }
    };
    match role {
        "source" => tw("orange-400", "orange-300", 0.8, 0.7),
        "destination" => tw("emerald-500", "emerald-300", 0.8, 0.7),
        "retained" => tw("sky-500", "sky-300", 0.8, 0.7),
        "fee" => tk.muted_fg.alpha(0.5),
        "external" => tw("violet-500", "violet-300", 0.7, 0.6),
        "suspense" => tw("amber-500", "amber-300", 0.8, 0.7),
        _ => tk.muted_fg.alpha(0.4),
    }
}

/// `RoleDot`.
pub fn role_dot(tk: &Tk, role: &str) -> El {
    crate::ui::dot(tk.r(0.5), role_accent(tk, role)).a11y_hidden()
}

/// `GuidedFormSection`: title, one-line hint, an optional trailing action.
fn section(tk: &Tk, title: &str, hint: &str, action: Option<El>, body: El) -> El {
    let head = col()
        .min_w(0.0)
        .grow(1.0)
        .gap(tk.r(0.125))
        .child(
            text(title.to_owned())
                .font(tk.tsm(500.0).lh(tk.r(1.25)))
                .color(tk.fg)
                .role("heading"),
        )
        .child(
            text(hint.to_owned())
                .font(tk.txs(400.0).lh(tk.r(1.25)))
                .color(tk.muted_fg)
                .wrap(),
        );
    let mut top = row().w_full().gap(tk.r(1.0)).items_start().child(head);
    if let Some(a) = action {
        top = top.child(a);
    }
    col().w_full().gap(tk.r(0.75)).child(top).child(body)
}

fn short_id(v: &str) -> String {
    if v.chars().count() <= 18 {
        return v.to_owned();
    }
    let h: String = v.chars().take(10).collect();
    let tl: String = v.chars().skip(v.chars().count() - 6).collect();
    format!("{h}…{tl}")
}

/// `GuidedLegRow`.
fn leg_row(app: &App, tk: &Tk, layer: &mut Vec<El>, f: &FormUi, idx: usize, wide: bool) -> El {
    let leg = &f.legs[idx];
    let enabled = !f.pending();
    let conversion = f.conservation_mode == "conversion";
    let suspense = leg.role == "suspense";
    let manual = !suspense && !leg.origin_mode;
    let origin = !suspense && leg.origin_mode;
    let origin_needs_time = origin
        && leg
            .origin
            .as_ref()
            .is_none_or(|o| o.transaction_id.is_none());
    let show_at = suspense
        || origin_needs_time
        || (!leg.origin_mode
            && (leg.location_kind == "untracked" || leg.location_kind == "wallet"));
    let key = leg.key.clone();
    let can_remove = f.legs.len() > 2;

    let ordinal = tr(
        "review:swap.components.form.leg.ordinal",
        &[("index", &(idx + 1).to_string())],
    );
    let mut head = row()
        .w_full()
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .child(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .child(role_dot(tk, &leg.role))
                .child(text(ordinal).font(tk.txs(400.0).mono()).color(tk.muted_fg))
                .child(
                    text("·")
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .a11y_hidden(),
                )
                .child(
                    text(model::role_label(&leg.role))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                ),
        );
    if can_remove {
        let k = key.clone();
        head = head.child(
            kit::icon_btn(
                tk,
                format!("cc-leg-rm-{key}"),
                Var::Ghost,
                tk.r(2.0),
                &I::TRASH_2,
                tk.r(1.0),
                t("review:swap.components.form.removeLeg"),
            )
            .mr(-tk.r(0.5))
            .mt(-tk.r(0.25))
            .on_press(move |a: &mut App| remove_leg(a, &k)),
        );
    }

    // Fields, in the web's 12-column grid at `lg` (2 columns below it).
    let mut cells: Vec<(f32, El)> = vec![];
    let role_sel = Sel::LegRole(key.clone());
    let role_key = format!("cc-leg-role-{key}");
    if app.custody.comps.popup == Some(Popup::Select(role_sel.clone())) {
        let k = key.clone();
        select_menu(
            tk,
            layer,
            &role_key,
            [
                "source",
                "destination",
                "fee",
                "retained",
                "external",
                "suspense",
            ]
            .iter()
            .map(|r| (r.to_string(), model::role_label(r), Some(r.to_string())))
            .collect(),
            &leg.role,
            move |a, v| {
                let (k, v) = (k.clone(), v.to_owned());
                patch(a, move |f| {
                    if let Some(l) = f.legs.iter_mut().find(|l| l.key == k) {
                        l.role = v;
                    }
                });
            },
        );
    }
    let role_value = row()
        .gap(tk.r(0.5))
        .center_y()
        .child(role_dot(tk, &leg.role))
        .child(
            text(model::role_label(&leg.role))
                .font(tk.tsm(400.0))
                .color(tk.fg),
        );
    cells.push((
        3.0,
        labelled(
            tk,
            &t("review:swap.components.form.leg.role"),
            select_trigger(
                app,
                tk,
                role_key,
                &t("review:swap.components.form.leg.role"),
                role_value,
                enabled,
                role_sel,
            ),
        ),
    ));
    let (k1, k2) = (key.clone(), key.clone());
    let amount = input(
        tk,
        &leg.amount,
        format!("cc-leg-amount-{key}"),
        &t("review:swap.components.form.leg.amount"),
        "0.00000000",
        true,
        enabled,
        move |a| leg_edit(a, &k1, LegField::Amount),
    );
    let asset = input(
        tk,
        &leg.asset,
        format!("cc-leg-asset-{key}"),
        &t("review:swap.components.form.leg.asset"),
        "",
        true,
        enabled,
        move |a| leg_edit(a, &k2, LegField::Asset),
    );
    cells.push((
        4.0,
        row()
            .w_full()
            .gap(tk.r(0.5))
            .child(col().grow(1.0).w(0.0).min_w(0.0).child(labelled(
                tk,
                &t("review:swap.components.form.leg.amount"),
                amount,
            )))
            .child(col().w(tk.r(5.5)).child(labelled(
                tk,
                &t("review:swap.components.form.leg.asset"),
                asset,
            ))),
    ));
    if manual {
        let sel = Sel::LegLocation(key.clone());
        let lk = format!("cc-leg-loc-{key}");
        if app.custody.comps.popup == Some(Popup::Select(sel.clone())) {
            let k = key.clone();
            select_menu(
                tk,
                layer,
                &lk,
                model::LOCATION_KINDS
                    .iter()
                    .map(|x| {
                        (
                            x.to_string(),
                            t(&format!("review:swap.components.form.leg.locationKind.{x}")),
                            None,
                        )
                    })
                    .collect(),
                &leg.location_kind,
                move |a, v| {
                    let (k, v) = (k.clone(), v.to_owned());
                    patch(a, move |f| {
                        if let Some(l) = f.legs.iter_mut().find(|l| l.key == k) {
                            l.location_kind = v;
                        }
                    });
                },
            );
        }
        let v = text(t(&format!(
            "review:swap.components.form.leg.locationKind.{}",
            leg.location_kind
        )))
        .font(tk.tsm(400.0))
        .color(tk.fg)
        .truncate();
        cells.push((
            5.0,
            labelled(
                tk,
                &t("review:swap.components.form.leg.location"),
                select_trigger(
                    app,
                    tk,
                    lk,
                    &t("review:swap.components.form.leg.location"),
                    v,
                    enabled,
                    sel,
                ),
            ),
        ));
    }
    if origin {
        let o = leg.origin.clone().unwrap_or_default();
        let (label, title) = if let Some(tx) = &o.transaction_id {
            (
                tr(
                    "review:swap.components.form.leg.originTransaction",
                    &[("id", &short_id(tx))],
                ),
                Some(tx.clone()),
            )
        } else if let Some(w) = &o.wallet_id {
            (
                tr(
                    "review:swap.components.form.leg.originWallet",
                    &[("id", &short_id(w))],
                ),
                Some(w.clone()),
            )
        } else {
            (
                tr(
                    "review:swap.components.form.leg.originLocation",
                    &[("rail", o.rail.as_deref().unwrap_or(""))],
                ),
                None,
            )
        };
        let mut lbl = text(label)
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .truncate()
            .min_w(0.0)
            .shrink();
        if let Some(tt) = title {
            lbl = lbl.help(tt);
        }
        let k = key.clone();
        let change = kit::btn_xs(
            tk,
            format!("cc-leg-change-{key}"),
            Var::Ghost,
            None,
            &t("review:swap.components.form.leg.changeLocation"),
            enabled,
        )
        .on_press(move |a: &mut App| {
            let k = k.clone();
            patch(a, move |f| {
                if let Some(l) = f.legs.iter_mut().find(|l| l.key == k) {
                    l.origin_mode = false;
                    l.origin = None;
                }
            });
        });
        let boxed = row()
            .w_full()
            .h(tk.r(2.25))
            .pl(tk.r(0.75))
            .pr(tk.r(0.25))
            .gap(tk.r(0.5))
            .center_y()
            .justify_between()
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .fill(tk.muted.alpha(0.4))
            .child(lbl)
            .child(change);
        cells.push((
            5.0,
            labelled(
                tk,
                &t("review:swap.components.form.leg.existingLocation"),
                boxed,
            ),
        ));
    }
    if manual {
        let (field, label, ph, mono) = match leg.location_kind.as_str() {
            "transaction" => (
                LegField::TransactionRef,
                "transactionRef",
                "transactionRefPlaceholder",
                true,
            ),
            "wallet" => (
                LegField::WalletRef,
                "walletRef",
                "walletRefPlaceholder",
                false,
            ),
            _ => (
                LegField::Untracked,
                "untrackedWallet",
                "untrackedWalletPlaceholder",
                false,
            ),
        };
        let edit = match field {
            LegField::TransactionRef => &leg.transaction_ref,
            LegField::WalletRef => &leg.wallet_ref,
            _ => &leg.untracked,
        };
        let k = key.clone();
        let l = t(&format!("review:swap.components.form.leg.{label}"));
        let i = input(
            tk,
            edit,
            format!("cc-leg-{label}-{key}"),
            &l,
            &t(&format!("review:swap.components.form.leg.{ph}")),
            mono,
            enabled,
            move |a| leg_edit(a, &k, field),
        );
        cells.push((7.0, labelled(tk, &l, i)));
    }
    if show_at {
        let k = key.clone();
        let l = t("review:swap.components.form.leg.occurredAt");
        // The web's `datetime-local` input; Rune has no native picker, so the same value
        // (`YYYY-MM-DDTHH:MM:SS`, local time) is typed.
        let i = input(
            tk,
            &leg.occurred_at,
            format!("cc-leg-at-{key}"),
            &l,
            "YYYY-MM-DDTHH:MM:SS",
            true,
            enabled,
            move |a| leg_edit(a, &k, LegField::OccurredAt),
        );
        cells.push((5.0, labelled(tk, &l, i)));
    }
    if conversion {
        let (k1, k2) = (key.clone(), key.clone());
        let lu = t("review:swap.components.form.leg.valuationUnit");
        let la = t("review:swap.components.form.leg.valuationAmount");
        cells.push((
            4.0,
            labelled(
                tk,
                &lu,
                input(
                    tk,
                    &leg.valuation_unit,
                    format!("cc-leg-vu-{key}"),
                    &lu,
                    &t("review:swap.components.form.leg.valuationUnitPlaceholder"),
                    true,
                    enabled,
                    move |a| leg_edit(a, &k1, LegField::ValuationUnit),
                ),
            ),
        ));
        cells.push((
            4.0,
            labelled(
                tk,
                &la,
                input(
                    tk,
                    &leg.valuation_amount,
                    format!("cc-leg-va-{key}"),
                    &la,
                    &t("review:swap.components.form.leg.valuationAmountPlaceholder"),
                    true,
                    enabled,
                    move |a| leg_edit(a, &k2, LegField::ValuationAmount),
                ),
            ),
        ));
    }
    let grid = grid12(tk, cells, wide);
    let mut body = col().w_full().gap(tk.r(0.75)).child(head).child(grid);
    if suspense {
        body = body.child(
            text(t("review:swap.components.form.leg.suspenseHint"))
                .font(tk.txs(400.0).lh(tk.r(1.25)))
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    // `relative rounded-lg border bg-card/40 p-4 pl-5`, the role's accent as an `inset-y-3 left-0
    // w-1 rounded-r` rail.
    row()
        .key(format!("cc-leg-{key}"))
        .w_full()
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .fill(tk.card.alpha(0.4))
        .role("group")
        .label(model::role_label(&leg.role))
        .child(
            el().w(tk.r(0.25))
                .my(tk.r(0.75))
                .self_stretch()
                .radius(2.0)
                .fill(role_accent(tk, &leg.role))
                .a11y_hidden(),
        )
        .child(
            col()
                .grow(1.0)
                .w(0.0)
                .min_w(0.0)
                .padding(tk.r(1.0))
                .child(body),
        )
}

/// A 12-column grid (`lg:grid-cols-12` with `col-span-*`), wrapping rows like CSS grid; below
/// `lg` two equal columns.
fn grid12(tk: &Tk, cells: Vec<(f32, El)>, wide: bool) -> El {
    let gap = tk.r(0.75);
    let mut out = col().w_full().gap(gap);
    let mut line = row().w_full().gap(gap).items_end();
    let mut used = 0.0;
    let cols = if wide { 12.0 } else { 2.0 };
    for (span, cell) in cells {
        let span = if wide { span } else { 1.0 };
        if used + span > cols {
            if used < cols {
                line = line.child(el().grow(cols - used).w(0.0));
            }
            out = out.child(line);
            line = row().w_full().gap(gap).items_end();
            used = 0.0;
        }
        line = line.child(col().grow(span).w(0.0).min_w(0.0).child(cell));
        used += span;
    }
    if used > 0.0 {
        if used < cols {
            line = line.child(el().grow(cols - used).w(0.0));
        }
        out = out.child(line);
    }
    out
}

/// `legDisplayLabel`: "2 · Destination · 0.99 BTC".
fn leg_display(leg: &LegUi, idx: usize) -> String {
    let amount = leg.amount.text.trim();
    let base = format!("{} · {}", idx + 1, model::role_label(&leg.role));
    if amount.is_empty() {
        base
    } else {
        format!(
            "{base} · {amount} {}",
            if leg.asset.text.trim().is_empty() {
                "BTC"
            } else {
                leg.asset.text.trim()
            }
        )
    }
}

/// `AllocationRow`.
fn alloc_row(app: &App, tk: &Tk, layer: &mut Vec<El>, f: &FormUi, idx: usize, wide: bool) -> El {
    let a = &f.allocs[idx];
    let enabled = !f.pending();
    let conversion = f.conservation_mode == "conversion";
    let key = a.key.clone();
    let mut pick = |sink: bool| -> El {
        let sel = if sink {
            Sel::AllocSink(key.clone())
        } else {
            Sel::AllocSource(key.clone())
        };
        let anchor = format!("cc-alloc-{}-{key}", if sink { "sink" } else { "source" });
        let current = if sink { &a.sink_key } else { &a.source_key };
        let options: Vec<&LegUi> = f
            .legs
            .iter()
            .filter(|l| {
                if sink {
                    model::is_sink_role(&l.role)
                } else {
                    l.role == "source"
                }
            })
            .collect();
        if app.custody.comps.popup == Some(Popup::Select(sel.clone())) {
            let k = key.clone();
            let opts = options
                .iter()
                .map(|l| {
                    (
                        l.key.clone(),
                        leg_display(l, f.legs.iter().position(|x| x.key == l.key).unwrap()),
                        Some(l.role.clone()),
                    )
                })
                .collect();
            select_menu(tk, layer, &anchor, opts, current, move |app, v| {
                let (k, v) = (k.clone(), v.to_owned());
                patch(app, move |f| {
                    if let Some(x) = f.allocs.iter_mut().find(|x| x.key == k) {
                        if sink {
                            x.sink_key = v;
                        } else {
                            x.source_key = v;
                        }
                    }
                });
            });
        }
        let chosen = f.legs.iter().enumerate().find(|(_, l)| &l.key == current);
        let value = match chosen {
            Some((i, l)) => row()
                .min_w(0.0)
                .gap(tk.r(0.5))
                .center_y()
                .child(role_dot(tk, &l.role))
                .child(
                    text(leg_display(l, i))
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .truncate(),
                ),
            None => text(t("review:swap.components.form.allocations.selectLeg"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .truncate(),
        };
        let label = t(if sink {
            "review:swap.components.form.allocations.sink"
        } else {
            "review:swap.components.form.allocations.source"
        });
        labelled(
            tk,
            &label,
            select_trigger(app, tk, anchor, &label, value, enabled, sel),
        )
    };
    let k1 = key.clone();
    let amount_label = t(if conversion {
        "review:swap.components.form.allocations.sourceAmount"
    } else {
        "review:swap.components.form.allocations.amount"
    });
    let amount = labelled(
        tk,
        &amount_label,
        input(
            tk,
            &a.amount,
            format!("cc-alloc-amt-{key}"),
            &amount_label,
            "0.00000000",
            true,
            enabled,
            move |app| alloc_edit(app, &k1, false),
        ),
    );
    let k3 = key.clone();
    let remove = kit::icon_btn(
        tk,
        format!("cc-alloc-rm-{key}"),
        Var::Ghost,
        tk.r(2.25),
        &I::TRASH_2,
        tk.r(1.0),
        t("review:swap.components.form.allocations.remove"),
    )
    .on_press(move |app: &mut App| remove_alloc(app, &k3));
    let mut r = if wide {
        row().w_full().gap(tk.r(0.75)).items_end()
    } else {
        col().w_full().gap(tk.r(0.75))
    };
    let grow = |e: El, g: f32| {
        if wide {
            col().grow(g).w(0.0).min_w(0.0).child(e)
        } else {
            e
        }
    };
    r = r.child(grow(pick(false), 1.3));
    if wide {
        r = r.child(
            ic(&I::ARROW_RIGHT, tk.r(1.0), tk.muted_fg)
                .mb(tk.r(0.625))
                .a11y_hidden(),
        );
    }
    r = r.child(grow(pick(true), 1.3)).child(grow(amount, 1.0));
    if conversion {
        let k2 = key.clone();
        let l = t("review:swap.components.form.allocations.sinkAmount");
        r = r.child(grow(
            labelled(
                tk,
                &l,
                input(
                    tk,
                    &a.sink_amount,
                    format!("cc-alloc-sink-amt-{key}"),
                    &l,
                    "0.00000000",
                    true,
                    enabled,
                    move |app| alloc_edit(app, &k2, true),
                ),
            ),
            1.0,
        ));
    }
    r = r.child(if wide {
        remove
    } else {
        row().w_full().justify_end().child(remove)
    });
    col()
        .key(format!("cc-alloc-{key}"))
        .w_full()
        .padding(tk.r(0.75))
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .child(r)
}

/// `movementTotals` per role group and asset; and whether sources equal sinks.
fn movement(f: &FormUi) -> Option<El0> {
    let mut groups: [(&str, Vec<(String, i128)>); 5] = [
        ("sources", vec![]),
        ("destinations", vec![]),
        ("fees", vec![]),
        ("external", vec![]),
        ("suspense", vec![]),
    ];
    for l in &f.legs {
        let Some(amount) = model::decimal_btc_to_msat(l.amount.text.trim()) else {
            continue;
        };
        let asset = if l.asset.text.trim().is_empty() {
            "BTC".to_owned()
        } else {
            l.asset.text.trim().to_uppercase()
        };
        let gi = match l.role.as_str() {
            "source" => 0,
            "destination" | "retained" => 1,
            "fee" => 2,
            "external" => 3,
            _ => 4,
        };
        let g = &mut groups[gi].1;
        match g.iter_mut().find(|(a, _)| *a == asset) {
            Some(x) => x.1 += amount,
            None => g.push((asset, amount)),
        }
    }
    let sum = |g: &Vec<(String, i128)>| g.iter().map(|x| x.1).sum::<i128>();
    let sources = sum(&groups[0].1);
    let sinks: i128 = groups[1..].iter().map(|g| sum(&g.1)).sum();
    if sources == 0 && sinks == 0 {
        return None;
    }
    let source_role = |r: &str| r == "source";
    let src_msat: i128 = f
        .legs
        .iter()
        .filter(|l| source_role(&l.role))
        .filter_map(|l| model::decimal_btc_to_msat(l.amount.text.trim()))
        .sum();
    let sink_msat: i128 = f
        .legs
        .iter()
        .filter(|l| model::is_sink_role(&l.role))
        .filter_map(|l| model::decimal_btc_to_msat(l.amount.text.trim()))
        .sum();
    let balanced = src_msat > 0 && src_msat == sink_msat;
    let mut assets: Vec<String> = groups
        .iter()
        .flat_map(|g| g.1.iter().map(|x| x.0.clone()))
        .collect();
    assets.sort();
    assets.dedup();
    let default_asset = groups[0]
        .1
        .first()
        .map(|x| x.0.clone())
        .unwrap_or_else(|| "BTC".into());
    Some(El0 {
        groups: groups.map(|(k, v)| (k, v)).to_vec(),
        balanced,
        difference: sinks - sources,
        mixed: assets.len() > 1,
        default_asset,
        sources,
    })
}

/// What the movement summary shows.
struct El0 {
    groups: Vec<(&'static str, Vec<(String, i128)>)>,
    balanced: bool,
    difference: i128,
    mixed: bool,
    default_asset: String,
    sources: i128,
}

/// `MovementSummary`: "sources = destinations + fees (+ external, suspense)".
fn movement_summary(app: &App, tk: &Tk, m: El0) -> El {
    let hidden = app.prefs.hide_sensitive();
    let warn_text = super::tw(if tk.dark { "amber-100" } else { "amber-900" });
    let status = if m.balanced {
        row()
            .gap(tk.r(0.25))
            .center_y()
            .child(ic(&I::CHECK, tk.r(0.875), tk.muted_fg))
            .child(
                text(t("review:swap.components.form.balance.balanced"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            )
    } else {
        row()
            .gap(tk.r(0.25))
            .center_y()
            .child(ic(&I::TRIANGLE_ALERT, tk.r(0.875), warn_text))
            .child(
                text(t("review:swap.components.form.balance.unbalanced"))
                    .font(tk.txs(500.0))
                    .color(warn_text),
            )
    };
    let head = row()
        .w_full()
        .gap(tk.r(1.5))
        .center_y()
        .justify_between()
        .child(
            text(t("review:swap.components.form.balance.title").to_uppercase())
                .font(tk.t2xs(500.0).tracking(0.14 * tk.r(0.694)))
                .color(tk.muted_fg),
        )
        .child(status);
    let mut line = row()
        .w_full()
        .gap(tk.r(0.5))
        .flex_wrap()
        .items_end()
        .role("group");
    let mut shown = 0;
    for (k, g) in &m.groups {
        let always = matches!(*k, "sources" | "destinations" | "fees");
        let total: i128 = g.iter().map(|x| x.1).sum();
        if !always && total == 0 {
            continue;
        }
        let value = if g.is_empty() {
            model::format_custody_msat(0, &m.default_asset)
        } else {
            g.iter()
                .map(|(a, v)| model::format_custody_msat(*v, a))
                .collect::<Vec<_>>()
                .join(" + ")
        };
        let mut cell = row().gap(tk.r(0.5)).items_end();
        if shown > 0 {
            cell = cell.child(
                text(if shown == 1 { "=" } else { "+" })
                    .font(tk.tsm(400.0).mono())
                    .color(tk.muted_fg)
                    .a11y_hidden(),
            );
        }
        cell = cell
            .child(
                text(t(&format!("review:swap.components.form.balance.{k}")))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            )
            .child(sensitive(
                value,
                tk.tsm(400.0).mono().features("'tnum' 1"),
                tk.fg,
                hidden,
            ));
        line = line.child(cell);
        shown += 1;
    }
    let mut c = col()
        .key("cc-movement")
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(0.75))
        .gap(tk.r(0.5))
        .radius(tk.r(0.5))
        .role("status")
        .child(head)
        .child(line);
    c = if m.balanced {
        c.border(1.0, tk.border).fill(tk.muted.alpha(0.3))
    } else {
        c.border(1.0, super::tw("amber-500").alpha(0.4))
            .fill(super::tw("amber-500").alpha(0.1))
    };
    if !m.balanced && m.sources > 0 {
        let amount = if m.mixed {
            format!("{} msat", m.difference)
        } else {
            model::format_custody_msat(m.difference, &m.default_asset)
        };
        let amount = if hidden {
            crate::theme::MASK.to_owned()
        } else {
            amount
        };
        c = c.child(
            text(tr(
                "review:swap.components.form.balance.difference",
                &[("amount", &amount)],
            ))
            .font(tk.txs(400.0))
            .color(warn_text)
            .wrap(),
        );
    }
    c
}

/// `CustodyErrorList`.
fn error_list(tk: &Tk, key: &str, title: &str, issues: &[model::Issue], destructive: bool) -> El {
    let (fg, bg, border) = if destructive {
        (
            tk.destructive,
            tk.destructive.alpha(0.1),
            tk.destructive.alpha(0.4),
        )
    } else if tk.dark {
        (
            super::tw("amber-100"),
            super::tw("amber-950").alpha(0.3),
            super::tw("amber-400").alpha(0.3),
        )
    } else {
        (
            super::tw("amber-950"),
            super::tw("amber-50"),
            super::tw("amber-300").alpha(0.6),
        )
    };
    let mut list = col().w_full().gap(tk.r(0.25)).pl(tk.r(1.25)).mt(tk.r(0.25));
    for i in issues {
        list = list.child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .items_start()
                .child(text("•").font(tk.tsm(400.0)).color(fg).a11y_hidden())
                .child(
                    text(model::local_issue_text(i))
                        .font(tk.tsm(400.0))
                        .color(fg)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }
    col()
        .key(key.to_owned())
        .w_full()
        .padding(tk.r(0.75))
        .radius(tk.r(0.375))
        .border(1.0, border)
        .fill(bg)
        .role("alert")
        .child(text(title.to_owned()).font(tk.tsm(500.0)).color(fg))
        .child(list)
}

/// `GuidedNotice`.
fn notice(tk: &Tk, key: &str, success: bool, s: String) -> El {
    let (fg, bg, border) = if !success {
        (
            tk.destructive,
            tk.destructive.alpha(0.1),
            tk.destructive.alpha(0.4),
        )
    } else if tk.dark {
        (
            super::tw("emerald-100"),
            super::tw("emerald-950").alpha(0.3),
            super::tw("emerald-400").alpha(0.3),
        )
    } else {
        (
            super::tw("emerald-950"),
            super::tw("emerald-50"),
            super::tw("emerald-200"),
        )
    };
    col()
        .key(key.to_owned())
        .w_full()
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .radius(tk.r(0.375))
        .border(1.0, border)
        .fill(bg)
        .role(if success { "status" } else { "alert" })
        .child(text(s).font(tk.tsm(400.0)).color(fg).wrap())
}

pub fn fact(tk: &Tk, label: String, value: El) -> El {
    row()
        .w_full()
        .gap(tk.r(1.0))
        .items_start()
        .child(
            text(label)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .w(tk.r(7.0)),
        )
        .child(col().grow(1.0).w(0.0).min_w(0.0).child(value))
}

/// `GuidedServerPreview`: every accounting fact the daemon will persist, labelled.
pub fn server_preview(app: &App, tk: &Tk, result: &Value, activates: bool) -> El {
    let comps: Vec<Value> = result["components"].as_array().cloned().unwrap_or_else(|| {
        if result["component"].is_object() {
            vec![result["component"].clone()]
        } else {
            vec![]
        }
    });
    let (count, active, draft) = match result.get("summary").filter(|s| s.is_object()) {
        Some(s) => (
            s["count"].as_u64().unwrap_or(0) as usize,
            s["active"].as_u64().unwrap_or(0).to_string(),
            s["draft"].as_u64().unwrap_or(0).to_string(),
        ),
        None => (
            comps.len(),
            if activates { comps.len() } else { 0 }.to_string(),
            if activates { 0 } else { comps.len() }.to_string(),
        ),
    };
    let mut c = col()
        .key("cc-server-preview")
        .w_full()
        .gap(tk.r(1.0))
        .child(
            col()
                .w_full()
                .gap(tk.r(0.25))
                .child(
                    text(t("review:swap.components.form.review.title"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .role("heading"),
                )
                .child(
                    text(tc(
                        if activates {
                            "review:swap.components.form.review.activates"
                        } else {
                            "review:swap.components.form.review.drafts"
                        },
                        count,
                        &[("active", &active), ("draft", &draft)],
                    ))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .wrap(),
                )
                .child(
                    text(tr(
                        "review:swap.components.form.inputVersion",
                        &[("version", &result["input_version"].to_string())],
                    ))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
                ),
        );
    for comp in &comps {
        c = c.child(component_preview(app, tk, comp));
    }
    c
}

/// One planned component as the daemon resolved it: type and mode, evidence, policy, notes,
/// every leg with its exact amount and references, the allocations, and open issues.
pub fn component_preview(app: &App, tk: &Tk, comp: &Value) -> El {
    let hidden = app.prefs.hide_sensitive();
    let ctype = comp["component_type"].as_str().unwrap_or("");
    let mode = comp["conservation_mode"].as_str().unwrap_or("");
    let mut head = row()
        .w_full()
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
        .child(
            text("·")
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .a11y_hidden(),
        )
        .child(
            text(t(&format!("review:swap.components.mode.{mode}")))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg),
        );
    if comp["evidence_kind"].is_string() || comp["evidence_grade"].is_string() {
        let ev = tr(
            "review:swap.components.audit.evidence",
            &[
                ("kind", comp["evidence_kind"].as_str().unwrap_or("—")),
                ("grade", comp["evidence_grade"].as_str().unwrap_or("—")),
            ],
        );
        head = head.child(
            row()
                .px(tk.r(0.5))
                .py(tk.r(0.125))
                .radius(999.0)
                .border(1.0, tk.border)
                .child(text(ev).font(tk.t2xs(400.0).mono()).color(tk.fg)),
        );
    }
    let mut block = col().w_full().gap(tk.r(0.75)).child(head);
    if let Some(p) = comp["conversion_policy"].as_str().filter(|p| !p.is_empty()) {
        block = block.child(
            text(tr("review:swap.components.audit.policy", &[("policy", p)]))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        );
    }
    if let Some(n) = comp["notes"].as_str().filter(|n| !n.is_empty()) {
        block = block.child(
            text(n.to_owned())
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    let legs = comp["legs"].as_array().cloned().unwrap_or_default();
    let mut ol = col()
        .w_full()
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .fill(tk.background.alpha(0.6))
        .clip_rounded(tk.r(0.375));
    for (i, leg) in legs.iter().enumerate() {
        let asset = leg["asset"].as_str().unwrap_or("BTC");
        let role = leg["role"].as_str().unwrap_or("");
        let top = row()
            .w_full()
            .gap(tk.r(1.0))
            .center_y()
            .justify_between()
            .flex_wrap()
            .child(
                row()
                    .gap(tk.r(0.5))
                    .center_y()
                    .child(role_dot(tk, role))
                    .child(
                        text(tr(
                            "review:swap.components.form.leg.ordinal",
                            &[("index", &(i + 1).to_string())],
                        ))
                        .font(tk.txs(400.0).mono())
                        .color(tk.muted_fg),
                    )
                    .child(
                        text(model::role_label(role))
                            .font(tk.tsm(500.0))
                            .color(tk.fg),
                    ),
            )
            .child(sensitive(
                model::format_custody_msat_v(&leg["amount_msat"], asset),
                tk.tsm(400.0).mono().features("'tnum' 1"),
                tk.fg,
                hidden,
            ));
        let mut dl = col().w_full().gap(tk.r(0.25));
        let mono = |s: String| {
            text(s)
                .font(tk.txs(400.0).mono())
                .color(tk.fg)
                .wrap_anywhere()
        };
        let plain = |s: String| text(s).font(tk.txs(400.0)).color(tk.fg).wrap();
        if let Some(v) = leg["transaction_id"].as_str() {
            dl = dl.child(fact(
                tk,
                t("review:swap.components.form.review.transaction"),
                mono(v.to_owned()),
            ));
        }
        if let Some(v) = leg["wallet_id"].as_str() {
            dl = dl.child(fact(
                tk,
                t("review:swap.components.form.review.wallet"),
                mono(v.to_owned()),
            ));
        }
        if let Some(v) = leg["occurred_at"].as_str() {
            dl = dl.child(fact(
                tk,
                t("review:swap.components.form.review.occurredAt"),
                mono(v.to_owned()),
            ));
        }
        if let Some(v) = leg["rail"].as_str().filter(|v| !v.is_empty()) {
            dl = dl.child(fact(
                tk,
                t("review:swap.components.form.review.rail"),
                plain(v.to_owned()),
            ));
        }
        let scope: Vec<&str> = [leg["chain"].as_str(), leg["network"].as_str()]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect();
        if !scope.is_empty() {
            dl = dl.child(fact(
                tk,
                t("review:swap.components.form.review.network"),
                plain(scope.join(" · ")),
            ));
        }
        let conserved: Vec<&str> = [leg["exposure"].as_str(), leg["conservation_unit"].as_str()]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect();
        if !conserved.is_empty() {
            dl = dl.child(fact(
                tk,
                t("review:swap.components.form.review.conserved"),
                plain(conserved.join(" / ")),
            ));
        }
        if !leg["valuation_amount"].is_null() && leg.get("valuation_amount").is_some() {
            let amount = match &leg["valuation_amount"] {
                Value::String(s) => s.clone(),
                o => o.to_string(),
            };
            dl = dl.child(fact(
                tk,
                t("review:swap.components.form.review.valuation"),
                row()
                    .gap(tk.r(0.25))
                    .child(sensitive(amount, tk.txs(400.0).mono(), tk.fg, hidden))
                    .child(mono(
                        leg["valuation_unit"].as_str().unwrap_or("").to_owned(),
                    )),
            ));
        }
        if let Some(v) = leg["notes"].as_str().filter(|v| !v.is_empty()) {
            dl = dl.child(fact(
                tk,
                t("review:swap.components.form.review.notes"),
                plain(v.to_owned()),
            ));
        }
        let mut li = col()
            .w_full()
            .padding(tk.r(0.75))
            .gap(tk.r(0.5))
            .child(top)
            .child(dl);
        if i > 0 {
            li = li.border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
        }
        ol = ol.child(li);
    }
    block = block.child(ol);
    let allocs = comp["allocations"].as_array().cloned().unwrap_or_default();
    if !allocs.is_empty() {
        let mut ul = col().w_full().gap(tk.r(0.25));
        for a in &allocs {
            let si = legs
                .iter()
                .position(|l| l["id"] == a["source_leg_id"])
                .map(|i| i as i64)
                .unwrap_or(-1);
            let ki = legs
                .iter()
                .position(|l| l["id"] == a["sink_leg_id"])
                .map(|i| i as i64)
                .unwrap_or(-1);
            let asset = |i: i64| {
                legs.get(i.max(0) as usize)
                    .filter(|_| i >= 0)
                    .and_then(|l| l["asset"].as_str())
                    .unwrap_or("BTC")
                    .to_owned()
            };
            let mask = |s: String| {
                if hidden {
                    crate::theme::MASK.to_owned()
                } else {
                    s
                }
            };
            ul = ul.child(
                text(tr(
                    "review:swap.components.form.review.allocationEdge",
                    &[
                        ("source", &(si + 1).to_string()),
                        ("sink", &(ki + 1).to_string()),
                        (
                            "sourceAmount",
                            &mask(model::format_custody_msat_v(
                                &a["source_amount_msat"],
                                &asset(si),
                            )),
                        ),
                        (
                            "sinkAmount",
                            &mask(model::format_custody_msat_v(
                                &a["sink_amount_msat"],
                                &asset(ki),
                            )),
                        ),
                    ],
                ))
                .font(tk.txs(400.0).mono().features("'tnum' 1"))
                .color(tk.fg)
                .wrap(),
            );
        }
        block = block.child(
            col()
                .w_full()
                .gap(tk.r(0.25))
                .child(
                    text(t("review:swap.components.form.review.allocations").to_uppercase())
                        .font(tk.t2xs(500.0).tracking(0.14 * tk.r(0.694)))
                        .color(tk.muted_fg)
                        .role("heading"),
                )
                .child(ul),
        );
    }
    if let Some(issues) = comp["validation"]["issues"]
        .as_array()
        .filter(|i| !i.is_empty())
    {
        block = block.child(super::issue_box(
            tk,
            &t("review:swap.components.form.review.issues"),
            issues.iter().map(model::backend_issue_text).collect(),
        ));
    }
    block
}

/// The form's scrolling body.
pub fn body(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>, wide: bool) -> El {
    let _ = cx;
    let Some(f) = app.custody.comps.form.as_ref() else {
        return el();
    };
    let enabled = !f.pending();
    let form = f.model();
    let preview = model::preview_form(&form);
    let conversion = f.conservation_mode == "conversion";
    let scope = app.expected_scope().to_string();
    let can_confirm = f
        .reviewed
        .as_ref()
        .is_some_and(|r| model::can_confirm(Some(&r.commitment), &f.document(), &scope));

    // Interpretation.
    let type_sel = Sel::Type;
    if app.custody.comps.popup == Some(Popup::Select(type_sel.clone())) {
        select_menu(
            tk,
            layer,
            "cc-type",
            vec![
                (
                    "manual_bridge".into(),
                    t("review:swap.components.type.manual_bridge"),
                    None,
                ),
                ("swap".into(), t("review:swap.components.type.swap"), None),
            ],
            &f.component_type,
            |a, v| {
                let v = v.to_owned();
                patch(a, move |f| f.component_type = v);
            },
        );
    }
    let mode_sel = Sel::Mode;
    if app.custody.comps.popup == Some(Popup::Select(mode_sel.clone())) {
        select_menu(
            tk,
            layer,
            "cc-mode",
            vec![
                (
                    "quantity".into(),
                    t("review:swap.components.mode.quantity"),
                    None,
                ),
                (
                    "conversion".into(),
                    t("review:swap.components.mode.conversion"),
                    None,
                ),
            ],
            &f.conservation_mode,
            |a, v| {
                let v = v.to_owned();
                patch(a, move |f| f.conservation_mode = v);
            },
        );
    }
    let tv = text(model::t_or(
        &format!("review:swap.components.type.{}", f.component_type),
        &f.component_type,
    ))
    .font(tk.tsm(400.0))
    .color(tk.fg);
    let mv = text(t(&format!(
        "review:swap.components.mode.{}",
        f.conservation_mode
    )))
    .font(tk.tsm(400.0))
    .color(tk.fg);
    let pair = |a: El, b: El| {
        if wide {
            row()
                .w_full()
                .gap(tk.r(1.0))
                .child(col().grow(1.0).w(0.0).min_w(0.0).child(a))
                .child(col().grow(1.0).w(0.0).min_w(0.0).child(b))
        } else {
            col().w_full().gap(tk.r(1.0)).child(a).child(b)
        }
    };
    let mut interp = col().w_full().gap(tk.r(0.75)).child(pair(
        labelled(
            tk,
            &t("review:swap.components.form.componentType"),
            select_trigger(
                app,
                tk,
                "cc-type".into(),
                &t("review:swap.components.form.componentType"),
                tv,
                enabled,
                type_sel,
            ),
        ),
        labelled(
            tk,
            &t("review:swap.components.form.conservationMode"),
            select_trigger(
                app,
                tk,
                "cc-mode".into(),
                &t("review:swap.components.form.conservationMode"),
                mv,
                enabled,
                mode_sel,
            ),
        ),
    ));
    if conversion {
        let l = t("review:swap.components.form.conversionPolicy");
        let pol = labelled(
            tk,
            &l,
            input(
                tk,
                &f.conversion_policy,
                "cc-policy".into(),
                &l,
                &t("review:swap.components.form.conversionPolicyPlaceholder"),
                true,
                enabled,
                |a| &mut a.custody.comps.form.as_mut().unwrap().conversion_policy,
            ),
        );
        let reviewed_label = t("review:swap.components.form.conversionReviewed");
        let rev = row()
            .min_h(tk.r(2.25))
            .gap(tk.r(0.5))
            .center_y()
            .child(checkbox(
                tk,
                "cc-reviewed",
                f.conversion_reviewed,
                &reviewed_label,
                enabled,
                |a| patch(a, |f| f.conversion_reviewed = !f.conversion_reviewed),
            ))
            .child(
                text(reviewed_label.clone())
                    .font(tk.tsm(400.0))
                    .color(tk.fg)
                    .wrap(),
            );
        let cyan = super::tw("cyan-500");
        interp = interp.child(
            col()
                .w_full()
                .padding(tk.r(1.0))
                .radius(tk.r(0.5))
                .border(1.0, cyan.alpha(0.3))
                .fill(cyan.alpha(0.05))
                .child(if wide {
                    row()
                        .w_full()
                        .gap(tk.r(1.0))
                        .items_end()
                        .child(col().grow(1.0).w(0.0).child(pol))
                        .child(col().grow(1.0).w(0.0).child(rev))
                } else {
                    col().w_full().gap(tk.r(1.0)).child(pol).child(rev)
                }),
        );
    }
    let mut out = col()
        .key("cc-form-body")
        .w_full()
        .gap(tk.r(1.75))
        .child(section(
            tk,
            &t("review:swap.components.form.sections.interpretation"),
            &t("review:swap.components.form.sections.interpretationHint"),
            None,
            interp,
        ));

    // Legs.
    let mut legs = col().w_full().gap(tk.r(0.75));
    for i in 0..f.legs.len() {
        legs = legs.child(leg_row(app, tk, layer, f, i, wide));
    }
    let bitcoin_summary = f.legs.iter().all(|l| {
        let a = if l.asset.text.trim().is_empty() {
            "BTC".to_owned()
        } else {
            l.asset.text.trim().to_uppercase()
        };
        (a == "BTC" || a == "LBTC")
            && l.origin.as_ref().is_none_or(|o| {
                o.exposure.as_deref().is_none_or(|e| e == "bitcoin")
                    && o.conservation_unit.as_deref().is_none_or(|u| u == "msat")
            })
    });
    if f.conservation_mode == "quantity"
        && bitcoin_summary
        && let Some(m) = movement(f)
    {
        legs = legs.child(movement_summary(app, tk, m));
    }
    let add_leg_btn = kit::btn(
        tk,
        "cc-add-leg",
        Var::Outline,
        Some(&I::PLUS),
        &t("review:swap.components.form.addLeg"),
        enabled,
    )
    .on_press(add_leg);
    out = out.child(section(
        tk,
        &t("review:swap.components.form.legsTitle"),
        &t("review:swap.components.form.legsHint"),
        Some(add_leg_btn),
        legs,
    ));

    // Allocations.
    let sources = f.legs.iter().filter(|l| l.role == "source").count();
    let has_suspense = f.legs.iter().any(|l| l.role == "suspense");
    if !f.allocs.is_empty() || sources > 1 || has_suspense || conversion {
        let body = if f.allocs.is_empty() {
            col()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .radius(tk.r(0.5))
                .border(1.0, tk.border)
                .border_dash(4.0, 3.0)
                .child(
                    text(t("review:swap.components.form.allocations.empty"))
                        .font(tk.txs(400.0).lh(tk.r(1.25)))
                        .color(tk.muted_fg)
                        .wrap(),
                )
        } else {
            let mut c = col().w_full().gap(tk.r(0.75));
            for i in 0..f.allocs.len() {
                c = c.child(alloc_row(app, tk, layer, f, i, wide));
            }
            c
        };
        let add = kit::btn(
            tk,
            "cc-add-alloc",
            Var::Outline,
            Some(&I::PLUS),
            &t("review:swap.components.form.allocations.add"),
            enabled,
        )
        .on_press(add_alloc);
        out = out.child(section(
            tk,
            &t("review:swap.components.form.allocations.title"),
            &t("review:swap.components.form.allocations.hint"),
            Some(add),
            body,
        ));
    }

    // Evidence and notes.
    let k = crate::txdetail::widgets::form_kit(tk);
    let notes_label = t("review:swap.components.form.notes");
    let mut notes = TextArea::new(&f.notes)
        .label(notes_label.clone())
        .placeholder(t("review:swap.components.form.notesPlaceholder"))
        .rows(3)
        .width(None);
    if enabled {
        notes = notes.on_edit(|a: &mut App| &mut a.custody.comps.form.as_mut().unwrap().notes);
    }
    let notes = crate::txdetail::widgets::field(
        tk,
        notes.build(&k).key("cc-notes").min_h(tk.r(5.0)).w_full(),
    );
    let ek_label = t("review:swap.components.form.evidenceKind");
    let ek = col()
        .w_full()
        .gap(tk.r(0.375))
        .child(labelled(
            tk,
            &ek_label,
            input(
                tk,
                &f.evidence_kind,
                "cc-evidence-kind".into(),
                &ek_label,
                "manual_migration_review",
                true,
                enabled,
                |a| &mut a.custody.comps.form.as_mut().unwrap().evidence_kind,
            ),
        ))
        .child(
            text(t("review:swap.components.form.evidenceKindHint"))
                .font(tk.txs(400.0).lh(tk.r(1.25)))
                .color(tk.muted_fg)
                .wrap(),
        );
    let ev = if wide {
        row()
            .w_full()
            .gap(tk.r(1.0))
            .items_start()
            .child(
                col()
                    .grow(1.0)
                    .w(0.0)
                    .min_w(0.0)
                    .child(labelled(tk, &notes_label, notes)),
            )
            .child(col().w(tk.r(18.0)).child(ek))
    } else {
        col()
            .w_full()
            .gap(tk.r(1.0))
            .child(labelled(tk, &notes_label, notes))
            .child(ek)
    };
    out = out.child(section(
        tk,
        &t("review:swap.components.form.sections.evidence"),
        &t("review:swap.components.form.sections.evidenceHint"),
        None,
        ev,
    ));

    let show_validation = f.initial || model::has_input(&form);
    if show_validation && !preview.structural.is_empty() {
        out = out.child(error_list(
            tk,
            "cc-structural",
            &t("review:swap.components.structuralErrors"),
            &preview.structural,
            true,
        ));
    }
    if show_validation && !preview.activation.is_empty() {
        out = out.child(error_list(
            tk,
            "cc-activation",
            &t("review:swap.components.activationErrors"),
            &preview.activation,
            false,
        ));
    }
    if let (Some(r), true) = (&f.reviewed, can_confirm) {
        out = out.child(
            col()
                .key("cc-review")
                .w_full()
                .padding(tk.r(1.0))
                .radius(tk.r(0.5))
                .border(1.0, tk.primary.alpha(0.3))
                .fill(tk.muted.alpha(0.2))
                .child(server_preview(app, tk, &r.result, r.activate)),
        );
    }
    out
}

/// The footer: the result or error, Cancel / Start over, the two previews and the confirmation.
pub fn footer(app: &App, tk: &Tk, wide: bool) -> El {
    let Some(f) = app.custody.comps.form.as_ref() else {
        return el();
    };
    let pending = f.pending();
    let preview = model::preview_form(&f.model());
    let scope = app.expected_scope().to_string();
    let reviewed = f
        .reviewed
        .as_ref()
        .filter(|r| model::can_confirm(Some(&r.commitment), &f.document(), &scope));
    let has_review = reviewed.is_some();
    let can_activate = preview.structural.is_empty() && preview.activation.is_empty();
    let can_draft = preview.structural.is_empty();
    let mut c = col().key("cc-form-footer").w_full().gap(tk.r(0.75));
    if let Some(s) = &f.result {
        let count = s["count"].as_u64().unwrap_or(0) as usize;
        c = c.child(notice(
            tk,
            "cc-saved",
            true,
            tc(
                "review:swap.components.savedSummary",
                count,
                &[
                    ("active", &s["active"].as_u64().unwrap_or(0).to_string()),
                    ("draft", &s["draft"].as_u64().unwrap_or(0).to_string()),
                ],
            ),
        ));
    }
    if let Some(e) = &f.action_error {
        c = c.child(notice(tk, "cc-error", false, e.clone()));
    }
    let left = if f.edit.is_some() {
        kit::btn_md(
            tk,
            "cc-cancel",
            Var::Ghost,
            None,
            &t("review:swap.components.revisionDialog.cancel"),
            !pending,
        )
        .on_press(super::close_form)
    } else {
        kit::btn_md(
            tk,
            "cc-reset",
            Var::Ghost,
            Some(&I::ROTATE_CCW),
            &t("review:swap.components.form.reset"),
            !pending,
        )
        .on_press(reset)
    };
    let busy = |which: InFlight| f.in_flight == Some(which);
    let draft_var = if has_review { Var::Outline } else { Var::Ghost };
    let mut draft = md_busy(
        tk,
        "cc-preview-draft",
        draft_var,
        busy(InFlight::Draft),
        &I::SAVE,
        &t("review:swap.components.form.previewDraft"),
        !pending && can_draft,
    )
    .on_press(|a: &mut App| submit(a, false));
    if !has_review {
        // `variant="secondary"`: `bg-secondary`.
        draft = draft.fill(tk.muted).hover(tk.muted.alpha(0.8));
    }
    let act = md_busy(
        tk,
        "cc-preview-activate",
        if has_review {
            Var::Outline
        } else {
            Var::Default
        },
        busy(InFlight::Activate),
        &I::CHECK,
        &t("review:swap.components.form.previewActivation"),
        !pending && can_activate,
    )
    .on_press(|a: &mut App| submit(a, true));
    let mut right = row()
        .gap(tk.r(0.5))
        .flex_wrap()
        .justify_end()
        .child(draft)
        .child(act);
    if let Some(r) = reviewed {
        let b = md_busy(
            tk,
            "cc-confirm",
            Var::Default,
            busy(InFlight::Confirm),
            &I::CHECK,
            &t(if r.activate {
                "review:swap.components.form.confirmActivation"
            } else {
                "review:swap.components.form.confirmDraft"
            }),
            !pending,
        )
        .on_press(confirm);
        right = right.child(b);
    }
    let bar = if wide {
        row()
            .w_full()
            .gap(tk.r(0.5))
            .center_y()
            .justify_between()
            .child(left)
            .child(right)
    } else {
        col().w_full().gap(tk.r(0.5)).child(right).child(left)
    };
    c.child(bar)
}

/// shadcn `Button` (default size, `h-9`) whose glyph is the spinner while `busy`.
pub fn md_busy(
    tk: &Tk,
    key: &str,
    variant: Var,
    busy: bool,
    glyph: &'static Icon,
    label: &str,
    enabled: bool,
) -> El {
    crate::ui::button::button_busy(
        tk,
        key,
        variant,
        crate::ui::button::Size::Default,
        busy,
        Some(glyph),
        label,
        enabled,
    )
}
