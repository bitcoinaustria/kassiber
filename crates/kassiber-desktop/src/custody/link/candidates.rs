//! The list "Link manually" picks one side from (`LinkCandidates.tsx`): the book's sends or
//! receipts in one aligned table, ranked against the side already chosen. One search field and
//! one time (or year) choice; when the time leaves nothing that fits, the list says what it
//! looked through and offers the next wider time. Rows come from `ui.transactions.list`, a
//! local read.

use std::collections::HashSet;

use rune::{Cx, El, Key, KeyEvent, TextEdit, Tx, col, el, row, text};
use rune_kit::{Placement, TextInput};
use serde_json::{Value, json};

use super::super::kit::{self, Entry, Var};
use super::super::link_model::{
    self, Blocked, Candidate, Difference, FeeCheck, LegSide, LinkLeg, PICK_LIMIT, RECEIPT_WINDOWS,
    bps_text, date_part, date_time, time_part, window_key,
};
use super::super::model;
use super::super::unlink::common;
use crate::app::{App, Reply};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::popup::TriggerExt;
use crate::ui::{ic, sensitive};

/// The first year a bitcoin transaction can carry.
const FIRST_YEAR: i32 = 2009;

/// The picker's own state; a new side or anchor starts it over (`key`).
pub struct Pick {
    pub key: String,
    pub search: TextEdit,
    pub hours: Option<u32>,
    pub year: Option<i32>,
    /// The list read: the args it was made with, and what came back.
    pub args: Option<String>,
    pub rows: Option<Vec<Value>>,
    pub has_more: Option<bool>,
    pub loading: bool,
    pub fetching: bool,
    pub failed: bool,
}

impl Default for Pick {
    fn default() -> Self {
        Pick {
            key: String::new(),
            search: TextEdit::new(""),
            hours: Some(72),
            year: None,
            args: None,
            rows: None,
            has_more: None,
            loading: false,
            fetching: false,
            failed: false,
        }
    }
}

fn list_args(side: LegSide, anchor: Option<&LinkLeg>, p: &Pick) -> Value {
    let (query, _) = link_model::search_intent(&p.search.text);
    let (since, until) = match anchor {
        Some(a) if side == LegSide::Receipt => {
            link_model::window_bounds(a.occurred_at.as_deref(), p.hours)
        }
        Some(a) => link_model::window_bounds_before(a.occurred_at.as_deref(), p.hours),
        None => link_model::year_bounds(p.year),
    };
    let mut args = json!({"direction": if side == LegSide::Send { "outbound" } else { "inbound" }, "limit": PICK_LIMIT});
    if !query.is_empty() {
        args["query"] = json!(query);
    }
    if let Some(s) = since {
        args["since"] = json!(s);
    }
    if let Some(u) = until {
        args["until"] = json!(u);
    }
    args
}

/// Starts the list read when its args changed (`keepPreviousData`: the old rows stay).
pub fn poll(app: &mut App, side: LegSide, anchor: Option<&LinkLeg>) {
    let key = format!("{side:?}:{}", anchor.map(|a| a.id.as_str()).unwrap_or(""));
    if app.custody.link.pick.key != key {
        app.custody.link.pick = Pick {
            key,
            ..Pick::default()
        };
    }
    let args = list_args(side, anchor, &app.custody.link.pick);
    let s = args.to_string();
    if app.custody.link.pick.args.as_deref() == Some(s.as_str()) {
        return;
    }
    let p = &mut app.custody.link.pick;
    p.args = Some(s);
    if p.rows.is_none() {
        p.loading = true;
    }
    p.fetching = true;
    app.send("ui.transactions.list", args, "custody.link.list");
}

pub fn on_list(app: &mut App, r: &Reply) {
    let p = &mut app.custody.link.pick;
    p.loading = false;
    p.fetching = false;
    p.failed = r.error.is_some();
    if r.error.is_none() {
        p.rows = Some(r.data["txs"].as_array().cloned().unwrap_or_default());
        p.has_more = r.data["hasMore"].as_bool();
    }
}

fn refetch(app: &mut App) {
    app.custody.link.pick.args = None;
    app.custody.link.pick.failed = false;
}

/// The legs the list holds, with what their links leave open.
pub fn legs(app: &App, side: LegSide) -> Vec<LinkLeg> {
    let parts = super::super::open_parts(app);
    app.custody
        .link
        .pick
        .rows
        .as_ref()
        .map(|rows| {
            rows.iter()
                .filter_map(|tx| link_model::leg_from_tx(tx, Some(side)))
                .map(|l| link_model::with_open_part(l, parts))
                .collect()
        })
        .unwrap_or_default()
}

/// The ranked rows.
pub fn rows(app: &App, side: LegSide, anchor: Option<&LinkLeg>) -> Vec<Candidate> {
    let legs = legs(app, side);
    let mut suggested = HashSet::new();
    if let Some(a) = anchor {
        for c in app.custody.page.suggest.as_deref().unwrap_or(&[]) {
            let (o, i) = (
                c["out_id"].as_str().unwrap_or(""),
                c["in_id"].as_str().unwrap_or(""),
            );
            if side == LegSide::Receipt && o == a.id {
                suggested.insert(i.to_owned());
            }
            if side == LegSide::Send && i == a.id {
                suggested.insert(o.to_owned());
            }
        }
    }
    let ranked = match anchor {
        Some(a) => link_model::rank_candidates(a, &legs, &suggested),
        None => link_model::list_candidates(&legs),
    };
    link_model::by_typed_amount(
        ranked,
        link_model::search_intent(&app.custody.link.pick.search.text).1,
    )
}

fn relative_time(minutes: i64) -> String {
    let m = minutes.unsigned_abs() as usize;
    let time = if m < 90 {
        tc("review:transfersPage.link.minutes", m, &[])
    } else if m < 48 * 60 {
        tc(
            "review:transfersPage.link.hours",
            ((m as f64) / 60.0).round() as usize,
            &[],
        )
    } else {
        tc(
            "review:transfersPage.link.days",
            ((m as f64) / 1440.0).round() as usize,
            &[],
        )
    };
    tr(
        if minutes >= 0 {
            "review:transfersPage.link.after"
        } else {
            "review:transfersPage.link.before"
        },
        &[("time", &time)],
    )
}

/// A difference in a few words (`differenceShort`).
fn difference_short(d: &Difference) -> String {
    match d {
        Difference::OtherAsset => t("review:transfersPage.link.diff.otherAsset"),
        Difference::Fee {
            kind: FeeCheck::None,
            ..
        } => t("review:transfersPage.link.diff.same"),
        Difference::Fee {
            kind: FeeCheck::Negative,
            amount_msat,
            ..
        } => tr(
            "review:transfersPage.link.diff.more",
            &[("amount", &model::msat_sats(*amount_msat))],
        ),
        Difference::Fee {
            kind: FeeCheck::Mismatch,
            share_bps,
            ..
        } => tr(
            "review:transfersPage.link.diff.lessShare",
            &[("share", &bps_text(share_bps.unwrap_or(0)))],
        ),
        Difference::Fee { amount_msat, .. } => tr(
            "review:transfersPage.link.diff.less",
            &[("amount", &model::msat_sats(*amount_msat))],
        ),
    }
}

fn odd(d: Option<&Difference>) -> bool {
    matches!(
        d,
        Some(Difference::Fee {
            kind: FeeCheck::Negative | FeeCheck::Mismatch,
            ..
        })
    )
}

/// `transactionTypeLabel`: the daemon's stable English type, in the UI language.
fn type_label(v: Option<&str>) -> String {
    let Some(v) = v.filter(|s| !s.is_empty()) else {
        return String::new();
    };
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
        other => return other.to_owned(),
    };
    t(&format!("transactions:type.{key}"))
}

fn set_hours(app: &mut App, h: Option<u32>) {
    app.custody.link.pick.hours = h;
}

fn row_key(i: usize) -> String {
    format!("link-candidate-{i}")
}

/// The picker. `wide`: the frame is a dialog (`@md/@lg/@2xl/pick` columns shown).
#[allow(clippy::too_many_arguments)]
pub fn view(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    side: LegSide,
    anchor: Option<&LinkLeg>,
    selected: Option<&str>,
    wide: bool,
) -> El {
    let p = &app.custody.link.pick;
    // "Link several": the rows of the several side tick instead of picking.
    let ticking = super::multi::ticking(app, side);
    let rows = rows(app, side, anchor);
    let legs_n = legs(app, side).len();
    let receipt = side == LegSide::Receipt;
    let window_label = t(if receipt {
        "review:transfersPage.link.windowAfter"
    } else {
        "review:transfersPage.link.windowBefore"
    });
    // The search field.
    let n_rows = rows.len();
    let input = TextInput::new(&p.search)
        .label(t("review:transfersPage.link.search"))
        .placeholder(t("review:transfersPage.link.searchPlaceholder"))
        .leading_icon(&I::SEARCH)
        .leading_icon_size(tk.r(0.875))
        .height(tk.r(2.0))
        .text_style(tk.tsm(400.0))
        .on_edit(|a: &mut App| &mut a.custody.link.pick.search)
        .on_key(move |_a: &mut App, ev: &KeyEvent, tx: &mut Tx| {
            if ev.key == Key::Down && n_rows > 0 {
                tx.focus(row_key(0), true);
                return true;
            }
            false
        })
        .build(&tk.kit())
        .key("link-search")
        .grow(1.0)
        .min_w(0.0);
    // The time (or year) choice.
    let (sel_label, sel_value) = if anchor.is_some() {
        (
            window_label.clone(),
            t(&format!(
                "review:transfersPage.link.window.{}",
                window_key(p.hours)
            )),
        )
    } else {
        (
            t("review:transfersPage.link.year"),
            p.year
                .map(|y| y.to_string())
                .unwrap_or_else(|| t("review:transfersPage.link.window.any")),
        )
    };
    let menu_open = app.custody.link.menu == Some("window");
    let trigger =
        crate::txdetail::widgets::select_trigger(tk, "link-window", &sel_label, &sel_value, true)
            .w(tk.r(9.5))
            .h(tk.r(2.0))
            .select_trigger(menu_open, |a: &mut App| {
                a.custody.link.menu = if a.custody.link.menu == Some("window") {
                    None
                } else {
                    Some("window")
                };
            });
    if menu_open {
        let mut entries = vec![];
        if anchor.is_some() {
            for h in RECEIPT_WINDOWS {
                entries.push(Entry::Radio {
                    key: format!("link-window-{}", window_key(h)),
                    label: t(&format!(
                        "review:transfersPage.link.window.{}",
                        window_key(h)
                    )),
                    trailing: None,
                    checked: p.hours == h,
                    disabled: false,
                    on: std::rc::Rc::new(move |a: &mut App| set_hours(a, h)),
                });
            }
        } else {
            entries.push(Entry::Radio {
                key: "link-year-any".into(),
                label: t("review:transfersPage.link.window.any"),
                trailing: None,
                checked: p.year.is_none(),
                disabled: false,
                on: std::rc::Rc::new(|a: &mut App| a.custody.link.pick.year = None),
            });
            let now =
                crate::format::Day::from_epoch_days((crate::format::now_secs() / 86_400.0) as i64)
                    .y;
            for y in (FIRST_YEAR..=now).rev() {
                entries.push(Entry::Radio {
                    key: format!("link-year-{y}"),
                    label: y.to_string(),
                    trailing: None,
                    checked: p.year == Some(y),
                    disabled: false,
                    on: std::rc::Rc::new(move |a: &mut App| a.custody.link.pick.year = Some(y)),
                });
            }
        }
        kit::select_list(
            tk,
            layer,
            "link-window-menu",
            "link-window",
            Placement::BottomEnd,
            Some(tk.r(9.5)),
            entries,
            |a: &mut App| a.custody.link.menu = None,
        );
    }
    let filters = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .role("group")
        .label(t("review:transfersPage.link.filters"))
        .child(input)
        .child(trigger);

    let text_on = !p.search.text.trim().is_empty();
    let narrowed = text_on
        || if anchor.is_some() {
            p.hours.is_some()
        } else {
            p.year.is_some()
        };
    let wider = if anchor.is_some() && !text_on {
        link_model::wider_window(p.hours)
    } else {
        None
    };
    let searched = match (wider, p.hours) {
        (Some(_), Some(_)) => Some(tr(
            if receipt {
                "review:transfersPage.link.searchedReceipts"
            } else {
                "review:transfersPage.link.searchedSends"
            },
            &[(
                "span",
                &t(&format!(
                    "review:transfersPage.link.span.{}",
                    window_key(p.hours)
                )),
            )],
        )),
        _ => None,
    };
    let widen = || {
        wider.map(|w| {
            kit::btn_xs(
                tk,
                "link-widen",
                Var::Outline,
                None,
                &t(&format!(
                    "review:transfersPage.link.widen.{}",
                    window_key(w)
                )),
                true,
            )
            .on_press(move |a: &mut App| set_hours(a, w))
        })
    };
    let thin = !rows.is_empty()
        && !rows
            .iter()
            .any(|r| r.blocked.is_none() && (r.suggested || r.same_txid || r.close));

    // The table.
    let mut boxed = col()
        .key("link-pick-list")
        .w_full()
        .min_h(tk.r(11.0))
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .busy(p.fetching)
        .clip_rounded(tk.r(0.5));
    if p.loading {
        boxed = boxed.child(
            row()
                .w_full()
                .h(tk.r(6.0))
                .center()
                .gap(tk.r(0.5))
                .role("status")
                .child(kit::spinner(tk, tk.r(1.0), tk.muted_fg))
                .child(
                    text(t("review:transfersPage.link.loading"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                ),
        );
    } else if p.failed {
        boxed = boxed.child(
            col()
                .w_full()
                .padding(tk.r(1.0))
                .gap(tk.r(0.5))
                .role("alert")
                .child(
                    text(t("review:transfersPage.link.unavailable"))
                        .font(tk.tsm(400.0))
                        .color(tk.destructive),
                )
                .child(
                    row().child(
                        kit::btn(
                            tk,
                            "link-list-retry",
                            Var::Outline,
                            None,
                            &t("review:transfersPage.effects.retry"),
                            true,
                        )
                        .on_press(refetch),
                    ),
                ),
        );
    } else if rows.is_empty() {
        let mut c = col()
            .key("link-pick-empty")
            .w_full()
            .padding(tk.r(1.0))
            .gap(tk.r(0.25));
        c = c.child(
            text(searched.clone().unwrap_or_else(|| {
                t(if receipt {
                    "review:transfersPage.link.emptyReceipts"
                } else {
                    "review:transfersPage.link.emptySends"
                })
            }))
            .font(tk.tsm(500.0))
            .color(tk.fg)
            .wrap(),
        );
        if searched.is_some() {
            if let Some(w) = widen() {
                c = c.child(row().pt(tk.r(0.25)).child(w));
            }
        } else {
            c = c.child(
                text(t(if narrowed {
                    "review:transfersPage.link.emptyNarrowed"
                } else {
                    "review:transfersPage.link.emptyBook"
                }))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
            );
        }
        if narrowed && searched.is_none() {
            c = c.child(
                row().mt(tk.r(0.5)).child(
                    kit::btn(
                        tk,
                        "link-show-all",
                        Var::Outline,
                        None,
                        &t("review:transfersPage.link.showAll"),
                        true,
                    )
                    .on_press(|a: &mut App| {
                        let p = &mut a.custody.link.pick;
                        p.search = TextEdit::new("");
                        p.hours = None;
                        p.year = None;
                    }),
                ),
            );
        }
        boxed = boxed.child(c);
    } else {
        let anchored = anchor.is_some();
        let (w_date, w_amount, w_diff, w_booked, w_link) = (
            if wide { tk.r(7.75) } else { tk.r(7.0) },
            tk.r(8.0),
            if wide && anchored && !ticking {
                Some(tk.r(8.5))
            } else {
                None
            },
            if wide { Some(tk.r(7.5)) } else { None },
            tk.r(2.25),
        );
        let th = |s: String, w: Option<f32>, right: bool, pl: f32| {
            let mut c = row().py(tk.r(0.375)).pl(pl).pr(tk.r(0.5)).center_y();
            c = match w {
                Some(w) => c.w(w),
                None => c.grow(1.0).w(0.0).min_w(0.0),
            };
            if right {
                c = c.justify_end();
            }
            c.h(tk.r(crate::ui::table::HEAD_H_COMPACT))
                .py(0.0)
                .child(crate::ui::table::th_text_compact(tk, s))
        };
        // The table kit's compact header band (the picker's cells are `text-xs`).
        let mut head = crate::ui::table::head(tk);
        head = head
            .child(th(
                t("review:transfersPage.link.col.when"),
                Some(w_date),
                false,
                tk.r(0.75),
            ))
            .child(th(
                t("review:transfersPage.link.col.wallet"),
                None,
                false,
                tk.r(0.5),
            ))
            .child(th(
                t("review:transfersPage.link.col.amount"),
                Some(w_amount),
                true,
                tk.r(0.5),
            ));
        if let Some(w) = w_diff {
            head = head.child(th(
                t("review:transfersPage.link.col.difference"),
                Some(w),
                true,
                tk.r(0.5),
            ));
        }
        if let Some(w) = w_booked {
            head = head.child(th(
                t("review:transfersPage.link.col.bookedAs"),
                Some(w),
                false,
                tk.r(0.5),
            ));
        }
        head = head.child(
            row()
                .w(w_link)
                .label(t("review:transfersPage.showInTransactions")),
        );
        let mut body = col()
            .w_full()
            .role("grid")
            .label(t(if receipt {
                "review:transfersPage.link.receiptsLabel"
            } else {
                "review:transfersPage.link.sendsLabel"
            }))
            .focus_group();
        let n = rows.len();
        for (i, r) in rows.iter().enumerate() {
            let tick = ticking.then(|| {
                (
                    super::multi::is_ticked(app, &r.leg.id),
                    anchor.is_some_and(|a| super::multi::tickable(a, &r.leg)),
                )
            });
            body = body.child(candidate_row(
                app,
                cx,
                tk,
                i,
                n,
                r,
                anchored,
                selected == Some(r.leg.id.as_str()),
                side,
                wide,
                (w_date, w_amount, w_diff, w_booked, w_link),
                tick,
            ));
        }
        boxed =
            boxed
                .child(head)
                .child(body.max_h(tk.r(if ticking { 14.0 } else { 24.0 })).scroll(
                    rune::ScrollSpec {
                        stick_bottom: false,
                    },
                ));
    }
    let mut c = col()
        .key("link-pick")
        .w_full()
        .gap(tk.r(0.5))
        .child(filters)
        .child(boxed);
    if thin && searched.is_some() && !p.loading {
        let span = t(&format!(
            "review:transfersPage.link.span.{}",
            window_key(p.hours)
        ));
        let mut line = row()
            .key("link-pick-thin")
            .w_full()
            .gap(tk.r(0.75))
            .center_y()
            .flex_wrap()
            .child(
                text(tr(
                    if receipt {
                        "review:transfersPage.link.onlyReceipts"
                    } else {
                        "review:transfersPage.link.onlySends"
                    },
                    &[("span", &span)],
                ))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
            );
        if let Some(w) = widen() {
            line = line.child(w);
        }
        c = c.child(line);
    }
    let limited = p.has_more.unwrap_or(legs_n as u32 >= PICK_LIMIT);
    if limited && !rows.is_empty() {
        c = c.child(
            text(tc(
                if anchor.is_some() {
                    "review:transfersPage.link.limitedWindow"
                } else {
                    "review:transfersPage.link.limited"
                },
                legs_n,
                &[],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .wrap()
            .key("link-pick-limited"),
        );
    }
    c
}

#[allow(clippy::too_many_arguments)]
fn candidate_row(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    i: usize,
    n: usize,
    r: &Candidate,
    anchored: bool,
    selected: bool,
    side: LegSide,
    wide: bool,
    w: (f32, f32, Option<f32>, Option<f32>, f32),
    tick: Option<(bool, bool)>,
) -> El {
    let (w_date, w_amount, w_diff, w_booked, w_link) = w;
    let hidden = app.prefs.hide_sensitive();
    let leg = &r.leg;
    // Ticking: (ticked, may be ticked). A row the core would not take in a batch reads as blocked.
    let selected = tick.map_or(selected, |(on, _)| on);
    let blocked = r.blocked.is_some() || tick.is_some_and(|(_, ok)| !ok);
    let difference = if anchored && !blocked && tick.is_none() {
        r.difference.as_ref().map(difference_short)
    } else {
        None
    };
    let diff_color = if odd(r.difference.as_ref()) {
        kit::amber_text(tk)
    } else {
        tk.muted_fg
    };
    // When.
    let mut when = col()
        .w(w_date)
        .py(tk.r(0.5))
        .pl(tk.r(0.75))
        .pr(tk.r(0.5))
        .help(date_time(leg.occurred_at.as_deref()))
        .child(
            text(date_part(leg.occurred_at.as_deref()))
                .font(tk.tsm(400.0).features("'tnum' 1"))
                .color(if blocked { tk.muted_fg } else { tk.fg }),
        );
    if anchored && let Some(minutes) = r.minutes {
        when = when.child(
            text(relative_time(minutes))
                .font(tk.txs(400.0))
                .color(if r.before {
                    kit::amber_text(tk)
                } else {
                    tk.muted_fg
                }),
        );
    } else if let Some(tm) = time_part(leg.occurred_at.as_deref()) {
        when = when.child(text(tm).font(tk.txs(400.0)).color(tk.muted_fg));
    }
    // Wallet and what to know about it.
    let mut name = row().w_full().min_w(0.0).gap(tk.r(0.375)).center_y();
    if let Some((on, ok)) = tick {
        name = name.child(super::multi::check_box(tk, on, ok).a11y_hidden());
    } else if selected {
        name = name.child(ic(&I::CHECK, tk.r(0.875), tk.primary).a11y_hidden());
    } else if wide {
        name = name.child(kit::rail_icon(app, cx, tk, leg.rail));
    }
    name = name.child(kit::wallet_name(
        tk,
        &leg.wallet,
        if blocked {
            tk.tsm(400.0)
        } else {
            tk.tsm(500.0)
        },
        if blocked { tk.muted_fg } else { tk.fg },
        hidden,
    ));
    let mut notes = col()
        .w_full()
        .min_w(0.0)
        .mt(tk.r(0.125))
        .gap(tk.r(0.125))
        .items_start();
    let mut any_note = false;
    let small = |s: String, c| text(s).font(tk.txs(400.0)).color(c).wrap();
    match r.blocked {
        Some(Blocked::Linked) => {
            let s = match leg.link.as_ref().and_then(|l| l.wallet.clone()) {
                Some(wl) => tr("review:transfersPage.link.linkedTo", &[("wallet", &wl)]),
                None => t("review:transfersPage.link.alreadyLinked"),
            };
            notes = notes.child(sensitive(s, tk.txs(400.0), tk.muted_fg, hidden).wrap());
            any_note = true;
        }
        Some(Blocked::Ignored) => {
            notes = notes.child(small(t("review:transfersPage.link.ignored"), tk.muted_fg));
            any_note = true;
        }
        Some(Blocked::Rest) => {
            notes = notes.child(small(t("review:transfersPage.link.restOnly"), tk.muted_fg));
            any_note = true;
        }
        None => {
            let green = |label: String, sparkle: bool| {
                let fg = kit::tw(if tk.dark {
                    "emerald-300"
                } else {
                    "emerald-700"
                });
                let mut b = row()
                    .gap(tk.r(0.25))
                    .center_y()
                    .px(tk.r(0.375))
                    .radius(tk.r(0.375))
                    .border(1.0, kit::tw("emerald-400").alpha(0.4))
                    .fill(kit::tw("emerald-400").alpha(0.1));
                if sparkle {
                    b = b.child(ic(&I::SPARKLES, tk.r(0.75), fg).a11y_hidden());
                }
                b.child(text(label).font(tk.t2xs(500.0)).color(fg))
            };
            if r.suggested {
                notes = notes.child(green(t("review:transfersPage.link.suggested"), true));
                any_note = true;
            } else if r.same_txid {
                notes = notes.child(green(t("review:transfersPage.link.sameTxid"), false));
                any_note = true;
            } else if r.close {
                notes = notes.child(
                    row()
                        .px(tk.r(0.375))
                        .radius(tk.r(0.375))
                        .border(1.0, tk.border)
                        .child(
                            text(t("review:transfersPage.link.closeMatch"))
                                .font(tk.t2xs(500.0))
                                .color(tk.fg),
                        ),
                );
                any_note = true;
            }
            if link_model::leg_has_open_part(leg) {
                let left = link_model::linkable_msat(leg);
                let total = leg.open.map(|o| o.linked_msat).unwrap_or(0) + left;
                notes = notes.child(
                    sensitive(
                        tr(
                            "review:transfersPage.link.partLeft",
                            &[
                                ("left", &model::msat_btc(left)),
                                ("total", &model::msat_btc(total)),
                            ],
                        ),
                        tk.txs(400.0).features("'tnum' 1"),
                        tk.muted_fg,
                        hidden,
                    )
                    .key(format!("link-candidate-part-{i}")),
                );
                any_note = true;
            }
            if r.same_wallet {
                notes = notes.child(small(
                    t("review:transfersPage.link.sameWallet"),
                    kit::amber_text(tk),
                ));
                any_note = true;
            }
            if leg.held && w_booked.is_none() {
                notes = notes.child(small(
                    t("review:transfersPage.link.booked.held"),
                    kit::amber_text(tk),
                ));
                any_note = true;
            }
        }
    }
    if r.blocked.is_none() && tick.is_some_and(|(_, ok)| !ok) {
        notes = notes.child(
            small(super::strings::only_one_to_one(), tk.muted_fg)
                .key(format!("link-candidate-one-{i}")),
        );
        any_note = true;
    }
    let mut wallet = col()
        .grow(1.0)
        .w(0.0)
        .min_w(0.0)
        .py(tk.r(0.5))
        .px(tk.r(0.5))
        .child(name);
    if any_note {
        wallet = wallet.child(notes);
    }
    // Amount, the difference under it when there is no column for it.
    let style = if blocked {
        tk.tsm(400.0)
    } else {
        tk.tsm(500.0)
    };
    let mut amount = col()
        .w(w_amount)
        .py(tk.r(0.5))
        .px(tk.r(0.5))
        .items_end()
        .child(common::leg_amount(
            app,
            tk,
            &format!("link-cand-amt-{i}"),
            leg,
            style,
            false,
        ));
    if let (Some(d), None) = (&difference, w_diff) {
        amount = amount.child(sensitive(
            d.clone(),
            tk.txs(400.0).features("'tnum' 1"),
            diff_color,
            hidden,
        ));
    }
    let mut rw = row()
        .key(row_key(i))
        .w_full()
        .items_start()
        .role("row")
        .checked(selected)
        .label(format!(
            "{} {} {}",
            date_part(leg.occurred_at.as_deref()),
            leg.wallet,
            model::msat_in(leg.amount_msat, &leg.asset)
        ))
        .focusable(true)
        .focus_highlight(tk.muted.alpha(0.5))
        .on_key(move |_a: &mut App, ev: &KeyEvent, tx: &mut Tx| {
            let to = match ev.key {
                Key::Down => Some((i + 1).min(n - 1)),
                Key::Up => Some(i.saturating_sub(1)),
                Key::Home => Some(0),
                Key::End => Some(n - 1),
                _ => None,
            };
            match to {
                Some(j) => {
                    tx.focus(row_key(j), true);
                    true
                }
                None => false,
            }
        })
        .child(when)
        .child(wallet)
        .child(amount);
    if i + 1 < n {
        rw = rw
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .border_layout();
    }
    if selected {
        rw = rw.fill(tk.primary.alpha(0.05));
    }
    if blocked {
        rw = rw.disabled(true);
    } else {
        let l = leg.clone();
        rw = if tick.is_some() {
            rw.hover(tk.muted.alpha(0.5))
                .on_press(move |a: &mut App| super::multi::toggle(a, l.clone()))
        } else {
            rw.hover(tk.muted.alpha(0.5))
                .on_press(move |a: &mut App| super::choose(a, side, l.clone()))
        };
    }
    if let Some(wd) = w_diff {
        rw = rw.child(
            row()
                .w(wd)
                .py(tk.r(0.5))
                .px(tk.r(0.5))
                .justify_end()
                .child(sensitive(
                    difference.clone().unwrap_or_else(|| "—".into()),
                    tk.tsm(400.0).features("'tnum' 1"),
                    diff_color,
                    hidden,
                )),
        );
    }
    if let Some(wb) = w_booked {
        let booked = if leg.excluded {
            t("review:transfersPage.link.booked.ignored")
        } else if leg.held {
            t("review:transfersPage.link.booked.held")
        } else {
            Some(type_label(leg.booked_as.as_deref()))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "—".into())
        };
        rw = rw.child(
            row().w(wb).py(tk.r(0.5)).px(tk.r(0.5)).child(
                text(booked)
                    .font(tk.tsm(400.0))
                    .color(if leg.held && !leg.excluded {
                        kit::amber_text(tk)
                    } else {
                        tk.muted_fg
                    })
                    .truncate(),
            ),
        );
    }
    let id = leg.id.clone();
    rw.child(
        row()
            .w(w_link)
            .py(tk.r(0.375))
            .pr(tk.r(0.5))
            .justify_end()
            .child(
                kit::icon_btn(
                    tk,
                    format!("link-cand-open-{i}"),
                    Var::Ghost,
                    tk.r(1.5),
                    &I::ARROW_UP_RIGHT,
                    tk.r(0.875),
                    t("review:transfersPage.showInTransactions"),
                )
                .on_press(move |a: &mut App| {
                    super::close(a);
                    super::super::show_transaction(a, &id, None);
                }),
            ),
    )
    .child(el().size(0.0, 0.0))
}
