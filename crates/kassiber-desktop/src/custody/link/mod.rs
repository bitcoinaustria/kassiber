//! "Link manually" (`routes/transfers/LinkManuallyDialog.tsx`, `LinkCandidates.tsx`): the two
//! sides of a transfer on top, candidates for the open side ranked against the other, then the
//! check — the difference, what the link counts as and the review core's dry run — and one
//! confirmation that applies exactly that run. A dialog over the page, or a panel beside its
//! lists when the page is wide; one state, so a link in progress survives the switch.
//!
//! What a link counts as is stated where the core records it one way only (the same asset on
//! both sides); between two assets the owner answers one question with both consequences
//! spelled out. Nothing is linked before the owner confirms; the core plans, books or refuses.

pub mod candidates;
pub mod multi;
mod strings;

use rune::{Cx, El, Tx, col, el, row, text};
use serde_json::{Value, json};

use super::kit::{self, Var};
use super::link_model::{
    self, Difference, FeeCheck, LegSide, LinkLeg, LinkQuestion, PairWarning, RelinkPortion,
    RelinkTerms, bps_text,
};
use super::model;
use super::unlink::common::{self, Applied, Fallback, Legs, Owner, PStatus, Preview};
use crate::app::{App, Reply};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::{ic, sensitive};

/// Stored with the pair; the book keeps one language.
pub const MANUAL_LINK_REASON: &str = "Linked by hand on Transfers";
pub const RELINK_REASON: &str = "Linked again after unlinking";

/// Terms to propose, from the pair as it was before it was unlinked.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preset {
    pub kind: Option<String>,
    pub policy: Option<String>,
    pub terms: Option<RelinkTerms>,
}

/// `LinkStart`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinkStart {
    /// Start from this transaction: a send picks its receipt, a receipt its send.
    pub transaction_id: Option<String>,
    /// Both sides known: open on the check.
    pub send_id: Option<String>,
    pub receipt_id: Option<String>,
    pub preset: Option<Preset>,
    pub relink: bool,
}

/// What the page needs once a link is made.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkedResult {
    pub pair_id: Option<String>,
    pub send_id: String,
    pub receipt_id: String,
}

/// `ui.transfers.pair_options` for the pair in the check.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub key: String,
    pub loading: bool,
    pub policies: Option<Vec<Value>>,
}

#[derive(Default)]
pub struct State {
    pub start: Option<LinkStart>,
    pub legs: Legs,
    pub preview: Preview,
    pub send_id: Option<String>,
    pub receipt_id: Option<String>,
    /// The side whose list is open; None shows the check.
    pub picking: Option<LegSide>,
    pub notice: Option<&'static str>,
    placed: bool,
    /// What the list knew about a pick, until the transaction itself is read.
    pub picked_send: Option<LinkLeg>,
    pub picked_receipt: Option<LinkLeg>,
    pub pick: candidates::Pick,
    /// The owner's answer, and the pair it was given for.
    pub chosen: Option<&'static str>,
    check_key: String,
    pub options: Options,
    pub error: Option<String>,
    /// An open select of the picker.
    pub menu: Option<&'static str>,
    /// "Link several": on, the side of the one transaction (None: the send), the ticked
    /// transactions of the other side in the order ticked, and whether the check is shown.
    pub multi: bool,
    pub one: Option<LegSide>,
    pub several: Vec<LinkLeg>,
    pub reviewing: bool,
}

/// Opens "Link manually" (a new start replaces a link in progress).
pub fn open(app: &mut App, start: LinkStart) {
    let both = start.send_id.is_some() && start.receipt_id.is_some();
    let picking = if both {
        None
    } else if start.receipt_id.is_some() {
        Some(LegSide::Send)
    } else if start.send_id.is_some() {
        Some(LegSide::Receipt)
    } else {
        Some(LegSide::Send)
    };
    app.custody.link = State {
        send_id: start.send_id.clone(),
        receipt_id: start.receipt_id.clone(),
        picking,
        start: Some(start.clone()),
        ..State::default()
    };
    if !both && let Some(id) = &start.transaction_id {
        common::ensure_leg(app, Owner::Link, id);
    }
    for id in [start.send_id, start.receipt_id].into_iter().flatten() {
        common::ensure_leg(app, Owner::Link, &id);
    }
}

pub fn close(app: &mut App) {
    app.custody.link = State::default();
}

pub fn is_open(app: &App) -> bool {
    app.custody.link.start.is_some()
}

fn applying(app: &App) -> bool {
    app.custody.link.preview.applying
}

fn request_close(app: &mut App) {
    if !applying(app) {
        close(app);
    }
}

fn both(s: &State) -> bool {
    s.start
        .as_ref()
        .is_some_and(|st| st.send_id.is_some() && st.receipt_id.is_some())
}

/// Reading the transaction it was started from.
fn reading(app: &App) -> bool {
    let s = &app.custody.link;
    let Some(st) = &s.start else { return false };
    match &st.transaction_id {
        Some(id) if !both(s) => s.legs.map.get(id).is_none_or(|r| r.loading),
        _ => false,
    }
}

/// The two legs as read (with their open parts), and whether each is still loading.
fn sides(app: &App) -> ((Option<LinkLeg>, bool), (Option<LinkLeg>, bool)) {
    let s = &app.custody.link;
    let parts = super::open_parts(app);
    let (send, sl, _) = s.legs.leg(s.send_id.as_deref(), parts);
    let (receipt, rl, _) = s.legs.leg(s.receipt_id.as_deref(), parts);
    ((send, sl), (receipt, rl))
}

/// The other side as the list knew it, until it is read (`known`).
fn known(app: &App, side: LegSide) -> Option<LinkLeg> {
    let s = &app.custody.link;
    let ((send, _), (receipt, _)) = sides(app);
    let (leg, id, picked) = match side {
        LegSide::Send => (send, &s.send_id, &s.picked_send),
        LegSide::Receipt => (receipt, &s.receipt_id, &s.picked_receipt),
    };
    leg.or_else(|| picked.clone().filter(|p| Some(&p.id) == id.as_ref()))
}

/// A row was picked (`choose`).
pub fn choose(app: &mut App, side: LegSide, leg: LinkLeg) {
    let s = &mut app.custody.link;
    s.notice = None;
    let id = leg.id.clone();
    // A new one transaction starts the ticks over: what fits depends on it.
    if s.multi
        && side == multi::one_side(s)
        && (if side == LegSide::Send {
            &s.send_id
        } else {
            &s.receipt_id
        })
        .as_ref()
            != Some(&id)
    {
        s.several.clear();
    }
    match side {
        LegSide::Send => {
            s.picked_send = Some(leg);
            s.send_id = Some(id.clone());
            s.picking = if s.receipt_id.is_some() {
                None
            } else {
                Some(LegSide::Receipt)
            };
        }
        LegSide::Receipt => {
            s.picked_receipt = Some(leg);
            s.receipt_id = Some(id.clone());
            s.picking = if s.send_id.is_some() {
                None
            } else {
                Some(LegSide::Send)
            };
        }
    }
    common::ensure_leg(app, Owner::Link, &id);
}

/// Both sides read and no list open: the pair in the check.
fn ready(app: &App) -> Option<(LinkLeg, LinkLeg)> {
    let s = &app.custody.link;
    if s.picking.is_some() {
        return None;
    }
    let ((send, _), (receipt, _)) = sides(app);
    Some((send?, receipt?))
}

/// The core's own proposal for exactly this pair, when its matcher suggests it.
fn suggested(app: &App, send: &LinkLeg, receipt: &LinkLeg) -> Option<(String, String)> {
    let c = app.custody.page.suggest.as_deref()?.iter().find(|c| {
        c["out_id"].as_str() == Some(&send.id) && c["in_id"].as_str() == Some(&receipt.id)
    })?;
    let kind = c["rule_match"]["kind"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or(c["default_kind"].as_str())
        .unwrap_or("manual")
        .to_owned();
    let policy = c["rule_match"]["policy"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or(c["default_policy"].as_str())
        .unwrap_or("")
        .to_owned();
    Some((kind, policy))
}

/// What the check needs, worked out once for the view and the plan.
struct Check {
    question: LinkQuestion,
    reading: bool,
    sale_only: bool,
    carry_refused_reason: Option<String>,
    answer: Option<&'static str>,
    proposed: Option<&'static str>,
    proposed_before: bool,
    kind: String,
    portion: RelinkPortion,
}

fn check(app: &App, send: &LinkLeg, receipt: &LinkLeg) -> Check {
    let s = &app.custody.link;
    let preset = s.start.as_ref().and_then(|st| st.preset.clone());
    let question = link_model::link_question(send, receipt);
    let sug = suggested(app, send, receipt);
    let before = link_model::link_answer(preset.as_ref().and_then(|p| p.policy.as_deref()));
    let reading = question != LinkQuestion::None
        && (s.options.loading || s.options.key != pair_key(send, receipt));
    let policies = if question == LinkQuestion::None {
        None
    } else {
        s.options.policies.as_ref()
    };
    let carry_refused = policies.and_then(|ps| {
        ps.iter()
            .find(|r| r["policy"] == "carrying-value" && r["allowed"] == false)
    });
    let sale_only = carry_refused.is_some()
        && policies.is_some_and(|ps| {
            ps.iter()
                .any(|r| r["policy"] == "taxable" && r["allowed"] == true)
        });
    let proposed =
        before.or_else(|| link_model::link_answer(sug.as_ref().map(|(_, p)| p.as_str())));
    let answer = if question == LinkQuestion::None {
        Some("carrying-value")
    } else if reading {
        None
    } else if sale_only {
        Some("taxable")
    } else {
        s.chosen.or(proposed)
    };
    let kind = preset
        .as_ref()
        .and_then(|p| p.kind.clone())
        .or_else(|| sug.map(|(k, _)| k))
        .unwrap_or_else(|| link_model::default_pair_kind(send, receipt).to_owned());
    let portion = link_model::relink_portion(
        preset.as_ref().and_then(|p| p.terms.as_ref()),
        send.fee_msat,
        question != LinkQuestion::None,
    );
    Check {
        question,
        reading,
        sale_only,
        carry_refused_reason: carry_refused.and_then(|r| r["reason"].as_str().map(str::to_owned)),
        answer,
        proposed,
        proposed_before: before.is_some(),
        kind,
        portion,
    }
}

fn pair_key(send: &LinkLeg, receipt: &LinkLeg) -> String {
    format!("{}>{}", send.id, receipt.id)
}

fn operations(app: &App, send: &LinkLeg, receipt: &LinkLeg, c: &Check) -> Option<Value> {
    let answer = c.answer?;
    if c.portion == RelinkPortion::Unsendable {
        return None;
    }
    let relink = app.custody.link.start.as_ref().is_some_and(|s| s.relink);
    let mut op = json!({
        "type": "pair",
        "out_transaction_id": send.id,
        "in_transaction_id": receipt.id,
        "pair_kind": c.kind,
        "policy": answer,
        "reason": if relink { RELINK_REASON } else { MANUAL_LINK_REASON },
    });
    if let RelinkPortion::Part(m) = c.portion {
        op["out_amount_msat"] = json!(m as i64);
    }
    Some(json!([op]))
}

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let changed = step(app);
    changed | common::poll_preview(app, Owner::Link, tx)
}

/// Everything `poll` does but the preview's timer.
fn step(app: &mut App) -> bool {
    if !is_open(app) {
        common::sync_preview(app, Owner::Link, None);
        return false;
    }
    let mut changed = false;
    // A starting transaction takes its own side once it is read.
    let st = app.custody.link.start.clone().unwrap_or_default();
    if !app.custody.link.placed && !both(&app.custody.link) {
        if let Some(id) = st.transaction_id.clone() {
            if !reading(app) {
                let parts = super::open_parts(app);
                let (leg, _, _) = app.custody.link.legs.leg(Some(&id), parts);
                let s = &mut app.custody.link;
                s.placed = true;
                match leg {
                    None => s.notice = Some("unknown"),
                    Some(l) if l.side == LegSide::Send => {
                        s.send_id = Some(l.id.clone());
                        s.picking = Some(LegSide::Receipt);
                    }
                    Some(l) => {
                        s.receipt_id = Some(l.id.clone());
                        s.picking = Some(LegSide::Send);
                    }
                }
                changed = true;
            }
        } else {
            app.custody.link.placed = true;
        }
    }
    // A chosen transaction that the book no longer has opens its side again.
    let ((send, sl), (receipt, rl)) = sides(app);
    {
        let s = &mut app.custody.link;
        if s.send_id.is_some() && !sl && send.is_none() {
            s.send_id = None;
            s.picking = Some(LegSide::Send);
            s.notice = Some("gone");
            changed = true;
        } else if s.receipt_id.is_some() && !rl && receipt.is_none() {
            s.receipt_id = None;
            s.picking = s.picking.or(Some(LegSide::Receipt));
            s.notice = Some("gone");
            changed = true;
        }
    }
    changed |= multi::step(app);
    // The open list reads its rows.
    if let Some(side) = app.custody.link.picking.filter(|_| !reading(app)) {
        let anchor = known(
            app,
            if side == LegSide::Send {
                LegSide::Receipt
            } else {
                LegSide::Send
            },
        );
        candidates::poll(app, side, anchor.as_ref());
    }
    // The check: a new pair starts its answer and its read of the allowed answers over.
    let mut ops = None;
    if app.custody.link.multi {
        if !super::updating(app) {
            ops = multi::operations(app);
        }
    } else if let Some((send, receipt)) = ready(app) {
        let key = pair_key(&send, &receipt);
        if app.custody.link.check_key != key {
            let s = &mut app.custody.link;
            s.check_key = key.clone();
            s.chosen = None;
            s.error = None;
            s.options = Options::default();
            if link_model::link_question(&send, &receipt) != LinkQuestion::None {
                s.options = Options {
                    key: key.clone(),
                    loading: true,
                    policies: None,
                };
                app.send(
                    "ui.transfers.pair_options",
                    json!({"tx_out": send.id, "tx_in": receipt.id}),
                    "custody.link.options",
                );
            }
            changed = true;
        }
        let c = check(app, &send, &receipt);
        // While the journals recalculate, a plan would be read from a book about to change.
        if !super::updating(app) {
            ops = operations(app, &send, &receipt, &c);
        }
    }
    common::sync_preview(app, Owner::Link, ops);
    changed
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "custody.link.leg" => common::on_leg(app, Owner::Link, r),
        "custody.link.list" => candidates::on_list(app, r),
        "custody.link.options" => {
            let o = &mut app.custody.link.options;
            o.loading = false;
            // Unread or unreadable, both answers stay on offer and the dry run still refuses.
            o.policies = if r.error.is_none() {
                r.data["policies"].as_array().cloned()
            } else {
                None
            };
        }
        _ => {
            let Some(stage) = tag.strip_prefix("custody.link.pv.") else {
                return;
            };
            let failed = t("review:transfersPage.effects.unavailable");
            if let Some(applied) = common::on_preview_reply(app, Owner::Link, stage, r, &failed) {
                if app.custody.link.multi {
                    on_applied_several(app, applied);
                    return;
                }
                let Some((send, receipt)) = ready(app) else {
                    return;
                };
                match applied {
                    Applied::Ok(result) => {
                        let pair_id = common::applied_id(&result, "pair_id");
                        super::page::on_linked(
                            app,
                            LinkedResult {
                                pair_id,
                                send_id: send.id,
                                receipt_id: receipt.id,
                            },
                        );
                    }
                    Applied::Err { message, code } => {
                        if common::is_stale(code.as_deref()) {
                            // The book changed since the check: run it again rather than link on old figures.
                            app.custody.link.error = Some(t("review:transfersPage.link.stale"));
                            common::retry_preview(app, Owner::Link);
                        } else {
                            let refusal = refusal_text(code.as_deref())
                                .unwrap_or_else(|| t("review:transfersPage.link.failed"));
                            app.custody.link.error = Some(
                                [refusal, message]
                                    .into_iter()
                                    .filter(|s| !s.is_empty())
                                    .collect::<Vec<_>>()
                                    .join(" "),
                            );
                        }
                    }
                }
            }
        }
    }
}

/// "Link several" was applied (or refused) as one plan.
fn on_applied_several(app: &mut App, applied: Applied) {
    match applied {
        Applied::Ok(_) => {
            let Some(a) = multi::anchor(app) else {
                return close(app);
            };
            let first = app
                .custody
                .link
                .several
                .first()
                .map(|l| l.id.clone())
                .unwrap_or_default();
            let (send_id, receipt_id) = if a.side == LegSide::Send {
                (a.id, first)
            } else {
                (first, a.id)
            };
            // Several pairs were made: each is unlinked from its row, as any reviewed link.
            super::page::on_linked(
                app,
                LinkedResult {
                    pair_id: None,
                    send_id,
                    receipt_id,
                },
            );
        }
        Applied::Err { message, code } => {
            if common::is_stale(code.as_deref()) {
                // The book changed since the check: check again and ask for a fresh confirmation.
                app.custody.link.error = Some(t("review:transfersPage.link.stale"));
                common::retry_preview(app, Owner::Link);
            } else {
                let refusal = refusal_text(code.as_deref())
                    .unwrap_or_else(|| t("review:transfersPage.link.failed"));
                app.custody.link.error = Some(
                    [refusal, message]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
        }
    }
}

/// Escape: an open select first, then the dialog or panel (not while it applies).
pub fn escape(app: &mut App) -> bool {
    if app.custody.link.menu.take().is_some() {
        return true;
    }
    if !is_open(app) {
        return false;
    }
    request_close(app);
    true
}

pub fn reload(app: &mut App) {
    if is_open(app) {
        common::reload_legs(app, Owner::Link);
        app.custody.link.pick.args = None;
    }
}

/// The plain reason behind a refusal the core names by code (`refusalText`).
fn refusal_text(code: Option<&str>) -> Option<String> {
    let k = match code? {
        "conflict" | "custody_component_membership_conflict" => "conflict",
        "custody_legacy_group_invalid" | "custody_legacy_group_ambiguous" => "nothingLeft",
        "transfer_network_mismatch" => "network",
        "not_found" => "notFound",
        "validation" | "rail_policy_required" => "terms",
        _ => return None,
    };
    Some(t(&format!("review:transfersPage.link.refused.{k}")))
}

/// `differenceSentence`.
fn difference_sentence(d: &Difference, part: bool) -> String {
    let Difference::Fee {
        kind,
        amount_msat,
        share_bps,
    } = d
    else {
        return t("review:transfersPage.link.diff.otherAssetLong");
    };
    if part && *kind == FeeCheck::None {
        return t("review:transfersPage.link.partDone");
    }
    if part && *kind != FeeCheck::Negative {
        return tr(
            "review:transfersPage.link.partAfter",
            &[("amount", &model::msat_btc(*amount_msat))],
        );
    }
    let amount = model::msat_sats(*amount_msat);
    let share = match share_bps {
        None => "—".to_owned(),
        Some(0) => "<0.01".to_owned(),
        Some(b) => bps_text(*b),
    };
    match kind {
        FeeCheck::None => t("review:transfersPage.link.diff.sameLong"),
        FeeCheck::Negative => tr(
            "review:transfersPage.link.diff.moreLong",
            &[("amount", &amount)],
        ),
        FeeCheck::Mismatch => tr(
            "review:transfersPage.link.diff.mismatchLong",
            &[("amount", &amount), ("share", &share)],
        ),
        _ => tr(
            "review:transfersPage.link.diff.lessLong",
            &[("amount", &amount), ("share", &share)],
        ),
    }
}

/// `warningText`.
fn warning_text(key: PairWarning, send: &LinkLeg, receipt: &LinkLeg) -> String {
    let leg = if matches!(key, PairWarning::SendLinked | PairWarning::SendIgnored) {
        send
    } else {
        receipt
    };
    if matches!(key, PairWarning::SendLinked | PairWarning::ReceiptLinked)
        && let Some(w) = leg.link.as_ref().and_then(|l| l.wallet.clone())
    {
        return tr(
            &format!("review:transfersPage.link.warn.{}To", key.key()),
            &[("wallet", &w)],
        );
    }
    t(&format!("review:transfersPage.link.warn.{}", key.key()))
}

/// The content as a panel docked beside the page's lists (27 rem wide).
pub fn panel(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    if !is_open(app) {
        return col();
    }
    let (title, desc) = title_desc(app);
    let content = content(app, cx, tk, layer, false);
    common::panel_shell(
        app,
        cx,
        tk,
        "link-manually",
        title,
        text(desc).font(tk.tsm(400.0)).color(tk.muted_fg).wrap(),
        t("review:transfersPage.unlink.close"),
        !applying(app),
        request_close,
        content,
    )
}

/// The content as a dialog over the page.
pub fn dialog(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) {
    if !is_open(app) {
        return;
    }
    let (title, desc) = title_desc(app);
    let content = content(app, cx, tk, layer, true);
    common::dialog_shell(
        app,
        tk,
        layer,
        "link-manually",
        48.0,
        title,
        text(desc).font(tk.tsm(400.0)).color(tk.muted_fg).wrap(),
        request_close,
        content,
    );
}

fn title_desc(app: &App) -> (String, String) {
    let s = &app.custody.link;
    let relink = s.start.as_ref().is_some_and(|st| st.relink);
    if s.multi {
        return (
            strings::title(),
            if s.reviewing {
                strings::subtitle_check()
            } else {
                strings::subtitle()
            },
        );
    }
    (
        t(if relink {
            "review:transfersPage.link.titleAgain"
        } else {
            "review:transfersPage.link.title"
        }),
        t(if s.picking.is_some() {
            "review:transfersPage.link.subtitle"
        } else {
            "review:transfersPage.link.subtitleCheck"
        }),
    )
}

fn set_picking(app: &mut App, side: LegSide) {
    app.custody.link.picking = Some(side);
    app.custody.link.reviewing = false;
}

/// The flow: the notice, the two slots, early warnings, then the list or the check.
fn content(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>, wide: bool) -> Vec<El> {
    let s = &app.custody.link;
    let rd = reading(app);
    let ((send, sl), (receipt, rl)) = sides(app);
    let mut out = vec![];
    if let Some(n) = s.notice {
        out.push(
            row()
                .key("link-notice")
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .fill(tk.muted.alpha(0.3))
                .role("status")
                .child(
                    text(t(&format!("review:transfersPage.link.notice.{n}")))
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .wrap(),
                ),
        );
    }
    let slot = |side: LegSide, leg: &Option<LinkLeg>, loading: bool| -> El {
        let id = if side == LegSide::Send {
            &s.send_id
        } else {
            &s.receipt_id
        };
        let label = t(if side == LegSide::Send {
            "review:transfersPage.leg.sent"
        } else {
            "review:transfersPage.leg.received"
        });
        let active = s.picking == Some(side);
        let sk = if side == LegSide::Send {
            "send"
        } else {
            "receipt"
        };
        if id.is_none() {
            let line = if rd {
                t("review:transfersPage.link.reading")
            } else {
                t(&format!(
                    "review:transfersPage.link.slot.{}{}",
                    if active { "pick" } else { "open" },
                    if side == LegSide::Send {
                        "Send"
                    } else {
                        "Receipt"
                    }
                ))
            };
            let mut b = col()
                .key(format!("link-slot-{sk}"))
                .w_full()
                .min_h(tk.r(4.5))
                .px(tk.r(0.75))
                .py(tk.r(0.625))
                .gap(tk.r(0.25))
                .justify_center()
                .radius(tk.r(0.5))
                .role("button")
                .checked(active)
                .label(format!("{label}: {line}"))
                .focusable(!rd)
                .disabled(rd)
                .hover(tk.muted.alpha(0.4))
                .on_press(move |a: &mut App| set_picking(a, side))
                .child(
                    text(label.to_uppercase())
                        .font(tk.txs(500.0).tracking(0.025 * tk.r(0.75)))
                        .color(tk.muted_fg),
                )
                .child(
                    text(line)
                        .font(tk.tsm(400.0))
                        .color(if active { tk.fg } else { tk.muted_fg })
                        .wrap(),
                );
            b = if active {
                b.border(1.0, tk.primary).fill(tk.primary.alpha(0.05))
            } else {
                b.border(1.0, tk.border).border_dash(4.0, 3.0)
            };
            return b;
        }
        let listed = if side == LegSide::Send {
            &s.picked_send
        } else {
            &s.picked_receipt
        };
        let fb = listed
            .as_ref()
            .filter(|l| Some(&l.id) == id.as_ref())
            .map(|l| Fallback {
                wallet: l.wallet.clone(),
                rail: l.rail,
                amount_msat: Some(l.amount_msat),
                occurred_at: l.occurred_at.clone(),
            });
        let action = if active || applying(app) {
            None
        } else {
            Some(
                crate::txdetail::widgets::button(
                    tk,
                    format!("link-change-{sk}"),
                    Var::Outline,
                    crate::txdetail::widgets::Size::Xs,
                    None,
                    &t("review:transfersPage.link.change"),
                    true,
                )
                .h(tk.r(1.5))
                .label(t(if side == LegSide::Send {
                    "review:transfersPage.link.changeSend"
                } else {
                    "review:transfersPage.link.changeReceipt"
                }))
                .on_press(move |a: &mut App| set_picking(a, side)),
            )
        };
        common::move_leg(
            app,
            cx,
            tk,
            &format!("link-leg-{sk}"),
            label,
            leg.as_ref(),
            loading,
            fb.as_ref(),
            active,
            action,
            close,
        )
    };
    // "Link several": the other side is the tally of what is ticked.
    let several = s.multi.then(|| multi::one_side(s));
    let a = if several == Some(LegSide::Receipt) {
        multi::tally_slot(app, tk, LegSide::Send)
    } else {
        slot(LegSide::Send, &send, sl)
    };
    let b = if several == Some(LegSide::Send) {
        multi::tally_slot(app, tk, LegSide::Receipt)
    } else {
        slot(LegSide::Receipt, &receipt, rl)
    };
    out.push(if wide {
        row()
            .w_full()
            .gap(tk.r(0.5))
            .items_stretch()
            .child(a.grow(1.0).w(0.0))
            .child(
                row()
                    .center()
                    .child(ic(&I::ARROW_RIGHT, tk.r(1.0), tk.muted_fg).a11y_hidden()),
            )
            .child(b.grow(1.0).w(0.0))
    } else {
        col().w_full().gap(tk.r(0.5)).child(a).child(b)
    });
    // A side chosen from outside may already be linked or ignored: say so first.
    if s.picking.is_some() {
        let mut early = vec![];
        for leg in [&send, &receipt].into_iter().flatten() {
            if leg.excluded || (leg.linked && !link_model::leg_has_open_part(leg)) {
                let key = match (leg.side, leg.excluded) {
                    (LegSide::Send, true) => PairWarning::SendIgnored,
                    (LegSide::Send, false) => PairWarning::SendLinked,
                    (LegSide::Receipt, true) => PairWarning::ReceiptIgnored,
                    (LegSide::Receipt, false) => PairWarning::ReceiptLinked,
                };
                early.push(warning_text(key, leg, leg));
            }
        }
        if !early.is_empty() {
            out.push(common::warnings(app, tk, &early));
        }
    }
    if !applying(app)
        && !s.reviewing
        && !s.start.as_ref().is_some_and(|st| st.relink)
        && (s.picking.is_some() || s.multi)
    {
        out.push(multi::toggle_row(app, tk));
    }
    if rd {
        return out;
    }
    if s.multi {
        out.extend(multi::body(app, cx, tk, layer, wide));
        return out;
    }
    if let Some(side) = s.picking {
        let anchor = known(
            app,
            if side == LegSide::Send {
                LegSide::Receipt
            } else {
                LegSide::Send
            },
        );
        let selected = if side == LegSide::Send {
            s.send_id.clone()
        } else {
            s.receipt_id.clone()
        };
        out.push(candidates::view(
            app,
            cx,
            tk,
            layer,
            side,
            anchor.as_ref(),
            selected.as_deref(),
            wide,
        ));
        let mut foot = row().w_full().gap(tk.r(0.5)).center_y().justify_between();
        if s.send_id.is_some() && s.receipt_id.is_some() {
            foot = foot.child(
                kit::btn_md(
                    tk,
                    "link-keep",
                    Var::Ghost,
                    None,
                    &t(if side == LegSide::Send {
                        "review:transfersPage.link.keepSend"
                    } else {
                        "review:transfersPage.link.keepReceipt"
                    }),
                    true,
                )
                .on_press(|a: &mut App| a.custody.link.picking = None),
            );
        } else {
            foot = foot.child(el());
        }
        out.push(
            foot.child(
                kit::btn_md(
                    tk,
                    "link-cancel",
                    Var::Outline,
                    None,
                    &t("review:transfersPage.link.cancel"),
                    true,
                )
                .on_press(close),
            ),
        );
        return out;
    }
    match ready(app) {
        Some((send, receipt)) => out.push(check_view(app, tk, &send, &receipt, wide)),
        None => out.push(
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
        ),
    }
    out
}

/// A labelled line of the check: the label in its own column when there is room (`CheckRow`).
fn check_row(tk: &Tk, label: String, content: El, wide: bool) -> El {
    if wide {
        row()
            .w_full()
            .gap(tk.r(1.0))
            .items_start()
            .child(
                text(label)
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .w(tk.r(7.5))
                    .pt(1.0)
                    .wrap(),
            )
            .child(content.grow(1.0).w(0.0).min_w(0.0))
    } else {
        col()
            .w_full()
            .gap(tk.r(0.25))
            .child(text(label).font(tk.txs(500.0)).color(tk.muted_fg))
            .child(content)
    }
}

fn choose_answer(app: &mut App, v: &'static str) {
    app.custody.link.chosen = Some(v);
}

fn confirm(app: &mut App) {
    if app.custody.link.preview.applying
        || !matches!(app.custody.link.preview.status, PStatus::Ready { .. })
        || super::updating(app)
    {
        return;
    }
    app.custody.link.error = None;
    common::apply_preview(app, Owner::Link);
}

fn check_view(app: &App, tk: &Tk, send: &LinkLeg, receipt: &LinkLeg, wide: bool) -> El {
    let s = &app.custody.link;
    let hidden = app.prefs.hide_sensitive();
    let updating = super::updating(app);
    let c = check(app, send, receipt);
    let busy = s.preview.applying;
    let outcome = match &s.preview.status {
        PStatus::Ready { artifact, .. } => {
            Some(common::move_outcome(artifact, &send.id, &receipt.id))
        }
        _ => None,
    };
    let difference = link_model::difference_of(send, receipt);
    let warnings = link_model::pair_warnings(send, receipt);
    let creates_holds = outcome.is_some_and(|o| (o.send_held || o.receipt_held) && !o.was_held);
    let mut lines = vec![];
    if let Some(o) = outcome {
        if o.was_held && !o.send_held && !o.receipt_held {
            lines.push(t("review:transfersPage.link.effectReleased"));
        }
        if creates_holds {
            lines.push(tc(
                "review:transfersPage.link.effectCreatesHolds",
                (o.held_after.saturating_sub(o.held_before)).max(1) as usize,
                &[],
            ));
        } else if o.send_held || o.receipt_held {
            lines.push(t("review:transfersPage.link.effectStillHeld"));
        }
    }
    let anyway = creates_holds || warnings.iter().any(|(_, serious)| *serious);
    let shares = link_model::shares_with(send, receipt);
    let part_notes: Vec<String> = [send, receipt]
        .iter()
        .filter(|l| shares && link_model::leg_has_open_part(l) && l.open.is_some())
        .map(|l| {
            let o = l.open.unwrap();
            tr(
                if l.side == LegSide::Send {
                    "review:transfersPage.link.partNoteSend"
                } else {
                    "review:transfersPage.link.partNoteReceipt"
                },
                &[
                    ("linked", &model::msat_btc(o.linked_msat)),
                    ("left", &model::msat_btc(o.unlinked_msat)),
                ],
            )
        })
        .collect();
    let rest_possible = difference.kind() == Some(FeeCheck::Mismatch)
        && !link_model::leg_has_open_part(send)
        && shares
        && c.kind == "manual";
    let bitcoin = model::is_bitcoin(&send.asset) && model::is_bitcoin(&receipt.asset);
    let portion_line = |m: i128| {
        sensitive(
            tr(
                "review:transfersPage.link.portion",
                &[("amount", &model::msat_btc(m))],
            ),
            tk.txs(400.0),
            tk.muted_fg,
            hidden,
        )
        .wrap()
        .mt(tk.r(0.375))
    };

    let mut dl = col().w_full().gap(tk.r(0.75));
    // The difference.
    let mut diff = col().w_full().gap(tk.r(0.125));
    for n in &part_notes {
        diff = diff.child(common::para_s(app, tk, n.clone(), tk.muted_fg).key("link-part-note"));
    }
    diff = diff.child(
        common::para_s(
            app,
            tk,
            difference_sentence(&difference, shares && link_model::leg_has_open_part(send)),
            tk.fg,
        )
        .key("link-difference"),
    );
    dl = dl.child(check_row(
        tk,
        t("review:transfersPage.link.differenceTitle"),
        diff,
        wide,
    ));
    // What it counts as, or the question.
    if c.question == LinkQuestion::None {
        dl = dl.child(check_row(
            tk,
            t("review:transfersPage.link.countsAs"),
            common::para(
                tk,
                t(if bitcoin {
                    "review:transfersPage.link.ownBitcoin"
                } else {
                    "review:transfersPage.link.ownAsset"
                }),
                tk.fg,
            )
            .key("link-counts-as"),
            wide,
        ));
    } else if c.reading {
        dl = dl.child(check_row(
            tk,
            t("review:transfersPage.link.countsAs"),
            row()
                .gap(tk.r(0.5))
                .center_y()
                .role("status")
                .child(kit::spinner(tk, tk.r(1.0), tk.muted_fg))
                .child(common::para(
                    tk,
                    t("review:transfersPage.link.reading"),
                    tk.muted_fg,
                )),
            wide,
        ));
    } else if c.sale_only {
        let mut v = col()
            .key("link-counts-as")
            .w_full()
            .child(common::para(
                tk,
                t("review:transfersPage.link.onlySale"),
                tk.fg,
            ))
            .child(
                text(t(
                    if c.carry_refused_reason.as_deref() == Some("network_mismatch") {
                        "review:transfersPage.link.onlyWhy.network"
                    } else {
                        "review:transfersPage.link.onlyWhy.crossAsset"
                    },
                ))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
            );
        if let RelinkPortion::Part(m) = c.portion {
            v = v.child(portion_line(m));
        }
        dl = dl.child(check_row(
            tk,
            t("review:transfersPage.link.countsAs"),
            v,
            wide,
        ));
    } else {
        let options: [(&'static str, String, String); 2] = [
            (
                "carrying-value",
                t(if bitcoin {
                    "review:transfersPage.link.question.keepBitcoin"
                } else {
                    "review:transfersPage.link.question.keepAsset"
                }),
                t(if bitcoin {
                    "review:transfersPage.link.question.keepBitcoinHint"
                } else {
                    "review:transfersPage.link.question.keepAssetHint"
                }),
            ),
            (
                "taxable",
                t("review:transfersPage.link.question.sale"),
                t("review:transfersPage.link.question.saleHint"),
            ),
        ];
        let proposed_note = t(if c.proposed_before {
            "review:transfersPage.link.question.asBefore"
        } else {
            "review:transfersPage.link.suggested"
        });
        let mut group = if wide {
            row().w_full().gap(tk.r(0.5)).items_stretch()
        } else {
            col().w_full().gap(tk.r(0.5))
        }
        .key("link-question")
        .role("radiogroup")
        .label(t("review:transfersPage.link.question.title"))
        .focus_group();
        for (value, label, hint) in options {
            let checked = c.answer == Some(value);
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
            let mut title = row()
                .gap(tk.r(0.5))
                .center_y()
                .flex_wrap()
                .child(text(label.clone()).font(tk.tsm(500.0)).color(tk.fg));
            if c.proposed == Some(value) {
                title = title.child(
                    text(proposed_note.clone())
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                );
            }
            let mut opt = row()
                .key(format!("link-answer-{value}"))
                .min_w(0.0)
                .gap(tk.r(0.5))
                .items_start()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .radius(tk.r(0.5))
                .border(1.0, if checked { tk.primary } else { tk.border })
                .role("radio")
                .checked(checked)
                .label(label)
                .focusable(!busy)
                .disabled(busy)
                .child(dot.a11y_hidden())
                .child(
                    col()
                        .min_w(0.0)
                        .grow(1.0)
                        .gap(tk.r(0.125))
                        .child(title)
                        .child(text(hint).font(tk.txs(400.0)).color(tk.muted_fg).wrap()),
                );
            if checked {
                opt = opt.fill(tk.primary.alpha(0.05));
            }
            opt = if busy {
                opt.opacity(0.6)
            } else {
                opt.hover(tk.muted.alpha(0.4))
                    .on_press(move |a: &mut App| choose_answer(a, value))
            };
            group = group.child(if wide {
                opt.grow(1.0).w(0.0)
            } else {
                opt.w_full()
            });
        }
        let mut v = col().w_full().child(group);
        if let RelinkPortion::Part(m) = c.portion {
            v = v.child(portion_line(m));
        }
        dl = dl.child(check_row(
            tk,
            t("review:transfersPage.link.question.title"),
            v,
            wide,
        ));
    }
    // What changes.
    if !c.reading {
        let content = if c.portion == RelinkPortion::Unsendable {
            text(t("review:transfersPage.link.portionUnsendable"))
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .wrap()
                .role("alert")
                .key("link-portion-unsendable")
        } else if c.answer.is_some() {
            let refusal = match &s.preview.status {
                PStatus::Failed { code, .. } => refusal_text(code.as_deref()),
                _ => None,
            };
            let retry: Option<fn(&mut App)> =
                if matches!(s.preview.status, PStatus::Failed { .. }) && refusal.is_none() {
                    Some(|a: &mut App| common::retry_preview(a, Owner::Link))
                } else {
                    None
                };
            common::outcome_view(
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
            )
        } else {
            common::para(
                tk,
                t("review:transfersPage.link.question.unanswered"),
                tk.muted_fg,
            )
            .key("link-unanswered")
        };
        dl = dl.child(check_row(
            tk,
            t("review:transfersPage.link.changesTitle"),
            content,
            wide,
        ));
    }
    let mut out = col().key("link-check").w_full().gap(tk.r(1.0)).child(dl);
    if !warnings.is_empty() {
        let lines: Vec<String> = warnings
            .iter()
            .map(|(w, _)| {
                if *w == PairWarning::Mismatch && rest_possible {
                    t("review:transfersPage.link.warn.mismatchOrRest")
                } else {
                    warning_text(*w, send, receipt)
                }
            })
            .collect();
        out = out.child(common::warnings(app, tk, &lines));
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
    let relink = s.start.as_ref().is_some_and(|st| st.relink);
    let label = t(if busy {
        "review:transfersPage.link.linking"
    } else if anyway {
        "review:transfersPage.link.confirmAnyway"
    } else if relink {
        "review:transfersPage.link.confirmAgain"
    } else {
        "review:transfersPage.link.confirm"
    });
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
    out.child(
        row()
            .w_full()
            .justify_end()
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
                .on_press(close),
            )
            .child(go),
    )
}

/// Named states for `--scene`.
pub const SCENES: &[crate::app::Scene] = &[
    ("custody-link-new", |a| {
        start(a, LinkStart::default(), false)
    }),
    ("custody-link-send", |a| {
        let id = first_tx(a, true);
        start(
            a,
            LinkStart {
                transaction_id: id,
                ..Default::default()
            },
            false,
        )
    }),
    ("custody-link-check", |a| {
        let (s, r) = demo_pair(a, "manual");
        start(
            a,
            LinkStart {
                send_id: s,
                receipt_id: r,
                ..Default::default()
            },
            true,
        )
    }),
    ("custody-link-question", |a| {
        let (s, r) = demo_pair(a, "peg-out");
        start(
            a,
            LinkStart {
                send_id: s,
                receipt_id: r,
                ..Default::default()
            },
            true,
        )
    }),
    ("custody-link-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        let (s, r) = demo_pair(a, "manual");
        start(
            a,
            LinkStart {
                send_id: s,
                receipt_id: r,
                ..Default::default()
            },
            true,
        )
    }),
    ("custody-link-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        let id = first_tx(a, true);
        start(
            a,
            LinkStart {
                transaction_id: id,
                ..Default::default()
            },
            false,
        )
    }),
    ("custody-link-several", |a| {
        multi::scene(a, LegSide::Send, 0, false)
    }),
    ("custody-link-several-pick", |a| {
        multi::scene(a, LegSide::Send, 3, false)
    }),
    ("custody-link-several-sends", |a| {
        multi::scene(a, LegSide::Receipt, 2, false)
    }),
    ("custody-link-several-check", |a| {
        multi::scene(a, LegSide::Send, 3, true)
    }),
    ("custody-link-several-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        multi::scene(a, LegSide::Receipt, 2, true)
    }),
    ("custody-link-several-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        crate::i18n::set_lang(crate::i18n::Lang::De);
        multi::scene(a, LegSide::Send, 3, false)
    }),
    ("custody-link-several-check-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        crate::i18n::set_lang(crate::i18n::Lang::De);
        multi::scene(a, LegSide::Receipt, 2, true)
    }),
];

/// Opens the page and "Link manually" with `s`, waiting for the reads (and the dry run).
fn start(a: &mut App, s: LinkStart, plan: bool) {
    a.navigate("/transfers");
    super::prime(a);
    open(a, s);
    super::settle(a);
    // Placement, the list read and the pair options start from poll; run them here.
    for _ in 0..4 {
        step(a);
        super::settle(a);
    }
    if plan {
        common::start_now(a, Owner::Link);
        super::settle(a);
    }
}

/// The send of the newest outbound transaction in the book.
fn first_tx(a: &mut App, _send: bool) -> Option<String> {
    a.navigate("/transfers");
    super::prime(a);
    a.custody
        .page
        .moves
        .first()
        .map(|m| m.out_transaction_id.clone())
}

/// A pair's two sides from the page's lists: `manual` a same-asset move, otherwise that kind.
fn demo_pair(a: &mut App, kind: &str) -> (Option<String>, Option<String>) {
    a.navigate("/transfers");
    super::prime(a);
    let m = a
        .custody
        .page
        .moves
        .iter()
        .find(|m| {
            if kind == "manual" {
                m.from_rail == m.to_rail && m.pair_id.is_none()
            } else {
                m.pair_kind.as_deref() == Some(kind)
            }
        })
        .or(a.custody.page.moves.first());
    (
        m.map(|m| m.out_transaction_id.clone()),
        m.map(|m| m.in_transaction_id.clone()),
    )
}
