//! The Tax tab (TransactionTaxTab.tsx, TransactionTaxModel.ts) and the acquisition review
//! (AcquisitionReview.tsx): a reviewed, separate save through ui.review.plan / ui.review.apply.

use rune::{Color, El, col, row, text};
use serde_json::Value;

use super::draft::{self, Direction, f, opt_s, s};
use super::header::Cause;
use super::widgets::{callout, field_label, hint, select_menu, select_trigger, toggle_select};
use super::{Open, Select, Tab};
use crate::app::App;
use crate::format;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::popup::TriggerExt;

const UNCLASSIFIED: &str = "__unclassified__";

/// The "Plain English" sentence (TransactionDetailSheet.tsx:866-898).
pub fn narrative(o: &Open, r: &Value) -> String {
    let action = match draft::direction(r) {
        Direction::Receive => t("transactions:tax.narrative.received"),
        Direction::Send => t("transactions:tax.narrative.sent"),
        Direction::Transfer => t("transactions:tax.narrative.moved"),
    };
    let dir = if draft::direction(r) == Direction::Send {
        t("transactions:tax.narrative.to")
    } else {
        t("transactions:tax.narrative.from")
    };
    let counterparty =
        opt_s(r, "counter").unwrap_or_else(|| t("transactions:tax.narrative.theCounterparty"));
    let fiat = match f(r, "eur").filter(|x| *x != 0.0) {
        Some(v) => tr(
            "transactions:tax.narrative.worthAtTime",
            &[("value", &format::fiat(v.abs(), "EUR"))],
        ),
        None => t("transactions:tax.narrative.noFiatYet"),
    };
    let treatment = if o.draft.excluded {
        t("transactions:tax.narrative.excludedTreatment")
    } else {
        tr(
            "transactions:tax.narrative.currentTreatment",
            &[(
                "treatment",
                &draft::tax_label(draft::tax_option(&o.draft.at_regime, &o.draft.at_category)),
            )],
        )
    };
    tr(
        "transactions:tax.narrative.sentence",
        &[
            ("action", &action),
            ("amount", &draft::btc_amount(draft::amount_btc(r))),
            ("direction", &dir),
            ("counterparty", &counterparty),
            ("date", s(r, "date")),
            ("fiat", &fiat),
            ("treatment", &treatment),
        ],
    )
}

/// `summarizeTransactionTaxEffect(events)`.
struct Effect {
    state: &'static str,
    cost: Option<f64>,
    proceeds: Option<f64>,
    gain: Option<f64>,
}

fn sum(events: &[Value], key: &str) -> Option<f64> {
    let vals: Vec<f64> = events.iter().filter_map(|e| f(e, key)).collect();
    (!vals.is_empty()).then(|| vals.iter().sum())
}

fn effect(events: &[Value]) -> Effect {
    if events.is_empty() {
        return Effect {
            state: "pending",
            cost: None,
            proceeds: None,
            gain: None,
        };
    }
    let ty = |e: &Value| s(e, "entryType").to_owned();
    let disposal: Vec<Value> = events
        .iter()
        .filter(|e| {
            matches!(
                ty(e).as_str(),
                "disposal" | "fee" | "transfer_fee" | "neutral_swap"
            )
        })
        .cloned()
        .collect();
    let income: Vec<Value> = events
        .iter()
        .filter(|e| ty(e) == "income")
        .cloned()
        .collect();
    let transfer = events
        .iter()
        .any(|e| matches!(ty(e).as_str(), "transfer_in" | "transfer_out"));
    if !disposal.is_empty() {
        return Effect {
            state: "disposal",
            cost: sum(&disposal, "costBasisEur"),
            proceeds: sum(&disposal, "proceedsEur").or_else(|| sum(&disposal, "fiatValueEur")),
            gain: sum(&disposal, "gainLossEur"),
        };
    }
    if !income.is_empty() {
        return Effect {
            state: "income",
            cost: sum(&income, "costBasisEur").or_else(|| sum(&income, "fiatValueEur")),
            proceeds: None,
            gain: None,
        };
    }
    if transfer {
        return Effect {
            state: "transfer",
            cost: None,
            proceeds: None,
            gain: None,
        };
    }
    Effect {
        state: "acquisition",
        cost: sum(events, "fiatValueEur"),
        proceeds: None,
        gain: None,
    }
}

pub fn tax(app: &App, tk: &Tk, o: &Open, r: &Value, layer: &mut Vec<El>) -> El {
    let hide = app.prefs.hide_sensitive();
    let dirt = o.dirty();
    // `space-y-3`; the sentence box `rounded-md border bg-muted/50 p-3 text-sm leading-relaxed`
    // with its `mb-1 text-xs font-semibold uppercase text-muted-foreground` heading.
    let mut body = col().key("txd-tax").w_full().gap(tk.r(0.75));
    let box_ = || {
        col()
            .w_full()
            .min_w(0.0)
            .padding(tk.r(0.75))
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
    };
    body = body.child(
        box_()
            .fill(tk.muted.alpha(0.5))
            .child(
                row()
                    .mb(tk.r(0.25))
                    .gap(tk.r(0.375))
                    .center_y()
                    .child(
                        text(t("transactions:tax.plainEnglish").to_uppercase())
                            .font(tk.txs(600.0))
                            .color(tk.muted_fg),
                    )
                    .child(hint(
                        tk,
                        &t("transactions:tax.plainEnglish"),
                        t("transactions:tax.plainEnglishHint"),
                    )),
            )
            .child(
                super::widgets::sens(
                    tk,
                    narrative(o, r),
                    tk.tsm(400.0).lh(tk.r(0.875) * 1.625),
                    tk.fg,
                    hide,
                )
                .wrap(),
            ),
    );
    let q = super::header::quarantine(app, o, r);
    if q.as_ref()
        .is_some_and(|q| q.cause == Cause::Basis && q.target != Tab::Tax)
    {
        body = body.child(callout(
            tk,
            &tk.warn,
            &I::TRIANGLE_ALERT,
            &t("transactions:tax.basisBlockerTitle"),
            Some(tr(
                "transactions:tax.basisBlockerBody",
                &[("asset", opt_s(r, "asset").as_deref().unwrap_or("asset"))],
            )),
        ));
    }
    match draft::direction(r) {
        Direction::Transfer => {}
        Direction::Receive => body = body.child(acquisition(app, tk, o, r, layer)),
        Direction::Send => {
            let current = o.draft.kind.clone().unwrap_or_else(|| UNCLASSIFIED.into());
            let label = if current == UNCLASSIFIED {
                t("transactions:tax.kindUnclassified")
            } else {
                draft::kind_label(&current)
            };
            body = body.child(
                box_()
                    .fill(tk.background)
                    .items_start()
                    .gap(tk.r(0.375))
                    .child(field_label(
                        tk,
                        &t("transactions:tax.kind"),
                        Some(t("transactions:tax.kindHint")),
                        dirt.kind,
                    ))
                    .child(
                        select_trigger(
                            tk,
                            "txd-tax-kind",
                            &t("transactions:tax.kind"),
                            &label,
                            true,
                        )
                        .select_trigger(o.select == Some(Select::Kind), |a: &mut App| {
                            toggle_select(a, Select::Kind)
                        }),
                    ),
            );
            if o.select == Some(Select::Kind) {
                let mut opts = vec![(
                    UNCLASSIFIED.to_owned(),
                    t("transactions:tax.kindUnclassified"),
                )];
                opts.extend(
                    draft::OUTBOUND_KINDS
                        .iter()
                        .map(|k| (k.to_string(), draft::kind_label(k))),
                );
                select_menu(tk, layer, "txd-tax-kind", opts, &current, |a, v| {
                    if let Some(o) = a.detail.open.as_mut() {
                        o.draft.kind = (v != UNCLASSIFIED).then(|| v.to_owned());
                    }
                });
            }
        }
    }
    // Classification box.
    let opt = draft::tax_option(&o.draft.at_regime, &o.draft.at_category);
    let current = format!("{}:{}", opt.regime, opt.category);
    if o.select == Some(Select::Category) {
        let opts = draft::TAX_OPTIONS
            .iter()
            .map(|x| (format!("{}:{}", x.regime, x.category), draft::tax_label(x)))
            .collect();
        select_menu(tk, layer, "txd-tax-category", opts, &current, |a, v| {
            let (reg, cat) = v.split_once(':').unwrap_or((v, ""));
            let o2 = draft::tax_option(reg, cat);
            if let Some(o) = a.detail.open.as_mut() {
                o.draft.at_regime = o2.regime.into();
                o.draft.at_category = o2.category.into();
                o.draft.taxable = o2.taxable;
            }
        });
    }
    let taxable_label = t("transactions:tax.taxable");
    let excluded_label = t("transactions:tax.excluded");
    // Each switch sits in `flex h-9 items-center gap-2 rounded-md border px-2.5` with its
    // `text-xs` label (Excluded adds "(<kbd>e</kbd>)").
    let switch_box = |label_row: El, sw: El| {
        row()
            .h(tk.r(2.25))
            .px(tk.r(0.625))
            .gap(tk.r(0.5))
            .center_y()
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .child(label_row)
            .child(sw)
    };
    let small_label = |l: &str| {
        text(l.to_owned())
            .font(tk.txs(500.0).lh(tk.r(0.75)))
            .color(tk.fg)
    };
    let taxable = switch_box(
        row()
            .gap(tk.r(0.375))
            .center_y()
            .child(small_label(&taxable_label))
            .children(dirt.taxable.then(|| super::widgets::dirty_dot(tk))),
        super::widgets::ui_switch(
            tk,
            "txd-taxable",
            o.draft.taxable,
            &taxable_label,
            |a: &mut App| {
                if let Some(o) = a.detail.open.as_mut() {
                    o.draft.taxable = !o.draft.taxable;
                }
            },
        ),
    );
    let excluded = switch_box(
        row()
            .gap(tk.r(0.375))
            .center_y()
            .child(small_label(&excluded_label))
            .children(dirt.excluded.then(|| super::widgets::dirty_dot(tk)))
            .child(
                row()
                    .center_y()
                    .child(text("(").font(tk.txs(400.0)).color(tk.muted_fg))
                    .child(super::widgets::kbd(tk, "e"))
                    .child(text(")").font(tk.txs(400.0)).color(tk.muted_fg)),
            ),
        super::widgets::ui_switch(
            tk,
            "txd-excluded",
            o.draft.excluded,
            &excluded_label,
            |a: &mut App| {
                if let Some(o) = a.detail.open.as_mut() {
                    o.draft.excluded = !o.draft.excluded;
                }
            },
        ),
    );
    let category = col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.375))
        .child(field_label(
            tk,
            &t("transactions:tax.austrianCategory"),
            Some(t("transactions:tax.austrianCategoryHint")),
            dirt.at,
        ))
        .child(
            select_trigger(
                tk,
                "txd-tax-category",
                &t("transactions:tax.austrianCategory"),
                &draft::tax_label(opt),
                true,
            )
            .w_full()
            .select_trigger(o.select == Some(Select::Category), |a: &mut App| {
                toggle_select(a, Select::Category)
            }),
        );
    let toggles = row()
        .gap(tk.r(0.5))
        .flex_wrap()
        .center_y()
        .child(taxable)
        .child(excluded);
    // `grid gap-3 @2xl:grid-cols-[minmax(0,1fr)_auto] @2xl:items-end`: side by side once the tab
    // panel is 42 rem wide (a wide docked panel).
    let wide = super::view::tab_width() >= tk.r(42.0);
    let inner = if wide {
        row()
            .w_full()
            .gap(tk.r(0.75))
            .items_end()
            .child(category.grow(1.0))
            .child(toggles.justify_end())
    } else {
        col()
            .w_full()
            .gap(tk.r(0.75))
            .child(category)
            .child(toggles)
    };
    body = body.child(box_().fill(tk.background).child(inner));
    body.child(effect_table(app, tk, o))
}

fn effect_table(app: &App, tk: &Tk, o: &Open) -> El {
    let hide = app.prefs.hide_sensitive();
    let events: Vec<Value> = o
        .events
        .as_ref()
        .and_then(|e| e["events"].as_array().cloned())
        .unwrap_or_default();
    let eff = effect(&events);
    let money = |v: Option<f64>, fallback: String| -> (String, bool) {
        match v {
            Some(x) => (format::fiat(x, "EUR"), true),
            None => (fallback, false),
        }
    };
    let pending = t("transactions:tax.journalPending");
    let (cost_label, cost, proceeds, gain) = match eff.state {
        "transfer" => (
            t("transactions:tax.basisTreatment"),
            money(None, t("transactions:tax.basisCarriedForward")),
            money(None, t("transactions:tax.noDisposal")),
            money(None, t("transactions:tax.noRealization")),
        ),
        "disposal" => (
            t("transactions:tax.costBasis"),
            money(eff.cost, pending.clone()),
            money(eff.proceeds, pending.clone()),
            money(eff.gain, pending.clone()),
        ),
        "income" => (
            t("transactions:tax.costBasis"),
            money(eff.cost, pending.clone()),
            money(None, t("transactions:tax.incomeRecognized")),
            money(None, t("transactions:tax.taxableIncome")),
        ),
        "acquisition" => (
            t("transactions:tax.basisAdded"),
            money(eff.cost, pending.clone()),
            money(None, t("transactions:tax.noDisposal")),
            money(None, t("transactions:tax.notRealized")),
        ),
        _ => (
            t("transactions:tax.costBasis"),
            money(None, pending.clone()),
            money(None, pending.clone()),
            money(None, pending),
        ),
    };
    // `LedgerRow`: `grid min-h-10 grid-cols-[minmax(140px,0.9fr)_minmax(0,1.1fr)] items-center
    // gap-3 border-b px-3 py-2`, muted rows `bg-muted/50`; the label `text-xs
    // text-muted-foreground` with its hint, the value `text-sm font-medium text-right tabular-nums`.
    let line = |label: String,
                hint_text: String,
                (v, is_money): (String, bool),
                muted: bool,
                last: bool|
     -> El {
        row()
            .w_full()
            .min_h(tk.r(2.5))
            .px(tk.r(0.75))
            .py(tk.r(0.5))
            .gap(tk.r(0.75))
            .center_y()
            .fill(if muted {
                tk.muted.alpha(0.5)
            } else {
                Color::TRANSPARENT
            })
            .border_sides([0.0, 0.0, if last { 0.0 } else { 1.0 }, 0.0], tk.border)
            .child(
                row()
                    .gap(tk.r(0.375))
                    .center_y()
                    .grow(0.9)
                    .min_w(140.0)
                    .child(
                        text(label.clone())
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .truncate()
                            .help_when_truncated(label.clone()),
                    )
                    .child(hint(tk, &label, hint_text)),
            )
            .child(
                row()
                    .grow(1.1)
                    .min_w(0.0)
                    .justify_end()
                    .child(super::widgets::sens(
                        tk,
                        v,
                        tk.tsm(500.0).features("'tnum' 1"),
                        tk.fg,
                        hide && is_money,
                    )),
            )
    };
    let gain_muted = !matches!(eff.state, "disposal" | "income");
    let manual = o.draft.pricing_source_kind.as_deref() == Some("manual_override");
    let refs: Vec<&Value> = events
        .iter()
        .filter(|e| e.get("explanationReference").is_some_and(|x| !x.is_null()))
        .collect();
    let gain_last = !manual && refs.is_empty();
    // `overflow-hidden rounded-md border`; the header `border-b bg-muted px-3 py-1.5 text-2xs
    // font-semibold uppercase tracking-wide text-muted-foreground`.
    let mut table = col()
        .key("txd-tax-effect")
        .w_full()
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .clip_rounded(tk.r(0.375))
        .child(
            row()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.375))
                .fill(tk.muted)
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .child(
                    text(t("transactions:tax.projectedEffect").to_uppercase())
                        .font(tk.t2xs(600.0).tracking(0.25))
                        .color(tk.muted_fg),
                ),
        )
        .child(line(
            cost_label,
            t("transactions:tax.costBasisHint"),
            cost,
            false,
            false,
        ))
        .child(line(
            t("transactions:tax.proceeds"),
            t("transactions:tax.proceedsHint"),
            proceeds,
            false,
            false,
        ))
        .child(line(
            t("transactions:tax.gainLoss"),
            t("transactions:tax.gainLossHint"),
            gain,
            gain_muted,
            gain_last,
        ));
    if manual {
        let src = if o.draft.manual_source.trim().is_empty() {
            t("transactions:tax.sourceMissing")
        } else {
            o.draft.manual_source.clone()
        };
        table = table.child(line(
            t("transactions:tax.priceEvidence"),
            t("transactions:pricing.evidenceHint"),
            (src, false),
            true,
            refs.is_empty(),
        ));
    }
    // `TaxCalculationRows`: each realized disposal or income opens the tax engine's calculation
    // (`CapitalGainExplanation`: a ghost `h-7 px-2 text-xs` "Explain" button and its dialog).
    for (i, e) in refs.iter().enumerate() {
        let label = if refs.len() > 1 {
            format!("{} {}", t("transactions:tax.calculation"), i + 1)
        } else {
            t("transactions:tax.calculation")
        };
        let reference = e["explanationReference"].clone();
        let last = i + 1 == refs.len();
        let button = super::widgets::button(
            tk,
            format!("txd-explain-{i}"),
            super::widgets::Variant::Ghost,
            super::widgets::Size::Xs,
            None,
            &t("journals:explanation.open"),
            true,
        )
        .label(tr(
            "transactions:tax.explainCalculation",
            &[("label", &label)],
        ))
        .on_press(move |a: &mut App| super::actions::open_explanation(a, reference.clone()));
        table = table.child(
            row()
                .w_full()
                .min_h(tk.r(2.5))
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .gap(tk.r(0.75))
                .center_y()
                .border_sides([0.0, 0.0, if last { 0.0 } else { 1.0 }, 0.0], tk.border)
                .child(
                    row()
                        .gap(tk.r(0.375))
                        .center_y()
                        .grow(0.9)
                        .min_w(140.0)
                        .child(
                            text(label.clone())
                                .font(tk.txs(400.0))
                                .color(tk.muted_fg)
                                .truncate()
                                .help_when_truncated(label.clone()),
                        )
                        .child(hint(tk, &label, t("transactions:tax.calculationHint"))),
                )
                .child(
                    row()
                        .grow(1.1)
                        .min_w(0.0)
                        .justify_end()
                        .child(row().my(-tk.r(0.25)).child(button)),
                ),
        );
    }
    table
}

// ---- Acquisition review (§5.6) ----------------------------------------------------------------

fn acquisition(app: &App, tk: &Tk, o: &Open, r: &Value, layer: &mut Vec<El>) -> El {
    let hide = app.prefs.hide_sensitive();
    let locked = o.dirty().count > 0;
    let initial = opt_s(r, "kindOverride");
    let selected = o.acq_kind.clone().or(initial.clone());
    let current = selected.clone().unwrap_or_else(|| UNCLASSIFIED.into());
    let label = if current == UNCLASSIFIED {
        t("transactions:tax.kindUnclassified")
    } else {
        draft::kind_label(&current)
    };
    let changed = selected != initial;
    // `<section space-y-3 rounded-md border bg-background p-3>`.
    let mut c = col()
        .key("txd-acq")
        .w_full()
        .min_w(0.0)
        .items_start()
        .padding(tk.r(0.75))
        .gap(tk.r(0.75))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .fill(tk.background)
        .child(field_label(tk, &t("transactions:tax.kind"), None, false))
        .child(
            text(t("transactions:tax.acquisition.help"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        );
    let mut trigger = select_trigger(
        tk,
        "txd-acq-kind",
        &t("transactions:tax.kind"),
        &label,
        !locked,
    );
    if !locked {
        trigger = trigger.select_trigger(o.select == Some(Select::AcqKind), |a: &mut App| {
            toggle_select(a, Select::AcqKind)
        });
    }
    c = c.child(trigger);
    if o.select == Some(Select::AcqKind) {
        let mut opts = vec![(
            UNCLASSIFIED.to_owned(),
            t("transactions:tax.kindUnclassified"),
        )];
        opts.extend(
            draft::INBOUND_KINDS
                .iter()
                .map(|k| (k.to_string(), draft::kind_label(k))),
        );
        select_menu(tk, layer, "txd-acq-kind", opts, &current, |a, v| {
            if let Some(o) = a.detail.open.as_mut() {
                o.acq_kind = (v != UNCLASSIFIED).then(|| v.to_owned());
                o.acq_preview = None;
                o.acq_error = None;
            }
        });
    }
    if locked {
        c = c.child(
            text(t("transactions:tax.acquisition.dirty"))
                .font(tk.txs(400.0))
                .color(tk.warn.text)
                .wrap(),
        );
    }
    let can_preview = changed && !locked && !o.busy.acq;
    let mut buttons = row().gap(tk.r(0.375));
    let mut pv = super::widgets::button(
        tk,
        "txd-acq-preview",
        super::widgets::Variant::Outline,
        super::widgets::Size::Default,
        None,
        &t("transactions:tax.acquisition.preview"),
        can_preview,
    );
    if can_preview {
        pv = pv.on_press(super::actions::acq_preview);
    }
    buttons = buttons.child(pv);
    if o.busy.acq {
        buttons = buttons.child(
            super::widgets::spinner(tk, tk.r(0.875))
                .role("status")
                .label(t("common:state.loading")),
        );
    }
    if let Some(p) = &o.acq_preview {
        let ok = !locked && !o.busy.acq;
        let mut ap = super::widgets::button(
            tk,
            "txd-acq-apply",
            super::widgets::Variant::Default,
            super::widgets::Size::Default,
            None,
            &t("transactions:tax.acquisition.apply"),
            ok,
        );
        if ok {
            ap = ap.on_press(super::actions::acq_apply);
        }
        buttons = buttons.child(ap);
        let scope = o.acq_scope.clone().unwrap_or(Value::Null);
        let cur = s(&scope, "fiat_currency").to_owned();
        let version = scope["input_version"].to_string();
        c = c.child(
            text(tr(
                "transactions:tax.acquisition.scope",
                &[("currency", &cur), ("version", &version)],
            ))
            .font(tk.t2xs(400.0))
            .color(tk.muted_fg)
            .wrap(),
        );
        c = c.child(preview_rows(tk, p, hide));
        if p["after"]["report_ready"].as_bool() == Some(false) {
            c = c.child(callout(
                tk,
                &tk.warn,
                &I::TRIANGLE_ALERT,
                &t("transactions:tax.acquisition.blocked"),
                None,
            ));
            for bl in p["after"]["quarantines"]
                .as_array()
                .cloned()
                .unwrap_or_default()
            {
                let code = s(&bl, "reason").to_owned();
                let key = match code.as_str() {
                    "acquisition_valuation_unsupported" => {
                        "transactions:tax.acquisition.unsupportedValuation"
                    }
                    "basis_provenance_incomplete" => "transactions:tax.acquisition.incompleteBasis",
                    _ => "transactions:tax.acquisition.otherBlocker",
                };
                c = c.child(text(t(key)).font(tk.txs(400.0)).color(tk.fg).wrap());
            }
        }
    }
    c = c.child(buttons);
    if let Some(e) = &o.acq_error {
        c = c.child(
            text(e.clone())
                .font(tk.txs(400.0))
                .color(tk.destructive)
                .role("alert")
                .wrap(),
        );
    }
    c
}

fn preview_rows(tk: &Tk, p: &Value, hide: bool) -> El {
    let keys = [
        "acquisition_basis",
        "income",
        "disposal_basis",
        "proceeds",
        "gain_loss",
    ];
    let before = p["before"]["accounting_totals"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let after = p["after"]["accounting_totals"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let num = |v: &Value| {
        v.as_str()
            .and_then(|x| x.parse::<f64>().ok())
            .or_else(|| v.as_f64())
    };
    let mut c = col().w_full().gap(tk.r(0.25));
    for a in &after {
        let asset = s(a, "asset").to_owned();
        let b = before
            .iter()
            .find(|x| s(x, "asset") == asset)
            .cloned()
            .unwrap_or(Value::Null);
        c = c.child(text(asset.clone()).font(tk.t2xs(600.0)).color(tk.muted_fg));
        for k in keys {
            let fmt = |v: Option<f64>| v.map(|x| format!("{x:.2}")).unwrap_or_else(|| "—".into());
            c = c.child(
                row()
                    .w_full()
                    .gap(tk.r(0.5))
                    .child(
                        text(t(&format!("transactions:tax.acquisition.{k}")))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .grow(1.0),
                    )
                    .child(super::widgets::sens(
                        tk,
                        format!("{} → {}", fmt(num(&b[k])), fmt(num(&a[k]))),
                        tk.txs(500.0).mono(),
                        tk.fg,
                        hide,
                    )),
            );
        }
    }
    c
}
