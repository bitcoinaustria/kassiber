//! The Coins tab: `WalletCoinsBreakdown` (coin counts by size and by age since confirmation)
//! and `UtxosInventoryPanel` (the watch-only UTXO inventory, sortable, 50 rows at a time).

use rune::{Color, El, col, el, row, text};
use serde_json::Value;

use super::ui::{self, h};
use super::{Read, Sort, btc, count, f, s};
use crate::app::App;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Variant, button_surface};
use crate::ui::{ic, sensitive};

/// `UTXO_PAGE_SIZE`.
pub const PAGE: usize = 50;
const SIZE_EDGES: [i64; 4] = [100_000, 1_000_000, 10_000_000, 100_000_000];

fn rows(inv: &Value) -> Vec<Value> {
    inv.get("utxos")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn sat(r: &Value) -> i64 {
    f(r, "amount_sat").unwrap_or(0.0) as i64
}

/// `edgeLabel`: `₿ 0.001` for 100,000 sat.
fn edge_label(sat: i64) -> String {
    let p = (8.0 - (sat as f64).log10()).round().max(0.0) as usize;
    btc(sat as f64 / 1e8, p, false)
}

fn confirmed(r: &Value) -> bool {
    s(r, "confirmation_status") == Some("confirmed")
}

/// Seconds since the epoch of a block time.
fn block_secs(r: &Value) -> Option<f64> {
    s(r, "block_time").and_then(crate::format::parse_ts)
}

fn age_band(r: &Value, now: f64) -> &'static str {
    let Some(at) = block_secs(r).filter(|_| confirmed(r)) else {
        return "unconfirmed";
    };
    let days = (now - at) / 86_400.0;
    if days < 30.0 {
        "month"
    } else if days < 365.0 {
        "year"
    } else if days < 3.0 * 365.0 {
        "threeYears"
    } else {
        "older"
    }
}

struct Band {
    key: String,
    label: String,
    count: usize,
    sat: i64,
}

fn band_list(
    tk: &Tk,
    title: &str,
    bands: &[Band],
    total: usize,
    color: Color,
    hide: bool,
    mask: bool,
) -> El {
    let mut list = col().w_full();
    let n = bands.len();
    for (i, b) in bands.iter().enumerate() {
        let share = if total > 0 {
            b.count as f32 / total as f32
        } else {
            0.0
        };
        let amount = btc(b.sat as f64 / 1e8, 8, false);
        let bar = {
            let mut track = row()
                .grow(1.0)
                .min_w(0.0)
                .h(tk.r(0.5))
                .radius(tk.r(0.25))
                .fill(tk.muted)
                .clip_rounded(tk.r(0.25))
                .a11y_hidden();
            if !mask && b.count > 0 {
                track = track.child(
                    el().h_full()
                        .w_frac(share)
                        .min_w(4.0)
                        .radius(tk.r(0.25))
                        .fill(color),
                );
            }
            track
        };
        let right: El = if mask {
            sensitive(
                tc("connections:detail.coins.count", 0, &[]),
                tk.txs(400.0),
                tk.muted_fg,
                true,
            )
        } else if b.count > 0 {
            row()
                .gap(tk.r(0.375))
                .child(
                    text(tc("connections:detail.coins.count", b.count, &[]))
                        .font(tk.txs(500.0).features("'tnum' 1"))
                        .color(tk.fg),
                )
                .child(sensitive(
                    amount.clone(),
                    tk.txs(400.0).features("'tnum' 1"),
                    tk.muted_fg,
                    hide,
                ))
        } else {
            text("—").font(tk.txs(400.0)).color(tk.muted_fg)
        };
        let mut item = row()
            .key(format!("wd-band-{}", b.key))
            .w_full()
            .py(tk.r(0.5))
            .gap(tk.r(0.75))
            .center_y()
            .child(
                row().w(tk.r(7.5)).child(
                    text(b.label.clone())
                        .font(tk.txs(400.0).features("'tnum' 1"))
                        .color(tk.fg)
                        .truncate(),
                ),
            )
            .child(bar)
            .child(row().min_w(tk.r(9.0)).justify_end().child(right));
        if !mask {
            item = item.help(tc(
                "connections:detail.coins.bandTitle",
                b.count,
                &[
                    ("amount", &if hide { "—".to_owned() } else { amount }),
                    ("share", &format!("{}", (share * 100.0).round() as i64)),
                ],
            ));
        }
        if i + 1 < n {
            item = item
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border.alpha(tk.border.a * 0.6))
                .border_layout();
        }
        list = list.child(item);
    }
    col()
        .grow(1.0)
        .min_w(0.0)
        .child(
            text(title.to_owned())
                .font(tk.txs(500.0))
                .color(tk.muted_fg)
                .mb(tk.r(0.25))
                .role("heading"),
        )
        .child(list)
}

/// `WalletCoinsBreakdown` (nothing when the wallet holds no BTC coins).
fn breakdown(app: &App, tk: &Tk, inv: &Value, width: f32) -> Option<El> {
    let hide = app.prefs.hide_sensitive();
    let all = rows(inv);
    let coins: Vec<&Value> = all
        .iter()
        .filter(|r| {
            matches!(
                s(r, "asset").unwrap_or("").to_uppercase().as_str(),
                "BTC" | "LBTC" | "L-BTC"
            )
        })
        .collect();
    if coins.is_empty() {
        return None;
    }
    let mut sizes: Vec<Band> = SIZE_EDGES
        .iter()
        .enumerate()
        .map(|(i, e)| Band {
            key: format!("size-{e}"),
            label: if i == 0 {
                tr(
                    "connections:detail.coins.size.below",
                    &[("max", &edge_label(*e))],
                )
            } else {
                tr(
                    "connections:detail.coins.size.between",
                    &[
                        ("min", &edge_label(SIZE_EDGES[i - 1])),
                        ("max", &edge_label(*e)),
                    ],
                )
            },
            count: 0,
            sat: 0,
        })
        .collect();
    sizes.push(Band {
        key: "size-top".into(),
        label: tr(
            "connections:detail.coins.size.atLeast",
            &[("min", &edge_label(SIZE_EDGES[3]))],
        ),
        count: 0,
        sat: 0,
    });
    let order = ["unconfirmed", "month", "year", "threeYears", "older"];
    let mut ages: Vec<Band> = order
        .iter()
        .map(|k| Band {
            key: (*k).into(),
            label: t(&format!("connections:detail.coins.age.{k}")),
            count: 0,
            sat: 0,
        })
        .collect();
    let now = crate::format::now_secs();
    for cn in &coins {
        let v = sat(cn);
        let i = SIZE_EDGES
            .iter()
            .position(|e| v < *e)
            .unwrap_or(sizes.len() - 1);
        sizes[i].count += 1;
        sizes[i].sat += v;
        let a = age_band(cn, now);
        if let Some(b) = ages.iter_mut().find(|b| b.key == a) {
            b.count += 1;
            b.sat += v;
        }
    }
    if ages[0].count == 0 {
        ages.remove(0);
    }
    let mut amounts: Vec<i64> = coins.iter().map(|c| sat(c)).collect();
    amounts.sort_unstable();
    let largest = *amounts.last().unwrap_or(&0);
    let smallest = *amounts.first().unwrap_or(&0);
    let mid = amounts.len() / 2;
    let median = if amounts.len() % 2 == 1 {
        amounts[mid]
    } else {
        ((amounts[mid - 1] + amounts[mid]) as f64 / 2.0).round() as i64
    };
    let total = inv.get("summary").and_then(|x| f(x, "count")).filter(|_| {
        inv.get("summary")
            .and_then(|x| x.get("truncated"))
            .and_then(Value::as_bool)
            == Some(true)
    });
    let mut title = row().gap(tk.r(0.5)).center_y().flex_wrap().child(
        text(t("connections:detail.coins.title"))
            .font(tk.tsm(500.0))
            .color(tk.fg),
    );
    if let Some(tot) = total.filter(|tot| *tot as usize > all.len()) {
        title = title.child(
            text(tr(
                "connections:detail.coins.partial",
                &[("loaded", &count(all.len() as f64)), ("total", &count(tot))],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        );
    }
    let extremes = tr(
        "connections:detail.coins.extremes",
        &[
            ("largest", &btc(largest as f64 / 1e8, 8, false)),
            ("median", &btc(median as f64 / 1e8, 8, false)),
            ("smallest", &btc(smallest as f64 / 1e8, 8, false)),
        ],
    );
    let color = tk.chart_btc;
    let n = coins.len();
    let two = width >= tk.r(56.0);
    let lists = if two {
        row()
            .w_full()
            .gap(tk.r(2.0))
            .items_start()
            .child(band_list(
                tk,
                &t("connections:detail.coins.bySize"),
                &sizes,
                n,
                color,
                hide,
                hide,
            ))
            .child(band_list(
                tk,
                &t("connections:detail.coins.byAge"),
                &ages,
                n,
                color,
                hide,
                false,
            ))
    } else {
        col()
            .w_full()
            .gap(tk.r(1.0))
            .child(band_list(
                tk,
                &t("connections:detail.coins.bySize"),
                &sizes,
                n,
                color,
                hide,
                hide,
            ))
            .child(band_list(
                tk,
                &t("connections:detail.coins.byAge"),
                &ages,
                n,
                color,
                hide,
                false,
            ))
    };
    Some(
        ui::card(tk)
            .key("wd-coin-mix")
            .child(
                ui::strip(tk)
                    .justify_between()
                    .flex_wrap()
                    .child(title)
                    .child(sensitive(
                        extremes,
                        tk.txs(400.0).features("'tnum' 1"),
                        tk.muted_fg,
                        hide,
                    )),
            )
            .child(col().w_full().px(tk.r(1.0)).py(tk.r(0.75)).child(lists)),
    )
}

// ---- UTXO inventory ---------------------------------------------------------------------------

fn amount_msat(r: &Value) -> f64 {
    f(r, "amount_msat")
        .or_else(|| f(r, "amount_sat").map(|x| x * 1000.0))
        .or_else(|| f(r, "amount"))
        .unwrap_or(0.0)
}

fn outpoint(r: &Value) -> &str {
    s(r, "outpoint").unwrap_or("")
}

/// `compareText` (numeric, case-insensitive) approximated: digit runs compare by value.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    bi.next();
                }
                let o = na
                    .trim_start_matches('0')
                    .len()
                    .cmp(&nb.trim_start_matches('0').len())
                    .then(na.trim_start_matches('0').cmp(nb.trim_start_matches('0')));
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase());
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
                ai.next();
                bi.next();
            }
        }
    }
}

/// `sortUtxosForDisplay`.
pub fn sorted(mut rows: Vec<Value>, sort: Sort) -> Vec<Value> {
    if sort == Sort::Default {
        return rows;
    }
    rows.sort_by(|a, b| {
        let base = match sort {
            Sort::SizeDesc | Sort::SizeAsc => amount_msat(a).total_cmp(&amount_msat(b)),
            Sort::DateDesc | Sort::DateAsc => block_secs(a)
                .unwrap_or(0.0)
                .total_cmp(&block_secs(b).unwrap_or(0.0)),
            Sort::OutpointAsc | Sort::OutpointDesc => natural_cmp(outpoint(a), outpoint(b)),
            Sort::Default => std::cmp::Ordering::Equal,
        };
        let base = if matches!(sort, Sort::SizeDesc | Sort::DateDesc | Sort::OutpointDesc) {
            base.reverse()
        } else {
            base
        };
        base.then_with(|| natural_cmp(outpoint(a), outpoint(b)))
    });
    rows
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Column {
    Outpoint,
    Amount,
    Confirmed,
}

fn column_sorts(c: Column) -> (Sort, Sort, Sort) {
    // (asc, desc, default)
    match c {
        Column::Outpoint => (Sort::OutpointAsc, Sort::OutpointDesc, Sort::OutpointAsc),
        Column::Amount => (Sort::SizeAsc, Sort::SizeDesc, Sort::SizeDesc),
        Column::Confirmed => (Sort::DateAsc, Sort::DateDesc, Sort::DateDesc),
    }
}

/// `nextSortForColumn`.
fn next_sort(cur: Sort, c: Column) -> Sort {
    let (asc, desc, def) = column_sorts(c);
    if cur == def {
        return if def == desc { asc } else { desc };
    }
    if cur == asc || cur == desc {
        return Sort::Default;
    }
    def
}

fn direction(cur: Sort, c: Column) -> Option<bool> {
    let (asc, desc, _) = column_sorts(c);
    if cur == asc {
        Some(true)
    } else if cur == desc {
        Some(false)
    } else {
        None
    }
}

/// `formatOutpoint`: `f591a36e…5794b0:3`.
fn fmt_outpoint(v: &str) -> String {
    let (txid, vout) = v.split_once(':').unwrap_or((v, "?"));
    if txid.is_empty() {
        return v.to_owned();
    }
    let head: String = txid.chars().take(8).collect();
    let tail: String = txid
        .chars()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}:{vout}")
}

fn fmt_address(v: &str) -> String {
    if v.chars().count() <= 24 {
        return v.to_owned();
    }
    let head: String = v.chars().take(12).collect();
    let tail: String = v
        .chars()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

fn location(r: &Value) -> String {
    let branch = s(r, "branch_label");
    if let (Some(b), Some(i)) = (branch, r.get("address_index").and_then(Value::as_i64)) {
        return tr(
            "connections:utxos.locationLabel",
            &[("branch", b), ("index", &i.to_string())],
        );
    }
    s(r, "address_label")
        .or(branch)
        .map(str::to_owned)
        .unwrap_or_else(|| t("connections:utxos.watchTarget"))
}

fn status_label(r: &Value) -> String {
    if confirmed(r) {
        match r
            .get("confirmations")
            .and_then(Value::as_f64)
            .filter(|c| *c > 0.0)
        {
            Some(c) => tc("connections:utxos.confLabel", c as usize, &[]),
            None => t("connections:utxos.confirmedLabel"),
        }
    } else {
        t("connections:utxos.mempool")
    }
}

/// `toLocaleString(locale, { dateStyle: "medium", timeStyle: "short" })` in local time:
/// en `Sep 17, 2019, 12:47 PM`, de-AT `17.09.2019, 12:47`.
pub fn date_label(iso: &str) -> String {
    let Some(secs) = crate::format::parse_ts(iso) else {
        return iso.to_owned();
    };
    let local = secs + local_offset(secs) as f64;
    let day = crate::format::Day::from_epoch_days((local / 86_400.0).floor() as i64);
    let tod = local.rem_euclid(86_400.0) as i64;
    let (hh, mm) = (tod / 3600, (tod % 3600) / 60);
    match crate::i18n::lang() {
        crate::i18n::Lang::De => {
            format!("{:02}.{:02}.{}, {:02}:{:02}", day.d, day.m, day.y, hh, mm)
        }
        crate::i18n::Lang::En => {
            let (h12, ampm) = match hh {
                0 => (12, "AM"),
                1..=11 => (hh, "AM"),
                12 => (12, "PM"),
                _ => (hh - 12, "PM"),
            };
            format!("{}, {}:{:02}\u{202f}{ampm}", day.long(), h12, mm)
        }
    }
}

/// The local timezone's UTC offset (seconds) at `secs`, as the webview's `Date` applies it.
fn local_offset(secs: f64) -> i64 {
    crate::localtime::offset_secs(secs as i64)
}

fn utxo_amount_text(r: &Value) -> String {
    let v = f(r, "amount")
        .map(|x| format!("{x:.8}"))
        .unwrap_or_default();
    match s(r, "asset").unwrap_or("BTC") {
        "BTC" => format!("₿ {v}"),
        a => format!("{v} {a}"),
    }
}

/// `AmountDisplay`: `₿ 0.54271688` for BTC; other assets carry their ticker once.
fn amount_display(tk: &Tk, r: &Value, hide: bool, style: rune::TextStyle) -> El {
    let v = f(r, "amount")
        .map(|x| format!("{x:.8}"))
        .unwrap_or_else(|| r.get("amount").map(|x| x.to_string()).unwrap_or_default());
    let asset = s(r, "asset").unwrap_or("BTC");
    if asset == "BTC" {
        sensitive(format!("₿ {v}"), style.features("'tnum' 1"), tk.fg, hide)
    } else {
        row()
            .gap(tk.r(0.25))
            .items_end()
            .child(sensitive(v, style.features("'tnum' 1"), tk.fg, hide))
            .child(
                text(asset.to_owned())
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            )
    }
}

fn sort_head(tk: &Tk, label: &str, col_: Column, cur: Sort, right: bool) -> El {
    let dir = direction(cur, col_);
    let key = match col_ {
        Column::Outpoint => "outpoint",
        Column::Amount => "amount",
        Column::Confirmed => "confirmed",
    };
    // The table kit's sortable header label.
    let b = crate::ui::table::sort_label(
        tk,
        format!("wd-utxo-sort-{key}"),
        label,
        label.to_owned(),
        dir,
        right,
    )
    .on_press(move |a: &mut App| {
        a.wallets.detail.utxo_sort = next_sort(a.wallets.detail.utxo_sort, col_);
        a.wallets.detail.utxo_visible = PAGE;
    });
    if right {
        row().w_full().justify_end().child(b)
    } else {
        b
    }
}

fn empty_state(app: &App, tk: &Tk, title: &str, body: &str) -> El {
    let running = super::sync_running(app);
    let mut glyph = ic(&I::REFRESH_CW, tk.r(1.0), tk.fg);
    if running {
        glyph = glyph.looping(rune::LoopSpec::rotation(1.0));
    }
    let label = t(if running {
        "connections:utxos.refreshing"
    } else {
        "connections:utxos.refresh"
    });
    let mut b = button_surface(
        tk,
        row()
            .key("wd-utxo-refresh")
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
        b = b.on_press(|a: &mut App| super::actions::sync(a, false));
    }
    col()
        .w_full()
        .px(tk.r(1.25))
        .py(tk.r(2.0))
        .gap(tk.r(0.75))
        .items_start()
        .child(
            col()
                .gap(tk.r(0.25))
                .child(text(title.to_owned()).font(tk.tsm(500.0)).color(tk.fg))
                .child(
                    text(body.to_owned())
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                ),
        )
        .child(b)
}

fn inventory(app: &App, tk: &Tk, width: f32) -> El {
    let d = &app.wallets.detail;
    let hide = app.prefs.hide_sensitive();
    let inv = d.utxos.ready();
    let all = inv.map(rows).unwrap_or_default();
    let total = inv
        .and_then(|i| {
            i.get("summary")
                .and_then(|x| f(x, "count"))
                .or_else(|| i.get("freshness").and_then(|x| f(x, "active_count")))
        })
        .unwrap_or(all.len() as f64);
    let returned = inv
        .and_then(|i| i.get("summary").and_then(|x| f(x, "returned_count")))
        .unwrap_or(all.len() as f64);
    let truncated = inv
        .and_then(|i| {
            i.get("summary")
                .and_then(|x| x.get("truncated"))
                .and_then(Value::as_bool)
        })
        .unwrap_or(false);
    let unsupported = inv.and_then(|i| {
        i.get("support")
            .and_then(|x| x.get("supported"))
            .and_then(Value::as_bool)
    }) == Some(false);
    let liquid_blocked = inv.and_then(|i| i.get("support").and_then(|x| s(x, "status")))
        == Some("liquid_unblind_blocked");
    let title = t(if liquid_blocked {
        "connections:utxos.liquidNeedsUnblinding"
    } else {
        "connections:utxos.unavailableTitle"
    });
    let pending = all.iter().filter(|r| !confirmed(r)).count();
    let summary = match &d.utxos {
        Read::Loading => t("connections:utxos.loading"),
        Read::Failed(_) => t("connections:utxos.couldNotLoadTitle"),
        Read::Ready(_) if unsupported => title.clone(),
        Read::Ready(_) if truncated => tr(
            "connections:utxos.countOf",
            &[("returned", &count(returned)), ("total", &count(total))],
        ),
        Read::Ready(_) if pending > 0 => tc(
            "connections:utxos.summary.withPending",
            total as usize,
            &[("pending", &pending.to_string())],
        ),
        Read::Ready(_) => tc(
            "connections:utxos.summary.allConfirmed",
            total as usize,
            &[],
        ),
    };
    let head = ui::strip(tk)
        .justify_between()
        .child(
            text(t("connections:utxos.title"))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        )
        .child(
            text(summary)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .truncate(),
        );
    let card = ui::card(tk).key("wd-utxos").child(head);
    let content: El = match &d.utxos {
        Read::Loading => col()
            .w_full()
            .padding(tk.r(1.0))
            .gap(tk.r(0.75))
            .child(ui::skeleton(tk, None, tk.r(2.25)))
            .child(ui::skeleton(tk, None, tk.r(2.25)))
            .child(ui::skeleton(tk, Some(tk.r(30.0)), tk.r(2.25))),
        Read::Failed(e) => empty_state(app, tk, &t("connections:utxos.couldNotLoadTitle"), e),
        Read::Ready(i) if unsupported => {
            let msg = i
                .get("support")
                .and_then(|x| s(x, "message"))
                .map(str::to_owned)
                .unwrap_or_else(|| t("connections:utxos.unsupportedFallback"));
            col().w_full().px(tk.r(1.25)).py(tk.r(2.0)).child(
                row()
                    .gap(tk.r(0.75))
                    .items_start()
                    .child(ic(&I::TRIANGLE_ALERT, tk.r(1.0), h("#d97706")).mt(tk.r(0.125)))
                    .child(
                        col()
                            .gap(tk.r(0.25))
                            .child(text(title).font(tk.tsm(500.0)).color(tk.fg))
                            .child(text(msg).font(tk.tsm(400.0)).color(tk.muted_fg).wrap()),
                    ),
            )
        }
        Read::Ready(_) if all.is_empty() => empty_state(
            app,
            tk,
            &t("connections:utxos.noneTitle"),
            &t("connections:utxos.noneBody"),
        ),
        Read::Ready(i) => table(app, tk, i, all, total, returned, truncated, hide, width),
    };
    card.child(content)
}

#[allow(clippy::too_many_arguments)]
fn table(
    app: &App,
    tk: &Tk,
    inv: &Value,
    all: Vec<Value>,
    total: f64,
    returned: f64,
    truncated: bool,
    hide: bool,
    width: f32,
) -> El {
    let d = &app.wallets.detail;
    let sorted_rows = sorted(all, d.utxo_sort);
    let shown = d.utxo_visible.min(sorted_rows.len());
    let mut c = col().w_full();
    if inv
        .get("freshness")
        .and_then(|x| x.get("stale"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        let when = inv
            .get("freshness")
            .and_then(|x| s(x, "last_synced_at").or_else(|| s(x, "last_seen_at")))
            .map(date_label);
        let (fill, ink) = if tk.dark {
            (h("#451a03").alpha(0.3), h("#fde68a"))
        } else {
            (h("#fffbeb"), h("#92400e"))
        };
        c = c.child(
            row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.625))
                .gap(tk.r(0.5))
                .items_start()
                .fill(fill)
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .border_layout()
                .child(ic(&I::TRIANGLE_ALERT, tk.r(0.875), ink).mt(tk.r(0.125)))
                .child(
                    text(match when {
                        Some(w) => tr("connections:utxos.staleWithDate", &[("date", &w)]),
                        None => t("connections:utxos.stale"),
                    })
                    .font(tk.txs(400.0))
                    .color(ink)
                    .wrap(),
                ),
        );
    }
    // The table sits in `CardContent p-0` but the rows keep the table's `p-2` cells; widths
    // follow the browser's auto layout at this width.
    // The browser's auto table layout: each column's widest cell, then the spare width shared
    // in proportion to those widths.
    let shown_rows = &sorted_rows[..shown];
    let chars = |s: &str| s.chars().count() as f32;
    let hdr = |k: &str| chars(&t(k)) * 7.4 + 20.0;
    let amount_w = shown_rows
        .iter()
        .map(|r| chars(&utxo_amount_text(r)) * 7.0)
        .fold(hdr("connections:utxos.column.amount"), f32::max);
    let loc_w = shown_rows
        .iter()
        .map(|r| {
            (chars(&location(r)) * 6.6).max(
                s(r, "address")
                    .map(|a| chars(&fmt_address(a)) * 6.5 + 22.0)
                    .unwrap_or(0.0),
            )
        })
        .fold(hdr("connections:utxos.column.location"), f32::max);
    let conf_w = shown_rows
        .iter()
        .map(|r| {
            chars(
                &s(r, "block_time")
                    .map(date_label)
                    .unwrap_or_else(|| "—".into()),
            ) * 5.7
        })
        .fold(hdr("connections:utxos.column.confirmed"), f32::max);
    let status_w = shown_rows
        .iter()
        .map(|r| chars(&status_label(r)) * 5.7 + 18.0)
        .fold(hdr("connections:utxos.column.status"), f32::max);
    let op_w = shown_rows
        .iter()
        .map(|r| chars(&fmt_outpoint(outpoint(r))) * 6.5 + 26.0)
        .fold(hdr("connections:utxos.column.outpoint"), f32::max);
    let pad = tk.r(1.0);
    let content = [amount_w, loc_w, conf_w, status_w, op_w];
    let total_w: f32 = content.iter().sum::<f32>() + 5.0 * pad;
    let avail = (width - 2.0 - 2.0 * tk.r(1.0)).max(total_w);
    let spare = avail - total_w;
    let sum: f32 = content.iter().sum();
    let col_w = |c: f32| c + pad + spare * c / sum;
    let (w_amount, w_loc, w_conf, w_status) = (
        col_w(amount_w),
        col_w(loc_w),
        col_w(conf_w),
        col_w(status_w),
    );
    let mut hr = crate::ui::table::head(tk)
        .px(tk.r(1.0))
        .h(tk.r(crate::ui::table::HEAD_H))
        .center_y();
    hr = hr
        .child(row().w(w_amount).px(tk.r(0.5)).child(sort_head(
            tk,
            &t("connections:utxos.column.amount"),
            Column::Amount,
            d.utxo_sort,
            false,
        )))
        .child(
            row()
                .w(w_loc)
                .px(tk.r(0.5))
                .child(crate::ui::table::th_text(
                    tk,
                    t("connections:utxos.column.location"),
                )),
        )
        .child(row().w(w_conf).px(tk.r(0.5)).child(sort_head(
            tk,
            &t("connections:utxos.column.confirmed"),
            Column::Confirmed,
            d.utxo_sort,
            false,
        )))
        .child(
            row()
                .w(w_status)
                .px(tk.r(0.5))
                .child(crate::ui::table::th_text(
                    tk,
                    t("connections:utxos.column.status"),
                )),
        )
        .child(row().grow(1.0).min_w(0.0).px(tk.r(0.5)).child(sort_head(
            tk,
            &t("connections:utxos.column.outpoint"),
            Column::Outpoint,
            d.utxo_sort,
            true,
        )));
    let mut body = col().w_full().px(tk.r(1.0));
    for (i, r) in sorted_rows.iter().take(shown).enumerate() {
        let reference = s(r, "transaction_id")
            .or_else(|| s(r, "txid"))
            .unwrap_or("")
            .to_owned();
        let op = outpoint(r).to_owned();
        let key = s(r, "id").unwrap_or(&op).to_string();
        let copied_op = d.is_copied(&format!("op-{key}"), app.clock);
        let copied_ad = d.is_copied(&format!("ad-{key}"), app.clock);
        let mempool = !confirmed(r);
        let mut loc = col().min_w(0.0).child(crate::ui::table::cell_text(
            location(r),
            tk.tsm(400.0),
            tk.fg,
        ));
        if let Some(addr) = s(r, "address") {
            loc = loc.child(
                row()
                    .min_w(0.0)
                    .gap(tk.r(0.25))
                    .center_y()
                    .child(
                        sensitive(fmt_address(addr), tk.txs(400.0).mono(), tk.muted_fg, hide)
                            .truncate(),
                    )
                    .child(ui::copy_button(
                        tk,
                        &format!("ad-{key}"),
                        addr,
                        Some(t("connections:utxos.copyAddress")),
                        true,
                        copied_ad,
                    )),
            );
        }
        // A chip only while the coin waits for a block; a confirmed one shows none.
        let status = if confirmed(r) {
            el()
        } else {
            crate::ui::table::chip(tk, crate::ui::table::Tone::Info, None, status_label(r))
        };
        let rf = reference.clone();
        let op_btn = row()
            .key(format!("wd-utxo-op-{key}"))
            .role("button")
            .label(if hide {
                crate::ui::hidden_amount()
            } else {
                fmt_outpoint(&op)
            })
            .help(tr(
                "connections:utxos.openLocalTransaction",
                &[("outpoint", &fmt_outpoint(&op))],
            ))
            .focusable(true)
            .on_press(move |a: &mut App| super::actions::open_utxo_transaction(a, &rf))
            .child(sensitive(
                fmt_outpoint(&op),
                tk.txs(400.0).mono(),
                tk.fg,
                hide,
            ));
        let rf2 = reference.clone();
        let mut tr_ = row()
            .key(format!("wd-utxo-{key}"))
            .w_full()
            .min_h(tk.r(3.6))
            .center_y()
            .fill(if mempool {
                tk.muted.alpha(0.2)
            } else {
                Color::TRANSPARENT
            })
            .hover(tk.muted.alpha(0.35))
            .role("button")
            .label(utxo_label(r, hide))
            .focusable(true)
            .on_press(move |a: &mut App| super::actions::open_utxo_transaction(a, &rf2))
            .child(row().w(w_amount).px(tk.r(0.5)).child(amount_display(
                tk,
                r,
                hide,
                tk.tsm(500.0),
            )))
            .child(row().w(w_loc).px(tk.r(0.5)).child(loc))
            .child(
                row().w(w_conf).px(tk.r(0.5)).child(
                    text(
                        s(r, "block_time")
                            .map(date_label)
                            .unwrap_or_else(|| "—".into()),
                    )
                    .font(tk.txs(400.0).features("'tnum' 1"))
                    .color(tk.muted_fg),
                ),
            )
            .child(row().w(w_status).px(tk.r(0.5)).child(status))
            .child(
                row()
                    .grow(1.0)
                    .min_w(0.0)
                    .px(tk.r(0.5))
                    .gap(tk.r(0.25))
                    .justify_end()
                    .center_y()
                    .child(op_btn)
                    .child(ui::copy_button(
                        tk,
                        &format!("op-{key}"),
                        &op,
                        Some(t("connections:utxos.copyOutpoint")),
                        true,
                        copied_op,
                    )),
            );
        if i + 1 < shown {
            tr_ = tr_
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .border_layout();
        }
        body = body.child(tr_);
    }
    // shadcn's `overflow-x-auto` table container: below the columns' least widths the rows
    // scroll sideways inside the card instead of running past its edge.
    let overflow = total_w > width - 2.0 - 2.0 * tk.r(1.0) + 0.5;
    let grid = col().w(avail + 2.0 * tk.r(1.0)).child(hr).child(body);
    c = c.child(crate::ui::table::hscroll_if(
        "wd-utxo-scroll",
        t("connections:utxos.title"),
        overflow,
        grid,
    ));
    if shown < sorted_rows.len() || truncated {
        let (loaded, tot) = if truncated {
            (returned, total)
        } else {
            (shown as f64, sorted_rows.len() as f64)
        };
        let mut foot = row()
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
                    "connections:utxos.loadedRows",
                    &[("loaded", &count(loaded)), ("total", &count(tot))],
                ))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
            );
        if shown < sorted_rows.len() {
            let more = (sorted_rows.len() - shown).min(PAGE);
            let len = sorted_rows.len();
            foot = foot.child(
                super::txs::ghost_xs(
                    tk,
                    "wd-utxo-more",
                    &tc("connections:utxos.showMore", more, &[]),
                    None,
                )
                .on_press(move |a: &mut App| {
                    a.wallets.detail.utxo_visible = (a.wallets.detail.utxo_visible + PAGE).min(len)
                }),
            );
        }
        c = c.child(foot);
    }
    c
}

/// The row's accessible name: its cells' text (amount, location, address, date, status,
/// outpoint), masked when values are hidden.
fn utxo_label(r: &Value, hide: bool) -> String {
    let mask = |v: String| {
        if hide {
            crate::theme::MASK.to_owned()
        } else {
            v
        }
    };
    let v = f(r, "amount")
        .map(|x| format!("{x:.8}"))
        .unwrap_or_default();
    let asset = s(r, "asset").unwrap_or("BTC");
    let amount = if asset == "BTC" {
        format!("₿ {v}")
    } else {
        format!("{v} {asset}")
    };
    let mut parts = vec![mask(amount), location(r)];
    if let Some(a) = s(r, "address") {
        parts.push(mask(fmt_address(a)));
    }
    parts.push(
        s(r, "block_time")
            .map(date_label)
            .unwrap_or_else(|| "—".into()),
    );
    parts.push(status_label(r));
    parts.push(mask(fmt_outpoint(outpoint(r))));
    parts.join(" ")
}

pub fn panel(app: &App, tk: &Tk, width: f32) -> Vec<El> {
    let mut out = vec![];
    if let Some(inv) = app.wallets.detail.utxos.ready()
        && let Some(b) = breakdown(app, tk, inv, width)
    {
        out.push(b);
    }
    out.push(inventory(app, tk, width));
    out
}
