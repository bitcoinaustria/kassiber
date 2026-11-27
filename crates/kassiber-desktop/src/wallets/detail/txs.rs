//! The tabs strip and the Transactions tab: `WalletTransactionsTable` (the wallet's latest 100
//! rows, 25 at a time, opening in the docked detail), its loading skeleton and empty state.

use rune::{Color, El, col, el, row, text};
use serde_json::Value;

use super::ui::{self, h};
use super::{Read, Tab, btc, count, f, s};
use crate::app::App;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button, button_surface};
use crate::ui::{ic, sensitive};
use crate::wallets::conn::Conn;

/// `PAGE_SIZE`.
pub const PAGE: usize = 25;

fn tab_count(tk: &Tk, n: f64) -> El {
    text(count(n))
        .font(tk.txs(400.0).features("'tnum' 1"))
        .color(tk.muted_fg)
}

pub fn tabs(app: &App, tk: &Tk, c: &Conn) -> El {
    let d = &app.wallets.detail;
    let tx_count = c
        .transaction_count
        .unwrap_or_else(|| d.txs.ready().map(|v| v.len() as f64).unwrap_or(0.0));
    let inv = d.utxos.ready();
    let tracked = inv.is_none_or(|i| {
        i.get("support")
            .and_then(|s| s.get("supported"))
            .and_then(Value::as_bool)
            != Some(false)
    });
    let utxo_active = if d.utxos.loading() {
        None
    } else {
        inv.and_then(super::hero::active_count)
    };
    let mut list = row()
        .h(tk.r(2.25))
        .padding(3.0)
        .radius(tk.r(0.5))
        .fill(tk.muted)
        .center_y()
        .role("tablist")
        .focus_group();
    for tab in Tab::ALL {
        let active = d.tab == tab;
        let name = t(&format!("connections:detail.tabs.{}", tab.key()));
        let ink = if active || !tk.dark {
            tk.fg
        } else {
            tk.muted_fg
        };
        let mut trig = row()
            .key(format!("wd-tab-{}", tab.key()))
            .h_full()
            .px(tk.r(0.75))
            .gap(tk.r(0.375))
            .center()
            .radius(tk.r(0.375))
            .border(1.0, Color::TRANSPARENT)
            .border_layout()
            .role("tab")
            .checked(active)
            .label(match tab {
                Tab::Transactions => format!("{name} {}", count(tx_count)),
                Tab::Coins if tracked && utxo_active.is_some() => {
                    format!("{name} {}", count(utxo_active.unwrap_or(0.0)))
                }
                _ => name.clone(),
            })
            .focusable(true)
            .on_press(move |a: &mut App| a.wallets.detail.tab = tab)
            .child(text(name).font(tk.tsm(500.0)).color(ink));
        if active {
            trig = if tk.dark {
                trig.fill(tk.input.alpha(tk.input.a * 0.3))
                    .border(1.0, tk.input)
            } else {
                trig.fill(tk.background)
            }
            .box_shadow(rune::Shadow {
                y: 1.0,
                blur: 3.0,
                color: Color::BLACK.alpha(0.1),
                ..Default::default()
            });
        }
        match tab {
            Tab::Transactions => trig = trig.child(tab_count(tk, tx_count)),
            Tab::Coins => {
                if let (true, Some(n)) = (tracked, utxo_active) {
                    trig = trig.child(tab_count(tk, n));
                }
            }
            Tab::Details => {}
        }
        list = list.child(trig);
    }
    row().w_full().child(list)
}

/// One row of the table, as `toDashboardTransaction` shapes it.
struct RowData {
    id: String,
    date: String,
    flow: &'static str,
    type_label: String,
    counterparty: String,
    amount_btc: f64,
    fiat: Option<f64>,
    fiat_cur: Option<String>,
    status: &'static str,
}

/// `fmtShortTxid`.
fn short_txid(v: &str) -> String {
    if v.is_empty() {
        return "no id".into();
    }
    if v.chars().count() <= 18 {
        return v.to_owned();
    }
    let head: String = v.chars().take(10).collect();
    let tail: String = v
        .chars()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

/// `transactionTypeLabel`: the daemon's English type labels map to the bundle; tags show as is.
fn type_label(v: &str) -> String {
    let key = match v {
        "Buy" => "buy",
        "Deposit" => "deposit",
        "Income" => "income",
        "Wages" => "wages",
        "Mining" => "mining",
        "Staking" => "staking",
        "Interest" => "interest",
        "Airdrop" => "airdrop",
        "Hard fork" => "hardFork",
        "Acquired" => "acquired",
        "LN invoice" => "lnInvoice",
        "Channel close" => "channelClose",
        "Sell" => "sell",
        "Withdrawal" => "withdrawal",
        "Spend" => "spend",
        "Gift" => "gift",
        "Donation" => "donation",
        "Lost" => "lost",
        "Stolen" => "stolen",
        "Expense" => "expense",
        "LN payment" => "lnPayment",
        "Channel open" => "channelOpen",
        "Transfer" => "transfer",
        "Swap" => "swap",
        "Fee" => "fee",
        _ => return v.to_owned(),
    };
    t(&format!("transactions:type.{key}"))
}

fn parse_row(tx: &Value) -> RowData {
    let ty = s(tx, "type").unwrap_or("");
    let amount_sat = f(tx, "amountSat").unwrap_or(0.0);
    let fee = f(tx, "feeSat").unwrap_or(0.0);
    let display_sat = if ty == "Consolidation" && amount_sat == 0.0 && fee != 0.0 {
        -fee.abs()
    } else {
        amount_sat
    };
    let eur = tx.get("eur").and_then(Value::as_f64);
    let rate = tx.get("rate").and_then(Value::as_f64);
    let fiat = eur.or_else(|| rate.map(|r| display_sat / 1e8 * r));
    let internal = tx.get("internal").and_then(Value::as_bool).unwrap_or(false);
    let flow = if internal || matches!(ty, "Transfer" | "Consolidation" | "Rebalance") {
        "transfer"
    } else if matches!(ty, "Swap" | "Mint" | "Melt") {
        "swap"
    } else if amount_sat >= 0.0 {
        "incoming"
    } else {
        "outgoing"
    };
    let tag = s(tx, "tag").unwrap_or("");
    let tags: Vec<&str> = if tag.is_empty() {
        vec![ty]
    } else {
        tag.split(',')
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .collect()
    };
    let type_label = match tags.first().filter(|x| !x.is_empty()) {
        Some(tg) => type_label(tg),
        None => t(&format!("overview:txFlow.{flow}")),
    };
    let status = match s(tx, "queueStatus") {
        Some("review") => "review",
        Some("pending") => "pending",
        Some("failed") => "failed",
        Some("completed") => "confirmed",
        _ => {
            if internal {
                "pending"
            } else if f(tx, "conf").unwrap_or(0.0) > 0.0 {
                "confirmed"
            } else if tag.to_lowercase().contains("review") {
                "review"
            } else {
                "pending"
            }
        }
    };
    let note = s(tx, "note").map(str::trim).filter(|x| !x.is_empty());
    let counter = s(tx, "counter").map(str::trim).filter(|x| !x.is_empty());
    let counterparty = note
        .or(counter)
        .map(str::to_owned)
        .unwrap_or_else(|| short_txid(s(tx, "externalId").or_else(|| s(tx, "id")).unwrap_or("")));
    RowData {
        id: s(tx, "id").unwrap_or("").to_owned(),
        date: crate::format::tx_date(
            s(tx, "date").unwrap_or(""),
            f(tx, "conf").unwrap_or(0.0) > 0.0,
        ),
        flow,
        type_label,
        counterparty,
        amount_btc: display_sat / 1e8,
        fiat,
        fiat_cur: s(tx, "fiatCurrency").map(str::to_owned),
        status,
    }
}

/// `overviewFlowStyles`: (fill, ink, ring).
fn flow_chip(tk: &Tk, flow: &str) -> (Color, Color, Color) {
    let (l50, l700, ring_l, d900, d_alpha, d300, ring_d) = match flow {
        "incoming" => (
            "#ecfdf5",
            "#047857",
            h("#059669").alpha(0.20),
            "#064e3b",
            0.25,
            "#6ee7b7",
            h("#34d399").alpha(0.2),
        ),
        "outgoing" => (
            "#fef2f2",
            "#b91c1c",
            h("#dc2626").alpha(0.15),
            "#7f1d1d",
            0.25,
            "#fca5a5",
            h("#f87171").alpha(0.2),
        ),
        "transfer" => (
            "#fafafa",
            "#3f3f46",
            h("#71717a").alpha(0.20),
            "#27272a",
            0.70,
            "#d4d4d8",
            h("#a1a1aa").alpha(0.2),
        ),
        _ => (
            "#f0f9ff",
            "#0369a1",
            h("#0284c7").alpha(0.20),
            "#0c4a6e",
            0.25,
            "#7dd3fc",
            h("#38bdf8").alpha(0.2),
        ),
    };
    if tk.dark {
        (h(d900).alpha(d_alpha), h(d300), ring_d)
    } else {
        (h(l50), h(l700), ring_l)
    }
}

fn amount_ink(tk: &Tk, flow: &str) -> Color {
    match (flow, tk.dark) {
        ("incoming", false) => h("#047857"),
        ("incoming", true) => h("#6ee7b7"),
        ("outgoing", false) => h("#b91c1c"),
        ("outgoing", true) => h("#fca5a5"),
        _ => tk.muted_fg,
    }
}

/// The Transactions tab: skeleton while loading, the table, or the empty state.
pub fn panel(app: &App, tk: &Tk, c: &Conn, width: f32) -> El {
    let d = &app.wallets.detail;
    let rows: Vec<Value> = match &d.txs {
        Read::Ready(v) => v.clone(),
        _ => app.snapshot().map(|_| vec![]).unwrap_or_default(),
    };
    if d.txs.loading() && rows.is_empty() {
        return ui::card(tk).child(
            col()
                .w_full()
                .padding(tk.r(1.0))
                .gap(tk.r(0.75))
                .child(ui::skeleton(tk, None, tk.r(2.0)))
                .child(ui::skeleton(tk, None, tk.r(2.0)))
                .child(ui::skeleton(tk, Some(width * 0.66), tk.r(2.0))),
        );
    }
    let transactions = if rows.is_empty() {
        empty(app, tk)
    } else {
        table(app, tk, c, &rows, width)
    };
    if d.bark_activity.is_empty() {
        transactions
    } else {
        col()
            .w_full()
            .gap(tk.r(0.75))
            .child(bark_activity(app, tk))
            .child(transactions)
    }
}

fn bark_activity(app: &App, tk: &Tk) -> El {
    const PAGE: usize = 10;
    let d = &app.wallets.detail;
    let visible = d.bark_activity_visible.max(PAGE);
    let hidden = app.prefs.hide_sensitive();
    let mut card = ui::card(tk)
        .w_full()
        .padding(tk.r(1.0))
        .gap(tk.r(0.5))
        .child(
            text(t("connections:detail.barkActivity.title"))
                .font(tk.tsm(600.0))
                .color(tk.fg),
        );
    for activity in d.bark_activity.iter().take(visible) {
        let kind = s(activity, "kind").unwrap_or_default();
        let kind_key = match kind {
            "refresh" => "refresh",
            "ark-send" | "ark-receive" => "ark",
            "lightning-send" | "lightning-receive" => "lightning",
            "send-onchain" | "onchain-send" | "onchain-receive" | "onchain-internal" => "onchain",
            "board" => "board",
            "offboard" => "offboard",
            _ => "exit",
        };
        let description = s(activity, "comment").filter(|value| !value.is_empty());
        let labels = activity["labels_json"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" · ");
        let amount = activity["amount_msat"].as_i64().unwrap_or_default();
        let fee = activity["fee_msat"].as_i64();
        let status = match s(activity, "transfer_status") {
            Some("paired") => Some("paired"),
            Some("missing_txid") => Some("missingTxid"),
            Some("missing_sync_provenance") => Some("missingSyncProvenance"),
            Some("journal_pending") => Some("journalPending"),
            Some("no_pair") if s(activity, "payout_kind").is_none() => Some("noPair"),
            Some("owned_fanout_unresolved" | "custody_quantity_unresolved") => Some("ambiguous"),
            Some("transfer_network_mismatch") => Some("networkMismatch"),
            Some("recorded_transfer_authority_conflict" | "transfer_pair_dependency_blocked") => {
                Some("authority")
            }
            _ => None,
        };
        let purpose = match s(activity, "payout_kind") {
            Some("refund") => Some("refund"),
            Some("store_payout" | "pull_payment") => Some("payout"),
            _ => None,
        };
        let mut detail = col().grow(1.0).w(0.0).gap(tk.r(0.125)).child(
            text(t(&format!(
                "connections:detail.barkActivity.kinds.{kind_key}"
            )))
            .font(tk.tsm(500.0))
            .color(tk.fg),
        );
        for value in description
            .into_iter()
            .chain((!labels.is_empty()).then_some(labels.as_str()))
        {
            detail = detail.child(sensitive(value.to_owned(), tk.txs(400.0), tk.fg, hidden).wrap());
        }
        for key in purpose.into_iter().chain(status) {
            detail = detail.child(
                text(t(&format!("connections:detail.barkActivity.{key}")))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap(),
            );
        }
        let mut amounts = col().gap(tk.r(0.125)).items_end().child(sensitive(
            crate::format::signed_msat_sats(amount.into()),
            tk.tsm(500.0),
            tk.fg,
            hidden,
        ));
        if let Some(fee) = fee.filter(|fee| *fee > 0) {
            amounts = amounts.child(sensitive(
                tr(
                    "connections:detail.barkActivity.fee",
                    &[("amount", &crate::format::signed_msat_sats(-(fee as i128)))],
                ),
                tk.txs(400.0),
                tk.muted_fg,
                hidden,
            ));
        }
        card = card.child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .py(tk.r(0.25))
                .child(
                    text(crate::format::tx_date(
                        s(activity, "occurred_at").unwrap_or(""),
                        true,
                    ))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
                )
                .child(detail)
                .child(amounts),
        );
    }
    if d.bark_activity.len() > visible {
        card = card.child(
            button(
                tk,
                "wd-bark-activity-more",
                Variant::Outline,
                Size::Sm,
                None,
                &t("connections:detail.barkActivity.more"),
                true,
            )
            .on_press(move |app: &mut App| {
                app.wallets.detail.bark_activity_visible = visible + PAGE
            }),
        );
    }
    card
}

fn empty(app: &App, tk: &Tk) -> El {
    let running = super::sync_running(app);
    let mut glyph = ic(&I::REFRESH_CW, tk.r(1.0), tk.fg);
    if running {
        glyph = glyph.looping(rune::LoopSpec::rotation(1.0));
    }
    let label = t(if running {
        "connections:detail.refreshing"
    } else {
        "connections:detail.recentTransactions.refreshNow"
    });
    let mut refresh = button_surface(
        tk,
        row()
            .key("wd-empty-refresh")
            .h(tk.r(2.0))
            .px(tk.r(0.625))
            .gap(tk.r(0.375))
            .center(),
        Variant::Outline,
    )
    .role("button")
    .label(label.clone())
    .disabled(running)
    .focusable(!running)
    .opacity(if running { 0.5 } else { 1.0 })
    .child(glyph)
    .child(text(label).font(tk.tsm(500.0)).color(tk.fg));
    if !running {
        refresh = refresh.on_press(|a: &mut App| super::actions::sync(a, false));
    }
    ui::card(tk)
        .child(
            row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.625))
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .border_layout()
                .child(
                    text(t("connections:detail.recentTransactions.title"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                ),
        )
        .child(
            col()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(1.5))
                .gap(tk.r(0.75))
                .items_start()
                .child(
                    text(t("connections:detail.recentTransactions.empty"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                )
                .child(
                    row().gap(tk.r(0.5)).child(refresh).child(
                        button(
                            tk,
                            "wd-empty-import",
                            Variant::Outline,
                            Size::Sm,
                            Some(&I::PLUS),
                            &t("connections:detail.recentTransactions.importFile"),
                            true,
                        )
                        .role("link")
                        .on_press(|a: &mut App| a.navigate("/imports")),
                    ),
                ),
        )
}

fn table(app: &App, tk: &Tk, c: &Conn, rows: &[Value], width: f32) -> El {
    let d = &app.wallets.detail;
    let hide = app.prefs.hide_sensitive();
    let (cur, _) = super::fiat(app);
    let show_type = width >= tk.r(42.0);
    let show_value = width >= tk.r(48.0);
    let show_status = width >= tk.r(36.0);
    let total = c.transaction_count.unwrap_or(rows.len() as f64);
    let visible: Vec<RowData> = rows.iter().take(d.tx_visible).map(parse_row).collect();
    let remaining = rows.len().saturating_sub(visible.len());
    // Column widths: content-sized (the table's auto layout), the description takes the rest.
    let w_date = tk.r(1.5) + 6.5 * 16.0;
    // The other columns size to their widest cell (estimated from the text, as the table's auto
    // layout measures it); the description takes the rest (`max-w-0 w-full`).
    let chars = |s: &str| s.chars().count() as f32;
    let hdr = |k: &str| chars(&t(k)) * 7.4;
    let w_type = visible
        .iter()
        .map(|r| {
            rune::text::measure(&r.type_label, &[], tk.tsm(400.0), None).0
                + 1.0
                + tk.r(crate::ui::table::LEAD + crate::ui::table::LEAD_GAP)
        })
        .fold(hdr("connections:detail.transactionsTable.type"), f32::max)
        + tk.r(1.0);
    let w_amount = visible
        .iter()
        .map(|r| chars(&btc(r.amount_btc, 8, r.amount_btc != 0.0)) * 7.0)
        .fold(hdr("connections:detail.transactionsTable.amount"), f32::max)
        + tk.r(1.0);
    let w_value = visible
        .iter()
        .map(|r| {
            chars(
                &r.fiat
                    .map(|x| crate::format::fiat(x.abs(), r.fiat_cur.as_deref().unwrap_or(&cur)))
                    .unwrap_or_else(|| t("common:state.unpriced")),
            ) * 5.7
        })
        .fold(hdr("connections:detail.transactionsTable.value"), f32::max)
        + tk.r(1.0);
    let w_status = visible
        .iter()
        .filter(|r| r.status != "confirmed")
        .map(|r| chars(&t(&format!("overview:txStatus.{}", r.status))) * 5.6 + 12.8)
        .fold(hdr("connections:detail.transactionsTable.status"), f32::max)
        + tk.r(1.5);
    use crate::ui::table as tb;
    let head = |s: &str| tb::th_text(tk, s.to_owned());
    let mut hr = tb::head(tk).h(tk.r(tb::HEAD_H)).center_y();
    hr = hr.child(
        row()
            .w(w_date)
            .pl(tk.r(1.0))
            .pr(tk.r(0.5))
            .child(head(&t("connections:detail.transactionsTable.date"))),
    );
    if show_type {
        hr = hr.child(
            row()
                .w(w_type)
                .px(tk.r(0.5))
                .child(head(&t("connections:detail.transactionsTable.type"))),
        );
    }
    hr = hr.child(
        row()
            .grow(1.0)
            .min_w(0.0)
            .px(tk.r(0.5))
            .child(head(&t("connections:detail.transactionsTable.description"))),
    );
    hr = hr.child(
        row()
            .w(w_amount)
            .px(tk.r(0.5))
            .justify_end()
            .child(head(&t("connections:detail.transactionsTable.amount"))),
    );
    if show_value {
        hr = hr.child(
            row()
                .w(w_value)
                .px(tk.r(0.5))
                .justify_end()
                .child(head(&t("connections:detail.transactionsTable.value"))),
        );
    }
    if show_status {
        hr = hr.child(
            row()
                .w(w_status)
                .pl(tk.r(0.5))
                .pr(tk.r(1.0))
                .child(head(&t("connections:detail.transactionsTable.status"))),
        );
    }
    let mut body = col().w_full();
    let n = visible.len();
    for (i, r) in visible.into_iter().enumerate() {
        let id = r.id.clone();
        let (fb, ff, fr) = flow_chip(tk, r.flow);
        let tr0 = tb::tr(tk, i + 1 == n)
            .key(format!("wd-tx-{}", r.id))
            .min_h(tk.r(2.75))
            .role("button")
            .label(row_label(&r, hide, &cur))
            .focusable(true);
        let mut tr_ = tb::interactive(tr0, tk, false)
            .on_press(move |a: &mut App| crate::txdetail::open(a, &id, "details"));
        tr_ = tr_.child(
            row().w(w_date).pl(tk.r(1.0)).pr(tk.r(0.5)).child(
                text(r.date.clone())
                    .font(tk.txs(400.0).features("'tnum' 1"))
                    .color(tk.muted_fg),
            ),
        );
        if show_type {
            // The direction tile in the leading slot, then the type.
            let glyph = match r.flow {
                "incoming" => &I::ARROW_DOWN_RIGHT,
                "outgoing" => &I::ARROW_UP_RIGHT,
                _ => &I::ARROW_LEFT_RIGHT,
            };
            let tile = row()
                .size(tk.r(tb::LEAD), tk.r(tb::LEAD))
                .center()
                .radius(tk.r(0.375))
                .fill(fb)
                .border(1.0, fr)
                .border_layout()
                .a11y_hidden()
                .child(ic(glyph, tk.r(0.875), ff));
            tr_ = tr_.child(
                row()
                    .w(w_type)
                    .px(tk.r(0.5))
                    .gap(tk.r(tb::LEAD_GAP))
                    .center_y()
                    .min_w(0.0)
                    .child(tile)
                    .child(tb::cell_text(r.type_label.clone(), tk.tsm(400.0), tk.fg)),
            );
        }
        let desc = if hide {
            sensitive(r.counterparty.clone(), tk.tsm(400.0), tk.fg, true).truncate()
        } else {
            tb::cell_text(r.counterparty.clone(), tk.tsm(400.0), tk.fg)
        };
        tr_ = tr_.child(row().grow(1.0).w(0.0).min_w(0.0).px(tk.r(0.5)).child(desc));
        let amount = btc(r.amount_btc, 8, r.amount_btc != 0.0);
        tr_ = tr_.child(
            row()
                .w(w_amount)
                .px(tk.r(0.5))
                .justify_end()
                .child(sensitive(
                    amount,
                    tk.tsm(500.0).features("'tnum' 1"),
                    amount_ink(tk, r.flow),
                    hide,
                )),
        );
        if show_value {
            let v = match r.fiat {
                None => t("common:state.unpriced"),
                Some(x) => crate::format::fiat(x.abs(), r.fiat_cur.as_deref().unwrap_or(&cur)),
            };
            tr_ = tr_.child(
                row()
                    .w(w_value)
                    .px(tk.r(0.5))
                    .justify_end()
                    .child(sensitive(
                        v,
                        tk.txs(400.0).features("'tnum' 1"),
                        tk.muted_fg,
                        hide,
                    )),
            );
        }
        if show_status {
            // A chip only while the transaction needs a look; a confirmed one shows none.
            let cell = row().w(w_status).pl(tk.r(0.5)).pr(tk.r(1.0)).min_w(0.0);
            let tone = match r.status {
                "pending" => Some(tb::Tone::Warning),
                "review" => Some(tb::Tone::Info),
                "confirmed" => None,
                _ => Some(tb::Tone::Danger),
            };
            tr_ = tr_.child(match tone {
                Some(tone) => cell.child(tb::chip(
                    tk,
                    tone,
                    None,
                    t(&format!("overview:txStatus.{}", r.status)),
                )),
                None => cell,
            });
        }
        body = body.child(tr_);
    }
    let label = c.label.clone();
    let mut actions = row().gap(tk.r(0.25)).center_y();
    if remaining > 0 {
        let more = remaining.min(PAGE);
        actions = actions.child(
            ghost_xs(
                tk,
                "wd-tx-more",
                &tc("connections:detail.transactionsTable.showMore", more, &[]),
                None,
            )
            .on_press(|a: &mut App| a.wallets.detail.tx_visible += PAGE),
        );
    }
    actions = actions.child(
        ghost_xs(
            tk,
            "wd-tx-open-all",
            &t("connections:detail.transactionsTable.openAll"),
            Some(&I::ARROW_RIGHT),
        )
        .role("link")
        .on_press(move |a: &mut App| {
            a.navigate(&format!(
                "/transactions?wallet={}",
                super::notices_enc(&label)
            ))
        }),
    );
    let footer = row()
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(0.5))
        .gap(tk.r(0.5))
        .center_y()
        .justify_between()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout()
        .child(
            text(tr(
                "connections:detail.transactionsTable.showing",
                &[("shown", &count(n as f64)), ("total", &count(total))],
            ))
            .font(tk.txs(400.0).features("'tnum' 1"))
            .color(tk.muted_fg),
        )
        .child(actions);
    // shadcn's `overflow-x-auto` container: the description (`max-w-0 w-full`) gives way
    // first; below the other columns' widths the table scrolls sideways.
    let least = w_date
        + w_amount
        + tk.r(1.0)
        + [
            (show_type, w_type),
            (show_value, w_value),
            (show_status, w_status),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, w)| w)
        .sum::<f32>();
    let (table_w, overflow) = crate::ui::table::fit(width - 2.0, least);
    let grid = col().w(table_w).child(hr).child(body);
    let grid = crate::ui::table::hscroll_if(
        "wd-tx-scroll",
        t("connections:detail.recentTransactions.title"),
        overflow,
        grid,
    );
    ui::card(tk).child(grid).child(footer)
}

/// The row's accessible name: its cells' text, as the web's `role="button"` row reads.
fn row_label(r: &RowData, hide: bool, cur: &str) -> String {
    let mask = |s: String| {
        if hide {
            crate::theme::MASK.to_owned()
        } else {
            s
        }
    };
    let value = match r.fiat {
        None => t("common:state.unpriced"),
        Some(x) => crate::format::fiat(x.abs(), r.fiat_cur.as_deref().unwrap_or(cur)),
    };
    format!(
        "{} {} {} {} {} {}",
        r.date,
        r.type_label,
        mask(r.counterparty.clone()),
        mask(btc(r.amount_btc, 8, r.amount_btc != 0.0)),
        mask(value),
        t(&format!("overview:txStatus.{}", r.status))
    )
}

/// `Button variant="ghost" size="sm" className="h-7 gap-1 px-2 text-xs"`, trailing icon.
pub fn ghost_xs(tk: &Tk, key: &str, label: &str, trailing: Option<&'static rune::Icon>) -> El {
    let mut b = button_surface(
        tk,
        row()
            .key(key.to_owned())
            .h(tk.r(1.75))
            .px(tk.r(0.5))
            .gap(tk.r(0.25))
            .center(),
        Variant::Ghost,
    )
    .role("button")
    .label(label.to_owned())
    .focusable(true)
    .child(text(label.to_owned()).font(tk.txs(500.0)).color(tk.fg));
    if let Some(g) = trailing {
        b = b.child(ic(g, tk.r(0.875), tk.fg));
    }
    b
}

#[allow(dead_code)]
fn _e() -> El {
    el()
}
