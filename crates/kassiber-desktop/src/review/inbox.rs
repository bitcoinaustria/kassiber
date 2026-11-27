//! The Review screen (`routes/Review.tsx`): everything that waits on the owner, in one place —
//! moves between wallets to confirm (`ReviewTransfers.tsx`), then what cannot be booked yet.

use rune::{Color, Cx, El, col, el, row, text};
use serde_json::Value;

use super::kit::{self, Var};
use super::model::{self, s};
use super::state::{self, Review};
use super::{Popup, cards, dialogs, toggle};
use crate::app::App;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::popup::TriggerExt;
use crate::ui::{ic, sensitive, surface};

/// Candidates listed before the rest fold away.
const SHOWN: usize = 6;

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let rv = &app.review.rv;
    let updating = Review::refreshing(app);
    let failed_run = rv.q.attention().and_then(|a| a.last_error()).is_some();
    let clear = !updating
        && !failed_run
        && !rv.commercial_error
        && rv.commercial.as_ref().is_some_and(Vec::is_empty)
        && app.badges.as_ref().is_some_and(|b| {
            b.quarantine_attention.or(b.quarantine).unwrap_or(0.0) == 0.0
                && !b.journals_needs_processing
        })
        && rv.tr.live.as_ref().is_some_and(Vec::is_empty);
    let mut page = col()
        .key("review")
        .w_full()
        .padding(tk.page_pad())
        .gap(tk.section_gap())
        .fill(tk.background);
    if updating {
        page = page.child(
            row()
                .key("rv-updating")
                .gap(tk.r(0.5))
                .center_y()
                .role("status")
                .child(kit::spinner(tk, tk.r(0.875)))
                .child(
                    text(t("review:inbox.updating"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                ),
        );
    }
    if clear {
        // An owner request over the web's edge-to-edge card, which was easy to miss: the empty
        // state sits in the middle of the free space, centred, without a card around it.
        let win_h = if app.shell.window_h > 0.0 {
            app.shell.window_h
        } else {
            900.0
        };
        let free = (win_h - crate::shell::window_top_inset() - 2.0 * tk.page_pad() - tk.r(4.0))
            .max(tk.r(16.0));
        let ok = kit::tw(if tk.dark {
            "emerald-400"
        } else {
            "emerald-600"
        });
        page = page.child(
            col()
                .key("review-clear")
                .w_full()
                .min_h(free)
                .center()
                .role("status")
                .child(
                    col()
                        .w_full()
                        .max_w(tk.r(32.0))
                        .items_center()
                        .gap(tk.r(0.5))
                        .child(
                            row()
                                .size(tk.r(3.0), tk.r(3.0))
                                .radius(tk.r(1.5))
                                .center()
                                .fill(ok.alpha(0.12))
                                .mb(tk.r(0.5))
                                .child(ic(&I::CIRCLE_CHECK, tk.r(1.5), ok)),
                        )
                        .child(
                            text(t("review:inbox.clear.title"))
                                .font(tk.tbase(600.0))
                                .color(tk.fg)
                                .align(rune::Align::Center)
                                .role("heading")
                                .level(2),
                        )
                        .child(
                            text(t("review:inbox.clear.body"))
                                .font(tk.tsm(400.0))
                                .color(tk.muted_fg)
                                .wrap()
                                .align(rune::Align::Center),
                        ),
                ),
        );
        dialogs::mount(app, tk, layer);
        if app.ov.add_open {
            crate::overview::add_connection::dialog(app, cx, tk, layer);
        }
        return page;
    }
    // Quick yes-or-no decisions first, wider apart than the rows inside them (`space-y-6`).
    // Keyed, so the "Updating" line above it does not remount it (focus stays in the list).
    let mut sections = col().key("rv-sections").w_full().gap(tk.r(1.5));
    if let Some(tr) = transfers(app, cx, tk, layer) {
        sections = sections.child(tr);
    }
    if let Some(invoices) = invoice_matches(app, tk) {
        sections = sections.child(invoices);
    }
    sections = sections.child(cards::section(app, cx, tk, layer));
    page = page.child(sections);
    dialogs::mount(app, tk, layer);
    if app.ov.add_open {
        crate::overview::add_connection::dialog(app, cx, tk, layer);
    }
    page
}

fn invoice_matches(app: &App, tk: &Tk) -> Option<El> {
    let review = &app.review.rv;
    let links = review.commercial.as_deref().unwrap_or_default();
    if links.is_empty() && !review.commercial_error {
        return None;
    }
    let mut section = col().w_full().gap(tk.r(0.5)).child(
        text(t("review:inbox.invoices.title"))
            .font(tk.tsm(600.0))
            .color(tk.fg)
            .role("heading")
            .level(2),
    );
    if review.commercial_error {
        section = section.child(text(t("review:inbox.invoices.unavailable")).color(tk.muted_fg));
    }
    for link in links {
        let tx = s(link, "transaction_id").to_owned();
        section = section.child(
            surface(tk)
                .key(format!("rv-invoice-{}", s(link, "id")))
                .w_full()
                .padding(tk.card_pad())
                .gap(tk.r(0.25))
                .child(sensitive(
                    s(link, "invoice_id").to_owned(),
                    tk.tsm(500.0),
                    tk.fg,
                    app.prefs.hide_sensitive(),
                ))
                .child(
                    text(t(super::invoice_match_reason(&s(link, "match_status"))))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                )
                .child(
                    kit::link(
                        tk,
                        format!("rv-invoice-open-{tx}"),
                        &t("review:inbox.invoices.open"),
                        true,
                    )
                    .on_press(move |a: &mut App| crate::txdetail::open(a, &tx, "details")),
                ),
        );
    }
    Some(section)
}
fn info_toggle(a: &mut App, key: &str) {
    toggle(a, Popup::Info(key.to_owned()));
}

/// `ReviewTransfers`.
fn transfers(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> Option<El> {
    let trs = &app.review.rv.tr;
    let title = t("review:inbox.transfers.title");
    if trs.live.is_none() && trs.loading {
        // Holds the place, so the causes below do not jump down when it loads.
        return Some(
            col()
                .key("rv-transfers")
                .w_full()
                .gap(tk.r(0.75))
                .busy(true)
                .label(title.clone())
                .child(
                    text(title)
                        .font(tk.tsm(600.0))
                        .color(tk.fg)
                        .role("heading")
                        .level(2),
                )
                .child(surface(tk).w_full().h(tk.r(5.0))),
        );
    }
    if trs.failed {
        return Some(
            surface(tk)
                .key("rv-transfers")
                .w_full()
                .padding(tk.card_pad())
                .role("alert")
                .child(
                    text(title)
                        .font(tk.tsm(600.0))
                        .color(tk.fg)
                        .role("heading")
                        .level(2),
                )
                .child(
                    text(t("review:inbox.transfers.unavailable"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .mt(tk.r(0.25)),
                ),
        );
    }
    let cands = trs.candidates();
    if cands.is_empty() && trs.undo.is_none() && trs.error.is_none() {
        return None;
    }
    let hidden = app.prefs.hide_sensitive();
    let exact = model::bulk_pairable(&cands);
    let refreshing = Review::refreshing(app);
    let bulk_busy = app.review.rv.bulk.as_ref().is_some_and(|b| b.busy);
    let mut heading = row()
        .gap(tk.r(0.5))
        .items_end()
        .flex_wrap()
        .role("heading")
        .level(2)
        .label(title.clone())
        .child(text(title.clone()).font(tk.tsm(600.0)).color(tk.fg));
    if !cands.is_empty() {
        heading = heading.child(
            text(tc("review:inbox.transfers.count", cands.len(), &[]))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        );
    }
    let ik = "rv-transfers-about";
    let left = row()
        .min_w(0.0)
        .gap(tk.r(0.5))
        .center_y()
        .child(heading)
        .child(kit::info_note(
            tk,
            ik,
            t("review:inbox.transfers.aboutLabel"),
            &[
                t("review:inbox.transfers.why"),
                t("review:inbox.transfers.policyNote"),
            ],
            app.review.popup == Some(Popup::Info(ik.into())),
            layer,
            info_toggle,
        ));
    let mut right = row().gap(tk.r(0.5)).center_y().flex_wrap();
    // Several ticked suggestions are set aside in one step.
    let chosen = state::chosen(trs).len();
    if chosen > 0 {
        let enabled = trs.saving == 0 && !bulk_busy;
        let mut b = kit::btn(
            tk,
            "rv-dismiss-selected",
            Var::Outline,
            None,
            &tc("review:inbox.transfers.dismissSelected", chosen, &[]),
            enabled,
        );
        if enabled {
            b = b.on_press(state::dismiss_selected);
        }
        right = right.child(b);
    }
    // While the journals recalculate the daemon could link more than the list shows, so the bulk
    // step waits for the fresh list.
    if !exact.is_empty() {
        let enabled = !refreshing && trs.saving == 0 && !bulk_busy;
        let mut b = kit::btn(
            tk,
            "rv-pair-exact",
            Var::Default,
            None,
            &tc("review:inbox.transfers.pairExact", exact.len(), &[]),
            enabled,
        );
        if enabled {
            b = b.on_press(state::prepare_bulk);
        }
        right = right.child(b);
    }
    // "Link manually" on Transfers, on a new link.
    let manual = t("review:inbox.transfers.linkManually");
    right = right.child(
        row()
            .key("rv-link-manually")
            .h(tk.r(2.0))
            .px(tk.r(0.75))
            .gap(tk.r(0.375))
            .center_y()
            .radius(tk.r(0.375))
            .hover(if tk.dark {
                tk.accent.alpha(0.5)
            } else {
                tk.accent
            })
            .role("link")
            .label(manual.clone())
            .focusable(true)
            .on_press(|a: &mut App| a.navigate("/transfers?link=new"))
            .child(ic(&I::LINK_2, tk.r(0.875), tk.fg))
            .child(text(manual).font(tk.tsm(500.0)).color(tk.fg)),
    );
    let mut sec = col()
        .key("rv-transfers")
        .w_full()
        .gap(tk.r(0.75))
        .role("region")
        .label(title)
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .center_y()
                .justify_between()
                .child(left)
                .child(right),
        );
    // Both live regions stay mounted, so a screen reader announces what appears in them.
    let mut status = row()
        .gap(tk.r(0.5))
        .center_y()
        .flex_wrap()
        .role("status")
        .live_region(rune::LiveRegion::Polite);
    if trs.saving > 0 {
        status = status.child(
            row()
                .gap(tk.r(0.375))
                .center_y()
                .child(kit::spinner(tk, tk.r(0.75)))
                .child(
                    text(t("review:inbox.transfers.saving"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                ),
        );
    } else if let Some(u) = &trs.undo {
        status = status
            .child(sensitive(
                u.label.clone(),
                tk.txs(400.0),
                tk.muted_fg,
                hidden,
            ))
            .child(
                kit::link(tk, "rv-undo", &t("review:inbox.transfers.undo"), true)
                    .on_press(state::run_undo),
            );
    }
    // `empty:hidden`: an empty live region takes no room (nor the gap above it).
    if trs.saving == 0 && trs.undo.is_none() {
        status = status.h(0.0).mt(-tk.r(0.75));
    }
    sec = sec.child(status);
    if let Some(e) = &trs.error {
        sec = sec.child(
            text(e.clone())
                .font(tk.txs(400.0))
                .color(tk.destructive)
                .role("alert")
                .wrap(),
        );
    }
    if !cands.is_empty() {
        let width = cx.layout_size("rv-moves").map(|[w, _]| w).unwrap_or(1100.0);
        let wide = width >= tk.r(60.0);
        let shown: Vec<&Value> = if trs.expanded {
            cands.iter().collect()
        } else {
            cands.iter().take(SHOWN).collect()
        };
        let mut ul = surface(tk)
            .key("rv-moves")
            .w_full()
            .role("list")
            .clip_rounded(tk.r(0.75));
        for (i, c) in shown.iter().enumerate() {
            let mut r = candidate_row(app, cx, tk, c, wide, hidden, layer);
            if i > 0 {
                r = r.border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
            }
            ul = ul.child(r);
        }
        sec = sec.child(ul);
    }
    if cands.len() > SHOWN {
        let label = if trs.expanded {
            t("review:inbox.transfers.showFewer")
        } else {
            tc("review:inbox.transfers.showMore", cands.len() - SHOWN, &[])
        };
        sec = sec.child(
            kit::btn(tk, "rv-moves-more", Var::Ghost, None, &label, true)
                .expanded(trs.expanded)
                .on_press(|a: &mut App| a.review.rv.tr.expanded = !a.review.rv.tr.expanded),
        );
    }
    Some(sec)
}
fn chip(
    tk: &Tk,
    label: String,
    glyph: Option<&'static rune::Icon>,
    fg: Color,
    bg: Color,
    border: Color,
) -> El {
    let _ = (bg, border);
    let mut r = row().gap(tk.r(0.25)).center_y();
    if let Some(g) = glyph {
        r = r.child(ic(g, tk.r(0.75), fg));
    }
    r.child(text(label).font(tk.t2xs(500.0).lh(tk.r(1.0))).color(fg))
}

/// Tailwind tints for the chips: (text, fill, border).
fn tint(tk: &Tk, name: &str) -> (Color, Color, Color) {
    let h = |x: &str| rune::color::hex(x).unwrap();
    match (name, tk.dark) {
        ("sky", false) => (h("#075985"), h("#f0f9ff"), h("#bae6fd")),
        ("sky", true) => (
            h("#e0f2fe"),
            h("#082f49").alpha(0.4),
            h("#38bdf8").alpha(0.3),
        ),
        ("amber", false) => (h("#92400e"), h("#fffbeb"), h("#fde68a")),
        ("amber", true) => (
            h("#fef3c7"),
            h("#451a03").alpha(0.4),
            h("#fbbf24").alpha(0.3),
        ),
        ("emerald", false) => (h("#065f46"), h("#ecfdf5"), h("#a7f3d0")),
        ("emerald", true) => (
            h("#d1fae5"),
            h("#022c22").alpha(0.4),
            h("#34d399").alpha(0.3),
        ),
        (_, false) => (h("#991b1b"), h("#fef2f2"), h("#fecaca")),
        (_, true) => (
            h("#fee2e2"),
            h("#450a0a").alpha(0.4),
            h("#f87171").alpha(0.3),
        ),
    }
}

/// The first button of a row (`button:not([role="checkbox"])`): Link, or Review….
pub fn primary_key(c: &Value) -> String {
    let key = model::candidate_key(c);
    if model::needs_review(c) {
        format!("rv-review-{key}")
    } else {
        format!("rv-pair-{key}")
    }
}

/// `DropdownMenuItem` (`px-2 py-1.5 text-sm`, a muted `size-4` glyph).
fn menu_item(tk: &Tk, key: String, glyph: &'static rune::Icon, label: String, name: String) -> El {
    row()
        .key(key)
        .w_full()
        .h(tk.r(0.375) * 2.0 + tk.r(1.25))
        .px(tk.r(0.5))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.25))
        .hover(tk.accent)
        .focus_highlight(tk.accent)
        .role("menuitem")
        .label(name)
        .focusable(true)
        .child(ic(glyph, tk.r(1.0), tk.muted_fg))
        .child(text(label).font(tk.tsm(400.0)).color(tk.fg))
}

/// The row's "More" menu: set aside, another receipt on Transfers, and both sides on
/// Transactions (`TransactionsMenuItems`).
fn move_menu(tk: &Tk, layer: &mut Vec<El>, anchor: &str, c: &Value, route: &str) {
    let key = model::candidate_key(c);
    let (out_id, in_id) = (s(c, "out_id"), s(c, "in_id"));
    let c1 = c.clone();
    let close = |a: &mut App| a.review.popup = None;
    let (o1, o2, i1) = (out_id.clone(), out_id.clone(), in_id.clone());
    let mut p = crate::ui::popup::Popup::menu(anchor, close);
    let (k, l) = (
        format!("rv-menu-dismiss-{key}"),
        t("review:inbox.transfers.dismiss"),
    );
    p.item(
        menu_item(
            tk,
            k.clone(),
            &I::X,
            l.clone(),
            tr("review:inbox.transfers.dismissLabel", &[("route", route)]),
        )
        .on_press(move |a: &mut App| {
            a.review.popup = None;
            state::decide(a, &c1, false);
        }),
        k,
        l,
        true,
        false,
    );
    let (k, l) = (
        format!("rv-menu-relink-{key}"),
        t("review:inbox.transfers.linkDifferently"),
    );
    p.item(
        menu_item(
            tk,
            k.clone(),
            &I::LINK_2,
            l.clone(),
            tr(
                "review:inbox.transfers.linkDifferentlyLabel",
                &[("route", route)],
            ),
        )
        .role("menuitem")
        .on_press(move |a: &mut App| {
            a.review.popup = None;
            a.navigate(&format!("/transfers?link={o1}"));
        }),
        k,
        l,
        true,
        false,
    );
    p.add(el().h(1.0).w_full().my(tk.r(0.25)).fill(tk.border));
    let (k, l) = (
        format!("rv-menu-sent-{key}"),
        t("review:transfersPage.rowMenu.showSent"),
    );
    p.item(
        menu_item(tk, k.clone(), &I::ARROW_UP_RIGHT, l.clone(), l.clone()).on_press(
            move |a: &mut App| {
                a.review.popup = None;
                a.navigate(&format!("/transactions?tx={o2}"));
            },
        ),
        k,
        l,
        true,
        false,
    );
    let (k, l) = (
        format!("rv-menu-received-{key}"),
        t("review:transfersPage.rowMenu.showReceived"),
    );
    p.item(
        menu_item(tk, k.clone(), &I::ARROW_UP_RIGHT, l.clone(), l.clone()).on_press(
            move |a: &mut App| {
                a.review.popup = None;
                a.navigate(&format!("/transactions?tx={i1}"));
            },
        ),
        k,
        l,
        true,
        false,
    );
    // `DropdownMenuContent`: the glass popover (`rounded-md border p-1`) with its drop shadow.
    p.mount(
        tk,
        layer,
        col().min_w(tk.r(8.0)).padding(tk.r(0.25)),
        |l| {
            l.radius(tk.r(0.375))
                .fill(tk.popover)
                .border(1.0, tk.fg.alpha(0.10))
                .box_shadow(rune::Shadow {
                    y: 16.0,
                    blur: 40.0,
                    spread: -18.0,
                    color: Color::BLACK.alpha(if tk.dark { 0.8 } else { 0.55 }),
                    ..rune::Shadow::default()
                })
        },
        rune_kit::Placement::BottomEnd,
        4.0,
    );
}

/// `CandidateRow`: tick and date, route, amount, what linking records and why it is suggested,
/// then Link (or Review… for anything less than an exact, uncontested match) and the menu.
fn candidate_row(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    c: &Value,
    wide: bool,
    hidden: bool,
    layer: &mut Vec<El>,
) -> El {
    let key = model::candidate_key(c);
    let route = tr(
        "review:inbox.transfers.route",
        &[
            ("from", &s(c, "out_wallet_label")),
            ("to", &s(c, "in_wallet_label")),
        ],
    );
    let ty = model::move_type(c);
    // Only a known treatment gets a chip; an unknown one is never shown as tax-neutral.
    let (_, terms_policy) = model::candidate_terms(c);
    let policy = match terms_policy.as_str() {
        "carrying-value" => Some(true),
        "taxable" => Some(false),
        _ => None,
    };
    let review = model::needs_review(c);
    let evidence = if s(c, "method") == "ownership_graph" {
        "ownership"
    } else if s(c, "confidence") == "exact" {
        "exact"
    } else {
        "likely"
    };
    let (oa, ia) = (s(c, "out_asset"), s(c, "in_asset"));
    let same = model::is_bitcoin(&oa) == model::is_bitcoin(&ia)
        && (model::is_bitcoin(&oa) || oa.to_uppercase() == ia.to_uppercase());
    let fee = model::num(c, "swap_fee_msat");
    let show_received = !same || fee <= 0.0;
    let date = sensitive(
        crate::format::short_date(c["out_occurred_at"].as_str()),
        if wide { tk.txs(400.0) } else { tk.t2xs(400.0) }.features("'tnum' 1"),
        tk.muted_fg,
        hidden,
    );
    let k0 = key.clone();
    let tick = kit::checkbox(
        tk,
        format!("rv-select-{key}"),
        app.review.rv.tr.selected.contains(&key),
        true,
        tr("review:inbox.transfers.selectLabel", &[("route", &route)]),
    )
    .on_press(move |a: &mut App| state::toggle_selected(a, &k0));
    let date = row().gap(tk.r(0.625)).center_y().child(tick).child(date);
    // From over to (the table kit's stacked route), fitted to the cell's last laid-out width.
    let route_key = format!("rv-route-{key}");
    let route_w = cx
        .layout_size(&route_key)
        .map(|[w, _]| w)
        .filter(|w| *w > 0.0)
        .unwrap_or(tk.r(16.0));
    let (out_kind, in_kind) = (s(c, "out_wallet_kind"), s(c, "in_wallet_kind"));
    let route_el =
        row()
            .key(route_key)
            .min_w(0.0)
            .grow(1.0)
            .w(0.0)
            .child(crate::custody::kit::route_stack(
                app,
                cx,
                tk,
                &s(c, "out_wallet_label"),
                &s(c, "in_wallet_label"),
                crate::custody::model::rail_for_leg(Some(&oa), Some(&out_kind)),
                crate::custody::model::rail_for_leg(Some(&ia), Some(&in_kind)),
                false,
                route_w,
            ));
    use crate::ui::table as tb;
    let mut amount = col().items_end().child(tb::num(
        tk,
        model::leg_amount(model::num(c, "out_amount_msat"), &oa),
        tk.fg,
        hidden,
    ));
    amount = if show_received {
        amount.child(
            row()
                .gap(tk.r(0.25))
                .child(
                    text("→")
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .a11y_hidden(),
                )
                .child(tb::num_sub(
                    tk,
                    model::leg_amount(model::num(c, "in_amount_msat"), &ia),
                    hidden,
                )),
        )
    } else {
        let f = if model::is_bitcoin(&oa) {
            model::msat(fee)
        } else {
            model::leg_amount(fee, &oa)
        };
        amount.child(tb::num_sub(
            tk,
            tr("review:inbox.transfers.fee", &[("amount", &f)]),
            hidden,
        ))
    };
    let amount = amount.min_w(tk.r(9.0));
    // `flex-wrap gap-x-2 gap-y-0.5`.
    let mut kind = row()
        .min_w(0.0)
        .gaps(tk.r(0.125), tk.r(0.5))
        .center_y()
        .flex_wrap();
    let glyph: &'static rune::Icon = match ty {
        "transfer" => &I::ARROW_LEFT_RIGHT,
        "bitcoinSwap" => &I::REPEAT,
        _ => &I::SHUFFLE,
    };
    if ty == "transfer" {
        kind = kind.child(
            row()
                .gap(tk.r(0.25))
                .center_y()
                .child(ic(glyph, tk.r(0.75), tk.muted_fg))
                .child(
                    text(t("review:inbox.transfers.type.transfer"))
                        .font(tk.t2xs(400.0))
                        .color(tk.muted_fg),
                ),
        );
    } else {
        let (fg, bg, b) = tint(tk, if ty == "bitcoinSwap" { "sky" } else { "amber" });
        kind = kind.child(chip(
            tk,
            t(&format!("review:inbox.transfers.type.{ty}")),
            Some(glyph),
            fg,
            bg,
            b,
        ));
        if let Some(carries) = policy {
            let (fg, bg, b) = tint(tk, if carries { "emerald" } else { "red" });
            let (label, hint) = if carries {
                (
                    t("review:inbox.transfers.policy.carrying"),
                    t("review:inbox.transfers.policy.carryingHint"),
                )
            } else {
                (
                    t("review:inbox.transfers.policy.taxable"),
                    t("review:inbox.transfers.policy.taxableHint"),
                )
            };
            kind = kind.child(chip(tk, label, None, fg, bg, b).help(hint));
        }
    }
    let dot = match evidence {
        "likely" => kit::tw("amber-500"),
        "exact" => kit::tw("emerald-500"),
        _ => kit::tw("sky-500"),
    };
    kind = kind.child(
        row()
            .gap(tk.r(0.25))
            .center_y()
            .child(crate::ui::dot(tk.r(0.375), dot))
            .child(
                text(t(&format!(
                    "review:inbox.transfers.evidenceShort.{evidence}"
                )))
                .font(tk.t2xs(400.0))
                .color(tk.muted_fg),
            ),
    );
    kind = kind.child(
        text(model::suggestion_reason(c))
            .font(tk.t2xs(400.0))
            .color(tk.muted_fg)
            .wrap()
            .max_w(tk.r(10.0)),
    );
    if c["rule_match"].is_object() {
        let name = model::opt(&c["rule_match"], "rule_name")
            .unwrap_or_else(|| t("review:swap.rules.unnamed"));
        kind = kind.child(
            row()
                .px(tk.r(0.375))
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .child(
                    text(tr("review:swap.table.ruleBadge", &[("name", &name)]))
                        .font(tk.t2xs(500.0).lh(tk.r(1.0)))
                        .color(tk.fg),
                ),
        );
    }
    let conflict = model::num(c, "conflict_size") as usize;
    if conflict > 1 {
        kind = kind.child(
            text(tc("review:inbox.transfers.competing", conflict - 1, &[]))
                .font(tk.t2xs(400.0))
                .color(kit::amber_text(tk)),
        );
    }
    // An exact match without competitors links in one click; anything less opens the receipt
    // picker on Transfers, where the alternatives are the choice.
    let first = if review {
        let href = format!("/transfers?link={}", s(c, "out_id"));
        crate::ui::button::button(
            tk,
            primary_key(c),
            Var::Outline,
            crate::ui::button::Size::Xs,
            None,
            &t("review:inbox.transfers.review"),
            true,
        )
        .role("link")
        .label(tr(
            "review:inbox.transfers.reviewLabel",
            &[("route", &route)],
        ))
        .on_press(move |a: &mut App| a.navigate(&href))
    } else {
        let c1 = c.clone();
        crate::ui::button::button(
            tk,
            primary_key(c),
            Var::Default,
            crate::ui::button::Size::Xs,
            Some(&I::CHECK),
            &t("review:inbox.transfers.pair"),
            true,
        )
        .label(tr("review:inbox.transfers.pairLabel", &[("route", &route)]))
        .on_press(move |a: &mut App| state::decide(a, &c1, true))
    };
    let anchor = format!("rv-more-{key}");
    let open = app.review.popup == Some(Popup::MoveMenu(key.clone()));
    let k2 = key.clone();
    let more = crate::txdetail::widgets::icon_button(
        tk,
        anchor.clone(),
        Var::Ghost,
        tk.r(1.75),
        &I::ELLIPSIS,
        tk.r(0.875),
        tr("review:inbox.transfers.moreLabel", &[("route", &route)]),
    )
    .menu_trigger(open, move |a: &mut App| {
        toggle(a, Popup::MoveMenu(k2.clone()))
    });
    if open {
        move_menu(tk, layer, &anchor, c, &route);
    }
    let buttons = row()
        .gap(tk.r(0.125))
        .center_y()
        .justify_end()
        .child(first)
        .child(more);
    if wide {
        row()
            .key(format!("rv-move-{key}"))
            .w_full()
            .px(tk.r(0.75))
            .py(tk.r(tb::PY))
            .hover(tk.muted.alpha(0.35))
            .gap(tk.r(1.0))
            .center_y()
            .role("listitem")
            .child(date)
            .child(route_el)
            .child(amount)
            .child(kind.w(tk.r(10.0)))
            .child(buttons)
    } else {
        col()
            .key(format!("rv-move-{key}"))
            .w_full()
            .px(tk.r(0.75))
            .py(tk.r(tb::PY))
            .hover(tk.muted.alpha(0.35))
            .gap(tk.r(0.25))
            .role("listitem")
            .child(
                row()
                    .w_full()
                    .gap(tk.r(0.75))
                    .center_y()
                    .child(route_el)
                    .child(amount),
            )
            .child(
                row()
                    .w_full()
                    .gap(tk.r(0.75))
                    .center_y()
                    .child(date)
                    .child(kind.grow(1.0))
                    .child(buttons),
            )
    }
}
