//! The detail header (TransactionDetailHeader.tsx): back, chips, amount then/now, description,
//! actions, the process timeline and review checklist; plus the review/quarantine banner and
//! quarantine classification (TransactionDetailSheet.tsx:526-864, SheetParts.tsx).

use rune::{Color, El, col, el, row, text};
use serde_json::Value;

use super::draft::{self, Flow, f, opt_s, s};
use super::{Open, Tab};
use crate::app::{App, Currency};
use crate::format;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::{Tk, Tone};
use crate::ui::ic;

// ---- Quarantine classification (§2.4) --------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    Split,
    Sync,
    Transfer,
    Basis,
    Pricing,
}

/// The cause copy of a known reason (`causeCopy`): title, why, what to provide.
pub struct Explanation {
    pub title: String,
    pub why: String,
    pub provide: String,
    pub why_quotes_amounts: bool,
}

pub struct Quarantine {
    pub reason: String,
    /// The checklist and fallback banner's reading (sync, then basis, then transfer).
    pub cause: Cause,
    pub target: Tab,
    /// A known cause's copy (shared quarantine reading), else the generic banner.
    pub explanation: Option<Explanation>,
    /// Whether "Exclude" can be an answer (`exclusionFitsReason`).
    pub exclusion_fits: bool,
}

/// The journal quarantine of the row (only while not excluded), classified as
/// TransactionDetailSheet.tsx:526-590 does: the reason is the Review page's override when it
/// hosts the sheet, known reasons use the shared quarantine reading (with the daemon's
/// classification of this row when the page has it), others fall back to keyword rules.
pub fn quarantine(app: &App, o: &Open, r: &Value) -> Option<Quarantine> {
    use crate::review::model as qm;
    let ctx = review_context(app, o, r);
    let reason = quarantine_reason(app, o, r)?;
    if o.draft.excluded {
        return None;
    }
    let low = reason.to_lowercase();
    let ctx = ctx.filter(|c| c.reason.to_lowercase() == low);
    let evidence = ctx
        .as_ref()
        .map(|c| c.evidence.clone())
        .unwrap_or(Value::Null);
    let known = qm::cause_key(&low, &evidence, &Value::Null).is_some();
    let category = ctx
        .as_ref()
        .map(|c| c.category.clone())
        .unwrap_or_else(|| qm::category_for_reason(&low, &Value::Null, &Value::Null).to_owned());
    let sync = low.contains("ownership_transfer_amount_mismatch")
        || low == "ownership_transfer_source_missing";
    let basis = if known {
        category == "missing_acquisition_history"
    } else {
        ["basis", "lot", "insufficient"]
            .iter()
            .any(|k| low.contains(k))
    };
    let transfer = low == "native_transition_ambiguous"
        || ["ownership_transfer", "transfer", "pair", "swap"]
            .iter()
            .any(|k| low.contains(k));
    let split = low.contains("transfer_fee_implausible");
    let target = if known {
        Tab::parse(qm::sheet_tab(&low, &category, &evidence))
    } else if split || sync {
        Tab::Details
    } else if transfer {
        Tab::Linked
    } else if basis {
        Tab::Tax
    } else {
        Tab::Pricing
    };
    let cause = if sync {
        Cause::Sync
    } else if basis {
        Cause::Basis
    } else if transfer {
        Cause::Transfer
    } else if split {
        Cause::Split
    } else {
        Cause::Pricing
    };
    let explanation = known.then(|| {
        let root = ctx.as_ref().and_then(|c| c.root_label.clone());
        let c = qm::cause_copy_with_root(
            &low,
            &category,
            &evidence,
            opt_s(r, "wallet").as_deref(),
            opt_s(r, "asset").as_deref(),
            root.as_deref(),
        );
        Explanation {
            title: c.title,
            why: c.why,
            provide: c.provide,
            why_quotes_amounts: c.why_quotes_amounts,
        }
    });
    let exclusion_fits = qm::exclusion_fits_reason(&low, &category, &evidence);
    Some(Quarantine {
        reason,
        cause,
        target,
        explanation,
        exclusion_fits,
    })
}

/// `quarantineReasonOverride ?? transaction.quarantineReason`.
pub fn quarantine_reason(app: &App, o: &Open, r: &Value) -> Option<String> {
    review_context(app, o, r)
        .map(|c| c.reason)
        .or_else(|| opt_s(r, "quarantineReason"))
}

/// `detailContextFor`: the Review page's reading of the shown row (its row on the page, else
/// the one the click came with). None outside the Review page.
fn review_context(app: &App, o: &Open, r: &Value) -> Option<super::QContext> {
    let host = super::review_host(app)?;
    crate::review::detail_context(app, s(r, "id"), host.opened.as_ref())
        .filter(|_| o.review.is_some())
}

/// `transactionFlowStyles`: (text, fill, border) — e.g. incoming `border-emerald-600/20
/// bg-emerald-50 text-emerald-700 dark:bg-emerald-900/25 dark:text-emerald-300`.
fn flow_tone(tk: &Tk, fl: Flow) -> (Color, Color, Color) {
    let h = |x: &str| rune::color::hex(x).unwrap();
    // (light text, light fill, dark text, dark fill + alpha, border 600)
    let (lt, lf, dt, df, da, b) = match fl {
        Flow::Incoming => ("#047857", "#ecfdf5", "#6ee7b7", "#064e3b", 0.25, "#059669"),
        Flow::Outgoing => ("#b91c1c", "#fef2f2", "#fca5a5", "#7f1d1d", 0.25, "#dc2626"),
        Flow::Transfer => ("#3f3f46", "#fafafa", "#d4d4d8", "#27272a", 0.70, "#71717a"),
        Flow::Swap => ("#0369a1", "#f0f9ff", "#7dd3fc", "#0c4a6e", 0.25, "#0284c7"),
    };
    if tk.dark {
        (h(dt), h(df).alpha(da), h(b).alpha(0.2))
    } else {
        (h(lt), h(lf), h(b).alpha(0.2))
    }
}

/// `HeaderChip` (`Badge variant="outline" className="gap-1 rounded-md"`): `px-2 py-0.5 text-xs
/// font-medium`, 1 px border.
fn chip(
    tk: &Tk,
    glyph: Option<(&'static rune::Icon, Color)>,
    label: &str,
    fg: Color,
    bg: Color,
    border: Color,
) -> El {
    let mut c = row()
        .px(tk.r(0.5))
        .py(tk.r(0.125))
        .gap(tk.r(0.25))
        .center_y()
        .radius(tk.r(0.375))
        .fill(bg)
        .border(1.0, border);
    if let Some((g, ink)) = glyph {
        c = c.child(ic(g, tk.r(0.75), ink));
    }
    c.child(text(label.to_owned()).font(tk.txs(500.0)).color(fg))
}

/// `networkLabel(paymentMethod)`.
pub fn network_label(r: &Value) -> String {
    match s(r, "paymentMethod") {
        "On-chain" => t("transactions:networkLabel.onChain"),
        "" => s(r, "chain").to_owned(),
        other => other.to_owned(),
    }
}

/// `displayedTransactionStatus`.
pub fn displayed_status(o: &Open, r: &Value) -> String {
    let authored = opt_s(r, "reviewStatus")
        .or_else(|| opt_s(r, "status"))
        .unwrap_or_default();
    if o.draft.review_status != authored && !o.draft.review_status.is_empty() {
        return o.draft.review_status.clone();
    }
    opt_s(r, "queueStatus")
        .or_else(|| opt_s(r, "reviewStatus"))
        .or_else(|| opt_s(r, "status"))
        .unwrap_or_else(|| "pending".into())
}

fn now_rate(app: &App) -> Option<f64> {
    app.snapshot().map(|sn| sn.rate()).filter(|x| *x > 0.0)
}

pub fn header(app: &App, tk: &Tk, o: &Open, r: &Value, sheet: bool) -> El {
    let hide = app.prefs.hide_sensitive();
    let fl = draft::flow(r);
    let amount_btc = draft::amount_btc(r);
    let then = f(r, "eur").map(f64::abs);
    let fiat_first = app.prefs.currency == Currency::Fiat && then.is_some();
    let prefix = match fl {
        Flow::Incoming => "+",
        Flow::Outgoing => "-",
        _ => "",
    };
    let impact = match fl {
        Flow::Outgoing => -1.0,
        _ => 1.0,
    };
    let rate = f(r, "rate").filter(|x| *x != 0.0);
    // "Then" is in the transaction's own fiat currency; "now" in the market rate's. The change
    // compares the two rates, so it shows only when they are the same currency.
    let then_currency = super::tax::row_currency(Some(app), r);
    let now_currency = app
        .snapshot()
        .map(|s| s.fiat_currency())
        .unwrap_or_else(|| then_currency.clone());
    let now = now_rate(app)
        .filter(|_| amount_btc != 0.0)
        .map(|nr| nr * amount_btc * impact);
    let change = match (now_rate(app), rate) {
        _ if now_currency != then_currency => None,
        (Some(nr), Some(rt)) => Some((nr - rt) / rt * 100.0),
        _ => None,
    };

    // Top row: back, chips.
    let mut top = row().w_full().gap(tk.r(0.375)).center_y().flex_wrap();
    if let Some(prev) = o.trail.last() {
        let label = draft::short_txid(prev);
        top = top.child(
            row()
                .key("txd-back")
                .h(tk.r(1.75))
                .px(tk.r(0.5))
                .gap(tk.r(0.375))
                .center_y()
                .radius(tk.r(0.375))
                .hover(tk.accent)
                .role("button")
                .label(format!("{} {label}", t("transactions:detailHeader.back")))
                .focusable(true)
                .on_press(super::back)
                .child(ic(&I::ARROW_LEFT, tk.r(0.875), tk.fg))
                .child(
                    text(t("transactions:detailHeader.back"))
                        .font(tk.txs(500.0))
                        .color(tk.fg),
                )
                .child(super::widgets::sens(
                    tk,
                    label,
                    tk.txs(500.0).mono(),
                    tk.muted_fg,
                    hide,
                )),
        );
    }
    let (ffg, fbg, fbd) = flow_tone(tk, fl);
    top = top.child(chip(tk, None, &fl.label(), ffg, fbg, fbd));
    top = top.child(chip(
        tk,
        Some((&I::NETWORK, tk.muted_fg)),
        &network_label(r),
        tk.fg,
        Color::TRANSPARENT,
        tk.border,
    ));
    let status = displayed_status(o, r);
    let conf = f(r, "conf").unwrap_or(0.0);
    if !(status == "completed" || (status == "pending" && conf <= 0.0)) {
        let (glyph, tone): (&'static rune::Icon, Tone) = match status.as_str() {
            "failed" => (&I::CIRCLE_X, tk.alert),
            "review" => (&I::ROTATE_CCW, tk.info),
            "completed" => (&I::CIRCLE_CHECK, tk.good),
            _ => (&I::CLOCK, tk.warn),
        };
        top = top.child(chip(
            tk,
            Some((glyph, tone.text)),
            &t(draft::status_key(&status)),
            tone.text,
            tone.bg,
            tone.border,
        ));
    }

    // Actions (right, `gap-2`): "Ask about this" (AI on) and the explorer button.
    let mut actions = row().gap(tk.r(0.5)).center_y();
    if app.prefs.ai_features {
        actions = actions.child(
            super::widgets::button(
                tk,
                "txd-ask",
                super::widgets::Variant::Outline,
                super::widgets::Size::Sm,
                Some(&I::SPARKLES),
                &t("assistant:panel.askAbout"),
                true,
            )
            .on_press({
                let (id, date, held) = (
                    s(r, "id").to_owned(),
                    opt_s(r, "date"),
                    opt_s(r, "quarantineReason").is_some(),
                );
                move |a: &mut App| crate::assistant::ask_about(a, &id, date.as_deref(), held)
            }),
        );
    }
    if super::dialogs::explorer_target(app, r).is_some() {
        actions = actions.child(
            super::widgets::icon_button(
                tk,
                "txd-explorer",
                super::widgets::Variant::Outline,
                tk.r(2.25),
                &I::EXTERNAL_LINK,
                tk.r(1.0),
                t("transactions:detailHeader.openExplorer"),
            )
            .on_press(|a: &mut App| {
                if let Some(o) = a.detail.open.as_mut() {
                    o.explorer_open = true;
                    o.explorer_error = None;
                }
            }),
        );
    }
    if sheet {
        actions = actions.child(
            super::widgets::icon_btn(
                tk,
                "txd-close",
                &I::X,
                t("transactions:detailHeader.close"),
                tk.r(2.0),
            )
            .on_press(super::close),
        );
    }

    // Title (`SheetTitle text-2xl sm:text-3xl font-semibold tabular-nums`, `flex-wrap
    // items-baseline gap-x-3 gap-y-1`): the signed headline amount, then (`text-sm sm:text-base
    // font-medium text-muted-foreground`) the other denomination, then and now. Each amount is a
    // currency toggle when the row has a fiat value.
    let toggle = |a: &mut App| {
        a.prefs.currency = if a.prefs.currency == Currency::Btc {
            Currency::Fiat
        } else {
            Currency::Btc
        };
    };
    let priced = then.is_some();
    let amount_btn = |key: &str, s: String, style: rune::TextStyle, color: Color| -> El {
        let mut e = row().key(key.to_owned()).child(super::widgets::sens(
            tk,
            s.clone(),
            style,
            color,
            hide,
        ));
        if priced {
            // `CurrencyToggleText`: the amount names it, the switch it makes describes it.
            let hint = if fiat_first {
                t("chrome:currencyToggleText.showBitcoin")
            } else {
                t("chrome:currencyToggleText.showFiat")
            };
            e = e
                .role("button")
                .label(s.clone())
                .description(hint.clone())
                .focusable(true)
                .help(hint)
                .on_press(move |a: &mut App| toggle(a));
        }
        e
    };
    let big = tk.ty_amount();
    let then_text = then.map(|v| format::fiat(v, &then_currency));
    let btc_text = draft::btc_amount(amount_btc);
    let headline = row()
        .key("txd-amount")
        .min_w(0.0)
        .child(text(prefix.to_owned()).font(big).color(tk.fg))
        .child(amount_btn(
            "txd-amount-main",
            if fiat_first {
                then_text.clone().unwrap_or_default()
            } else {
                btc_text.clone()
            },
            big,
            tk.fg,
        ));
    let sec = tk.tbase(500.0);
    // The web's line is one inline span: it wraps at its spaces, and only there. A Rune row wraps
    // between its children, so every word is a child of its own (with the space after it), the
    // amounts stay whole (their formatter's spaces don't break), and "now" keeps the change
    // beside it (`ml-1`, no space between them). The row shrinks to the column, as the span does.
    let mut secondary = row().center_y().flex_wrap();
    let word = |s: &str| text(s.to_owned()).font(sec).color(tk.muted_fg);
    let words = |mut r: El, s: &str| -> El {
        for w in s.split_inclusive(' ') {
            r = r.child(word(w));
        }
        r
    };
    let now_part = if priced { now } else { None };
    // The space after an amount belongs to the amount when more words follow.
    let spaced = |s: &str, more: bool| if more { format!("{s} ") } else { s.to_owned() };
    if fiat_first {
        secondary = words(
            secondary,
            &format!("{} · ", t("transactions:detailHeader.valueThen")),
        );
        secondary = secondary.child(
            amount_btn("txd-amount-other", btc_text.clone(), sec, tk.muted_fg)
                .child(word(if now_part.is_some() { " " } else { "" })),
        );
    } else if let Some(th) = then_text.clone() {
        secondary = words(secondary, "≈ ");
        secondary =
            secondary.child(amount_btn("txd-amount-other", th, sec, tk.muted_fg).child(word(" ")));
        secondary = words(
            secondary,
            &spaced(
                &t("transactions:detailHeader.valueThen"),
                now_part.is_some(),
            ),
        );
    } else {
        // `MISSING_FIAT_LABEL` (lib/currency.ts), translated.
        secondary = secondary.child(
            text(t("common:state.unpriced"))
                .font(sec)
                .color(tk.warn.solid),
        );
    }
    if let Some(nv) = now_part {
        secondary = words(secondary, "· ");
        secondary = secondary.child(
            row()
                .child(super::widgets::sens(
                    tk,
                    format::fiat(nv.abs(), &now_currency),
                    sec,
                    tk.muted_fg,
                    hide,
                ))
                .child(word(" ")),
        );
        let label = t("transactions:detailHeader.valueNow");
        let (head, last) = match label.rsplit_once(' ') {
            Some((h, l)) => (format!("{h} "), l.to_owned()),
            None => (String::new(), label.clone()),
        };
        secondary = words(secondary, &head);
        let mut tail = row().center_y().child(word(&last));
        if let Some(c) = change {
            // `ml-1 tabular-nums`, emerald-600/400 or red-600/400.
            let ink = if c >= 0.0 {
                tk.good.solid
            } else {
                tk.alert.solid
            };
            tail = tail.child(
                text(format!("({}{:.1}%)", if c >= 0.0 { "+" } else { "" }, c))
                    .font(sec.features("'tnum' 1"))
                    .color(ink)
                    .ml(tk.r(0.25)),
            );
        }
        secondary = secondary.child(tail);
    }
    let title = row()
        .w_full()
        .flex_wrap()
        .gaps(tk.r(0.25), tk.r(0.75))
        .items_end()
        .child(headline)
        .child(secondary);

    // Description line.
    // Description (`SheetDescription mt-1 flex-wrap gap-x-2 text-sm`): counterparty (medium,
    // foreground) ·, the wallet (muted) · unless a pair or the same name, then the date.
    let muted = tk.tsm(400.0);
    let counterparty = opt_s(r, "counter")
        .map(|c| super::graph::route::arrows(c.trim()))
        .filter(|c| !c.is_empty());
    let wallet = opt_s(r, "account");
    let mut desc = row().gap(tk.r(0.5)).center_y().flex_wrap();
    if let Some(c) = &counterparty {
        desc = desc
            .child(super::widgets::sens(
                tk,
                c.clone(),
                tk.tsm(500.0),
                tk.fg,
                hide,
            ))
            .child(text("·").font(muted).color(tk.muted_fg));
    }
    let same = match (&wallet, &counterparty) {
        (Some(w), Some(c)) => w.trim().eq_ignore_ascii_case(c.trim()),
        _ => false,
    };
    if r.get("pair").is_none_or(Value::is_null) && !same {
        desc = desc
            .child(super::widgets::sens(
                tk,
                wallet.unwrap_or_else(|| t("transactions:detailHeader.unassignedWallet")),
                muted,
                tk.muted_fg,
                hide,
            ))
            .child(text("·").font(muted).color(tk.muted_fg));
    }
    desc = desc.child(
        row()
            .gap(tk.r(0.25))
            .center_y()
            .child(ic(&I::CALENDAR_CLOCK, tk.r(0.875), tk.muted_fg))
            .child(text(s(r, "date").to_owned()).font(muted).color(tk.muted_fg)),
    );

    // `SheetHeader border-b`: `px-4 pt-5 pb-4 sm:px-6 sm:pt-6`, `justify-between gap-4`; chips
    // `mb-2`, description `mt-1`; then the timeline bar.
    let _ = sheet;
    col()
        .key("txd-header")
        .w_full()
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .child(
            row()
                .w_full()
                .px(tk.r(1.5))
                .pt(tk.r(1.5))
                .pb(tk.r(1.0))
                .gap(tk.r(1.0))
                .items_start()
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(top.mb(tk.r(0.5)))
                        .child(title)
                        .child(desc.mt(tk.r(0.25))),
                )
                .child(actions),
        )
        // `SheetHeader` keeps its `gap-1.5` between the two rows.
        .child(el().h(tk.r(0.375)))
        .child(trail_bar(app, tk, o, r))
}

impl Tk {
    /// `text-2xl sm:text-3xl tabular-nums`.
    pub fn ty_amount(&self) -> rune::TextStyle {
        rune::TextStyle::new(self.r(1.875), 600.0)
            .lh(self.r(2.25))
            .features("'tnum' 1")
    }
}

// ---- Timeline + checklist (§2.3) ------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Done,
    Warn,
    Current,
    Idle,
}

/// One `StatusTimeline` step: a 1 rem ring (`border bg-…/15 text-…`, a check, warning or the
/// number in text-2xs) and the label (`text-xs`, `cursor-help` with its hint as the title).
fn step(tk: &Tk, n: usize, state: Step, label: &str, hint_text: &str) -> El {
    let h = |x: &str| rune::color::hex(x).unwrap();
    let tone = |solid: &str, dark_text: &str, light_text: &str| {
        (
            h(solid).alpha(0.4),
            h(solid).alpha(0.15),
            if tk.dark { h(dark_text) } else { h(light_text) },
        )
    };
    let (border, fill, ink) = match state {
        Step::Done => tone("#10b981", "#34d399", "#059669"),
        Step::Warn => tone("#f59e0b", "#fbbf24", "#d97706"),
        Step::Current => tone("#3b82f6", "#60a5fa", "#2563eb"),
        Step::Idle => (tk.border, tk.muted.alpha(0.4), tk.muted_fg),
    };
    let dot = row()
        .size(tk.r(1.0), tk.r(1.0))
        .radius(tk.r(0.5))
        .center()
        .fill(fill)
        .border(1.0, border)
        .child(match state {
            Step::Done => ic(&I::CHECK, tk.r(0.625), ink),
            Step::Warn => ic(&I::TRIANGLE_ALERT, tk.r(0.625), ink),
            _ => text(n.to_string()).font(tk.t2xs(400.0)).color(ink),
        });
    let label_ink = match state {
        Step::Warn => {
            if tk.dark {
                h("#fcd34d")
            } else {
                h("#b45309")
            }
        }
        Step::Idle => tk.muted_fg,
        _ => tk.fg,
    };
    row()
        .role("listitem")
        .label(label.to_owned())
        .gap(tk.r(0.375))
        .center_y()
        .child(dot.a11y_hidden())
        .child(
            text(label.to_owned())
                .font(tk.txs(400.0))
                .color(label_ink)
                .help(hint_text.to_owned()),
        )
}

pub struct ChecklistItem {
    pub label: String,
    pub done: bool,
    pub warn: bool,
    pub tab: Tab,
}

pub fn checklist(app: &App, o: &Open, r: &Value) -> Vec<ChecklistItem> {
    let missing = draft::pricing_missing(&o.draft, r);
    let q = quarantine(app, o, r);
    let pricing_blocker = missing && !o.draft.excluded;
    let pricing_label = if missing {
        t("transactions:sheet.checklist.setPricingSource")
    } else {
        let kind = o.draft.pricing_source_kind.clone().unwrap_or_default();
        tr(
            "transactions:sheet.checklist.pricedVia",
            &[("label", &draft::source_kind_phrase(&kind))],
        )
    };
    let labeled = o.draft.label != "Unlabeled" && !o.draft.label.trim().is_empty();
    let tax_label = if o.draft.excluded {
        t("transactions:sheet.checklist.excludedFromReports")
    } else {
        tr(
            "transactions:sheet.checklist.taxLabel",
            &[(
                "label",
                &draft::tax_short(draft::tax_option(&o.draft.at_regime, &o.draft.at_category)),
            )],
        )
    };
    let quarantine_label = match &q {
        Some(q) => t(match q.cause {
            Cause::Sync => "transactions:sheet.checklist.resolveSyncMismatch",
            Cause::Basis => "transactions:sheet.checklist.restoreBasis",
            Cause::Transfer => "transactions:sheet.checklist.reviewPairing",
            _ => "transactions:sheet.checklist.resolveQuarantine",
        }),
        None if pricing_blocker => t("transactions:sheet.checklist.pricingIncomplete"),
        None => t("transactions:sheet.checklist.noQuarantine"),
    };
    let q_ok = q.is_none() && !pricing_blocker;
    vec![
        ChecklistItem {
            label: pricing_label,
            done: !missing,
            warn: missing,
            tab: Tab::Pricing,
        },
        ChecklistItem {
            label: if labeled {
                tr(
                    "transactions:sheet.checklist.labeled",
                    &[("label", &draft::label_text(&o.draft.label))],
                )
            } else {
                t("transactions:sheet.checklist.pickLabel")
            },
            done: labeled,
            warn: false,
            tab: Tab::Classify,
        },
        ChecklistItem {
            label: tax_label,
            done: true,
            warn: false,
            tab: Tab::Tax,
        },
        ChecklistItem {
            label: quarantine_label,
            done: q_ok,
            warn: !q_ok,
            tab: q.as_ref().map(|q| q.target).unwrap_or(Tab::Pricing),
        },
    ]
}

fn trail_bar(app: &App, tk: &Tk, o: &Open, r: &Value) -> El {
    let status = opt_s(r, "status")
        .or_else(|| opt_s(r, "queueStatus"))
        .unwrap_or_else(|| "pending".into());
    let displayed = displayed_status(o, r);
    let missing = draft::pricing_missing(&o.draft, r);
    let quarantined = quarantine(app, o, r).is_some();
    let settled = match status.as_str() {
        "completed" => (Step::Done, t("transactions:sheet.timeline.settledOnChain")),
        "pending" => (
            Step::Current,
            t("transactions:sheet.timeline.waitingConfirmation"),
        ),
        "failed" => (Step::Warn, t("transactions:sheet.timeline.settlementIssue")),
        _ => (Step::Idle, t("transactions:sheet.timeline.settlementIssue")),
    };
    let journal_label = if o.draft.excluded {
        t("transactions:sheet.timeline.excluded")
    } else if quarantined {
        t("transactions:sheet.timeline.quarantined")
    } else if missing {
        t("transactions:sheet.timeline.pendingJournal")
    } else {
        t("transactions:sheet.timeline.journaled")
    };
    let journal_state = if !o.draft.excluded && !quarantined && !missing {
        Step::Done
    } else if !o.draft.excluded {
        Step::Warn
    } else {
        Step::Idle
    };
    let sep = || ic(&I::CHEVRON_RIGHT, tk.r(0.75), tk.muted_fg);
    // `<ol flex-wrap gap-x-2 gap-y-1>`; each `<li gap-1.5>` ends with its chevron.
    let timeline = row()
        .role("list")
        .gaps(tk.r(0.25), tk.r(0.375))
        .center_y()
        .flex_wrap()
        .child(step(
            tk,
            1,
            Step::Done,
            &t("transactions:sheet.timeline.imported"),
            &t("transactions:sheet.timeline.importedHint"),
        ))
        .child(sep())
        .child(step(
            tk,
            2,
            settled.0,
            &t(draft::status_key(&status)),
            &settled.1,
        ))
        .child(sep())
        .child(step(
            tk,
            3,
            if displayed != "review" {
                Step::Done
            } else {
                Step::Idle
            },
            &if displayed == "review" {
                t("transactions:sheet.timeline.needsReview")
            } else {
                t("transactions:sheet.timeline.reviewed")
            },
            &t("transactions:sheet.timeline.reviewedHint"),
        ))
        .child(sep())
        .child(step(
            tk,
            4,
            journal_state,
            &journal_label,
            &t("transactions:sheet.timeline.journaledHint"),
        ));
    let items = checklist(app, o, r);
    let done = items.iter().filter(|i| i.done).count();
    // Whether the checklist fits beside the timeline (`flex-wrap justify-between gap-x-6`):
    // measured as the web lays them out, so a narrow panel puts it on its own line.
    let m = |s: &str| rune::text::measure(s, &[], tk.txs(400.0), None).0;
    let step_labels = [
        t("transactions:sheet.timeline.imported"),
        t(draft::status_key(&status)),
        t("transactions:sheet.timeline.reviewed"),
        journal_label.clone(),
    ];
    let timeline_w: f32 = step_labels
        .iter()
        .map(|l| tk.r(1.0) + tk.r(0.375) + m(l))
        .sum::<f32>()
        + 3.0 * (tk.r(0.75) + 2.0 * tk.r(0.375));
    let list_w: f32 = tk.r(0.875)
        + tk.r(0.25)
        + m("3/4")
        + tk.r(0.25)
        + items
            .iter()
            .map(|i| {
                tk.r(0.375) + tk.r(1.0) + 2.0 + m(&i.label) + if i.done { tk.r(1.0) } else { 0.0 }
            })
            .sum::<f32>();
    let panel_w = if app.detail.maximized {
        app.shell.window_w
    } else {
        app.detail.width
    };
    let together = timeline_w + tk.r(1.5) + list_w <= panel_w - 2.0 * tk.r(1.5);
    // `ReviewChecklist`: `flex-wrap gap-1.5 text-xs`; the count (`mr-1 gap-1 font-medium
    // text-muted-foreground`, ListChecks 0.875 rem) then rounded-full pills.
    let mut list = row()
        .role("group")
        .label(t("transactions:sheet.checklist.title"))
        .gap(tk.r(0.375))
        .center_y()
        .child(
            row()
                .mr(tk.r(0.25))
                .gap(tk.r(0.25))
                .center_y()
                .child(ic(&I::LIST_CHECKS, tk.r(0.875), tk.muted_fg))
                .child(
                    text(format!("{done}/{}", items.len()))
                        .font(tk.txs(500.0).features("'tnum' 1"))
                        .color(tk.muted_fg),
                ),
        );
    for (i, item) in items.into_iter().enumerate() {
        let h = |x: &str| rune::color::hex(x).unwrap();
        // done `border-emerald-500/30 text-foreground`; warn `border-amber-500/40
        // text-amber-700 dark:text-amber-300`; else `border-dashed text-muted-foreground`.
        let (fg, border) = if item.done {
            (tk.fg, h("#10b981").alpha(0.3))
        } else if item.warn {
            (
                if tk.dark { h("#fcd34d") } else { h("#b45309") },
                h("#f59e0b").alpha(0.4),
            )
        } else {
            (tk.muted_fg, tk.border)
        };
        let check_ink = if tk.dark { h("#34d399") } else { h("#059669") };
        let tab = item.tab;
        let mut pill = row()
            .key(format!("txd-check-{i}"))
            .px(tk.r(0.5))
            .py(tk.r(0.125))
            .gap(tk.r(0.25))
            .center_y()
            .radius(tk.r(0.75))
            .fill(tk.background.alpha(0.6))
            .border(1.0, border)
            .hover(tk.muted)
            .role("button")
            .label(item.label.clone())
            .focusable(true)
            .on_press(move |a: &mut App| {
                if let Some(o) = a.detail.open.as_mut() {
                    o.tab = tab;
                }
            });
        if !item.done && !item.warn {
            pill = pill.border_dash(3.0, 2.0);
        }
        if item.done {
            pill = pill.child(ic(&I::CHECK, tk.r(0.75), check_ink));
        }
        list = list.child(pill.child(text(item.label).font(tk.txs(400.0)).color(fg)));
    }
    // `flex-wrap items-center justify-between gap-x-6 gap-y-2 border-t bg-muted/50 px-4 py-2
    // sm:px-6`.
    row()
        .w_full()
        .px(tk.r(1.5))
        .py(tk.r(0.5))
        .gaps(tk.r(0.5), tk.r(1.5))
        .center_y()
        .justify_between()
        .flex_wrap()
        .fill(tk.muted.alpha(0.5))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .child(timeline)
        // Measured to fit beside the timeline: one line (a wrapping row would fold early).
        .child(if together {
            list
        } else {
            list.flex_wrap().w_full()
        })
}

// ---- Review banner (§2.5) -------------------------------------------------------------------

pub fn banner(app: &App, tk: &Tk, o: &Open, r: &Value) -> Option<El> {
    let hide = app.prefs.hide_sensitive();
    let q = quarantine(app, o, r);
    let missing = draft::pricing_missing(&o.draft, r) && !o.draft.excluded;
    if q.is_none() && !(o.tab != Tab::Pricing && missing) {
        return None;
    }
    let date = s(r, "date").to_owned();
    let (title, reason, hint_text, button, target): (String, String, Option<String>, String, Tab) =
        match &q {
            Some(Quarantine {
                explanation: Some(e),
                target,
                ..
            }) => (
                e.title.clone(),
                e.why.clone(),
                Some(e.provide.clone()),
                t(match target {
                    Tab::Pricing => "transactions:sheet.banner.viewPricing",
                    Tab::Tax => "transactions:sheet.banner.viewBasisContext",
                    Tab::Linked => "transactions:sheet.banner.viewLinked",
                    _ => "transactions:sheet.banner.viewDetails",
                }),
                *target,
            ),
            Some(q) if q.cause == Cause::Basis => (
                t("transactions:tax.basisBlockerTitle"),
                tr(
                    "transactions:tax.basisBlockerBody",
                    &[("asset", opt_s(r, "asset").as_deref().unwrap_or("asset"))],
                ),
                None,
                t("transactions:sheet.banner.viewBasisContext"),
                q.target,
            ),
            Some(q) => (
                t("transactions:sheet.banner.journalQuarantine"),
                tr(
                    "transactions:sheet.banner.journalBlocker",
                    &[("reason", &q.reason.replace('_', " "))],
                ),
                None,
                t(match q.cause {
                    Cause::Sync => "transactions:sheet.banner.viewDetails",
                    Cause::Transfer => "transactions:sheet.banner.viewLinked",
                    _ => "transactions:sheet.banner.viewPricing",
                }),
                q.target,
            ),
            None if f(r, "eur").is_none() => (
                t("transactions:sheet.banner.missingFiatPrice"),
                tr(
                    "transactions:sheet.banner.noFiatRecorded",
                    &[("date", &date)],
                ),
                Some(t("transactions:sheet.banner.readinessHint")),
                t("transactions:sheet.banner.openPricing"),
                Tab::Pricing,
            ),
            None if o.draft.pricing_source_kind.is_none() => (
                t("transactions:sheet.banner.noPricingSource"),
                t("transactions:sheet.banner.noPersistedSource"),
                Some(t("transactions:sheet.banner.readinessHint")),
                t("transactions:sheet.banner.openPricing"),
                Tab::Pricing,
            ),
            None => (
                t("transactions:sheet.banner.pricingFlagged"),
                t("transactions:sheet.banner.missingOrUnderReview"),
                Some(t("transactions:sheet.banner.readinessHint")),
                t("transactions:sheet.banner.openPricing"),
                Tab::Pricing,
            ),
        };
    // `QuarantineBanner`: `flex flex-wrap items-start gap-3 rounded-md border border-l-4
    // border-amber-500/40 border-l-amber-500 bg-amber-50/60 px-3 py-2.5 dark:bg-amber-900/15`.
    let hx = |x: &str| rune::color::hex(x).unwrap();
    let amber500 = hx("#f59e0b");
    let (icon_ink, title_ink, fill) = if tk.dark {
        (hx("#fbbf24"), hx("#fcd34d"), hx("#78350f").alpha(0.15))
    } else {
        (hx("#d97706"), hx("#b45309"), hx("#fffbeb").alpha(0.6))
    };
    let mut buttons = row().gap(tk.r(0.375)).flex_wrap();
    let show_primary = !(q.is_some() && o.tab == target);
    if show_primary {
        buttons = buttons.child(
            super::widgets::button(
                tk,
                "txd-banner-go",
                super::widgets::Variant::Outline,
                super::widgets::Size::Sm,
                None,
                &button,
                true,
            )
            .gap(tk.r(0.375))
            .child(ic(&I::ARROW_RIGHT, tk.r(0.875), tk.fg))
            .on_press(move |a: &mut App| {
                if let Some(o) = a.detail.open.as_mut() {
                    o.tab = target;
                }
            }),
        );
    }
    if q.as_ref().is_none_or(|q| q.exclusion_fits) {
        buttons = buttons.child(
            super::widgets::button(
                tk,
                "txd-banner-exclude",
                super::widgets::Variant::Outline,
                super::widgets::Size::Sm,
                None,
                &t("transactions:sheet.banner.exclude"),
                true,
            )
            .on_press(|a: &mut App| {
                if let Some(o) = a.detail.open.as_mut() {
                    o.draft.excluded = true;
                }
            }),
        );
    }
    let hint_body = hint_text.unwrap_or_else(|| t("transactions:sheet.quarantineHint"));
    let hidden = hide
        && q.as_ref().is_some_and(|q| {
            q.explanation
                .as_ref()
                .map_or(q.cause == Cause::Basis, |e| e.why_quotes_amounts)
        });
    Some(
        row()
            .key("txd-banner")
            .w_full()
            .role("status")
            .label(title.clone())
            .gap(tk.r(0.75))
            .items_start()
            .flex_wrap()
            .px(tk.r(0.75))
            .py(tk.r(0.625))
            .radius(tk.r(0.375))
            .fill(fill)
            .border(1.0, amber500.alpha(0.4))
            .border_sides([1.0, 1.0, 1.0, 4.0], amber500)
            .child(ic(&I::TRIANGLE_ALERT, tk.r(1.0), icon_ink).mt(tk.r(0.125)))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(tk.r(0.125))
                    .child(
                        row()
                            .gap(tk.r(0.375))
                            .center_y()
                            .child(text(title).font(tk.tsm(500.0)).color(title_ink))
                            .child(super::widgets::hint_named(
                                tk,
                                t("transactions:infoHint.quarantineMeaning"),
                                hint_body,
                                title_ink,
                            )),
                    )
                    .child(
                        super::widgets::sens(
                            tk,
                            reason,
                            tk.txs(400.0),
                            title_ink.alpha(0.8),
                            hidden,
                        )
                        .wrap(),
                    ),
            )
            .child(buttons),
    )
}
