//! The table card (TransactionsTable.tsx): toolbar with the active-filter chips, Filters menu
//! and Expand; the sortable header with its column filters; virtual rows; the "⋯" row menu;
//! the footer; the exclude confirmation.

use std::borrow::Cow;

use super::menu::Entry;
use rune::{Color, Cx, El, GrowthSpec, ScrollSpec, Tx, col, el, row, text};
use rune_kit::Placement;
use serde_json::json;

use super::model::*;
use super::style::{Chip, amount_color, coarse_chip, flow_chip, pricing_chip, status_chip, tw};
use super::{Popup, changed, fiat_mode, load_more, open_detail};
use crate::app::{App, Currency};
use crate::format;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::popup::TriggerExt;
use crate::ui::{ic, sensitive, surface};

const W_AMOUNT: f32 = 150.0;
const W_ACCOUNTING: f32 = 190.0;
const W_PRICING: f32 = 180.0;
const W_NETWORK: f32 = 150.0;
const W_STATUS: f32 = 130.0;
const W_ACTIONS: f32 = 48.0;
/// `min-w-[1080px]`: the table's least width; narrower cards scroll it sideways.
const W_TABLE: f32 = 1080.0;
/// A row with a pricing summary: p-2, a 1.5 rem chip, 0.25 rem, two 1 rem lines, p-2, the border.
const ROW_EST: f32 = 69.3;
/// `MISSING_FIAT_LABEL`: the web hard-codes it (lib/currency.ts), so it stays untranslated here too.
const UNPRICED: &str = "Unpriced";

/// Which optional columns the viewport shows (`md:` / `lg:` / `xl:` on the web).
#[derive(Clone, Copy)]
struct Cols {
    accounting: bool,
    pricing: bool,
    network: bool,
    /// The Transaction column's `w-[34%]` while the table sits at its least width (scrolling
    /// sideways) with room to spare; otherwise it takes what the fixed columns leave.
    lead: Option<f32>,
    /// The Transaction column's inner text width, for a route row's "From → To" line.
    route_w: f32,
}

impl Cols {
    /// The fixed columns' widths together.
    fn fixed(self) -> f32 {
        W_AMOUNT
            + W_STATUS
            + W_ACTIONS
            + [
                (self.accounting, W_ACCOUNTING),
                (self.pricing, W_PRICING),
                (self.network, W_NETWORK),
            ]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, w)| w)
            .sum::<f32>()
    }
}

fn cols(app: &App, cx: &Cx<App>) -> Cols {
    let w = cols_w(app, cx);
    Cols {
        accounting: w >= 768.0,
        pricing: w >= 1024.0,
        network: w >= 1280.0,
        lead: None,
        route_w: 0.0,
    }
}

/// The window's width, which the web's `md:`/`lg:`/`xl:` breakpoints read.
fn cols_w(app: &App, cx: &Cx<App>) -> f32 {
    cx.layout_size("sh-root")
        .map(|[w, _]| w)
        .filter(|w| *w > 0.0)
        .unwrap_or(if app.shell.window_w > 0.0 {
            app.shell.window_w
        } else {
            1440.0
        })
}

/// A ringed chip: `inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium`
/// with a 1 pt inset ring (status and pricing styles).
fn ring_chip(tk: &Tk, c: Chip, glyph: Option<&'static rune::Icon>, label: String) -> El {
    let mut r = row()
        .px(tk.r(0.5))
        .py(tk.r(0.25))
        .gap(tk.r(0.25))
        .center_y()
        .radius(tk.r(0.375))
        .fill(c.bg)
        .border(1.0, c.ring)
        .min_w(0.0);
    if let Some(g) = glyph {
        r = r.child(ic(g, tk.r(0.75), c.fg));
    }
    r.child(text(label).font(tk.txs(500.0)).color(c.fg).truncate())
}

/// shadcn `Badge` with `rounded-md`: `border px-2 py-0.5 text-xs font-medium`; `secondary` is
/// `bg-secondary` with a transparent border, `outline` is `border-border text-foreground`.
fn badge(tk: &Tk, label: String, outline: bool, hidden: bool) -> El {
    // Rune draws borders inside the box; the web's 1 pt border adds to it, hence the +1.
    let r = row()
        .px(tk.r(0.5) + 1.0)
        .py(tk.r(0.125) + 1.0)
        .radius(tk.r(0.375))
        .center_y()
        .min_w(0.0);
    let r = if outline {
        r.border(1.0, tk.border)
    } else {
        r.fill(tk.accent)
    };
    r.child(sensitive(label, tk.txs(500.0), tk.fg, hidden).truncate())
}

/// The loan badge: an outline `Badge` with `loanRoleBadgeClassName`'s border and text.
fn loan_badge(tk: &Tk, role: &str) -> El {
    let (key, fam) = match role {
        "collateral_release" => ("returnedBadge", "emerald"),
        "loan_principal_received" => ("principalReceivedBadge", "sky"),
        "loan_principal_repaid" => ("principalRepaidBadge", "sky"),
        _ => ("collateralBadge", "amber"),
    };
    let fg = tw(fam, if tk.dark { 400 } else { 700 });
    row()
        .px(tk.r(0.5) + 1.0)
        .py(tk.r(0.125) + 1.0)
        .gap(tk.r(0.25))
        .center_y()
        .radius(tk.r(0.375))
        .border(1.0, tw(fam, 500).alpha(0.4))
        .child(ic(&I::COINS, tk.r(0.75), fg))
        .child(
            text(t(&format!("transactions:table.row.collateral.{key}")))
                .font(tk.txs(500.0))
                .color(fg),
        )
}

fn loan_accounting(role: &str) -> String {
    t(&format!(
        "transactions:table.row.collateral.{}",
        match role {
            "collateral_release" => "returnedAccounting",
            "loan_principal_received" => "principalReceivedAccounting",
            "loan_principal_repaid" => "principalRepaidAccounting",
            _ => "collateralAccounting",
        }
    ))
}

// ---- Row ----------------------------------------------------------------------------------

fn row_el(app: &App, tk: &Tk, i: usize, c: Cols) -> El {
    let ui = &app.txs;
    let Some(&ri) = ui.filtered.get(i) else {
        // The loader row.
        let label = if ui.loading_more {
            t("transactions:table.virtual.loadingMore")
        } else {
            String::new()
        };
        return row()
            .key("txs-loader")
            .w_full()
            .h(64.0)
            .center()
            .child(text(label).font(tk.txs(400.0)).color(tk.muted_fg));
    };
    let r = &ui.rows[ri];
    // A built row keeps its subtree until something it shows changes.
    let version = (
        ui.generation,
        crate::i18n::lang().code(),
        app.prefs.currency == Currency::Fiat,
        app.prefs.hide_sensitive(),
        tk.dark,
        tk.rem.to_bits(),
        ui.highlighted.contains(&r.id),
        ui.popup == Some(Popup::RowMenu(r.id.clone())),
        ui.loans.get(&r.id).cloned(),
        (
            c.accounting,
            c.pricing,
            c.network,
            c.lead.map(f32::to_bits),
            c.route_w.to_bits(),
        ),
    );
    rune::memo(format!("txs-row-{}", r.id), version, || {
        build_row(app, tk, r, c)
    })
}

/// One `TableCell` (`p-2 align-middle`), `w` wide including its padding.
/// (Applied to the cell's own content element: one node fewer per cell, for mount cost.)
fn cell(tk: &Tk, w: Option<f32>, child: El) -> El {
    let c = child.padding(tk.r(0.5)).min_w(0.0);
    match w {
        Some(w) => c.w(w),
        None => c.grow(1.0),
    }
}

/// A paired move or swap row's title and its two wallets.
struct RouteRow {
    title: String,
    from: String,
    to: String,
}

use crate::txdetail::graph::route::arrows;

/// The route of a row that moves between two wallets: a daemon pair (a reviewed or automatic
/// move, a linked swap) names both wallets; a transfer or swap without one still may, as
/// "From -> To" in its wallet text. None for every other row.
fn route_of(r: &Row, flow: Flow) -> Option<RouteRow> {
    let str_of = |p: &serde_json::Value, k: &str| {
        p.get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned()
    };
    if let Some(p) = &r.pair {
        let (from, to) = (str_of(p, "outWallet"), str_of(p, "inWallet"));
        if !from.is_empty() && !to.is_empty() {
            let (oa, ia) = (str_of(p, "outAsset"), str_of(p, "inAsset"));
            let swap =
                str_of(p, "type") == "swap" || (!oa.is_empty() && !ia.is_empty() && oa != ia);
            let title = if swap && !oa.is_empty() && !ia.is_empty() {
                format!("{} {oa} → {ia}", t("transactions:classification.swap"))
            } else if swap {
                t("transactions:classification.swap")
            } else {
                t("transactions:classification.transfer")
            };
            return Some(RouteRow { title, from, to });
        }
    }
    if !matches!(flow, Flow::Transfer | Flow::Swap | Flow::Layer) {
        return None;
    }
    match r.wallet_legs().as_slice() {
        [from, to] => {
            let fallback = if flow == Flow::Transfer {
                t("transactions:classification.transfer")
            } else {
                t("transactions:classification.swap")
            };
            let own = arrows(r.counterparty.trim());
            Some(RouteRow {
                title: if own.is_empty() { fallback } else { own },
                from: from.clone(),
                to: to.clone(),
            })
        }
        _ => None,
    }
}

fn build_row(app: &App, tk: &Tk, r: &Row, c: Cols) -> El {
    let ui = &app.txs;
    let hide = app.prefs.hide_sensitive();
    let flow = r.display_flow(&ui.candidates);
    let fiat = app.prefs.currency == Currency::Fiat;
    let loan = ui.loans.get(&r.id);
    let cur = r.fiat_currency.clone();
    let xs = tk.txs(400.0);
    // Cell 1: the 2 rem flow tile (`mt-0.5 rounded-md border`), `gap-3`, then the title line and
    // the wrapping `text-xs` wallet · date (· txid) line.
    let fc = flow_chip(tk, flow);
    let glyph = match flow {
        Flow::Incoming => &I::ARROW_DOWN_RIGHT,
        Flow::Outgoing => &I::ARROW_UP_RIGHT,
        _ => &I::ARROW_LEFT_RIGHT,
    };
    // The table kit's leading slot: the direction glyph on its tinted 1.5 rem tile.
    let lead = crate::ui::table::LEAD;
    let tile = row()
        .size(tk.r(lead), tk.r(lead))
        .center()
        .radius(tk.r(0.375))
        .fill(fc.bg)
        .border(1.0, fc.ring)
        .border_layout()
        .a11y_hidden()
        .child(ic(glyph, tk.r(0.875), fc.fg));
    // A paired move or swap: its kind as the title ("Transfer", "Swap LBTC → BTC") and its two
    // wallets on their own line, "From → To", each name cut on its own.
    let route = route_of(r, flow);
    let row_title = match &route {
        Some(rt) => rt.title.clone(),
        None => arrows(&r.title()),
    };
    let title = if row_title.is_empty() {
        sensitive(short_txid(&r.txn_id), tk.tsm(500.0).mono(), tk.fg, hide)
    } else {
        sensitive(row_title.clone(), tk.tsm(500.0), tk.fg, hide)
            .help_when_truncated(row_title.clone())
    };
    // The kind chip only when it says something the title doesn't.
    let cls = r
        .classification(flow)
        .filter(|c| route.is_none() || !row_title.to_lowercase().starts_with(&c.to_lowercase()));
    let line1 = if cls.is_none() && loan.is_none() {
        title.truncate().min_w(0.0)
    } else {
        let mut l = row()
            .w_full()
            .gap(tk.r(0.375))
            .center_y()
            .flex_wrap()
            .min_w(0.0)
            .child(title.truncate().min_w(0.0).shrink());
        if let Some(cls) = cls {
            l = l.child(badge(tk, cls, false, false));
        }
        if let Some(role) = loan {
            l = l.child(loan_badge(tk, role));
        }
        l
    };
    // `flex flex-wrap gap-x-1.5 gap-y-1 text-xs`: wallet · date (· txid) wrap only between
    // items. One text does the same: no-break spaces inside items, an en space (0.5 em =
    // `gap-x-1.5`) between them, and the 0.25 rem row gap as extra line height.
    let wallet = if r.wallet.is_empty() {
        r.payment_method.clone()
    } else {
        r.wallet.clone()
    };
    // No break inside an item: no-break spaces, and word joiners around hyphens (UAX #14
    // breaks after a hyphen, which split "2025-11-09").
    let nb = |s: &str| s.replace(' ', "\u{a0}").replace('-', "\u{2060}-\u{2060}");
    let mut content = match &route {
        Some(_) => nb(&r.date),
        None => format!("{}\u{a0}·\u{2002}{}", nb(&wallet), nb(&r.date)),
    };
    let mut spans = Vec::new();
    if !row_title.is_empty() {
        content.push_str("\u{a0}·\u{2002}");
        let at = content.len();
        content.push_str(&short_txid(&r.txn_id));
        spans.push(rune::Span::new(at, content.len()).mono());
    }
    let line2_style = xs.lh(tk.r(1.0) + tk.r(0.125));
    // A route row's last line stays one line: the date, then the short txid cut if it must be.
    let route_meta = route.as_ref().map(|_| {
        let mut l = row()
            .w_full()
            .min_w(0.0)
            .gap(tk.r(0.375))
            .center_y()
            .mt(tk.r(0.25) - tk.r(0.0625))
            .child(
                text(r.date.clone())
                    .font(line2_style.features("'tnum' 1"))
                    .color(tk.muted_fg)
                    .single_line(),
            );
        if !row_title.is_empty() {
            let id = short_txid(&r.txn_id);
            l = l
                .child(text("·").font(line2_style).color(tk.muted_fg).a11y_hidden())
                .child(if hide {
                    sensitive(id, line2_style.mono(), tk.muted_fg, true).single_line()
                } else {
                    crate::ui::table::cell_text(id, line2_style.mono(), tk.muted_fg).shrink()
                });
        }
        l
    });
    let line2 = if hide {
        sensitive(content, line2_style, tk.muted_fg, true)
    } else {
        rune::rich(content, spans)
            .font(line2_style)
            .color(tk.muted_fg)
    }
    .wrap()
    .w_full()
    .mt(tk.r(0.25) - tk.r(0.0625));
    let mut text_col = col().grow(1.0).min_w(0.0).child(line1);
    if let Some(rt) = &route {
        let style = tk.txs(400.0).lh(tk.r(1.0) + tk.r(0.125));
        text_col = text_col.child(
            crate::txdetail::graph::route::pair_names(
                &rt.from,
                &rt.to,
                style,
                tk.muted_fg,
                tk.r(0.75),
                tk.r(0.25),
                hide,
                c.route_w,
            )
            .mt(tk.r(0.25) - tk.r(0.0625)),
        );
    }
    let cell1 = row()
        .w_full()
        .gap(tk.r(crate::ui::table::LEAD_GAP))
        .items_start()
        .min_w(0.0)
        .child(tile)
        .child(text_col.child(route_meta.unwrap_or(line2)));
    // Cell 2: the amount (`text-sm font-semibold tabular-nums`, a currency toggle) over the
    // other currency (`mt-1 text-xs`).
    let signed = matches!(flow, Flow::Incoming | Flow::Outgoing);
    let primary = if signed {
        let sbtc = r.signed_btc(flow);
        if fiat {
            match r.signed_fiat(flow) {
                Some(v) => format::signed_fiat(v, &cur),
                None => UNPRICED.into(),
            }
        } else {
            format::btc(sbtc, 8, true)
        }
    } else if fiat {
        r.shown_fiat()
            .map(|v| format::fiat(v, &cur))
            .unwrap_or_else(|| UNPRICED.into())
    } else {
        format!("₿{:.8}", r.shown_btc())
    };
    let secondary = if fiat {
        format::btc(r.shown_btc(), 8, false)
    } else {
        r.shown_fiat()
            .map(|v| format::fiat(v, &cur))
            .unwrap_or_else(|| UNPRICED.into())
    };
    // A recorded pair fee on its own line, in the display currency.
    let fee_line = r.paired.as_ref().filter(|p| p.fee_btc > 0.0).map(|p| {
        let amount = if fiat {
            p.fee_fiat
                .map(|v| format::fiat(v, &cur))
                .unwrap_or_else(|| UNPRICED.into())
        } else {
            format!("₿{:.8}", p.fee_btc)
        };
        tr(
            "transactions:table.pairedTransfer.fee",
            &[("amount", &amount)],
        )
    });
    let toggle_label = if fiat {
        t("chrome:currencyToggleText.showBitcoin")
    } else {
        t("chrome:currencyToggleText.showFiat")
    };
    let cell2 = col()
        .w_full()
        .items_end()
        .child(
            sensitive(
                primary.clone(),
                tk.tsm(600.0).features("'tnum' 1"),
                amount_color(tk, flow),
                hide,
            )
            .key(format!("txs-amt-{}", r.id))
            .role("button")
            .label(primary)
            .help(toggle_label)
            .cursor(rune::Cursor::Pointer)
            .on_press(|a: &mut App| {
                a.prefs.currency = if a.prefs.currency == Currency::Fiat {
                    Currency::Btc
                } else {
                    Currency::Fiat
                };
                let f = fiat_mode(a);
                a.txs.refilter(f);
            }),
        )
        .child(sensitive(secondary, xs.features("'tnum' 1"), tk.muted_fg, hide).mt(tk.r(0.25)))
        .children(
            fee_line
                .map(|f| sensitive(f, xs.features("'tnum' 1"), tk.muted_fg, hide).mt(tk.r(0.125))),
        );
    let mut cells = row()
        .w_full()
        .center_y()
        .child(cell(tk, c.lead, cell1))
        .child(cell(tk, Some(W_AMOUNT), cell2));
    // Cell 3: accounting: up to two outline tag badges (+N), then the tax label.
    if c.accounting {
        let mut tags = row().gap(tk.r(0.25)).flex_wrap().min_w(0.0);
        let extra: Vec<&String> = r.tags.iter().filter(|x| **x != r.label).collect();
        for tg in extra.iter().take(2) {
            tags = tags.child(badge(tk, (*tg).clone(), true, hide));
        }
        if extra.len() > 2 {
            tags = tags.child(badge(tk, format!("+{}", extra.len() - 2), true, false));
        }
        let acc = loan
            .map(|l| loan_accounting(l))
            .unwrap_or_else(|| r.tax_short(flow));
        let mut ac = col().w_full().min_w(0.0);
        if !extra.is_empty() {
            ac = ac.child(tags);
        }
        cells = cells.child(cell(
            tk,
            Some(W_ACCOUNTING),
            ac.child(
                text(acc)
                    .font(xs)
                    .color(tk.muted_fg)
                    .truncate()
                    .mt(tk.r(0.25)),
            ),
        ));
    }
    // Cell 4: pricing: the source chip and the coarse badge (`gap-1.5`), the per-BTC line, the
    // muted provider summary.
    if c.pricing {
        let (style, label) = r.pricing_pill();
        let mut top = row()
            .gap(tk.r(0.375))
            .center_y()
            .min_w(0.0)
            .child(ring_chip(tk, pricing_chip(tk, style), None, label));
        if r.pricing_quality.as_deref() == Some("coarse_fallback") {
            let cc = coarse_chip(tk);
            top = top.child(
                row()
                    .px(tk.r(0.375))
                    .py(tk.r(0.125))
                    .gap(tk.r(0.25))
                    .center_y()
                    .radius(tk.r(0.375))
                    .fill(cc.bg)
                    .help(t("transactions:table.coarse.title"))
                    .child(ic(&I::TRIANGLE_ALERT, tk.r(0.75), cc.fg))
                    .child(
                        text(t("transactions:table.coarse.badge"))
                            .font(tk.t2xs(500.0))
                            .color(cc.fg),
                    ),
            );
        }
        let rate_line = match r.rate {
            Some(rate) => tr(
                "transactions:table.value.perBtc",
                &[("value", &format::fiat(rate, &cur))],
            ),
            None => t("transactions:table.value.awaitingPrice"),
        };
        let mut pc = col().w_full().min_w(0.0).items_start().child(top).child(
            sensitive(rate_line, xs, tk.muted_fg, hide)
                .truncate()
                .mt(tk.r(0.25)),
        );
        let detail = r.pricing_detail();
        if !detail.is_empty() {
            pc = pc.child(
                text(detail.clone())
                    .font(tk.t2xs(400.0))
                    .color(tk.muted_fg.alpha(0.8))
                    .truncate()
                    .help(detail),
            );
        }
        cells = cells.child(cell(tk, Some(W_PRICING), pc));
    }
    // Cell 5: network: `rounded-md border px-2 py-0.5 text-xs font-normal text-muted-foreground`.
    if c.network {
        let label = if matches!(flow, Flow::Swap | Flow::Layer) {
            arrows(&r.rail_label())
        } else {
            r.payment_method.clone()
        };
        // The rail's logo in the leading slot, then its name (a lucide glyph for a non-rail).
        use crate::ui::table as tb;
        let lead = match tb::rail_of(&label) {
            Some(rail) => tb::lead(tk, tb::rail_logo(app, tk, rail, &label)),
            None => tb::lead_glyph(tk, &I::LANDMARK, tk.muted_fg),
        };
        cells = cells.child(cell(
            tk,
            Some(W_NETWORK),
            row()
                .w_full()
                .min_w(0.0)
                .gap(tk.r(tb::LEAD_GAP))
                .center_y()
                .child(lead)
                .child(tb::cell_text(label, xs, tk.muted_fg)),
        ));
    }
    // Cell 6: status chip, then taxable/excluded.
    let sg = match r.status {
        Status::Completed => &I::CIRCLE_CHECK,
        Status::Pending => &I::CLOCK,
        Status::Failed => &I::CIRCLE_X,
        Status::Review => &I::ROTATE_CCW,
    };
    let sub = if loan.is_some() {
        t("transactions:table.row.loanNonEvent")
    } else if r.excluded {
        t("transactions:table.row.excluded")
    } else if r.taxable_or_default(flow) {
        t("transactions:table.row.taxable")
    } else {
        t("transactions:table.row.notTaxable")
    };
    cells = cells.child(cell(
        tk,
        Some(W_STATUS),
        // The status chip only when it needs a look: a completed row shows no badge.
        {
            let mut c = col().w_full().items_start();
            if r.status != Status::Completed {
                c = c
                    .child(ring_chip(
                        tk,
                        status_chip(tk, r.status),
                        Some(sg),
                        r.status.label(),
                    ))
                    .child(text(sub).font(xs).color(tk.muted_fg).mt(tk.r(0.25)));
            } else {
                c = c.child(text(sub).font(xs).color(tk.muted_fg));
            }
            c
        },
    ));
    // Cell 7: the "⋯" ghost icon button (`size-8`).
    let id = r.id.clone();
    let menu_open = ui.popup == Some(Popup::RowMenu(r.id.clone()));
    let trigger = row()
        .key(format!("txs-rowmenu-{}", r.id))
        .size(tk.r(2.0), tk.r(2.0))
        .center()
        .radius(tk.r(0.375))
        .hover(tk.accent)
        .role("button")
        .label(tr(
            "transactions:table.row.actionsAria",
            &[("txid", &r.txn_id)],
        ))
        .focusable(true)
        .menu_trigger(menu_open, move |a: &mut App| {
            let p = Popup::RowMenu(id.clone());
            a.txs.popup = if a.txs.popup.as_ref() == Some(&p) {
                None
            } else {
                Some(p)
            };
        })
        .child(ic(&I::ELLIPSIS, tk.r(1.0), tk.muted_fg));
    cells = cells.child(cell(
        tk,
        Some(W_ACTIONS),
        col().items_start().child(trigger),
    ));
    let open_id = r.id.clone();
    let highlighted = ui.highlighted.contains(&r.id);
    // `<tr onClick>`: a plain row (no accessible control), hover `bg-muted/35`.
    // `border-b`: the collapsed table border adds 0.9 pt under each row (measured 69.3 pt pitch).
    row()
        .w_full()
        .pb(0.9)
        .fill(if highlighted {
            tk.primary.alpha(0.10)
        } else {
            Color::TRANSPARENT
        })
        .hover(tk.muted.alpha(0.35))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .role("row")
        .cursor(rune::Cursor::Pointer)
        .on_press(move |a: &mut App| open_detail(a, &open_id, "details"))
        .child(cells)
}

// ---- Row menu -----------------------------------------------------------------------------

fn item(tk: &Tk, key: String, glyph: &'static rune::Icon, label: String, danger: bool) -> El {
    let color = if danger { tk.destructive } else { tk.fg };
    row()
        .key(key)
        .w_full()
        .min_h(tk.r(2.0))
        .px(tk.r(0.5))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.25))
        .hover(tk.accent)
        .focus_highlight(tk.accent)
        .role("menuitem")
        .label(label.clone())
        .focusable(true)
        .child(ic(
            glyph,
            tk.r(1.0),
            if danger { tk.destructive } else { tk.muted_fg },
        ))
        .child(text(label).font(tk.tsm(400.0)).color(color))
}

fn sep(tk: &Tk) -> El {
    el().h(1.0).w_full().my(tk.r(0.25)).fill(tk.border)
}

fn row_menu(app: &App, tk: &Tk, id: &str, layer: &mut Vec<El>) {
    let Some(r) = app.txs.rows.iter().find(|r| r.id == id) else {
        return;
    };
    let flow = r.display_flow(&app.txs.candidates);
    let loan = app.txs.loans.get(&r.id).cloned();
    let copy_id = r.explorer_id.clone().unwrap_or_else(|| r.txn_id.clone());
    let (a, b, c) = (r.id.clone(), r.id.clone(), r.id.clone());
    let mut p = crate::ui::popup::Popup::menu(format!("txs-rowmenu-{id}"), |s: &mut App| {
        s.txs.popup = None
    })
    .list_key("txs-rowmenu-list");
    let add = |p: &mut crate::ui::popup::Popup,
               key: &str,
               glyph: &'static rune::Icon,
               label: String,
               destructive: bool,
               on: Box<dyn Fn(El) -> El>| {
        p.item(
            on(item(tk, key.into(), glyph, label.clone(), destructive)),
            key.to_owned(),
            label,
            true,
            false,
        );
    };
    add(
        &mut p,
        "txs-menu-view",
        &I::EYE,
        t("transactions:table.row.viewDetails"),
        false,
        Box::new(move |e| {
            let a = a.clone();
            e.on_press(move |s: &mut App| open_detail(s, &a, "details"))
        }),
    );
    add(
        &mut p,
        "txs-menu-classify",
        &I::PENCIL,
        t("transactions:table.row.classify"),
        false,
        Box::new(move |e| {
            let b = b.clone();
            e.on_press(move |s: &mut App| open_detail(s, &b, "classify"))
        }),
    );
    add(
        &mut p,
        "txs-menu-copy",
        &I::COPY,
        t("transactions:table.row.copyId"),
        false,
        Box::new(move |e| {
            let copy_id = copy_id.clone();
            e.on_press_tx(move |s: &mut App, tx: &mut Tx| {
                crate::settings::effects::copy(s, tx, copy_id.clone());
                s.txs.popup = None;
            })
        }),
    );
    if app.prefs.ai_features {
        let (id, date, held) = (r.id.clone(), r.date.clone(), r.quarantine_reason.is_some());
        add(
            &mut p,
            "txs-menu-ask",
            &I::SPARKLES,
            t("assistant:panel.askAbout"),
            false,
            Box::new(move |e| {
                let (id, date) = (id.clone(), date.clone());
                e.on_press(move |s: &mut App| {
                    s.txs.popup = None;
                    crate::assistant::ask_about(s, &id, Some(&date), held);
                })
            }),
        );
    }
    // (key, label key, target)
    let mut loans: Vec<(&str, &str, &'static str)> = Vec::new();
    match (&loan, flow) {
        (Some(role), Flow::Outgoing) if role != "loan_principal_repaid" => loans.push((
            "txs-menu-loan-1",
            "changePrincipalRepaid",
            "principal-repaid",
        )),
        (Some(role), Flow::Incoming) if role != "loan_principal_received" => loans.push((
            "txs-menu-loan-1",
            "changePrincipalReceived",
            "principal-received",
        )),
        (None, Flow::Outgoing) => {
            loans.push(("txs-menu-loan-1", "markPrincipalRepaid", "principal-repaid"));
            loans.push(("txs-menu-loan-2", "markCollateral", "collateral"));
        }
        (None, Flow::Incoming) => {
            loans.push((
                "txs-menu-loan-1",
                "markPrincipalReceived",
                "principal-received",
            ));
            loans.push(("txs-menu-loan-2", "markReturned", "returned"));
        }
        _ => {}
    }
    if !loans.is_empty() || loan.is_some() {
        p.add(sep(tk));
    }
    for (key, label, target) in loans {
        let id = c.clone();
        add(
            &mut p,
            key,
            &I::COINS,
            t(&format!("transactions:table.row.collateral.{label}")),
            false,
            Box::new(move |e| {
                let id = id.clone();
                e.on_press(move |s: &mut App| {
                    s.txs.popup = None;
                    s.send(
                        "ui.loans.mark",
                        json!({"txid": id, "as": target}),
                        "txs.loan_mark",
                    );
                })
            }),
        );
    }
    if loan.is_some() {
        let id = c.clone();
        add(
            &mut p,
            "txs-menu-unmark",
            &I::LINK_2_OFF,
            t("transactions:table.row.collateral.unmark"),
            false,
            Box::new(move |e| {
                let id = id.clone();
                e.on_press(move |s: &mut App| {
                    s.txs.popup = None;
                    s.send("ui.loans.unmark", json!({"txid": id}), "txs.loan_unmark");
                })
            }),
        );
    }
    let ex = c.clone();
    p.add(sep(tk));
    add(
        &mut p,
        "txs-menu-exclude",
        &I::X,
        t("transactions:table.row.exclude"),
        true,
        Box::new(move |e| {
            let ex = ex.clone();
            e.on_press(move |s: &mut App| {
                s.txs.popup = None;
                s.txs.confirm_exclude = Some(ex.clone());
            })
        }),
    );
    let list = col().min_w(240.0).padding(4.0).label(tr(
        "transactions:table.row.actionsAria",
        &[("txid", &r.txn_id)],
    ));
    p.mount(
        tk,
        layer,
        list,
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
                .key("txs-rowmenu")
        },
        Placement::BottomEnd,
        4.0,
    );
}

// ---- Toolbar: chips, Filters, Expand ------------------------------------------------------

/// `filterChipClassName`: `h-6 gap-1 rounded-md px-2 text-xs font-medium` gray with an inset
/// ring (`bg-gray-50 text-gray-600 ring-gray-500/10`, dark `bg-gray-800/50 text-gray-400
/// ring-gray-400/20`), and a 0.75 rem ×.
fn chip(tk: &Tk, key: String, label: String, aria: String, clear: fn(&mut App)) -> El {
    let (bg, fg, ring) = if tk.dark {
        (
            tw("gray", 800).alpha(0.5),
            tw("gray", 400),
            tw("gray", 400).alpha(0.2),
        )
    } else {
        (tw("gray", 50), tw("gray", 600), tw("gray", 500).alpha(0.1))
    };
    row()
        .key(key)
        .h(tk.r(1.5))
        .px(tk.r(0.5))
        .gap(tk.r(0.25))
        .center_y()
        .radius(tk.r(0.375))
        .fill(bg)
        .border(1.0, ring)
        .role("button")
        .label(aria)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(clear)
        .child(text(label).font(tk.txs(500.0)).color(fg).truncate())
        .child(ic(&I::X, tk.r(0.75), fg))
}

/// The active-filter chips ("Filters:" … "Clear all"), or None.
fn chips(app: &App, tk: &Tk) -> Option<El> {
    let f = &app.txs.filters;
    if !f.any_chip() {
        return None;
    }
    let mut r = row()
        .gap(tk.r(0.5))
        .center_y()
        .flex_wrap()
        .min_w(0.0)
        .child(
            text(t("transactions:table.filters"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        );
    if let Some(c) = &f.chart {
        let label = c.label();
        r = r.child(chip(
            tk,
            "txs-chip-chart".into(),
            tr("transactions:table.chip.chartPrefix", &[("label", &label)]),
            tr("transactions:table.chip.clearChart", &[("label", &label)]),
            |a| {
                a.txs.filters.chart = None;
                changed(a);
            },
        ));
    }
    if let Some(q) = f.quick {
        r = r.child(chip(
            tk,
            "txs-chip-quick".into(),
            q.label(),
            tr(
                "transactions:table.chip.clearQuick",
                &[("label", &q.label())],
            ),
            |a| {
                a.txs.filters.quick = None;
                changed(a);
            },
        ));
    }
    if let Some(b) = &f.breakdown {
        r = r.child(chip(
            tk,
            "txs-chip-breakdown".into(),
            b.label(),
            tr(
                "transactions:table.chip.clearBreakdown",
                &[("label", &b.label())],
            ),
            |a| {
                a.txs.filters.breakdown = None;
                changed(a);
            },
        ));
    }
    if !f.txids.is_empty() {
        r = r.child(chip(
            tk,
            "txs-chip-txids".into(),
            tc("transactions:table.chip.transactionSet", f.txids.len(), &[]),
            t("transactions:table.chip.clearTransactionSet"),
            |a| {
                a.txs.filters.txids.clear();
                changed(a);
            },
        ));
    }
    if let Some(s) = f.status {
        r = r.child(chip(
            tk,
            "txs-chip-status".into(),
            s.label(),
            tr(
                "transactions:table.chip.clearStatus",
                &[("label", &s.label())],
            ),
            |a| {
                a.txs.filters.status = None;
                changed(a);
            },
        ));
    }
    if let Some(fl) = f.flow {
        r = r.child(chip(
            tk,
            "txs-chip-flow".into(),
            fl.label(),
            tr(
                "transactions:table.chip.clearFlow",
                &[("label", &fl.label())],
            ),
            |a| {
                a.txs.filters.flow = None;
                changed(a);
            },
        ));
    }
    if let Some(p) = &f.payment {
        r = r.child(chip(
            tk,
            "txs-chip-payment".into(),
            p.clone(),
            tr("transactions:table.chip.clearPayment", &[("label", p)]),
            |a| {
                a.txs.filters.payment = None;
                changed(a);
            },
        ));
    }
    if f.with_fees {
        r = r.child(chip(
            tk,
            "txs-chip-fees".into(),
            t("transactions:table.withFees"),
            t("transactions:table.chip.clearWithFees"),
            |a| {
                a.txs.filters.with_fees = false;
                changed(a);
            },
        ));
    }
    r = r.child(
        row()
            .key("txs-clear-all")
            .role("button")
            .label(t("transactions:table.clearAll"))
            .focusable(true)
            .on_press(|a: &mut App| {
                a.txs.filters.clear_all();
                changed(a);
            })
            .child(
                text(t("transactions:table.clearAll"))
                    .font(tk.txs(400.0))
                    .color(tk.destructive),
            ),
    );
    Some(r)
}

fn check(key: &str, label: String, checked: bool, on: impl Fn(&mut App) + 'static) -> Entry {
    Entry::Check {
        key: key.to_owned(),
        label,
        checked,
        on: std::rc::Rc::new(on),
    }
}

fn status_entries(f: &Filters) -> Vec<Entry> {
    let mut e = vec![
        Entry::Label(t("transactions:table.filter.statusLabel")),
        check(
            "txs-f-status-all",
            t("transactions:table.filter.allStatuses"),
            f.status.is_none(),
            |a| {
                a.txs.filters.status = None;
                changed(a);
            },
        ),
    ];
    for s in Status::ALL {
        e.push(check(
            &format!("txs-f-status-{}", s.label()),
            s.label(),
            f.status == Some(s),
            move |a| {
                a.txs.filters.status = Some(s);
                changed(a);
            },
        ));
    }
    e
}

fn flow_entries(f: &Filters) -> Vec<Entry> {
    let mut e = vec![
        Entry::Label(t("transactions:table.filter.flowLabel")),
        check(
            "txs-f-flow-all",
            t("transactions:table.filter.allFlows"),
            f.flow.is_none(),
            |a| {
                a.txs.filters.flow = None;
                changed(a);
            },
        ),
    ];
    for fl in Flow::ALL {
        e.push(check(
            &format!("txs-f-flow-{}", fl.label()),
            fl.label(),
            f.flow == Some(fl),
            move |a| {
                a.txs.filters.flow = Some(fl);
                changed(a);
            },
        ));
    }
    e
}

fn network_entries(f: &Filters) -> Vec<Entry> {
    let mut e = vec![
        Entry::Label(t("transactions:table.filter.networkLabel")),
        check(
            "txs-f-net-all",
            t("transactions:table.filter.allNetworks"),
            f.payment.is_none(),
            |a| {
                a.txs.filters.payment = None;
                changed(a);
            },
        ),
    ];
    for p in PAYMENTS.iter() {
        e.push(check(
            &format!("txs-f-net-{p}"),
            p.to_string(),
            f.payment.as_deref() == Some(*p),
            move |a| {
                a.txs.filters.payment = Some(p.to_string());
                changed(a);
            },
        ));
    }
    e
}

fn wallet_entries(app: &App) -> Vec<Entry> {
    let f = &app.txs.filters;
    let mut e = vec![
        Entry::Label(t("transactions:table.filter.walletLabel")),
        check(
            "txs-f-wallet-all",
            t("transactions:table.filter.allWallets"),
            f.leg_wallet().is_none(),
            |a| {
                if a.txs.filters.leg_wallet().is_some() {
                    a.txs.filters.breakdown = None;
                }
                changed(a);
            },
        ),
    ];
    for w in app.txs.wallet_options() {
        let checked = f.leg_wallet() == Some(w.as_str());
        let key = w.clone();
        e.push(check(&format!("txs-f-wallet-{w}"), w, checked, move |a| {
            a.txs.filters.breakdown = Some(Breakdown {
                network: false,
                key: key.clone(),
                leg: true,
            });
            changed(a);
        }));
    }
    e
}

fn fee_entries(f: &Filters) -> Vec<Entry> {
    vec![
        Entry::Label(t("transactions:table.filter.feesLabel")),
        check(
            "txs-f-fees-all",
            t("transactions:table.filter.allFees"),
            !f.with_fees,
            |a| {
                a.txs.filters.with_fees = false;
                changed(a);
            },
        ),
        check(
            "txs-f-fees-with",
            t("transactions:table.withFees"),
            f.with_fees,
            |a| {
                a.txs.filters.with_fees = true;
                changed(a);
            },
        ),
    ]
}

fn filters_menu(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let f = &app.txs.filters;
    let n = f.active_count();
    let open = matches!(app.txs.popup, Some(Popup::Filters | Popup::FiltersSub(_)));
    // Button outline sm, `h-9 gap-2` with an icon (`px-2.5`), `border-primary` while filtered.
    let mut trigger = row()
        .key("txs-filters")
        .h(tk.r(2.25))
        .px(tk.r(0.625) + 1.0)
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.375))
        .fill(outline_fill(tk))
        .border(
            1.0,
            if n > 0 {
                tk.primary
            } else {
                outline_border(tk)
            },
        )
        .hover(outline_hover(tk))
        .role("button")
        .label(t("transactions:table.filter.menuAria"))
        .focusable(true)
        .menu_trigger(open, |a: &mut App| {
            a.txs.popup = if matches!(a.txs.popup, Some(Popup::Filters | Popup::FiltersSub(_))) {
                None
            } else {
                Some(Popup::Filters)
            }
        })
        .child(ic(&I::SLIDERS_HORIZONTAL, tk.r(1.0), tk.fg))
        .child(
            text(t("transactions:table.filter.menuTrigger"))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        );
    if n > 0 {
        // `grid min-w-4 place-items-center rounded-full bg-primary px-1 text-2xs font-semibold leading-4`.
        trigger = trigger.child(
            row()
                .min_w(tk.r(1.0))
                .h(tk.r(1.0))
                .px(tk.r(0.25))
                .center()
                .radius(tk.r(0.5))
                .fill(tk.primary)
                .child(
                    text(n.to_string())
                        .font(tk.t2xs(600.0).lh(tk.r(1.0)))
                        .color(tk.primary_fg),
                ),
        );
    }
    if open {
        let wallets = app.txs.wallet_options();
        let sub = match app.txs.popup {
            Some(Popup::FiltersSub(s)) => Some(s),
            _ => None,
        };
        let entries = vec![
            Entry::Label(t("transactions:table.filter.menuLabel")),
            Entry::Sep,
            Entry::Sub {
                name: "status",
                label: t("transactions:table.filter.statusTrigger"),
                dot: f.status.is_some(),
                disabled: false,
                width: 180.0,
                max_h: None,
                entries: status_entries(f),
            },
            Entry::Sub {
                name: "flow",
                label: t("transactions:table.filter.flowTrigger"),
                dot: f.flow.is_some(),
                disabled: false,
                width: 190.0,
                max_h: None,
                entries: flow_entries(f),
            },
            Entry::Sub {
                name: "network",
                label: t("transactions:table.filter.networkTrigger"),
                dot: f.payment.is_some(),
                disabled: false,
                width: 200.0,
                max_h: None,
                entries: network_entries(f),
            },
            Entry::Sub {
                name: "wallet",
                label: t("transactions:table.filter.walletTrigger"),
                dot: f.leg_wallet().is_some(),
                disabled: wallets.is_empty(),
                width: 220.0,
                max_h: Some(320.0),
                entries: wallet_entries(app),
            },
            Entry::Sub {
                name: "fees",
                label: t("transactions:table.filter.feesTrigger"),
                dot: f.with_fees,
                disabled: false,
                width: 180.0,
                max_h: None,
                entries: fee_entries(f),
            },
        ];
        // `w-[220px]` content less its padding and border.
        super::menu::mount(
            tk,
            layer,
            "txs-filters-menu",
            "txs-filters",
            Placement::BottomEnd,
            220.0,
            None,
            sub,
            entries,
        );
    }
    trigger
}

fn outline_fill(tk: &Tk) -> Color {
    if tk.dark {
        tk.input.alpha(tk.input.a * 0.3)
    } else {
        tk.background
    }
}
fn outline_border(tk: &Tk) -> Color {
    if tk.dark { tk.input } else { tk.border }
}
fn outline_hover(tk: &Tk) -> Color {
    if tk.dark {
        tk.input.alpha(tk.input.a * 0.5)
    } else {
        tk.accent
    }
}

/// Expand / Restore: a 2.25 rem outline icon button in both modes (its text is `sr-only`).
fn expand_button(app: &App, tk: &Tk) -> El {
    let ex = app.txs.expanded;
    row()
        .key("txs-expand")
        .size(tk.r(2.25), tk.r(2.25))
        .center()
        .radius(tk.r(0.375))
        .fill(outline_fill(tk))
        .border(1.0, outline_border(tk))
        .hover(outline_hover(tk))
        .role("button")
        .label(if ex {
            t("transactions:table.expand.collapseAria")
        } else {
            t("transactions:table.expand.expandAria")
        })
        .help(if ex {
            t("transactions:table.expand.collapseAria")
        } else {
            t("transactions:table.expand.expandAria")
        })
        .focusable(true)
        .on_press_tx(|a: &mut App, tx: &mut Tx| {
            a.txs.expanded = !a.txs.expanded;
            a.txs.popup = None;
            tx.scroll_to_instant("app-main", 0.0, 0.0);
            tx.scroll_to_instant("txs-scroll", 0.0, 0.0);
        })
        .child(ic(
            if ex { &I::MINIMIZE_2 } else { &I::MAXIMIZE_2 },
            tk.r(1.0),
            tk.fg,
        ))
}

/// The table's actions as the expanded period nav shows them: chips, Filters, Restore.
pub fn actions(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let mut r = row().gap(tk.r(0.5)).center_y().justify_end().min_w(0.0);
    if let Some(c) = chips(app, tk) {
        // Expanded: `mr-3 justify-end sm:mr-4`.
        r = r.child(c.justify_end().mr(tk.r(1.0)));
    }
    r.child(filters_menu(app, tk, layer))
        .child(expand_button(app, tk))
}

// ---- Header -------------------------------------------------------------------------------

fn sort_button(app: &App, tk: &Tk, key: SortKey) -> El {
    let cur = app
        .txs
        .filters
        .sort
        .filter(|(k, _)| *k == key)
        .map(|(_, d)| d);
    let (label, aria) = match (key, cur) {
        (SortKey::Date, None) => (
            t("transactions:table.column.transaction"),
            t("transactions:table.sort.dateInactive"),
        ),
        (SortKey::Date, Some(Dir::Asc)) => (
            t("transactions:table.column.transaction"),
            t("transactions:table.sort.dateAscending"),
        ),
        (SortKey::Date, Some(Dir::Desc)) => (
            t("transactions:table.column.transaction"),
            t("transactions:table.sort.dateDescending"),
        ),
        (SortKey::Amount, None) => (
            t("transactions:table.column.amount"),
            t("transactions:table.sort.amountInactive"),
        ),
        (SortKey::Amount, Some(Dir::Desc)) => (
            t("transactions:table.column.amount"),
            t("transactions:table.sort.amountDescending"),
        ),
        (SortKey::Amount, Some(Dir::Asc)) => (
            t("transactions:table.column.amount"),
            t("transactions:table.sort.amountAscending"),
        ),
    };
    let glyph = match cur {
        None => &I::ARROW_UP_DOWN,
        Some(Dir::Asc) => &I::ARROW_UP,
        Some(Dir::Desc) => &I::ARROW_DOWN,
    };
    let color = if cur.is_some() { tk.fg } else { tk.muted_fg };
    // `inline-flex h-7 items-center gap-1 rounded-md px-1.5 hover:bg-muted`.
    row()
        .key(format!(
            "txs-sort-{}",
            if key == SortKey::Date {
                "date"
            } else {
                "amount"
            }
        ))
        .h(tk.r(1.75))
        .px(tk.r(0.375))
        .gap(tk.r(0.25))
        .center_y()
        .radius(tk.r(0.375))
        .hover(tk.muted)
        .role("button")
        .label(aria.clone())
        .help(aria)
        .focusable(true)
        .on_press(move |a: &mut App| {
            let cur = a
                .txs
                .filters
                .sort
                .filter(|(k, _)| *k == key)
                .map(|(_, d)| d);
            a.txs.filters.sort = match (key, cur) {
                // Date: none → asc → desc → asc …, never clears.
                (SortKey::Date, Some(Dir::Asc)) => Some((key, Dir::Desc)),
                (SortKey::Date, _) => Some((key, Dir::Asc)),
                // Amount: none → desc → asc → none.
                (SortKey::Amount, None) => Some((key, Dir::Desc)),
                (SortKey::Amount, Some(Dir::Desc)) => Some((key, Dir::Asc)),
                (SortKey::Amount, Some(Dir::Asc)) => None,
            };
            a.txs.scroll_top = true;
            changed(a);
        })
        .child(text(label).font(tk.tsm(500.0)).color(color))
        .child(ic(glyph, tk.r(0.875), color))
}

fn header_filter(app: &App, tk: &Tk, layer: &mut Vec<El>, status: bool) -> El {
    let f = &app.txs.filters;
    let (popup, label, aria, active, entries) = if status {
        (
            Popup::StatusHeader,
            t("transactions:table.column.status"),
            t("transactions:table.filter.statusAria"),
            f.status.is_some(),
            status_entries(f),
        )
    } else {
        (
            Popup::NetworkHeader,
            t("transactions:table.column.network"),
            t("transactions:table.filter.paymentAria"),
            f.payment.is_some(),
            network_entries(f),
        )
    };
    let open = app.txs.popup.as_ref() == Some(&popup);
    let color = if active { tk.fg } else { tk.muted_fg };
    let p2 = popup.clone();
    let mut trigger = row()
        .key(if status {
            "txs-head-status"
        } else {
            "txs-head-network"
        })
        .h(tk.r(1.75))
        .px(tk.r(0.375))
        .gap(tk.r(0.25))
        .center_y()
        .radius(tk.r(0.375))
        .hover(tk.muted)
        .role("button")
        .label(aria.clone())
        .help(aria)
        .focusable(true)
        .menu_trigger(open, move |a: &mut App| {
            a.txs.popup = if a.txs.popup.as_ref() == Some(&p2) {
                None
            } else {
                Some(p2.clone())
            }
        })
        .child(text(label).font(tk.tsm(500.0)).color(color))
        .child(ic(&I::SLIDERS_HORIZONTAL, tk.r(0.875), color));
    if active {
        trigger = trigger.child(crate::ui::dot(tk.r(0.375), tk.primary));
    }
    if open {
        let w = if status { 180.0 } else { 200.0 };
        super::menu::mount(
            tk,
            layer,
            if status {
                "txs-head-status-menu"
            } else {
                "txs-head-network-menu"
            },
            if status {
                "txs-head-status"
            } else {
                "txs-head-network"
            },
            Placement::BottomEnd,
            w,
            None,
            None,
            entries,
        );
    }
    trigger
}

/// The sticky header row: `h-10 bg-muted/50 border-b`, `TableHead` = `px-2 align-middle
/// text-sm font-medium text-muted-foreground`.
fn header(app: &App, tk: &Tk, layer: &mut Vec<El>, c: Cols) -> El {
    let th = |w: Option<f32>, child: El| {
        let r = row()
            .h(tk.r(2.5))
            .px(tk.r(0.5))
            .center_y()
            .min_w(0.0)
            .child(child);
        match w {
            Some(w) => r.w(w),
            None => r.grow(1.0),
        }
    };
    let plain = |k: &str| text(t(k)).font(tk.tsm(500.0)).color(tk.muted_fg);
    let mut r = row()
        .w_full()
        .fill(tk.muted.alpha(0.5))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .role("row")
        .child(th(c.lead, sort_button(app, tk, SortKey::Date)).key("txs-th-lead"))
        .child(th(
            Some(W_AMOUNT),
            row()
                .w_full()
                .justify_end()
                .child(sort_button(app, tk, SortKey::Amount)),
        ));
    if c.accounting {
        r = r.child(th(
            Some(W_ACCOUNTING),
            plain("transactions:table.column.accounting"),
        ));
    }
    if c.pricing {
        r = r.child(th(
            Some(W_PRICING),
            plain("transactions:table.column.pricing"),
        ));
    }
    if c.network {
        r = r.child(th(Some(W_NETWORK), header_filter(app, tk, layer, false)));
    }
    r.child(th(Some(W_STATUS), header_filter(app, tk, layer, true)))
        .child(th(Some(W_ACTIONS), el()))
}

// ---- Card ---------------------------------------------------------------------------------

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let ui = &app.txs;
    let hide = app.prefs.hide_sensitive();
    let _ = hide;
    let win_h = cx
        .layout_size("sh-root")
        .map(|[_, h]| h)
        .filter(|h| *h > 0.0)
        .unwrap_or(if app.shell.window_h > 0.0 {
            app.shell.window_h
        } else {
            900.0
        });
    // The web sizes the card to max(80 % of the window, what fits under it); expanded, to the
    // dashboard's bottom less 8 pt.
    let card_h = if ui.expanded {
        // Title bar 40 + panel 1, the nav, the page gap; the card stops the page padding plus
        // 5.8 pt above the window's bottom edge (the shell's bottom gutter).
        let top = 41.0
            + cx.layout_size("transactions-period-nav")
                .map(|[_, h]| h)
                .unwrap_or(tk.page_pad() + tk.r(2.25))
            + tk.section_gap();
        (win_h - top - 5.8 - tk.page_pad()).max((win_h * 0.8).floor() - 160.0)
    } else {
        (win_h * 0.8).floor()
    };
    let mut card = surface(tk)
        .key("transactions-table")
        .w_full()
        .h(card_h)
        .padding(1.0)
        .clip_rounded(tk.r(0.75))
        .busy(ui.refreshing);
    if !ui.expanded {
        // `flex gap-2`: the `text-base font-medium` title and the chips; then the actions.
        let mut left = row()
            .gap(tk.r(0.5))
            .center_y()
            .min_w(0.0)
            .grow(1.0)
            .flex_wrap()
            .child(
                text(t("transactions:table.title"))
                    .font(tk.tbase(500.0))
                    .color(tk.fg),
            );
        if let Some(c) = chips(app, tk) {
            left = left.child(c);
        }
        // `sm:flex-row sm:items-center sm:gap-4 sm:px-6 sm:py-3.5`.
        let toolbar = row()
            .key("txs-toolbar")
            .w_full()
            .px(tk.card_pad())
            .py(tk.r(0.875))
            .gap(tk.r(1.0))
            .center_y()
            .on_double_press_tx(|a: &mut App, tx: &mut Tx| {
                a.txs.expanded = true;
                tx.scroll_to_instant("app-main", 0.0, 0.0);
            })
            .child(left)
            .child(
                row()
                    .gap(tk.r(0.5))
                    .center_y()
                    .child(filters_menu(app, tk, layer))
                    .child(expand_button(app, tk)),
            );
        card = card.child(toolbar);
    }
    let count = ui.filtered.len() + usize::from(ui.has_more);
    let tk2 = tk.clone();
    let mut c = cols(app, cx);
    let pad = 2.0 * tk.card_pad();
    let overflow = crate::ui::table::overflows("txs-scroll", W_TABLE + pad);
    // The viewport's inner width (before its first layout: the window less the rail, the page
    // padding and the card's), never less than the table's minimum.
    let table_w =
        crate::ui::table::width("txs-scroll", W_TABLE + pad, cols_w(app, cx) - 140.0) - pad;
    if overflow && table_w - c.fixed() >= 0.34 * table_w {
        c.lead = Some((0.34 * table_w).floor());
    }
    // The cell less its padding, the leading tile and the gap after it.
    let lead_w = cx
        .layout_size("txs-th-lead")
        .map(|[w, _]| w)
        .filter(|w| *w > 0.0)
        .unwrap_or(c.lead.unwrap_or(table_w - c.fixed()));
    c.route_w = (lead_w
        - 2.0 * tk.r(0.5)
        - tk.r(crate::ui::table::LEAD)
        - tk.r(crate::ui::table::LEAD_GAP))
    .floor()
    .max(0.0);
    let version = (
        ui.list_key.len(),
        ui.filtered.len(),
        ui.filtered.first().copied(),
        ui.filtered.last().copied(),
        ui.filters.sort.is_some(),
        ui.rows.len(),
    );
    let rows = col()
        .key("txs-rows")
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows(
            count,
            ROW_EST,
            |s: &App, i| match s.txs.filtered.get(i) {
                Some(&ri) => Cow::Borrowed(s.txs.rows[ri].id.as_str()),
                None => Cow::Borrowed("txs-loader"),
            },
            move |s: &App, i| row_el(s, &tk2, i, c),
        )
        .virtual_rows_version(version)
        .role("grid")
        .label(t("transactions:table.title"))
        .focus_group()
        .on_visible_rows(|s: &mut App, r: std::ops::Range<usize>| {
            if r.end + 8 >= s.txs.filtered.len() && s.txs.has_more && !s.txs.loading_more {
                load_more(s);
                return true;
            }
            false
        });
    let body: El = if ui.filtered.is_empty() && !ui.has_more {
        let msg = if ui.refreshing {
            t("transactions:table.virtual.refreshing")
        } else {
            t("transactions:table.empty")
        };
        crate::ui::table::empty_text(tk, msg)
    } else {
        rows
    };
    // `min-h-0 flex-1 overflow-auto bg-card px-6 pb-4` (expanded adds `pt-4`) around a
    // `w-full min-w-[1080px] table-fixed` table: below 1080 pt the table keeps its columns and
    // scrolls sideways. The header (`sticky top-0 z-10 bg-card`) scrolls sideways with the rows
    // and sticks at the top.
    let head = row()
        .w_full()
        .fill(tk.card)
        .sticky_top(0.0)
        .child(header(app, tk, layer, c));
    let mut scroll = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(tk.card_pad())
        .pt(if ui.expanded { tk.r(1.0) } else { 0.0 })
        .pb(tk.r(1.0))
        // `tabIndex={0}`: the region is a Tab stop on the web.
        .focusable(true)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .opacity(if ui.refreshing { 0.6 } else { 1.0 });
    // The horizontal axis only while the table is wider than the card: a two-axis viewport
    // costs the scroll about 0.5 ms (p95) per frame even when it has nothing to move.
    let table = col().child(head).child(body);
    scroll = if overflow {
        scroll.scroll_xy().child(table.w(table_w))
    } else {
        scroll.child(table.w_full())
    };
    let scroll = crate::ui::table::region(
        scroll,
        "txs-scroll",
        t("transactions:table.virtual.scrollRegion"),
        overflow,
        true,
    );
    card = card.child(scroll);
    // Footer.
    let n = ui.filtered.len();
    // One line (an owner request; the web stacks a count and a `text-xs` hint): the count, and while
    // more pages exist, that they load on scroll.
    let line = if ui.refreshing {
        t("transactions:table.virtual.refreshing")
    } else if ui.has_more && ui.loading_more {
        tc("transactions:table.virtual.loadingMoreInline", n, &[])
    } else if ui.has_more {
        tc("transactions:table.virtual.loadedInline", n, &[])
    } else {
        tc("transactions:table.virtual.complete", n, &[])
    };
    let footer = row()
        .w_full()
        .px(tk.card_pad())
        .py(tk.r(0.5))
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout()
        .role("status")
        .child(text(line).font(tk.txs(400.0)).color(tk.muted_fg).truncate());
    card = card.child(footer);
    if let Some(Popup::RowMenu(id)) = &ui.popup {
        row_menu(app, tk, id, layer);
    }
    let _ = cx;
    card
}

/// The "Exclude this transaction" confirmation (the web uses the native `confirm`).
pub fn confirm_dialog(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let Some(id) = app.txs.confirm_exclude.clone() else {
        return;
    };
    let close = |s: &mut App| s.txs.confirm_exclude = None;
    let dialog = col()
        .key("txs-confirm")
        .w(420.0)
        .padding(tk.r(1.25))
        .gap(tk.r(1.0))
        .radius(tk.r(0.75))
        .fill(tk.popover)
        .border(1.0, tk.border)
        .role("alertdialog")
        .label(t("transactions:table.row.exclude"))
        .modal(true, close)
        .on_press(|_s: &mut App| {})
        .child(
            text(t("transactions:table.row.exclude"))
                .font(tk.tbase(600.0))
                .color(tk.fg),
        )
        .child(
            text(t("transactions:table.row.excludeConfirm"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        )
        .child(
            row()
                .w_full()
                .justify_end()
                .gap(tk.r(0.5))
                .child(
                    crate::ui::outline_button(
                        tk,
                        "txs-confirm-cancel",
                        None,
                        &t("common:actions.cancel"),
                    )
                    .autofocus()
                    .on_press(close),
                )
                .child(
                    row()
                        .key("txs-confirm-ok")
                        .h(tk.r(2.0))
                        .px(tk.r(0.75))
                        .center()
                        .radius(tk.r(0.375))
                        .fill(tk.destructive)
                        .role("button")
                        .label(t("common:actions.confirm"))
                        .focusable(true)
                        .on_press(move |s: &mut App| {
                            s.txs.confirm_exclude = None;
                            s.send(
                                "ui.transactions.metadata.update",
                                json!({"transaction": id, "excluded": true}),
                                "txs.exclude",
                            );
                        })
                        .child(
                            text(t("common:actions.confirm"))
                                .font(tk.tsm(500.0))
                                .color(Color::WHITE),
                        ),
                ),
        );
    layer.push(
        row()
            .key("txs-confirm-wrap")
            .label(t("common:actions.cancel"))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .center()
            .fill(Color::BLACK.alpha(if tk.dark { 0.5 } else { 0.25 }))
            .on_press(close)
            .child(dialog),
    );
}
