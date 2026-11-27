//! The page header (back, asset badge, name, kind · network, status, Refresh, More actions) and
//! the hero card: the balance with its fact list beside the balance history.

use rune::{Cx, El, col, row, text};
use serde_json::Value;

use super::ui;
use super::{Read, btc, count, f, s};
use crate::app::{App, Currency};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Variant, button_surface};
use crate::ui::popup::TriggerExt;
use crate::ui::{ic, sensitive};
use crate::wallets::conn::{Conn, kind_label};

pub fn header(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>, c: &Conn) -> El {
    let detail = super::wallet_detail(app, c);
    let network = [
        detail.and_then(|w| s(w, "chain")),
        detail.and_then(|w| s(w, "network")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    let sub = [
        Some(kind_label(c.display_kind())),
        Some(network.as_str()).filter(|x| !x.is_empty()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let mut meta = row()
        .min_w(0.0)
        .gap(tk.r(0.5))
        .center_y()
        .flex_wrap()
        .child(text(sub).font(tk.txs(400.0)).color(tk.muted_fg).truncate());
    if let Some(p) = crate::wallets::common::status_pill(tk, &c.status, false) {
        meta = meta.child(p);
    }
    if super::actions::is_deprecated(app, c) {
        meta = meta.child(
            row()
                .h(tk.r(1.25))
                .px(tk.r(0.375))
                .radius(tk.r(0.375))
                .fill(tk.muted)
                .center_y()
                .child(
                    text(t("connections:detail.deprecatedBadge"))
                        .font(tk.t2xs(500.0))
                        .color(tk.fg),
                ),
        );
    }
    let back = button_surface(
        tk,
        row().key("wd-back").size(tk.r(2.0), tk.r(2.0)).center(),
        Variant::Outline,
    )
    .role("link")
    .label(t("connections:detail.backToWallets"))
    .focusable(true)
    .on_press(|a: &mut App| a.navigate("/connections"))
    .child(ic(&I::ARROW_LEFT, tk.r(1.0), tk.fg));
    let left = row()
        .min_w(0.0)
        .grow(1.0)
        .gap(tk.r(0.75))
        .center_y()
        .child(back)
        .child(crate::wallets::common::asset_badge(
            app, cx, tk, c, None, true,
        ))
        .child(
            col()
                .min_w(0.0)
                .child(
                    text(c.label.clone())
                        .font(tk.tlg(600.0).lh(tk.r(1.4)))
                        .color(tk.fg)
                        .truncate()
                        .role("heading"),
                )
                .child(meta.mt(tk.r(0.25))),
        );
    let running = super::sync_running(app);
    let refresh_label = tr(
        "connections:detail.refreshAction",
        &[
            (
                "action",
                &t(if running {
                    "connections:detail.refreshing"
                } else {
                    "connections:detail.refresh"
                }),
            ),
            ("label", &c.label),
        ],
    );
    let mut glyph = ic(&I::REFRESH_CW, tk.r(1.0), tk.fg);
    if running {
        glyph = glyph.looping(rune::LoopSpec::rotation(1.0));
    }
    let mut refresh = button_surface(
        tk,
        row()
            .key("wd-refresh")
            .h(tk.r(2.0))
            .px(tk.r(0.625))
            .gap(tk.r(0.5))
            .center(),
        Variant::Outline,
    )
    .role("button")
    .label(refresh_label)
    .busy(running)
    .disabled(running)
    .focusable(!running)
    .opacity(if running { 0.5 } else { 1.0 })
    .child(glyph)
    .child(
        text(t("connections:detail.refresh"))
            .font(tk.tsm(500.0))
            .color(tk.fg),
    );
    if !running {
        refresh = refresh.on_press(|a: &mut App| super::actions::sync(a, false));
    }
    let more = button_surface(
        tk,
        row().key("wd-more").size(tk.r(2.0), tk.r(2.0)).center(),
        Variant::Outline,
    )
    .role("button")
    .label(t("connections:detail.moreActions"))
    .focusable(true)
    .menu_trigger(app.wallets.detail.menu, |a: &mut App| {
        a.wallets.detail.menu = !a.wallets.detail.menu
    })
    .child(ic(&I::ELLIPSIS, tk.r(1.0), tk.fg));
    if app.wallets.detail.menu {
        super::dialogs::menu(app, tk, layer, c);
    }
    row()
        .w_full()
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .child(left)
        .child(row().gap(tk.r(0.75)).center_y().child(refresh).child(more))
}

/// The UTXO fact (`utxoMetric`): value and detail.
fn utxo_metric(app: &App) -> (String, Option<String>) {
    match &app.wallets.detail.utxos {
        Read::Loading => ("—".into(), None),
        Read::Failed(_) => ("—".into(), None),
        Read::Ready(inv) => {
            if inv
                .get("support")
                .and_then(|s| s.get("supported"))
                .and_then(Value::as_bool)
                == Some(false)
            {
                return (
                    "—".into(),
                    Some(t("connections:detail.metric.utxosUnsupported")),
                );
            }
            let Some(active) = active_count(inv) else {
                return ("—".into(), None);
            };
            let pending = pending_count(inv);
            let truncated = inv
                .get("summary")
                .and_then(|s| s.get("truncated"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let detail = if active == 0.0 {
                Some(t("connections:detail.metric.utxosNone"))
            } else if pending > 0 {
                Some(tc("connections:detail.metric.utxosPending", pending, &[]))
            } else if truncated {
                None
            } else {
                Some(t("connections:detail.metric.utxosConfirmed"))
            };
            (count(active), detail)
        }
    }
}

/// `inventory.freshness.active_count ?? inventory.summary.count`.
pub fn active_count(inv: &Value) -> Option<f64> {
    inv.get("freshness")
        .and_then(|x| f(x, "active_count"))
        .or_else(|| inv.get("summary").and_then(|x| f(x, "count")))
}

pub fn pending_count(inv: &Value) -> usize {
    inv.get("utxos")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|r| s(r, "confirmation_status") != Some("confirmed"))
                .count()
        })
        .unwrap_or(0)
}

/// `Jan 2019` in the UI language (de-AT `Jän. 2019`), from an ISO timestamp.
pub fn month_year(iso: &str) -> String {
    let Some(d) = crate::format::Day::parse(iso) else {
        return iso.to_owned();
    };
    month_year_of(d.y, d.m)
}

pub fn month_year_of(y: i32, m: u32) -> String {
    const EN: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    const DE: [&str; 12] = [
        "Jän.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sep.", "Okt.", "Nov.",
        "Dez.",
    ];
    let names = if crate::i18n::lang() == crate::i18n::Lang::De {
        DE
    } else {
        EN
    };
    format!("{} {}", names[(m.clamp(1, 12) - 1) as usize], y)
}

/// `WalletFact`: label left, value and its context right, a hairline below (not the last).
fn fact(tk: &Tk, label: &str, value: &str, detail: Option<String>, last: bool) -> El {
    let mut dd = col().min_w(0.0).items_end().child(
        text(value.to_owned())
            .font(tk.tsm(500.0).features("'tnum' 1"))
            .color(tk.fg),
    );
    if let Some(d) = detail {
        dd = dd.child(text(d).font(tk.t2xs(400.0)).color(tk.muted_fg).truncate());
    }
    row()
        .w_full()
        .min_w(0.0)
        .py(tk.r(0.5))
        .gap(tk.r(0.75))
        .items_start()
        .justify_between()
        .border_sides(
            [0.0, 0.0, if last { 0.0 } else { 1.0 }, 0.0],
            tk.border.alpha(tk.border.a * 0.6),
        )
        .border_layout()
        .child(
            text(label.to_owned())
                .font(tk.txs(400.0).lh(tk.r(1.25)))
                .color(tk.muted_fg),
        )
        .child(dd)
}

pub fn hero(app: &App, tk: &Tk, c: &Conn, width: f32) -> El {
    let hide = app.prefs.hide_sensitive();
    let (cur, rate) = super::fiat(app);
    let missing = super::rate_missing(app);
    let show_fiat = app.prefs.currency == Currency::Fiat && !missing;
    let fiat_text = crate::format::fiat(c.balance * rate, &cur);
    let btc_text = btc(c.balance, 8, false);
    let (big, small) = if show_fiat {
        (fiat_text.clone(), btc_text.clone())
    } else {
        (btc_text, fiat_text)
    };
    let small = if missing {
        t("connections:metrics.balanceNoMarketRate")
    } else {
        small
    };
    let toggle_label = t(if app.prefs.currency == Currency::Btc {
        "chrome:currencyToggleText.showFiat"
    } else {
        "chrome:currencyToggleText.showBitcoin"
    });
    let balance = col()
        .min_w(0.0)
        .child(
            text(t("connections:detail.metric.balance"))
                .font(tk.txs(500.0))
                .color(tk.muted_fg),
        )
        .child(
            row()
                .key("wd-balance")
                .mt(tk.r(0.25))
                .min_w(0.0)
                .role("button")
                .label(if hide {
                    crate::ui::hidden_amount()
                } else {
                    big.clone()
                })
                .help(toggle_label)
                .focusable(true)
                .on_press(|a: &mut App| {
                    a.prefs.currency = if a.prefs.currency == Currency::Btc {
                        Currency::Fiat
                    } else {
                        Currency::Btc
                    }
                })
                .child(
                    sensitive(
                        big,
                        rune::TextStyle::new(tk.r(1.875), 600.0)
                            .lh(tk.r(2.25))
                            .tracking(-0.025 * tk.r(1.875))
                            .features("'tnum' 1"),
                        tk.fg,
                        hide,
                    )
                    .truncate(),
                ),
        )
        .child(
            sensitive(small, tk.tsm(400.0).features("'tnum' 1"), tk.muted_fg, hide).mt(tk.r(0.25)),
        );

    let (uv, ud) = utxo_metric(app);
    let tx_count = c.transaction_count.unwrap_or_else(|| {
        app.wallets
            .detail
            .txs
            .ready()
            .map(|v| v.len() as f64)
            .unwrap_or(0.0)
    });
    let dash_history = app
        .wallets
        .detail
        .dash
        .ready()
        .and_then(|d| d.get("history"));
    let span = match dash_history.and_then(|h| s(h, "earliest")) {
        Some(e) => {
            let l = dash_history.and_then(|h| s(h, "latest")).unwrap_or(e);
            tr(
                "connections:detail.metric.activitySpan",
                &[("from", &month_year(e)), ("to", &month_year(l))],
            )
        }
        None => t("connections:detail.metric.noActivity"),
    };
    // The overview's syncMode is backend_descriptor, file_import, btcpay, …; coverage names the
    // mode itself.
    let mode = c
        .coverage
        .as_ref()
        .and_then(|v| v["source_mode"].as_str())
        .map(str::to_owned)
        .or_else(|| match c.sync_mode.as_deref() {
            Some("file_import") => Some("file".into()),
            Some(m) if m.starts_with("backend_") || m == "btcpay" => Some("live".into()),
            _ => None,
        });
    let source = match mode.as_deref() {
        Some("live" | "hybrid") => t("connections:detail.source.live"),
        Some("file") => t("connections:detail.source.file"),
        _ => t("connections:detail.source.wallet"),
    };
    let backend = super::wallet_detail(app, c)
        .and_then(|w| w.get("backend"))
        .and_then(|b| s(b, "name"))
        .map(str::to_owned)
        .unwrap_or(source);
    let has_history = super::history::has_history(app);
    let wide = width >= tk.r(56.0);
    let three = !has_history && width >= tk.r(42.0);
    let facts_items = [
        (t("connections:detail.metric.utxos"), uv, ud),
        (
            t("connections:detail.metric.transactions"),
            count(tx_count),
            Some(span),
        ),
        (
            t("connections:detail.metric.lastSync"),
            c.last.clone(),
            Some(backend),
        ),
    ];
    let facts: El = if three {
        let mut r = row()
            .w_full()
            .gap(tk.r(1.5))
            .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
            .border_layout();
        for (l, v, d) in facts_items {
            r = r.child(col().grow(1.0).min_w(0.0).child(fact(tk, &l, &v, d, true)));
        }
        r
    } else {
        let mut cl = col()
            .w_full()
            .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
            .border_layout();
        let n = facts_items.len();
        for (i, (l, v, d)) in facts_items.into_iter().enumerate() {
            cl = cl.child(fact(tk, &l, &v, d, i + 1 == n));
        }
        cl
    };
    let left = col()
        .min_w(0.0)
        .padding(tk.r(1.0))
        .gap(tk.r(1.0))
        .justify_between()
        .child(balance)
        .child(facts);
    let card = ui::card(tk);
    if !has_history {
        return card.child(left.w_full());
    }
    // `grid-cols-[minmax(16rem,19rem)_minmax(0,1fr)]`: the fact column takes 19 rem; the chart
    // is `w-full` inside `-mx-1`, so 0.5 rem wider than its padded column.
    let lw = tk.r(19.0).min(width * 0.5).max(tk.r(16.0));
    let chart_w = if wide {
        width - 2.0 - lw - tk.r(2.0) + tk.r(0.5)
    } else {
        width - 2.0 - tk.r(2.0) + tk.r(0.5)
    };
    let chart = super::history::view(app, tk, c, chart_w);
    if wide {
        card.child(
            row()
                .w_full()
                .items_stretch()
                .child(
                    left.w(lw)
                        .border_sides([0.0, 1.0, 0.0, 0.0], tk.border)
                        .border_layout(),
                )
                .child(col().grow(1.0).min_w(0.0).padding(tk.r(1.0)).child(chart)),
        )
    } else {
        card.child(left.w_full()).child(
            col()
                .w_full()
                .padding(tk.r(1.0))
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .border_layout()
                .child(chart),
        )
    }
}
