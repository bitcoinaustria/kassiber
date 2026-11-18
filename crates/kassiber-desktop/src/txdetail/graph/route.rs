//! The route card above the transaction graph: how this transaction is tied to another of the
//! owner's wallets. An owner-requested redesign of the web's SwapRouteStrip, by case, read from
//! the graph read's `swapRoute` (`crates/journals/src/transaction_graph.rs::swap_route`):
//!
//! * **One transaction** (both legs carry the same txid and asset): a move the chain itself
//!   shows. One compact line, From → To, with the amount, the network fee and the network. No
//!   legs to pick: both are the same transaction, whose graph is below.
//! * **Swap** (`routeKind` swap: a different asset or network, e.g. a peg-out): the sent and
//!   the received leg, each its own transaction, and the recorded swap fee between them.
//! * **Linked across transactions** (same asset, different txids): two transactions joined
//!   by a link, with a dashed node between them because nothing in this book connects them
//!   (a wallet that isn't connected, an exchange), the time between and the difference.
//!
//! Every card says who made the link: the owner (manual, bulk or rule; a custody assertion,
//! said so) or journal processing from wallet or chain evidence (its reason). When both legs
//! are real transactions, a segmented control picks whose graph is drawn below.

use rune::{Color, El, col, el, row, text};
use rune_kit::Placement;
use serde_json::Value;

use super::Ctx;
use super::data::{Leg, LegKey, SwapRoute};
use super::model::{self, RouteKind};
use super::route_strings as S;
use crate::app::App;
use crate::custody::model::{Rail, lineage_evidence_key, move_type, rail_for_leg};
use crate::format;
use crate::i18n::{has, t, tr};
use crate::icons as I;
use crate::ui::table::{LEAD, LEAD_GAP, cell_text, rail_logo};
use crate::ui::{ic, sensitive};

/// ASCII arrows in daemon text ("Swap LBTC -> BTC") drawn as the arrow glyph.
pub fn arrows(s: &str) -> String {
    s.replace(" -> ", " → ").replace(" => ", " → ")
}

/// Which of the three shapes a route is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    /// Both legs are one on-chain transaction (same txid, same asset).
    SameTransaction,
    /// A different asset or network on each side, two transactions.
    Swap,
    /// The same asset in two different transactions, joined by a link.
    Linked,
}

fn norm(s: Option<&str>) -> String {
    s.unwrap_or("").trim().to_lowercase()
}

pub fn case(r: &SwapRoute) -> Case {
    if model::route_kind(r) == RouteKind::Swap {
        return Case::Swap;
    }
    let (a, b) = (
        norm(r.out_leg.txid.as_deref()),
        norm(r.in_leg.txid.as_deref()),
    );
    let same_asset = norm(r.out_leg.asset.as_deref()) == norm(r.in_leg.asset.as_deref());
    if !a.is_empty() && a == b && same_asset {
        Case::SameTransaction
    } else {
        Case::Linked
    }
}

/// Whether the owner made this link (by hand, with the bulk exact suggestions or by a rule).
pub fn owner_made(r: &SwapRoute) -> bool {
    matches!(
        r.pair_source.as_deref(),
        Some("manual" | "bulk_exact" | "bulk_selected" | "rule_auto")
    )
}

/// Who linked it and when: the Transfers page's own "How it was linked" sentences.
fn provenance(r: &SwapRoute) -> String {
    let date = crate::custody::link_model::date_part(r.created_at.as_deref());
    match r.pair_source.as_deref() {
        Some("manual") => tr("review:transfersPage.unlink.howYou", &[("date", &date)]),
        Some("bulk_exact" | "bulk_selected") => {
            tr("review:transfersPage.unlink.howBulk", &[("date", &date)])
        }
        Some("rule_auto") => tr("review:transfersPage.unlink.howRule", &[("date", &date)]),
        _ => {
            let reason = t(lineage_evidence_key(r.kind.as_deref().unwrap_or(""), ""));
            tr(
                "review:transfersPage.unlink.howAutomatic",
                &[("reason", &reason)],
            )
        }
    }
}

fn rail(leg: &Leg) -> Rail {
    rail_for_leg(leg.asset.as_deref(), leg.wallet.kind.as_deref())
}

fn rail_logo_of(c: &Ctx, rail: Rail) -> crate::logos::RailLogo {
    let _ = c;
    match rail {
        Rail::Onchain => crate::logos::RailLogo::Onchain,
        Rail::Lightning => crate::logos::RailLogo::Lightning,
        Rail::Liquid => crate::logos::RailLogo::Liquid,
    }
}

/// The leg's network as the card names it (`Liquid`, `Bitcoin`, else its rail).
fn network(leg: &Leg) -> String {
    leg.network
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| t(&format!("review:transfersPage.rail.{}", rail(leg).key())))
}

fn wallet(leg: &Leg) -> String {
    leg.wallet
        .label
        .clone()
        .filter(|w| !w.trim().is_empty())
        .unwrap_or_else(|| t("transactions:graph.swapRouteUnknownLeg"))
}

fn btc(v: f64) -> String {
    format::btc(v, 8, false)
}

/// A txid (64 hex) shortened as the Transactions table does, else None.
fn txid(leg: &Leg) -> Option<String> {
    leg.txid
        .clone()
        .filter(|x| x.len() == 64 && x.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|x| format::compact_id(&x))
}

fn seconds_between(r: &SwapRoute) -> Option<f64> {
    let a = format::parse_ts(r.out_leg.occurred_at.as_deref()?)?;
    let b = format::parse_ts(r.in_leg.occurred_at.as_deref()?)?;
    Some(b - a)
}

/// "From → To" in `avail` points: one line when both names fit, else the two stacked, the
/// second after a corner arrow (the Transfers table's route stack), each name then cut keeping
/// the end that tells wallets apart, the whole name on hover. It never runs past its box.
#[allow(clippy::too_many_arguments)]
pub fn pair_names(
    from: &str,
    to: &str,
    style: rune::TextStyle,
    color: Color,
    arrow: f32,
    gap: f32,
    hidden: bool,
    avail: f32,
) -> El {
    let measure = |s: &str| rune::text::measure(s, &[], style, None).0.ceil();
    let (wf, wt) = (measure(from), measure(to));
    let fit = |s: &str, w: f32| {
        crate::custody::kit::wallet_name_fit(s, style, color, hidden, w.max(style.size * 2.5))
    };
    if wf + wt + arrow + 2.0 * gap <= avail {
        return row()
            .min_w(0.0)
            .gap(gap)
            .center_y()
            .label(format!("{from} → {to}"))
            .child(fit(from, wf))
            .child(ic(&I::ARROW_RIGHT, arrow, color.alpha(0.8)).a11y_hidden())
            .child(fit(to, wt));
    }
    col()
        .w_full()
        .min_w(0.0)
        .label(format!("{from} → {to}"))
        .child(row().w_full().min_w(0.0).child(fit(from, avail)))
        .child(
            row()
                .w_full()
                .min_w(0.0)
                .gap(gap)
                .center_y()
                .child(ic(&I::CORNER_DOWN_RIGHT, arrow, color.alpha(0.8)).a11y_hidden())
                .child(fit(to, avail - arrow - gap)),
        )
}

/// The 1.5 rem leading slot with the rail's logo.
fn lead(c: &Ctx, leg: &Leg) -> El {
    let tk = c.tk;
    let r = rail(leg);
    row().size(tk.r(LEAD), tk.r(LEAD)).center().child(rail_logo(
        c.app,
        tk,
        rail_logo_of(c, r),
        &network(leg),
    ))
}

/// One leg: rail logo; "Sent · Liquid", the wallet, date · txid; the amount on the right.
fn leg_row(c: &Ctx, key: &str, label: String, leg: &Leg, amount: Option<f64>, width: f32) -> El {
    let tk = c.tk;
    // What the name may take: the card less its padding, the logo slot, the gaps and the amount.
    let amount_w = amount
        .map(|a| {
            rune::text::measure(&btc(a), &[], tk.tsm(600.0).features("'tnum' 1"), None)
                .0
                .ceil()
        })
        .unwrap_or(0.0);
    let name_w =
        (width - 2.0 * tk.r(0.75) - 2.0 - tk.r(LEAD) - tk.r(LEAD_GAP) - tk.r(0.5) - amount_w)
            .max(tk.r(4.0));
    let xs = tk.txs(400.0);
    let head = format!("{label} · {}", network(leg));
    let mut meta: Vec<String> = Vec::new();
    if let Some(d) = leg.occurred_at.as_deref().filter(|d| !d.is_empty()) {
        meta.push(crate::custody::link_model::date_time(Some(d)));
    }
    let id = txid(leg);
    let mut sub = row().w_full().min_w(0.0).gap(tk.r(0.375)).center_y();
    if let Some(m) = meta.first() {
        sub = sub.child(
            text(m.clone())
                .font(xs.features("'tnum' 1"))
                .color(tk.muted_fg)
                .single_line(),
        );
    }
    if let Some(x) = &id {
        if !meta.is_empty() {
            sub = sub.child(text("·").font(xs).color(tk.muted_fg).a11y_hidden());
        }
        sub = sub.child(if c.hide {
            sensitive(x.clone(), xs.mono(), tk.muted_fg, true)
        } else {
            cell_text(x.clone(), xs.mono(), tk.muted_fg)
        });
    }
    let mut right = col().items_end().pl(tk.r(0.5));
    if let Some(a) = amount {
        right = right.child(
            sensitive(btc(a), tk.tsm(600.0).features("'tnum' 1"), tk.fg, c.hide).single_line(),
        );
    }
    if let Some(asset) = leg.asset.as_deref().filter(|a| !a.is_empty()) {
        right = right.child(
            text(asset.to_owned())
                .font(xs)
                .color(tk.muted_fg)
                .single_line(),
        );
    }
    row()
        .key(key.to_owned())
        .w_full()
        .min_w(0.0)
        .gap(tk.r(LEAD_GAP))
        .items_start()
        .child(lead(c, leg))
        .child(
            // The label and wallet beside the amount; date · txid under both, the full width.
            col()
                .grow(1.0)
                .min_w(0.0)
                .w(0.0)
                .child(
                    row()
                        .w_full()
                        .min_w(0.0)
                        .items_start()
                        .child(
                            col()
                                .grow(1.0)
                                .min_w(0.0)
                                .w(0.0)
                                .child(
                                    text(head.to_uppercase())
                                        .font(tk.t2xs(500.0).tracking(0.25))
                                        .color(tk.muted_fg)
                                        .single_line()
                                        .truncate()
                                        .min_w(0.0),
                                )
                                .child(crate::custody::kit::wallet_name_fit(
                                    &wallet(leg),
                                    tk.tsm(500.0),
                                    tk.fg,
                                    c.hide,
                                    name_w,
                                )),
                        )
                        .child(right),
                )
                .child(sub),
        )
}

/// Between the two legs: a rule under the first logo and, beside it, what lies between.
fn connector(c: &Ctx, content: El) -> El {
    let tk = c.tk;
    let gutter = tk.r(LEAD);
    row()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(LEAD_GAP))
        .items_stretch()
        .child(
            col().w(gutter).items_center().child(
                el().w(1.0)
                    .min_h(tk.r(1.0))
                    .grow(1.0)
                    .fill(tk.border)
                    .a11y_hidden(),
            ),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .w(0.0)
                .py(tk.r(0.375))
                .gap(tk.r(0.25))
                .child(content),
        )
}

/// A muted label.
fn pill(c: &Ctx, s: String) -> El {
    let tk = c.tk;
    row().min_w(0.0).child(
        text(s.clone())
            .font(tk.t2xs(500.0).features("'tnum' 1"))
            .color(tk.muted_fg)
            .single_line()
            .truncate()
            .min_w(0.0)
            .help_when_truncated(s),
    )
}

/// A label that needs a look (the table kit's amber ink).
fn warn_pill(c: &Ctx, s: String) -> El {
    let tk = c.tk;
    let (_, fg, _) = crate::ui::table::tone(tk, crate::ui::table::Tone::Warning);
    row().min_w(0.0).child(
        text(s.clone())
            .font(tk.t2xs(500.0).features("'tnum' 1"))
            .color(fg)
            .single_line()
            .truncate()
            .min_w(0.0)
            .help_when_truncated(s),
    )
}

fn fee_percent(fee: f64, base: Option<f64>) -> Option<String> {
    let b = base.filter(|b| *b != 0.0)?;
    let pct = fee.abs() / b.abs() * 100.0;
    let prec = if pct < 0.1 {
        3
    } else if pct < 1.0 {
        2
    } else {
        1
    };
    Some(format!("{pct:.prec$}%"))
}

/// The swap's recorded fee as a pill that flips between the amount and its share of the send.
fn fee_toggle(c: &Ctx, r: &SwapRoute) -> Option<El> {
    let tk = c.tk;
    let fee = r.swap_fee_btc?;
    let base = r
        .out_full_amount_btc
        .or(r.out_amount_btc)
        .or(r.out_leg.amount_btc);
    let absolute = btc(fee);
    let relative = fee_percent(fee, base);
    let shown = if c.hide {
        t("transactions:graph.hidden")
    } else if c.app.graph.fee_absolute || relative.is_none() {
        absolute.clone()
    } else {
        relative.clone().unwrap_or_default()
    };
    let other = if c.app.graph.fee_absolute {
        relative.clone().unwrap_or_default()
    } else {
        absolute.clone()
    };
    let toggle = relative.is_some();
    let label = tr("transactions:graph.swapRouteFee", &[("fee", &shown)]);
    Some(
        row()
            .key(format!("graph-route-fee{}", c.sfx))
            .px(tk.r(0.375))
            .py(1.0)
            .radius(tk.r(0.375))
            .border(1.0, tk.border.alpha(0.7))
            .fill(tk.muted.alpha(0.35))
            .hover(tk.primary.alpha(0.1))
            .role("button")
            .checked(c.app.graph.fee_absolute)
            .label(label.clone())
            .help(other)
            .focusable(true)
            .on_press(move |a: &mut App| {
                if toggle {
                    a.graph.fee_absolute = !a.graph.fee_absolute;
                }
            })
            .child(
                text(label)
                    .font(tk.t2xs(500.0).features("'tnum' 1"))
                    .color(tk.muted_fg)
                    .blur(if c.hide { 3.5 } else { 0.0 })
                    .single_line(),
            ),
    )
}

fn policy_label(r: &SwapRoute) -> Option<String> {
    let p = r.policy.as_deref()?;
    let key = format!("transactions:graph.policy.{p}");
    Some(if has(&key) { t(&key) } else { p.to_owned() })
}

/// The pair id Unlink removes: the record's own pair (as the Linked tab's Unpair), else the
/// route's.
fn pair_id(record: &Value, r: &SwapRoute) -> Option<String> {
    record["pair"]["id"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| r.id.clone())
        .filter(|x| !x.is_empty())
}

/// Unlink is offered where the Linked tab offers Unpair, and only for a link the owner made
/// (an automatic link is undone on the Transfers page, with its own preview).
fn can_unlink(app: &App, record: &Value, r: &SwapRoute) -> bool {
    if !owner_made(r) || c_busy(app) {
        return false;
    }
    crate::txdetail::host_loans(app)
        || crate::txdetail::review_host(app).is_some()
            && crate::review::sheet_can_unpair(app, record)
}

fn c_busy(app: &App) -> bool {
    app.detail.open.as_ref().is_some_and(|o| o.busy.unpair)
}

fn close_menu(a: &mut App) {
    a.graph.route_menu = false;
}

/// The ⋯ trigger and, while open, its menu.
fn menu(c: &Ctx, record: &Value, r: &SwapRoute, from: &str, to: &str, layer: &mut Vec<El>) -> El {
    let tk = c.tk;
    let anchor = format!("graph-route-menu{}", c.sfx);
    let open = c.app.graph.route_menu && c.sfx.is_empty();
    let route_name = format!("{from} → {to}");
    let label = tr(
        "review:transfersPage.linked.actions.label",
        &[("route", &route_name)],
    );
    let trigger = row()
        .key(anchor.clone())
        .size(tk.r(1.75), tk.r(1.75))
        .center()
        .radius(tk.r(0.375))
        .hover(tk.accent)
        .role("button")
        .label(label.clone())
        .help(label)
        .expanded(open)
        .focusable(true)
        .on_press(|a: &mut App| a.graph.route_menu = !a.graph.route_menu)
        .child(ic(&I::ELLIPSIS, tk.r(1.0), tk.muted_fg));
    if open && let Some(pid) = pair_id(record, r) {
        let entries = vec![crate::custody::kit::Entry::item(
            "graph-route-unlink",
            Some(&I::UNLINK),
            t("review:swap.paired.unpair"),
            move |a: &mut App| crate::txdetail::actions::unpair(a, &pid),
        )];
        crate::custody::kit::menu(
            tk,
            layer,
            &format!("{anchor}-content"),
            &anchor,
            Placement::BottomEnd,
            None,
            entries,
            close_menu,
        );
    }
    trigger
}

/// The segmented control that picks whose graph is drawn below.
fn leg_picker(
    c: &Ctx,
    active: LegKey,
    on_select: &(impl Fn(&mut App, LegKey) + Clone + 'static),
) -> El {
    let tk = c.tk;
    let key = format!("graph-route-legs{}", c.sfx);
    let mut seg = row()
        .key(key.clone())
        .gap(tk.r(0.125))
        .padding(tk.r(0.125))
        .radius(tk.r(0.5))
        .fill(tk.muted)
        .role("radiogroup")
        .label(S::graph_of())
        .focus_group();
    for (leg, label) in [
        (LegKey::Out, t("review:transfersPage.leg.sent")),
        (LegKey::In, t("review:transfersPage.leg.received")),
    ] {
        let on = leg == active;
        let f = on_select.clone();
        seg = seg.child(
            row()
                .key(format!(
                    "{key}-{}",
                    if leg == LegKey::Out { "out" } else { "in" }
                ))
                .h(tk.r(1.5))
                .px(tk.r(0.625))
                .center()
                .radius(tk.r(0.375))
                .fill(if on {
                    tk.background
                } else {
                    Color::TRANSPARENT
                })
                .hover(if on { tk.background } else { tk.row_hover })
                .role("radio")
                .checked(on)
                .label(label.clone())
                .focusable(true)
                .focus_selected(on)
                .on_press(move |a: &mut App| f(a, leg))
                .child(
                    text(label)
                        .font(tk.txs(500.0))
                        .color(if on { tk.fg } else { tk.muted_fg })
                        .single_line(),
                ),
        );
    }
    row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .child(
            text(S::graph_of())
                .font(tk.txs(500.0))
                .color(tk.muted_fg)
                .a11y_hidden(),
        )
        .child(seg)
}

fn header(c: &Ctx, glyph: &'static rune::Icon, title: String, trailing: Option<El>) -> El {
    let tk = c.tk;
    row()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.5))
        .center_y()
        .child(ic(glyph, tk.r(1.0), tk.sky.solid).a11y_hidden())
        .child(
            text(title.clone())
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .single_line()
                .truncate()
                .min_w(0.0)
                .grow(1.0)
                .help_when_truncated(title),
        )
        .children(trailing)
}

fn frame(c: &Ctx) -> El {
    let tk = c.tk;
    col()
        .key(format!("graph-route{}", c.sfx))
        .w_full()
        .min_w(0.0)
        .padding(tk.r(0.75))
        .gap(tk.r(0.625))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .fill(tk.muted.alpha(0.25))
}

fn muted_line(c: &Ctx, s: String) -> El {
    text(s)
        .font(c.tk.txs(400.0))
        .color(c.tk.muted_fg)
        .wrap()
        .w_full()
}

/// The card for a route that isn't an exchange deposit or withdrawal.
/// `width` is the card's width at the last layout (0 before the first).
pub fn card(
    c: &Ctx,
    record: &Value,
    r: &SwapRoute,
    leg: Option<LegKey>,
    on_select: impl Fn(&mut App, LegKey) + Clone + 'static,
    layer: &mut Vec<El>,
    width: f32,
) -> El {
    let tk = c.tk;
    let width = if width > 0.0 { width } else { tk.r(28.0) };
    let shape = case(r);
    let (from, to) = (wallet(&r.out_leg), wallet(&r.in_leg));
    let unlink = can_unlink(c.app, record, r);
    let trailing = |layer: &mut Vec<El>| unlink.then(|| menu(c, record, r, &from, &to, layer));
    // What left the sending wallet (the whole row), what the link carries of it, what arrived.
    let out_full = r
        .out_full_amount_btc
        .or(r.out_amount_btc)
        .or(r.out_leg.amount_btc);
    let out_linked = r.out_amount_btc.or(r.out_leg.amount_btc).or(out_full);
    let out_amount = out_full;
    let in_amount = r.in_leg.amount_btc;

    if shape == Case::SameTransaction {
        // One line: rail, From → To; then amount · network fee · network; then why.
        let route = row()
            .w_full()
            .min_w(0.0)
            .gap(tk.r(LEAD_GAP))
            .center_y()
            .label(format!("{from} → {to}"))
            .child(lead(c, &r.out_leg))
            .child(pair_names(
                &from,
                &to,
                tk.tsm(500.0),
                tk.fg,
                tk.r(0.875),
                tk.r(0.375),
                c.hide,
                width - 2.0 * tk.r(0.75) - tk.r(LEAD) - tk.r(LEAD_GAP) - 2.0,
            ));
        let mut facts: Vec<El> = Vec::new();
        if let Some(a) = in_amount.or(out_amount) {
            facts.push(
                sensitive(btc(a), tk.txs(500.0).features("'tnum' 1"), tk.fg, c.hide).single_line(),
            );
        }
        if let Some(f) = r.out_leg.fee_btc.filter(|f| *f > 0.0) {
            facts.push(
                sensitive(
                    format!("{} {}", t("transactions:details.networkFee"), btc(f)),
                    tk.txs(400.0).features("'tnum' 1"),
                    tk.muted_fg,
                    c.hide,
                )
                .single_line(),
            );
        }
        facts.push(
            text(network(&r.out_leg))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .single_line(),
        );
        let mut line = row()
            .w_full()
            .flex_wrap()
            .gaps(tk.r(0.375), tk.r(0.25))
            .center_y()
            .pl(tk.r(LEAD) + tk.r(LEAD_GAP));
        let n = facts.len();
        for (i, f) in facts.into_iter().enumerate() {
            line = line.child(f);
            if i + 1 < n {
                line = line.child(
                    text("·")
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .a11y_hidden(),
                );
            }
        }
        return frame(c)
            .child(header(
                c,
                &I::ARROW_RIGHT_LEFT,
                t("review:transfersPage.linked.how.sameTransaction"),
                trailing(layer),
            ))
            .child(route)
            .child(line)
            .child(muted_line(c, provenance(r)).pl(tk.r(LEAD) + tk.r(LEAD_GAP)));
    }

    // Two transactions: sent leg, what lies between, received leg.
    let title = match (shape, model::route_kind(r)) {
        (Case::Swap, _) => t("transactions:graph.swapRouteTitle"),
        (_, RouteKind::Coinjoin) => t("transactions:graph.coinjoinRouteTitle"),
        _ if owner_made(r) => t("transactions:graph.transferRouteTitle"),
        _ => t("review:transfersPage.details.title"),
    };
    let mut between: Vec<El> = Vec::new();
    let gap_line = |between: &mut Vec<El>| {
        // The dashed node: nothing in the book connects the two transactions.
        let mut node = row()
            .key(format!("graph-route-gap{}", c.sfx))
            .min_w(0.0)
            .px(tk.r(0.5))
            .py(tk.r(0.25))
            .gap(tk.r(0.375))
            .center_y()
            .radius(tk.r(0.375))
            .border(1.0, tk.muted_fg.alpha(0.6))
            .border_dash(4.0, 3.0)
            .help(S::not_recorded_help())
            .child(ic(&I::CIRCLE_DASHED, tk.r(0.875), tk.muted_fg).a11y_hidden())
            .child(
                text(S::not_recorded())
                    .font(tk.txs(500.0))
                    .color(tk.muted_fg)
                    .single_line()
                    .truncate()
                    .min_w(0.0),
            );
        if let Some(s) = seconds_between(r).filter(|s| *s > 0.0) {
            node = node.child(
                text(format!("· {}", S::later(s)))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .single_line(),
            );
        }
        between.push(row().w_full().min_w(0.0).child(node));
    };
    let mut chips = row()
        .w_full()
        .flex_wrap()
        .gaps(tk.r(0.375), tk.r(0.25))
        .center_y();
    match shape {
        Case::Swap => {
            let ty = move_type(r.kind.as_deref(), rail(&r.out_leg), rail(&r.in_leg));
            chips = chips.child(pill(c, t(&format!("review:transfersPage.type.{ty}"))));
            if let Some(f) = fee_toggle(c, r) {
                chips = chips.child(f);
            }
            if let Some(p) = policy_label(r) {
                chips = chips.child(pill(c, p));
            }
            if let Some(s) = seconds_between(r).filter(|s| *s > 0.0) {
                chips = chips.child(
                    text(S::later(s))
                        .font(tk.t2xs(400.0))
                        .color(tk.muted_fg)
                        .single_line(),
                );
            }
            between.push(chips);
        }
        _ => {
            gap_line(&mut between);
            // Never absorbed: a fee the link records, the part of the send it leaves unlinked,
            // and any other difference between what it carries and what arrived.
            let shown = |v: f64, sign: bool| {
                if c.hide {
                    t("transactions:graph.hidden")
                } else {
                    format::btc(v, 8, sign)
                }
            };
            if let Some(f) = r.swap_fee_btc.filter(|f| *f != 0.0) {
                chips = chips.child(pill(
                    c,
                    format!(
                        "{}: {}",
                        t("review:transfersPage.unlink.feeTitle"),
                        shown(f, false)
                    ),
                ));
            }
            if let (Some(full), Some(linked)) = (out_full, out_linked) {
                let left = full - linked;
                if left > 0.5e-8 {
                    chips = chips.child(warn_pill(
                        c,
                        tr(
                            "review:transfersPage.linked.rest",
                            &[("left", &shown(left, false))],
                        ),
                    ));
                }
            }
            if let (Some(linked), Some(i)) = (out_linked, in_amount) {
                let diff = linked - i - r.swap_fee_btc.unwrap_or(0.0);
                if diff.abs() > 0.5e-8 {
                    chips = chips.child(pill(
                        c,
                        format!(
                            "{}: {}",
                            t("review:transfersPage.link.differenceTitle"),
                            shown(diff, diff < 0.0)
                        ),
                    ));
                }
            }
            if let Some(p) = policy_label(r) {
                chips = chips.child(pill(c, p));
            }
            between.push(chips);
        }
    }
    let mut mid = col().w_full().min_w(0.0).gap(tk.r(0.375));
    for b in between {
        mid = mid.child(b);
    }
    let mut card = frame(c)
        .child(header(
            c,
            if shape == Case::Swap {
                &I::ARROW_RIGHT_LEFT
            } else {
                &I::LINK_2
            },
            title,
            trailing(layer),
        ))
        .child(
            col()
                .w_full()
                .min_w(0.0)
                .child(leg_row(
                    c,
                    &format!("graph-route-out{}", c.sfx),
                    t("review:transfersPage.leg.sent"),
                    &r.out_leg,
                    out_amount,
                    width,
                ))
                .child(connector(c, mid))
                .child(leg_row(
                    c,
                    &format!("graph-route-in{}", c.sfx),
                    t("review:transfersPage.leg.received"),
                    &r.in_leg,
                    in_amount,
                    width,
                )),
        );
    let mut why = col()
        .w_full()
        .gap(tk.r(0.125))
        .child(muted_line(c, provenance(r)));
    if owner_made(r) {
        why = why.child(muted_line(c, S::owner_caveat()));
    }
    card = card.child(why);
    // Both legs are transactions of their own: pick whose graph is drawn.
    let selectable = |k: LegKey| !r.leg(k).reference().is_empty();
    if selectable(LegKey::Out) && selectable(LegKey::In) {
        let active = leg.unwrap_or(match r.current_leg.as_deref() {
            Some("in") => LegKey::In,
            _ => LegKey::Out,
        });
        card = card.child(leg_picker(c, active, &on_select));
    }
    card
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(v: serde_json::Value) -> SwapRoute {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn cases_follow_the_data() {
        let tx = "0ab5a0664d93e97599cd8f189a5d6e1db0738bd027d2d334247a3b6b61349b92";
        let same = route(
            serde_json::json!({"routeKind": "transfer", "policy": "carrying-value", "out": {"txid": tx, "asset": "LBTC"}, "in": {"txid": tx.to_uppercase(), "asset": "LBTC"}}),
        );
        assert_eq!(case(&same), Case::SameTransaction);
        let swap = route(
            serde_json::json!({"routeKind": "swap", "kind": "peg-out", "out": {"txid": "a", "asset": "LBTC"}, "in": {"txid": "b", "asset": "BTC"}}),
        );
        assert_eq!(case(&swap), Case::Swap);
        let linked = route(
            serde_json::json!({"routeKind": "transfer", "pairSource": "manual", "out": {"txid": "a", "asset": "BTC"}, "in": {"txid": "b", "asset": "BTC"}}),
        );
        assert_eq!(case(&linked), Case::Linked);
        assert!(owner_made(&linked));
        // A missing txid on one side is not proof of one transaction.
        let unknown = route(
            serde_json::json!({"routeKind": "transfer", "out": {"asset": "BTC"}, "in": {"txid": "b", "asset": "BTC"}}),
        );
        assert_eq!(case(&unknown), Case::Linked);
        assert!(!owner_made(&same));
    }
}
