//! The notices between the hero and the tabs, in the web's order: backend required, the sync
//! error, the balance reconciliation (only when the books and the chain differ) and the
//! review / unpriced attention line.

use rune::{Color, El, col, el, row, text};
use serde_json::Value;

use super::ui::{self, amber, h, red};
use super::{btc, s};
use crate::app::App;
use crate::i18n::{t, tc};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button};
use crate::ui::{ic, sensitive};
use crate::wallets::conn::Conn;

/// `buildBalanceReconciliation`: (available, imported sat, utxo sat).
pub fn reconciliation(balance_btc: f64, inv: Option<&Value>) -> Option<(i64, i64)> {
    let inv = inv?;
    if inv
        .get("support")
        .and_then(|s| s.get("supported"))
        .and_then(Value::as_bool)
        == Some(false)
    {
        return None;
    }
    let imported = (balance_btc * 1e8).round() as i64;
    let utxo: i64 = inv
        .get("totals")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|t| {
                    matches!(
                        s(t, "asset").unwrap_or("").to_uppercase().as_str(),
                        "BTC" | "LBTC" | "L-BTC"
                    )
                })
                .map(|t| t.get("amount_sat").and_then(Value::as_f64).unwrap_or(0.0) as i64)
                .sum()
        })
        .unwrap_or(0);
    Some((imported, utxo))
}

pub fn all(app: &App, tk: &Tk, c: &Conn) -> Vec<El> {
    let mut out = vec![];
    let hide = app.prefs.hide_sensitive();
    if super::backend_required(app, c) {
        let (fill, border, ink) = amber(tk);
        let route = if super::wallet_chain(app, c) == "liquid" {
            "/settings/liquid"
        } else {
            "/settings/bitcoin"
        };
        out.push(
            row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .gap(tk.r(0.75))
                .center_y()
                .radius(tk.r(0.5))
                .fill(fill)
                .border(1.0, border)
                .border_layout()
                .role("status")
                .child(
                    row()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(tk.r(0.75))
                        .items_start()
                        .child(ic(&I::WIFI_OFF, tk.r(1.0), ink).mt(tk.r(0.125)))
                        .child(
                            col()
                                .min_w(0.0)
                                .grow(1.0)
                                .child(
                                    text(t("connections:detail.sync.backendRequiredTitle"))
                                        .font(tk.tsm(500.0))
                                        .color(ink),
                                )
                                .child(
                                    text(t("connections:detail.sync.backendRequiredBody"))
                                        .font(tk.txs(400.0).lh(tk.r(1.25)))
                                        .color(ink)
                                        .wrap()
                                        .mt(tk.r(0.25)),
                                ),
                        ),
                )
                .child(
                    button(
                        tk,
                        "wd-add-backend",
                        Variant::Outline,
                        Size::Sm,
                        Some(&I::PLUS),
                        &t("connections:detail.sync.addBackend"),
                        true,
                    )
                    .role("link")
                    .on_press(move |a: &mut App| a.navigate(route)),
                ),
        );
    }
    if let Some(msg) = &app.wallets.detail.sync_error {
        let (fill, border, ink) = red(tk);
        // "Check certificate…" beside an error an untrusted or changed certificate caused.
        let backend = super::wallet_detail(app, c).and_then(|w| w.get("backend"));
        let named = backend.and_then(|b| s(b, "name").zip(Some(s(b, "kind").unwrap_or(""))));
        let offer = crate::settings::certtrust::offer(
            tk,
            &super::CERT_HOST,
            app.wallets.detail.cert_problem,
            named,
        );
        out.push(
            col()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .radius(tk.r(0.5))
                .fill(fill)
                .border(1.0, border)
                .border_layout()
                .role("status")
                .child(
                    text(msg.clone())
                        .font(tk.tsm(400.0))
                        .color(ink)
                        .wrap()
                        .w_full(),
                )
                .children(offer),
        );
    }
    if let Some((imported, utxo)) = reconciliation(c.balance, app.wallets.detail.utxos.ready()) {
        let delta = imported - utxo;
        if delta.abs() > 1 {
            out.push(reconcile_card(tk, imported, utxo, delta, hide));
        }
    }
    // The review queue count the Transactions page links to, and the unpriced count.
    let summary = app
        .wallets
        .detail
        .dash
        .ready()
        .and_then(|d| d.get("summary"));
    let n = |k: &str| {
        summary
            .and_then(|s| s.get(k))
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
    };
    let review = n("review") + n("pending") + n("failed");
    let missing: f64 = ["incoming", "outgoing", "transfers", "swaps"]
        .iter()
        .map(|k| {
            summary
                .and_then(|s| s.get(*k))
                .and_then(|x| x.get("missingPrice"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        })
        .sum();
    if review > 0.0 || missing > 0.0 {
        let (fill, border, ink) = amber(tk);
        let mut parts = vec![];
        if review > 0.0 {
            parts.push(tc(
                "connections:detail.attention.review",
                review as usize,
                &[],
            ));
        }
        if missing > 0.0 {
            parts.push(tc(
                "connections:detail.attention.prices",
                missing as usize,
                &[],
            ));
        }
        let label = c.label.clone();
        let mut actions = row().gap(tk.r(0.5)).flex_wrap();
        let small = |key: &str, l: String, quick: &'static str, label: String| {
            // The notice's ink carries into its buttons (`text-amber-900 dark:text-amber-200`).
            crate::txdetail::widgets::button_surface(
                tk,
                row()
                    .key(key.to_owned())
                    .h(tk.r(1.75))
                    .px(tk.r(0.625))
                    .center(),
                Variant::Outline,
            )
            .role("link")
            .label(l.clone())
            .focusable(true)
            .on_press(move |a: &mut App| {
                a.navigate(&format!(
                    "/transactions?wallet={}&quick={quick}",
                    enc(&label)
                ))
            })
            .child(text(l).font(tk.tsm(500.0)).color(ink))
        };
        if review > 0.0 {
            actions = actions.child(small(
                "wd-open-review",
                t("connections:detail.attention.openReview"),
                "review_queue",
                label.clone(),
            ));
        }
        if missing > 0.0 {
            actions = actions.child(small(
                "wd-open-prices",
                t("connections:detail.attention.openPrices"),
                "missing_price",
                label.clone(),
            ));
        }
        out.push(
            row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.625))
                .gap(tk.r(0.75))
                .center_y()
                .radius(tk.r(0.5))
                .fill(fill)
                .border(1.0, border)
                .border_layout()
                .role("status")
                .child(
                    row()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(tk.r(0.75))
                        .center_y()
                        .child(ic(&I::TRIANGLE_ALERT, tk.r(1.0), ink))
                        .child(
                            text(parts.join(" · "))
                                .font(tk.tsm(400.0))
                                .color(ink)
                                .wrap(),
                        ),
                )
                .child(actions),
        );
    }
    out
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|c| match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (c as char).to_string()
            }
            _ => format!("%{c:02X}"),
        })
        .collect()
}

/// The reconciliation card (`Card border-amber-300 py-3`): the on-chain inventory ≠ the books,
/// and the signed gap with its likely cause.
fn reconcile_card(tk: &Tk, imported: i64, utxo: i64, delta: i64, hide: bool) -> El {
    let warn_ink = if tk.dark { h("#fbbf24") } else { h("#d97706") };
    let figure = |title: &str, sat: i64, sub: &str| {
        col()
            .child(
                text(title.to_uppercase())
                    .font(tk.t2xs(500.0).tracking(0.25))
                    .color(tk.muted_fg),
            )
            .child(sensitive(
                btc(sat as f64 / 1e8, 8, false),
                tk.tbase(600.0).mono().features("'tnum' 1"),
                tk.fg,
                hide,
            ))
            .child(text(sub.to_owned()).font(tk.t2xs(400.0)).color(tk.muted_fg))
    };
    let gap = btc(delta.unsigned_abs() as f64 / 1e8, 8, false);
    let (pill, hint) = if delta < 0 {
        (
            crate::i18n::tr(
                "connections:detail.reconcile.onChainNotInBooks",
                &[("amount", &gap)],
            ),
            t("connections:detail.reconcile.inboundExcluded"),
        )
    } else {
        (
            crate::i18n::tr(
                "connections:detail.reconcile.inBooksNotOnChain",
                &[("amount", &gap)],
            ),
            t("connections:detail.reconcile.manualOrDuplicate"),
        )
    };
    let pill_ink = if tk.dark { h("#fcd34d") } else { h("#b45309") };
    let border = if tk.dark {
        h("#78350f").alpha(0.6)
    } else {
        h("#fcd34d")
    };
    ui::card(tk).border(1.0, border).child(
        row()
            .w_full()
            .px(tk.r(1.0))
            .py(tk.r(0.75))
            .gap(tk.r(1.0))
            .center_y()
            .justify_between()
            .child(
                row()
                    .gap(tk.r(1.0))
                    .center_y()
                    .child(
                        row()
                            .size(tk.r(2.25), tk.r(2.25))
                            .center()
                            .radius(tk.r(0.375))
                            .border(1.0, tk.border)
                            .fill(tk.muted.alpha(0.4))
                            .a11y_hidden()
                            .child(ic(&I::SCALE, tk.r(1.0), tk.muted_fg)),
                    )
                    .child(
                        row()
                            .gap(tk.r(1.0))
                            .items_start()
                            .child(figure(
                                &t("connections:detail.reconcile.onChainInventory"),
                                utxo,
                                &t("connections:detail.reconcile.watchOnlyTruth"),
                            ))
                            .child(
                                text("≠")
                                    .font(tk.tbase(600.0).mono())
                                    .color(warn_ink)
                                    .pt(tk.r(1.0))
                                    .a11y_hidden(),
                            )
                            .child(figure(
                                &t("connections:detail.reconcile.recordedInBooks"),
                                imported,
                                &t("connections:detail.reconcile.feedsTaxReports"),
                            )),
                    ),
            )
            .child(
                col()
                    .items_end()
                    .min_w(0.0)
                    .child(
                        row()
                            .gap(tk.r(0.375))
                            .px(tk.r(0.625))
                            .py(tk.r(0.25))
                            .radius(tk.r(5.0))
                            .fill(h("#f59e0b").alpha(0.1))
                            .border(1.0, h("#d97706").alpha(0.2))
                            .center_y()
                            .child(ic(&I::TRIANGLE_ALERT, tk.r(0.875), pill_ink))
                            .child(text(pill).font(tk.txs(500.0)).color(pill_ink)),
                    )
                    .child(
                        text(hint)
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .mt(tk.r(0.25))
                            .wrap(),
                    ),
            ),
    )
}

#[allow(dead_code)]
fn _unused(_: Color) -> El {
    el()
}
