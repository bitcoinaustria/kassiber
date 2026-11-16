//! "Link several" (owner request): one send linked to several receipts, or several sends to one
//! receipt, in the same "Link manually" panel. The owner chooses the one transaction first, then
//! ticks the other side in the same candidate list; a live tally shows what was sent, what was
//! received and what the difference is (the network fee, or a remainder that stays held, never
//! absorbed). "Review" asks the review core for one plan holding one `pair` operation per ticked
//! transaction, bound to the book and its input version; nothing is linked before Confirm.
//!
//! Only what the core already books is offered: same-asset `manual` links at carrying value, the
//! pairs that may share a transaction (`pair_authoring::reusable`); the core folds them into one
//! reviewed custody movement (tax.md, "a reviewed same-asset link can reuse one side"). Swaps and
//! rail changes stay one to one, so their rows cannot be ticked.

use rune::{Color, Cx, El, Shadow, col, el, row, text};
use serde_json::{Value, json};

use super::super::kit::{self, Var};
use super::super::link_model::{
    self, LegSide, LinkLeg, leg_has_open_part, linkable_msat, minutes_apart, shares_with,
};
use super::super::model;
use super::super::unlink::common::{self, MoveOutcome, Owner, PStatus};
use super::{State, strings};
use crate::app::App;
use crate::i18n::t;
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::{ic, sensitive};

/// The review core takes at most 50 operations in one plan.
pub const MAX_SEVERAL: usize = 50;

/// Stored with each pair; the book keeps one language.
pub const SEVERAL_REASON: &str = "Linked by hand on Transfers, several at once";

fn other(side: LegSide) -> LegSide {
    if side == LegSide::Send {
        LegSide::Receipt
    } else {
        LegSide::Send
    }
}

/// The side the owner starts from (one transaction).
pub fn one_side(s: &State) -> LegSide {
    s.one.unwrap_or(LegSide::Send)
}

fn one_id(s: &State) -> Option<&String> {
    match one_side(s) {
        LegSide::Send => s.send_id.as_ref(),
        LegSide::Receipt => s.receipt_id.as_ref(),
    }
}

/// The one transaction, as read (or as the list knew it).
pub fn anchor(app: &App) -> Option<LinkLeg> {
    let s = &app.custody.link;
    one_id(s)?;
    super::known(app, one_side(s))
}

/// Whether the list of `side` ticks rows rather than picking one.
pub fn ticking(app: &App, side: LegSide) -> bool {
    let s = &app.custody.link;
    s.multi && side != one_side(s) && one_id(s).is_some()
}

pub fn is_ticked(app: &App, id: &str) -> bool {
    app.custody.link.several.iter().any(|l| l.id == id)
}

/// Whether the core can take `leg` in a batch with `anchor`: free or still holding an open part
/// (a same-asset manual link may reuse it, `pair_authoring::reusable`, as the one-to-one picker
/// offers it), and a same-asset move that defaults to a manual link (no swap, no rail change).
pub fn tickable(anchor: &LinkLeg, leg: &LinkLeg) -> bool {
    if (leg.linked && !leg_has_open_part(leg)) || leg.excluded || leg.id == anchor.id {
        return false;
    }
    let (send, receipt) = if anchor.side == LegSide::Send {
        (anchor, leg)
    } else {
        (leg, anchor)
    };
    shares_with(send, receipt)
}

/// A row was pressed: tick or untick it (up to the plan's limit).
pub fn toggle(app: &mut App, leg: LinkLeg) {
    let s = &mut app.custody.link;
    s.error = None;
    if let Some(i) = s.several.iter().position(|l| l.id == leg.id) {
        s.several.remove(i);
    } else if s.several.len() < MAX_SEVERAL {
        s.several.push(leg);
    }
}

/// Turns "Link several" on or off, keeping what was chosen where it still fits.
pub fn set_multi(app: &mut App, on: bool) {
    let s = &mut app.custody.link;
    if s.multi == on {
        return;
    }
    s.error = None;
    s.chosen = None;
    s.menu = None;
    if on {
        let one = match (&s.send_id, &s.receipt_id) {
            (None, Some(_)) => LegSide::Receipt,
            _ => s
                .picking
                .filter(|_| s.send_id.is_none() && s.receipt_id.is_none())
                .unwrap_or(LegSide::Send),
        };
        // The other side, when one was chosen, becomes the first tick.
        let picked = match one {
            LegSide::Send => s.receipt_id.take().and(s.picked_receipt.clone()),
            LegSide::Receipt => s.send_id.take().and(s.picked_send.clone()),
        };
        s.multi = true;
        s.one = Some(one);
        s.several.clear();
        s.reviewing = false;
        let has_one = one_id(s).is_some();
        s.picking = Some(if has_one { other(one) } else { one });
        if let (Some(p), true) = (picked, has_one) {
            s.several.push(p);
        }
        // An untickable first pick is dropped by `step` once the one side is read.
    } else {
        let one = one_side(s);
        let first = (s.several.len() == 1).then(|| s.several[0].clone());
        s.multi = false;
        s.reviewing = false;
        s.several.clear();
        if let Some(l) = first {
            match one {
                LegSide::Send => {
                    s.receipt_id = Some(l.id.clone());
                    s.picked_receipt = Some(l);
                }
                LegSide::Receipt => {
                    s.send_id = Some(l.id.clone());
                    s.picked_send = Some(l);
                }
            }
            s.picking = None;
        } else {
            s.picking = Some(if one_id(s).is_some() { other(one) } else { one });
        }
    }
    let ids: Vec<String> = [&app.custody.link.send_id, &app.custody.link.receipt_id]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    for id in ids {
        common::ensure_leg(app, Owner::Link, &id);
    }
}

/// "Start from": which side is the one transaction (before it is chosen).
fn start_from(app: &mut App, side: LegSide) {
    let s = &mut app.custody.link;
    if one_id(s).is_some() {
        return;
    }
    s.one = Some(side);
    s.picking = Some(side);
}

/// The several side's list is open again (from the check).
fn back(app: &mut App) {
    if app.custody.link.preview.applying {
        return;
    }
    let s = &mut app.custody.link;
    s.reviewing = false;
    s.error = None;
    s.picking = Some(other(one_side(s)));
}

fn review(app: &mut App) {
    let s = &mut app.custody.link;
    if s.several.is_empty() || one_id(s).is_none() {
        return;
    }
    s.reviewing = true;
    s.error = None;
    s.picking = None;
}

/// `step`'s part for "Link several": keep the list on the right side and the ticks valid.
/// True when anything changed.
pub fn step(app: &mut App) -> bool {
    let mut changed = false;
    let anchor = anchor(app);
    let s = &mut app.custody.link;
    if !s.multi {
        return false;
    }
    let one = one_side(s);
    // The other side's id stays empty: the ticks are that side.
    let stray = match one {
        LegSide::Send => s.receipt_id.take(),
        LegSide::Receipt => s.send_id.take(),
    };
    changed |= stray.is_some();
    if one_id(s).is_some() && s.picking.is_none() && !s.reviewing {
        s.picking = Some(other(one));
        changed = true;
    }
    if one_id(s).is_none() && s.reviewing {
        s.reviewing = false;
        changed = true;
    }
    if let Some(a) = &anchor {
        let before = s.several.len();
        s.several.retain(|l| tickable(a, l));
        changed |= s.several.len() != before;
    }
    if s.several.is_empty() && s.reviewing {
        s.reviewing = false;
        s.picking = Some(other(one));
        changed = true;
    }
    changed
}

/// The operations the plan holds: one `pair` per ticked transaction.
pub fn operations(app: &App) -> Option<Value> {
    let s = &app.custody.link;
    if !s.multi || !s.reviewing || s.several.is_empty() {
        return None;
    }
    let a = anchor(app)?;
    let ops: Vec<Value> = s
        .several
        .iter()
        .take(MAX_SEVERAL)
        .map(|l| {
            let (send, receipt) = if a.side == LegSide::Send {
                (&a, l)
            } else {
                (l, &a)
            };
            json!({
                "type": "pair",
                "out_transaction_id": send.id,
                "in_transaction_id": receipt.id,
                "pair_kind": "manual",
                "policy": "carrying-value",
                "reason": SEVERAL_REASON,
            })
        })
        .collect();
    Some(Value::Array(ops))
}

// ---- The tally ---------------------------------------------------------------------------------

/// What was sent (principal), the sends' network fees, and what was received, in exact msat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub sent: i128,
    pub fee: i128,
    pub received: i128,
}

/// What the difference is, as the core books it: a send's network fee is a fee; any further
/// shortfall stays as held suspense; more received than sent cannot be explained by a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gap {
    None,
    Fee(i128),
    Rest { fee: i128, rest: i128 },
    More(i128),
}

fn fee_of(leg: &LinkLeg) -> i128 {
    if leg_has_open_part(leg) {
        0
    } else {
        leg.fee_msat.unwrap_or(0).max(0)
    }
}

pub fn tally(anchor: &LinkLeg, several: &[LinkLeg]) -> Tally {
    let many: i128 = several.iter().map(linkable_msat).sum();
    match anchor.side {
        LegSide::Send => Tally {
            sent: linkable_msat(anchor),
            fee: fee_of(anchor),
            received: many,
        },
        LegSide::Receipt => Tally {
            sent: many,
            fee: several.iter().map(fee_of).sum(),
            received: linkable_msat(anchor),
        },
    }
}

impl Tally {
    pub fn gap(&self) -> Gap {
        let rest = self.sent - self.received;
        if rest < 0 {
            Gap::More(-rest)
        } else if rest == 0 && self.fee == 0 {
            Gap::None
        } else if rest == 0 {
            Gap::Fee(self.fee)
        } else {
            Gap::Rest {
                fee: self.fee,
                rest,
            }
        }
    }
}

fn gap_sentence(g: Gap) -> String {
    let b = model::msat_btc;
    match g {
        Gap::None => strings::diff_none(),
        Gap::Fee(f) => strings::diff_fee(&model::msat_sats(f)),
        Gap::Rest { fee, rest } => strings::diff_rest(
            (fee > 0).then(|| model::msat_sats(fee)).as_deref(),
            &b(rest),
        ),
        Gap::More(m) => strings::diff_more(&b(m)),
    }
}

/// Batch-level warnings, as (text, serious).
fn warnings(anchor: &LinkLeg, several: &[LinkLeg], g: Gap) -> Vec<(String, bool)> {
    let mut w = vec![];
    if let Gap::More(_) = g {
        w.push((t("review:transfersPage.link.warn.negative"), true));
    }
    let before = several.iter().any(|l| {
        let (send, receipt) = if anchor.side == LegSide::Send {
            (anchor, l)
        } else {
            (l, anchor)
        };
        minutes_apart(send, receipt).is_some_and(|m| m < -60)
    });
    if before {
        w.push((strings::several_warn_before(), true));
    }
    if several.iter().any(|l| l.wallet == anchor.wallet) {
        w.push((strings::several_warn_same_wallet(), true));
    }
    w
}

// ---- Views ----------------------------------------------------------------------------------

/// shadcn `Checkbox` drawn inside a row that carries the control's name and state.
pub fn check_box(tk: &Tk, on: bool, enabled: bool) -> El {
    let b = row()
        .size(tk.r(1.0), tk.r(1.0))
        .center()
        .radius(4.0)
        .opacity(if enabled { 1.0 } else { 0.5 });
    if on {
        b.fill(tk.primary)
            .child(ic(&I::CHECK, tk.r(0.875), tk.primary_fg))
    } else {
        b.border(1.0, tk.input)
            .border_layout()
            .fill(if tk.dark {
                tk.input.alpha(tk.input.a * 0.3)
            } else {
                Color::TRANSPARENT
            })
            .box_shadow(Shadow {
                y: 1.0,
                blur: 2.0,
                color: Color::BLACK.alpha(0.05),
                ..Shadow::default()
            })
    }
}

/// The switch into and out of "Link several" (a checkbox with its one-line hint).
pub fn toggle_row(app: &App, tk: &Tk) -> El {
    let on = app.custody.link.multi;
    let enabled = !app.custody.link.preview.applying;
    let mut r = row()
        .key("link-several")
        .w_full()
        .gap(tk.r(0.625))
        .items_start()
        .role("checkbox")
        .checked(on)
        .label(strings::toggle())
        .help(strings::toggle_hint())
        .focusable(enabled)
        .disabled(!enabled)
        .child(check_box(tk, on, enabled).mt(tk.r(0.125)).a11y_hidden())
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(tk.r(0.125))
                .child(text(strings::toggle()).font(tk.tsm(500.0)).color(tk.fg))
                .child(
                    text(strings::toggle_hint())
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                ),
        );
    if enabled {
        r = r.on_press(move |a: &mut App| set_multi(a, !on));
    }
    r
}

/// The several side's slot: how many are ticked and their total, or what to do.
pub fn tally_slot(app: &App, tk: &Tk, side: LegSide) -> El {
    let s = &app.custody.link;
    let label = t(if side == LegSide::Send {
        "review:transfersPage.leg.sent"
    } else {
        "review:transfersPage.leg.received"
    });
    let n = s.several.len();
    let mut c = col()
        .key(format!(
            "link-tally-{}",
            if side == LegSide::Send {
                "send"
            } else {
                "receipt"
            }
        ))
        .w_full()
        .min_h(tk.r(4.5))
        .px(tk.r(0.75))
        .py(tk.r(0.625))
        .gap(tk.r(0.25))
        .justify_center()
        .radius(tk.r(0.5))
        .role("group")
        .label(label.clone());
    c = c.child(
        text(label.to_uppercase())
            .font(tk.txs(500.0).tracking(0.025 * tk.r(0.75)))
            .color(tk.muted_fg),
    );
    if n == 0 {
        let line = if one_id(s).is_some() {
            strings::slot_tick(side == LegSide::Receipt)
        } else {
            t(if side == LegSide::Send {
                "review:transfersPage.link.slot.openSend"
            } else {
                "review:transfersPage.link.slot.openReceipt"
            })
        };
        return c
            .border(1.0, tk.border)
            .border_dash(4.0, 3.0)
            .child(text(line).font(tk.tsm(400.0)).color(tk.muted_fg).wrap());
    }
    let total: i128 = s.several.iter().map(linkable_msat).sum();
    let asset = s.several[0].asset.clone();
    c.border(1.0, if s.reviewing { tk.border } else { tk.primary })
        .fill(if s.reviewing {
            Color::TRANSPARENT
        } else {
            tk.primary.alpha(0.05)
        })
        .child(text(strings::ticked(n)).font(tk.tsm(500.0)).color(tk.fg))
        .child(kit::asset_figure(
            tk,
            total,
            &asset,
            tk.tsm(400.0).features("'tnum' 1"),
            tk.muted_fg,
            app.prefs.hide_sensitive(),
        ))
}

/// Sent · Received · Difference, and what the difference is.
fn tally_view(app: &App, tk: &Tk, a: &LinkLeg, several: &[LinkLeg]) -> El {
    let hidden = app.prefs.hide_sensitive();
    let tl = tally(a, several);
    let fig = |label: String, msat: i128, key: &str| {
        col()
            .key(format!("link-tally-{key}"))
            .grow(1.0)
            .w(0.0)
            .min_w(0.0)
            .gap(tk.r(0.125))
            .role("group")
            .label(label.clone())
            .child(text(label).font(tk.txs(500.0)).color(tk.muted_fg))
            .child(kit::asset_figure(
                tk,
                msat,
                &a.asset,
                tk.tsm(500.0).features("'tnum' 1"),
                tk.fg,
                hidden,
            ))
    };
    // Principal against principal, as the one-to-one check compares them; the sends' network
    // fees are named under what was sent.
    let diff = (tl.sent - tl.received).abs();
    let mut sent = fig(t("review:transfersPage.leg.sent"), tl.sent, "sent");
    if tl.fee > 0 {
        sent = sent.child(sensitive(
            crate::i18n::tr(
                "review:transfersPage.leg.networkFee",
                &[("amount", &model::msat_sats(tl.fee))],
            ),
            tk.txs(400.0).features("'tnum' 1"),
            tk.muted_fg,
            hidden,
        ));
    }
    let figures = row()
        .w_full()
        .gap(tk.r(0.75))
        .child(sent)
        .child(fig(
            t("review:transfersPage.leg.received"),
            tl.received,
            "received",
        ))
        .child(fig(
            t("review:transfersPage.link.differenceTitle"),
            diff,
            "difference",
        ));
    let g = tl.gap();
    let color = match g {
        Gap::More(_) => tk.destructive,
        Gap::Rest { .. } => kit::amber_text(tk),
        _ => tk.muted_fg,
    };
    col()
        .key("link-tally")
        .w_full()
        .gap(tk.r(0.5))
        .px(tk.r(0.75))
        .py(tk.r(0.625))
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .fill(tk.muted.alpha(0.3))
        .role("status")
        .child(figures)
        .child(
            sensitive(gap_sentence(g), tk.tsm(400.0), color, hidden)
                .wrap()
                .key("link-tally-sentence"),
        )
}

/// Everything under the slots while "Link several" is on.
pub fn body(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>, wide: bool) -> Vec<El> {
    let s = &app.custody.link;
    let one = one_side(s);
    let mut out = vec![];
    let Some(a) = anchor(app).filter(|_| one_id(s).is_some()) else {
        if one_id(s).is_some() {
            // The one transaction is still being read.
            out.push(
                row()
                    .gap(tk.r(0.5))
                    .center_y()
                    .role("status")
                    .child(kit::spinner(tk, tk.r(1.0), tk.muted_fg))
                    .child(
                        text(t("review:transfersPage.link.reading"))
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg),
                    ),
            );
            return out;
        }
        out.push(start_group(app, tk, wide));
        out.push(super::candidates::view(
            app, cx, tk, layer, one, None, None, wide,
        ));
        out.push(
            row().w_full().justify_end().child(
                kit::btn_md(
                    tk,
                    "link-cancel",
                    Var::Outline,
                    None,
                    &t("review:transfersPage.link.cancel"),
                    true,
                )
                .on_press(super::close),
            ),
        );
        return out;
    };
    if s.reviewing {
        out.push(check_view(app, tk, &a, wide));
        return out;
    }
    match s.picking {
        Some(side) if side == one => {
            // Changing the one transaction.
            out.push(super::candidates::view(
                app,
                cx,
                tk,
                layer,
                one,
                None,
                one_id(s).map(String::as_str),
                wide,
            ));
            out.push(
                row()
                    .w_full()
                    .justify_between()
                    .child(
                        kit::btn_md(
                            tk,
                            "link-keep",
                            Var::Ghost,
                            None,
                            &t(if one == LegSide::Send {
                                "review:transfersPage.link.keepSend"
                            } else {
                                "review:transfersPage.link.keepReceipt"
                            }),
                            true,
                        )
                        .on_press(|a: &mut App| a.custody.link.picking = None),
                    )
                    .child(
                        kit::btn_md(
                            tk,
                            "link-cancel",
                            Var::Outline,
                            None,
                            &t("review:transfersPage.link.cancel"),
                            true,
                        )
                        .on_press(super::close),
                    ),
            );
        }
        _ => {
            out.push(
                text(strings::swaps_note())
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .key("link-several-note"),
            );
            out.push(super::candidates::view(
                app,
                cx,
                tk,
                layer,
                other(one),
                Some(&a),
                None,
                wide,
            ));
            let n = s.several.len();
            if n > 0 {
                out.push(tally_view(app, tk, &a, &s.several));
            }
            if n >= MAX_SEVERAL {
                out.push(
                    text(strings::limit(MAX_SEVERAL))
                        .font(tk.txs(400.0))
                        .color(kit::amber_text(tk))
                        .wrap()
                        .role("status")
                        .key("link-several-limit"),
                );
            }
            let go = kit::btn_md(
                tk,
                "link-review",
                Var::Default,
                None,
                &strings::review(n.max(1)),
                n > 0,
            )
            .on_press(review);
            out.push(
                row()
                    .w_full()
                    .justify_end()
                    .gap(tk.r(0.5))
                    .child(
                        kit::btn_md(
                            tk,
                            "link-cancel",
                            Var::Outline,
                            None,
                            &t("review:transfersPage.link.cancel"),
                            true,
                        )
                        .on_press(super::close),
                    )
                    .child(go),
            );
        }
    }
    out
}

/// "Start from": one send or one receipt (`role=radiogroup`, arrows move, Space/Enter picks).
fn start_group(app: &App, tk: &Tk, wide: bool) -> El {
    let one = one_side(&app.custody.link);
    let options = [
        (
            LegSide::Send,
            strings::one_send(),
            strings::one_send_hint(),
            "send",
        ),
        (
            LegSide::Receipt,
            strings::one_receipt(),
            strings::one_receipt_hint(),
            "receipt",
        ),
    ];
    let mut group = if wide {
        row().w_full().gap(tk.r(0.5)).items_stretch()
    } else {
        col().w_full().gap(tk.r(0.5))
    }
    .key("link-start-from")
    .role("radiogroup")
    .label(strings::start_from())
    .focus_group();
    for (side, label, hint, k) in options {
        let checked = one == side;
        let dot = row()
            .size(tk.r(0.875), tk.r(0.875))
            .mt(tk.r(0.25))
            .center()
            .radius(tk.r(0.4375))
            .border(1.0, if checked { tk.primary } else { tk.input })
            .child(if checked {
                crate::ui::dot(tk.r(0.4375), tk.primary)
            } else {
                el()
            });
        let mut opt = row()
            .key(format!("link-start-{k}"))
            .min_w(0.0)
            .gap(tk.r(0.5))
            .items_start()
            .px(tk.r(0.75))
            .py(tk.r(0.5))
            .radius(tk.r(0.5))
            .border(1.0, if checked { tk.primary } else { tk.border })
            .role("radio")
            .checked(checked)
            .label(format!("{}: {label}", strings::start_from()))
            .focusable(true)
            .hover(tk.muted.alpha(0.4))
            .on_press(move |a: &mut App| start_from(a, side))
            .child(dot.a11y_hidden())
            .child(
                col()
                    .min_w(0.0)
                    .grow(1.0)
                    .gap(tk.r(0.125))
                    .child(text(label).font(tk.tsm(500.0)).color(tk.fg))
                    .child(text(hint).font(tk.txs(400.0)).color(tk.muted_fg).wrap()),
            );
        if checked {
            opt = opt.fill(tk.primary.alpha(0.05));
        }
        group = group.child(if wide {
            opt.grow(1.0).w(0.0)
        } else {
            opt.w_full()
        });
    }
    col()
        .w_full()
        .gap(tk.r(0.375))
        .child(
            text(strings::start_from())
                .font(tk.txs(500.0))
                .color(tk.muted_fg)
                .a11y_hidden(),
        )
        .child(group)
}

/// What the preview says about every transaction in the batch.
fn outcome_of(artifact: &Value, ids: &[&str]) -> MoveOutcome {
    let held = |e: &Value| -> Vec<String> {
        e["quarantines"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|q| {
                        q.get("transaction_id").map(|v| {
                            v.as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| v.to_string())
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let (before, after) = (held(&artifact["before"]), held(&artifact["after"]));
    let any = |v: &Vec<String>| ids.iter().any(|id| v.iter().any(|x| x == id));
    MoveOutcome {
        send_held: any(&after),
        receipt_held: false,
        was_held: any(&before),
        reports_ready_after: artifact["after"]["report_ready"].as_bool().unwrap_or(false),
        held_before: artifact["before"]["quarantine_count"].as_u64().unwrap_or(0),
        held_after: artifact["after"]["quarantine_count"].as_u64().unwrap_or(0),
    }
}

/// One planned link: the ticked transaction against the one, as the plan names them.
fn link_line(app: &App, tk: &Tk, i: usize, l: &LinkLeg, planned: Option<&Value>) -> El {
    let hidden = app.prefs.hide_sensitive();
    // The one transaction is the same on every line: name the ticked side.
    let route = l.wallet.clone();
    let terms = planned.map(|op| {
        format!(
            "{} · {} · {}",
            op["type"].as_str().unwrap_or("pair"),
            op["pair_kind"].as_str().unwrap_or("manual"),
            op["policy"].as_str().unwrap_or("carrying-value")
        )
    });
    let mut left = col()
        .grow(1.0)
        .w(0.0)
        .min_w(0.0)
        .gap(tk.r(0.125))
        .child(sensitive(route, tk.tsm(500.0), tk.fg, hidden).truncate())
        .child(
            text(link_model::date_time(l.occurred_at.as_deref()))
                .font(tk.txs(400.0).features("'tnum' 1"))
                .color(tk.muted_fg),
        );
    if let Some(tm) = terms {
        left = left.child(
            text(tm)
                .font(tk.txs(400.0).features("'tnum' 1"))
                .color(tk.muted_fg)
                .key(format!("link-plan-op-{i}")),
        );
    }
    row()
        .key(format!("link-several-{i}"))
        .w_full()
        .gap(tk.r(0.75))
        .items_start()
        .py(tk.r(0.375))
        .role("listitem")
        .child(left)
        .child(kit::asset_figure(
            tk,
            linkable_msat(l),
            &l.asset,
            tk.tsm(400.0).features("'tnum' 1"),
            tk.fg,
            hidden,
        ))
}

fn confirm(app: &mut App) {
    if !matches!(app.custody.link.preview.status, PStatus::Ready { .. })
        || app.custody.link.preview.applying
        || super::super::updating(app)
    {
        return;
    }
    app.custody.link.error = None;
    common::apply_preview(app, Owner::Link);
}

/// The check before confirming: the links as planned, the tally, what it counts as, the
/// book's plan and what changes.
fn check_view(app: &App, tk: &Tk, a: &LinkLeg, wide: bool) -> El {
    let s = &app.custody.link;
    let busy = s.preview.applying;
    let updating = super::super::updating(app);
    let several = &s.several;
    let artifact = match &s.preview.status {
        PStatus::Ready { artifact, .. } => Some(artifact),
        _ => None,
    };
    let planned: Vec<Value> = artifact
        .and_then(|x| x["operations"].as_array().cloned())
        .unwrap_or_default();
    // The plan's operation for a ticked transaction, matched by its ids.
    let op_for = |l: &LinkLeg| {
        planned.iter().find(|op| {
            let (o, i) = (
                op["out_transaction_id"].as_str(),
                op["in_transaction_id"].as_str(),
            );
            if a.side == LegSide::Send {
                o == Some(a.id.as_str()) && i == Some(l.id.as_str())
            } else {
                o == Some(l.id.as_str()) && i == Some(a.id.as_str())
            }
        })
    };
    let mut list = col()
        .key("link-several-list")
        .w_full()
        .role("list")
        .label(strings::several_label(a.side == LegSide::Send));
    for (i, l) in several.iter().enumerate() {
        let mut line = link_line(app, tk, i, l, op_for(l));
        if i + 1 < several.len() {
            line = line
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .border_layout();
        }
        list = list.child(line);
    }
    let tl = tally(a, several);
    let g = tl.gap();
    let ids: Vec<&str> = std::iter::once(a.id.as_str())
        .chain(several.iter().map(|l| l.id.as_str()))
        .collect();
    let outcome = artifact.map(|x| outcome_of(x, &ids));
    let creates_holds = outcome.is_some_and(|o| o.send_held && !o.was_held);
    let mut lines = vec![];
    if let Some(o) = outcome {
        if o.was_held && !o.send_held {
            lines.push(t("review:transfersPage.link.effectReleased"));
        }
        if creates_holds {
            lines.push(crate::i18n::tc(
                "review:transfersPage.link.effectCreatesHolds",
                (o.held_after.saturating_sub(o.held_before)).max(1) as usize,
                &[],
            ));
        } else if o.send_held {
            lines.push(t("review:transfersPage.link.effectStillHeld"));
        }
    }
    let warns = warnings(a, several, g);
    let anyway = creates_holds || warns.iter().any(|(_, serious)| *serious);
    let bitcoin = model::is_bitcoin(&a.asset);

    let mut dl = col().w_full().gap(tk.r(0.75));
    dl = dl.child(super::check_row(tk, strings::links_title(), list, wide));
    dl = dl.child(super::check_row(
        tk,
        t("review:transfersPage.link.differenceTitle"),
        tally_view(app, tk, a, several),
        wide,
    ));
    dl = dl.child(super::check_row(
        tk,
        t("review:transfersPage.link.countsAs"),
        col()
            .key("link-counts-as")
            .w_full()
            .gap(tk.r(0.25))
            .child(common::para(
                tk,
                t(if bitcoin {
                    "review:transfersPage.link.ownBitcoin"
                } else {
                    "review:transfersPage.link.ownAsset"
                }),
                tk.fg,
            ))
            .child(
                text(strings::plan_counts())
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap(),
            ),
        wide,
    ));
    if let Some(x) = artifact {
        let version = x["base_input_version"]
            .as_u64()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "—".into());
        dl = dl.child(super::check_row(
            tk,
            strings::plan_title(),
            text(strings::plan_bound(&version))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap()
                .key("link-plan-bound"),
            wide,
        ));
    }
    let refusal = match &s.preview.status {
        PStatus::Failed { code, .. } if code.as_deref() == Some("custody_legacy_group_invalid") => {
            Some(strings::refused_fit(a.side == LegSide::Receipt))
        }
        PStatus::Failed { code, .. } => super::refusal_text(code.as_deref()),
        _ => None,
    };
    let retry: Option<fn(&mut App)> =
        if matches!(s.preview.status, PStatus::Failed { .. }) && refusal.is_none() {
            Some(|a: &mut App| common::retry_preview(a, Owner::Link))
        } else {
            None
        };
    let changes = common::outcome_view(
        app,
        tk,
        "link-preview",
        &s.preview,
        outcome,
        &lines,
        creates_holds,
        updating.then(|| t("review:transfersPage.link.waitingJournals")),
        refusal,
        retry,
    );
    dl = dl.child(super::check_row(
        tk,
        t("review:transfersPage.link.changesTitle"),
        changes,
        wide,
    ));
    let mut out = col().key("link-check").w_full().gap(tk.r(1.0)).child(dl);
    if !warns.is_empty() {
        let w: Vec<String> = warns.into_iter().map(|(s, _)| s).collect();
        out = out.child(common::warnings(app, tk, &w));
    }
    if let Some(e) = &s.error {
        out = out.child(
            text(e.clone())
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .wrap_anywhere()
                .role("alert"),
        );
    }
    let n = several.len();
    let label = if busy {
        t("review:transfersPage.link.linking")
    } else if anyway {
        t("review:transfersPage.link.confirmAnyway")
    } else {
        strings::confirm(n)
    };
    let enabled = !busy && !updating && matches!(s.preview.status, PStatus::Ready { .. });
    let go = if anyway {
        common::destructive_btn(tk, "link-confirm", &label, enabled, busy)
    } else {
        common::md_btn(
            tk,
            "link-confirm",
            Var::Default,
            None,
            &label,
            enabled,
            busy,
        )
    }
    .on_press(confirm);
    let back_btn = kit::btn_md(
        tk,
        "link-back",
        Var::Ghost,
        Some(&I::ARROW_LEFT),
        &t("common:actions.back"),
        !busy,
    )
    .on_press(back);
    out.child(
        row()
            .w_full()
            .justify_between()
            .gap(tk.r(0.5))
            .child(back_btn)
            .child(
                row()
                    .gap(tk.r(0.5))
                    .child(
                        kit::btn_md(
                            tk,
                            "link-check-cancel",
                            Var::Outline,
                            None,
                            &t("review:transfersPage.link.cancel"),
                            !busy,
                        )
                        .on_press(super::close),
                    )
                    .child(go),
            ),
    )
}

// ---- Headless states ---------------------------------------------------------------------------

/// Runs `step` and the reads it starts until nothing changes (headless scenes).
fn pump(a: &mut App) {
    for _ in 0..4 {
        super::step(a);
        super::super::settle(a);
    }
}

/// Opens "Link several" from `one`, picks the first free transaction on that side with at least
/// `ticks` transactions it can take, ticks them, and with `review` shows the planned check.
pub fn scene(a: &mut App, one: LegSide, ticks: usize, review_it: bool) {
    a.navigate("/transfers");
    super::super::prime(a);
    super::open(a, super::LinkStart::default());
    set_multi(a, true);
    start_from(a, one);
    a.custody.link.pick.year = None;
    pump(a);
    if ticks == 0 {
        return;
    }
    // Largest first: those have room for several on the other side.
    let mut firsts: Vec<LinkLeg> = super::candidates::rows(a, one, None)
        .into_iter()
        .filter(|r| r.blocked.is_none() && !r.leg.linked)
        .map(|r| r.leg)
        .collect();
    firsts.sort_by_key(|l| std::cmp::Reverse(linkable_msat(l)));
    firsts.truncate(40);
    for leg in firsts {
        super::choose(a, one, leg.clone());
        a.custody.link.pick.hours = Some(72);
        pump(a);
        let mut found = fits(a, &leg, ticks);
        if found.len() < ticks {
            a.custody.link.pick.hours = None;
            pump(a);
            found = fits(a, &leg, ticks);
        }
        if found.len() >= ticks {
            for l in found {
                toggle(a, l);
            }
            break;
        }
    }
    if review_it {
        review(a);
        pump(a);
        common::start_now(a, Owner::Link);
        super::super::settle(a);
    }
}

/// Up to `n` free transactions that fit `anchor`, while their total stays within it.
fn fits(a: &App, anchor: &LinkLeg, n: usize) -> Vec<LinkLeg> {
    let mut left = linkable_msat(anchor);
    let mut out = vec![];
    for r in super::candidates::rows(a, other(anchor.side), Some(anchor)) {
        if out.len() == n {
            break;
        }
        if r.blocked.is_some() || r.before || r.same_wallet || !tickable(anchor, &r.leg) {
            continue;
        }
        let m = linkable_msat(&r.leg);
        if m > left {
            continue;
        }
        left -= m;
        out.push(r.leg);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::super::model::Rail;
    use super::*;

    fn leg(
        id: &str,
        side: LegSide,
        asset: &str,
        rail: Rail,
        msat: i128,
        fee: Option<i128>,
    ) -> LinkLeg {
        LinkLeg {
            id: id.into(),
            external_id: None,
            wallet: format!("w-{id}"),
            asset: asset.into(),
            rail,
            occurred_at: Some("2024-01-01T00:00:00Z".into()),
            amount_msat: msat,
            fee_msat: fee,
            side,
            linked: false,
            link: None,
            open: None,
            held: false,
            excluded: false,
            booked_as: None,
            fiat: None,
            network: None,
        }
    }

    #[test]
    fn one_send_to_three_receipts_is_exact() {
        let s = leg(
            "s",
            LegSide::Send,
            "BTC",
            Rail::Onchain,
            2_400_000_000,
            Some(2_000_000),
        );
        let rs = [
            leg(
                "a",
                LegSide::Receipt,
                "BTC",
                Rail::Onchain,
                1_000_000_000,
                None,
            ),
            leg(
                "b",
                LegSide::Receipt,
                "BTC",
                Rail::Onchain,
                800_000_000,
                None,
            ),
            leg(
                "c",
                LegSide::Receipt,
                "BTC",
                Rail::Onchain,
                600_000_000,
                None,
            ),
        ];
        let tl = tally(&s, &rs);
        assert_eq!(
            tl,
            Tally {
                sent: 2_400_000_000,
                fee: 2_000_000,
                received: 2_400_000_000
            }
        );
        assert_eq!(tl.gap(), Gap::Fee(2_000_000));
    }

    #[test]
    fn two_sends_to_one_receipt_counts_both_fees() {
        let r = leg(
            "r",
            LegSide::Receipt,
            "BTC",
            Rail::Onchain,
            3_000_000_000,
            None,
        );
        let ss = [
            leg(
                "a",
                LegSide::Send,
                "BTC",
                Rail::Onchain,
                1_000_000_000,
                Some(1_000),
            ),
            leg(
                "b",
                LegSide::Send,
                "BTC",
                Rail::Onchain,
                2_000_000_001,
                Some(2_000),
            ),
        ];
        let tl = tally(&r, &ss);
        assert_eq!(
            tl.gap(),
            Gap::Rest {
                fee: 3_000,
                rest: 1
            }
        );
        let tl = tally(&r, &ss[..1]);
        assert_eq!(tl.gap(), Gap::More(2_000_000_000));
    }

    #[test]
    fn only_same_asset_moves_tick() {
        let s = leg("s", LegSide::Send, "BTC", Rail::Onchain, 1, None);
        assert!(tickable(
            &s,
            &leg("a", LegSide::Receipt, "BTC", Rail::Onchain, 1, None)
        ));
        assert!(!tickable(
            &s,
            &leg("l", LegSide::Receipt, "LBTC", Rail::Liquid, 1, None)
        ));
        assert!(!tickable(
            &s,
            &leg("n", LegSide::Receipt, "BTC", Rail::Lightning, 1, None)
        ));
        let mut linked = leg("k", LegSide::Receipt, "BTC", Rail::Onchain, 1, None);
        linked.linked = true;
        assert!(!tickable(&s, &linked));
        assert!(!tickable(&s, &s.clone()));
    }

    #[test]
    fn a_partly_linked_row_with_an_open_part_ticks() {
        use super::super::super::link_model::OpenPart;
        // Receipt R (1.0 BTC): 0.6 linked from another send, 0.4 still open.
        let s = leg(
            "s2",
            LegSide::Send,
            "BTC",
            Rail::Onchain,
            100_000_000_000,
            None,
        );
        let mut r = leg(
            "r",
            LegSide::Receipt,
            "BTC",
            Rail::Onchain,
            100_000_000_000,
            None,
        );
        r.linked = true;
        r.open = Some(OpenPart {
            linked_msat: 60_000_000_000,
            unlinked_msat: 40_000_000_000,
            can_add: true,
        });
        assert!(tickable(&s, &r));
        let r2 = leg(
            "r2",
            LegSide::Receipt,
            "BTC",
            Rail::Onchain,
            60_000_000_000,
            None,
        );
        assert_eq!(
            tally(&s, &[r.clone(), r2]).gap(),
            Gap::None,
            "R counts with its open part only"
        );
        // Nothing left to add, or the core will not add to it: one to one only.
        r.open = Some(OpenPart {
            linked_msat: 100_000_000_000,
            unlinked_msat: 0,
            can_add: true,
        });
        assert!(!tickable(&s, &r));
        r.open = Some(OpenPart {
            linked_msat: 60_000_000_000,
            unlinked_msat: 40_000_000_000,
            can_add: false,
        });
        assert!(!tickable(&s, &r));
        // A swap leg stays one to one even with an open part.
        let mut l = leg("l", LegSide::Receipt, "LBTC", Rail::Liquid, 1, None);
        l.linked = true;
        l.open = Some(OpenPart {
            linked_msat: 0,
            unlinked_msat: 1,
            can_add: true,
        });
        assert!(!tickable(&s, &l));
    }

    #[test]
    fn exact_gap_is_nothing() {
        assert_eq!(
            Tally {
                sent: 5,
                fee: 0,
                received: 5
            }
            .gap(),
            Gap::None
        );
    }
}
