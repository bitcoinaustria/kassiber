//! The Changes surface (assistant-panel/surfaces/ChangesSurface.tsx): recent edits to the book,
//! newest first, one line per edit that unfolds into each field's before and after, with undo.
//! It reads the Activity page's history (`ui.activity.history`, `limit: 30`, `include_stale:
//! false`, `ai_only` for "Assistant") and reverts through the same
//! `ui.transactions.history.revert` (after a confirm, then a notification). `row` and its
//! field diffs are shared with the Activity page.

use std::collections::HashSet;

use rune::{Color, El, Tx, col, row, text};
use serde_json::{Value, json};

use super::surfaces::{self, EmptyTone, hex};
use super::time;
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button, sens};
use crate::ui::ic;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    All,
    Ai,
}

#[derive(Default)]
pub struct State {
    pub source: Option<Source>,
    pub events: Vec<Value>,
    pub next_cursor: Option<String>,
    pub loading: bool,
    pub fetching: bool,
    pub fetching_next: bool,
    pub error: bool,
    /// The args of the list on screen (a reply for other args is dropped).
    pub args_key: String,
    pub expanded: HashSet<String>,
    /// The field whose header is under the pointer (its revert button shows).
    pub hover_field: Option<String>,
    pub confirm: Option<(Value, Option<Value>)>,
    pub reverting: bool,
    was_processing: bool,
    /// History reads in flight, in send order: (args, is a next page). None: asked before the
    /// surface closed, dropped when it answers.
    pending: Vec<(Option<String>, bool)>,
}

impl State {
    /// The surface closed: its state goes, but the reads still out keep their place so a
    /// reopened surface does not take their answers for its own.
    pub fn close(&mut self) {
        let pending = self.pending.iter().map(|&(_, more)| (None, more)).collect();
        *self = State {
            pending,
            ..State::default()
        };
    }

    pub fn source(&self) -> Source {
        self.source.unwrap_or(Source::All)
    }
}

fn args(s: &State) -> Value {
    let mut a = json!({"limit": 30, "include_stale": false});
    if s.source() == Source::Ai {
        a["ai_only"] = json!(true);
    }
    a
}

/// Every frame while the surface is a tab of the panel (mounted behind other tabs too, as the
/// web keeps it): the first page for the current filter, again after journals are processed.
pub fn poll(app: &mut App) -> bool {
    let processing = app.busy.processing;
    let s = &mut app.reports.diag.chg;
    if s.was_processing && !processing {
        s.args_key.clear();
    }
    s.was_processing = processing;
    let k = args(s).to_string();
    if k == s.args_key {
        return false;
    }
    s.args_key = k.clone();
    s.loading = true;
    s.fetching = true;
    s.error = false;
    s.events.clear();
    s.next_cursor = None;
    s.pending.push((Some(k.clone()), false));
    app.send(
        "ui.activity.history",
        serde_json::from_str(&k).unwrap_or(Value::Null),
        "reports.diag.chg.history",
    );
    true
}

fn refetch(app: &mut App) {
    let s = &mut app.reports.diag.chg;
    let k = args(s).to_string();
    s.args_key = k.clone();
    s.fetching = true;
    s.pending.push((Some(k.clone()), false));
    app.send(
        "ui.activity.history",
        serde_json::from_str(&k).unwrap_or(Value::Null),
        "reports.diag.chg.history",
    );
}

fn retry(app: &mut App) {
    app.reports.diag.chg.args_key.clear();
}

fn load_more(app: &mut App) {
    let s = &mut app.reports.diag.chg;
    let Some(cursor) = s.next_cursor.clone() else {
        return;
    };
    if s.fetching_next {
        return;
    }
    let mut a = args(s);
    a["cursor"] = json!(cursor);
    s.fetching_next = true;
    s.fetching = true;
    s.pending.push((Some(s.args_key.clone()), true));
    app.send("ui.activity.history", a, "reports.diag.chg.history");
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "reports.diag.chg.history" => {
            let s = &mut app.reports.diag.chg;
            let (k, more) = if s.pending.is_empty() {
                (None, false)
            } else {
                s.pending.remove(0)
            };
            if k.as_ref() != Some(&s.args_key) {
                s.fetching = !s.pending.is_empty();
                return;
            }
            s.fetching = !s.pending.is_empty();
            s.loading = false;
            if more {
                s.fetching_next = false;
            }
            if r.error.is_some() {
                s.error = true;
                return;
            }
            s.error = false;
            let evs = r.data["events"].as_array().cloned().unwrap_or_default();
            if more {
                s.events.extend(evs);
            } else {
                s.events = evs;
            }
            s.next_cursor = r.data["next_cursor"]
                .as_str()
                .map(str::to_owned)
                .filter(|c| !c.is_empty());
        }
        "reports.diag.chg.revert" => {
            app.reports.diag.chg.reverting = false;
            match &r.error {
                Some(e) => app.toast(
                    ToastTone::Error,
                    &t("review:activity.revert.failedTitle"),
                    Some(if e.is_empty() {
                        t("review:activity.revert.failedBody")
                    } else {
                        e.clone()
                    }),
                    None,
                ),
                None => app.toast(
                    ToastTone::Success,
                    &t("review:activity.revert.title"),
                    Some(t("review:activity.revert.body")),
                    None,
                ),
            }
            // The mutation invalidates the history: both lists read again.
            refetch(app);
            app.reports.diag.act.args_key_clear();
        }
        _ => {}
    }
}

/// `useActivityHistoryRevert().revert` for a confirmed target.
pub fn revert(app: &mut App, event: &Value, field: Option<&Value>, tag: &'static str) {
    let mut a = json!({
        "transaction": event["transaction_id"],
        "event": event["id"],
        "reason": match field {
            Some(f) => tr("review:activity.revert.reasonField", &[("label", f["label"].as_str().unwrap_or(""))]),
            None => t("review:activity.revert.reasonEvent"),
        },
    });
    if let Some(f) = field {
        a["field"] = f["field"].clone();
    }
    app.send("ui.transactions.history.revert", a, tag);
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

/// `historyTransactionLabel`.
pub fn tx_label(ev: &Value) -> String {
    [
        s(ev, "transaction_external_id"),
        ev["transaction"]["external_id"].as_str().unwrap_or(""),
        s(ev, "transaction_id"),
    ]
    .into_iter()
    .find(|v| !v.is_empty())
    .unwrap_or("")
    .to_owned()
}

/// `shortId`.
pub fn short_id(id: &str) -> String {
    let c: Vec<char> = id.chars().collect();
    if c.len() > 18 {
        format!(
            "{}…{}",
            c[..10].iter().collect::<String>(),
            c[c.len() - 6..].iter().collect::<String>()
        )
    } else {
        id.to_owned()
    }
}

/// `SOURCE_ICON` and `SOURCE_TONE`.
pub(super) fn source_glyph(tk: &Tk, source: &str) -> (&'static rune::Icon, Color) {
    match source {
        "ai_tool" => (&I::BOT, hex(if tk.dark { "#38bdf8" } else { "#0284c7" })),
        "gui" => (
            &I::MONITOR,
            hex(if tk.dark { "#34d399" } else { "#059669" }),
        ),
        _ => (&I::SQUARE_TERMINAL, tk.muted_fg),
    }
}

/// What a row does: the handlers differ between the panel and the Activity page (their own
/// expanded sets and confirms), the drawing does not.
pub struct RowCtx {
    pub key: &'static str,
    pub expanded: bool,
    pub hide: bool,
    pub reverting: bool,
    pub hover_field: Option<String>,
    pub toggle: fn(&mut App, &str),
    pub confirm: fn(&mut App, Value, Option<Value>),
    pub hover: fn(&mut App, Option<String>),
}

/// `ChangeRow`.
pub fn change_row(tk: &Tk, ev: &Value, c: &RowCtx) -> El {
    let id = s(ev, "id").to_owned();
    let (glyph, ink) = source_glyph(tk, s(ev, "source"));
    let stale = ev["report_anchor"]["stale_for_reports"].as_bool() == Some(true);
    let toggle = c.toggle;
    let tog = id.clone();
    let summary_btn = row()
        .key(format!("{}-ev-{id}", c.key))
        .grow(1.0)
        .min_w(0.0)
        .px(tk.r(0.25))
        .py(tk.r(0.125))
        .gap(tk.r(0.5))
        .items_start()
        .radius(tk.r(0.25))
        .role("button")
        .expanded(c.expanded)
        .label(s(ev, "summary").to_owned())
        .focusable(true)
        .on_press(move |a: &mut App| toggle(a, &tog))
        .child(row().pt(tk.r(0.125)).child(ic(
            if c.expanded {
                &I::CHEVRON_DOWN
            } else {
                &I::CHEVRON_RIGHT
            },
            tk.r(0.875),
            tk.muted_fg,
        )))
        .child(
            row()
                .pt(tk.r(0.125))
                .child(ic(glyph, tk.r(0.875), ink))
                .label(s(ev, "source_label").to_owned()),
        )
        .child(
            sens(
                tk,
                s(ev, "summary").to_owned(),
                tk.tsm(400.0),
                tk.fg,
                c.hide,
            )
            .truncate()
            .grow(1.0)
            .min_w(0.0),
        );
    let when = ev["changed_at"].as_str();
    let top = row()
        .w_full()
        .min_w(0.0)
        .px(tk.r(0.25))
        .py(tk.r(0.25))
        .gap(tk.r(0.25))
        .items_start()
        .radius(tk.r(0.375))
        .hover(tk.accent.alpha(0.4))
        .child(summary_btn)
        .child(
            text(time::history_relative(when, crate::format::now_secs()))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .help(time::history_date(when))
                .px(tk.r(0.25))
                .pt(tk.r(0.125)),
        );
    let txid = s(ev, "transaction_id").to_owned();
    let mut meta = row()
        .w_full()
        .min_w(0.0)
        .pb(tk.r(0.25))
        .pr(tk.r(0.5))
        .pl(tk.r(2.625))
        .gap(tk.r(0.375))
        .center_y()
        .child(
            row()
                .key(format!("{}-tx-{id}", c.key))
                .min_w(0.0)
                .shrink()
                .radius(tk.r(0.125))
                .role("link")
                .label(t("assistant:panel.changes.openTransaction"))
                .help(t("assistant:panel.changes.openTransaction"))
                .focusable(true)
                .on_press(move |a: &mut App| crate::txdetail::open(a, &txid, "details"))
                .child(
                    sens(
                        tk,
                        short_id(&tx_label(ev)),
                        rune::TextStyle::new(tk.r(0.6875), 400.0)
                            .lh(tk.r(1.0))
                            .mono(),
                        tk.muted_fg,
                        c.hide,
                    )
                    .truncate(),
                ),
        );
    if !s(ev, "wallet_label").is_empty() {
        meta = meta.child(
            sens(
                tk,
                format!("· {}", s(ev, "wallet_label")),
                tk.txs(400.0),
                tk.muted_fg,
                c.hide,
            )
            .truncate()
            .shrink()
            .min_w(0.0),
        );
    }
    let mut chips = row().gap(tk.r(0.25)).center_y();
    if stale {
        chips = chips.child(
            row()
                .px(tk.r(0.25))
                .radius(tk.r(0.125))
                .fill(hex("#f59e0b").alpha(0.1))
                .child(
                    text(t("transactions:history.staleReports"))
                        .font(tk.txs(400.0))
                        .color(hex(if tk.dark { "#fcd34d" } else { "#b45309" })),
                ),
        );
    }
    for fam in ev["families"].as_array().cloned().unwrap_or_default() {
        chips = chips.child(
            row()
                .px(tk.r(0.25))
                .radius(tk.r(0.125))
                .fill(tk.muted)
                .child(
                    text(fam.as_str().unwrap_or("").to_owned())
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                ),
        );
    }
    meta = meta.child(rune::el().grow(1.0)).child(chips);
    let mut li = col()
        .key(format!("{}-row-{id}", c.key))
        .w_full()
        .radius(tk.r(0.375))
        .role("listitem")
        .child(top)
        .child(meta);
    if c.expanded {
        li = li.fill(tk.muted.alpha(0.3));
        let mut body = col()
            .w_full()
            .px(tk.r(0.5))
            .pb(tk.r(0.5))
            .pl(tk.r(2.625))
            .gap(tk.r(0.5));
        if !s(ev, "reason").is_empty() {
            body = body.child(
                sens(
                    tk,
                    s(ev, "reason").to_owned(),
                    tk.txs(400.0),
                    tk.muted_fg,
                    c.hide,
                )
                .wrap(),
            );
        }
        for f in ev["fields"].as_array().cloned().unwrap_or_default() {
            body = body.child(field_diff(tk, ev, &f, c));
        }
        let e2 = ev.clone();
        let confirm = c.confirm;
        let mut undo = button(
            tk,
            format!("{}-undo-{id}", c.key),
            Variant::Outline,
            Size::Xs,
            Some(&I::ROTATE_CCW),
            &t("assistant:panel.changes.undo"),
            !c.reverting,
        );
        if !c.reverting {
            undo = undo.on_press(move |a: &mut App| confirm(a, e2.clone(), None));
        }
        body = body.child(row().child(undo));
        li = li.child(body);
    }
    li
}

/// `FieldDiff`: one field as a two-line diff, or its added and removed tags.
pub(super) fn field_diff(tk: &Tk, ev: &Value, f: &Value, c: &RowCtx) -> El {
    let label = s(f, "label").to_owned();
    let fid = format!("{}:{}", s(ev, "id"), s(f, "id"));
    let shown = c.hover_field.as_deref() == Some(fid.as_str());
    let (e2, f2) = (ev.clone(), f.clone());
    let confirm = c.confirm;
    let hover = c.hover;
    let aria = tr("transactions:history.revertFieldAria", &[("label", &label)]);
    let mut revert = row()
        .key(format!("{}-frevert-{fid}", c.key))
        .size(tk.r(1.5), tk.r(1.5))
        .center()
        .radius(tk.r(0.375))
        .hover(if tk.dark {
            tk.accent.alpha(0.5)
        } else {
            tk.accent
        })
        .role("button")
        .label(aria.clone())
        .help(aria)
        .focusable(!c.reverting)
        .disabled(c.reverting)
        .opacity(if shown {
            if c.reverting { 0.5 } else { 1.0 }
        } else {
            0.0
        })
        .child(ic(&I::ROTATE_CCW, tk.r(0.75), tk.muted_fg));
    if !c.reverting {
        revert = revert.on_press(move |a: &mut App| confirm(a, e2.clone(), Some(f2.clone())));
    }
    let hair = tk.border.alpha(tk.border.a * 0.7);
    let fid_h = fid.clone();
    let head = row()
        .w_full()
        .h(tk.r(1.75))
        .pl(tk.r(0.5))
        .pr(tk.r(0.25))
        .gap(tk.r(0.5))
        .center_y()
        .fill(tk.muted.alpha(0.4))
        .border_sides([0.0, 0.0, 1.0, 0.0], hair)
        .child(
            text(label)
                .font(tk.txs(500.0))
                .color(tk.fg)
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        )
        .child(
            text(s(f, "family").to_owned())
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        )
        .child(revert);
    let added: Vec<String> = f["diff"]["added"]
        .as_array()
        .map(|v| {
            v.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let removed: Vec<String> = f["diff"]["removed"]
        .as_array()
        .map(|v| {
            v.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let mut lines = col().w_full();
    if !added.is_empty() || !removed.is_empty() {
        for r in &removed {
            lines = lines.child(diff_line(tk, false, r, None, c.hide));
        }
        for a in &added {
            lines = lines.child(diff_line(tk, true, a, None, c.hide));
        }
    } else {
        lines = lines
            .child(diff_line(
                tk,
                false,
                s(f, "before_label"),
                Some(t("transactions:history.before")),
                c.hide,
            ))
            .child(diff_line(
                tk,
                true,
                s(f, "after_label"),
                Some(t("transactions:history.after")),
                c.hide,
            ));
    }
    let mut boxed = col()
        .w_full()
        .radius(tk.r(0.375))
        .border(1.0, hair)
        .border_layout()
        .fill(tk.background)
        .clip_rounded(tk.r(0.375))
        .on_hover_change(move |a: &mut App, on: bool, _: &mut Tx| {
            hover(a, if on { Some(fid_h.clone()) } else { None })
        })
        .child(head)
        .child(lines);
    if f["redacted"].as_bool() == Some(true) {
        boxed = boxed.child(
            row()
                .w_full()
                .px(tk.r(0.5))
                .py(tk.r(0.25))
                .border_sides([1.0, 0.0, 0.0, 0.0], hair)
                .child(
                    text(t("transactions:history.redacted"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                ),
        );
    }
    boxed
}

/// `DiffLine`: `+` in emerald, `−` in rose, `font-mono text-xs leading-5`.
fn diff_line(tk: &Tk, added: bool, value: &str, label: Option<String>, hide: bool) -> El {
    let (bg, ink) = if added {
        (
            hex("#10b981").alpha(0.1),
            hex(if tk.dark { "#6ee7b7" } else { "#065f46" }),
        )
    } else {
        (
            hex("#f43f5e").alpha(0.1),
            hex(if tk.dark { "#fda4af" } else { "#9f1239" }),
        )
    };
    let v = if value.is_empty() {
        "—".to_owned()
    } else {
        value.to_owned()
    };
    let style = tk.txs(400.0).lh(tk.r(1.25)).mono();
    let aria = match &label {
        Some(l) => format!("{l}: {v}"),
        None => v.clone(),
    };
    row()
        .w_full()
        .min_w(0.0)
        .px(tk.r(0.5))
        .gap(tk.r(0.5))
        .items_start()
        .fill(bg)
        .label(aria)
        .child(
            text(if added { "+" } else { "−" })
                .font(style)
                .color(ink.alpha(0.7))
                .w(tk.r(0.5))
                .a11y_hidden(),
        )
        .child(
            sens(tk, v, style, ink, hide)
                .wrap_anywhere()
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        )
}

/// The panel's row handlers.
fn panel_ctx(app: &App, id: &str) -> RowCtx {
    let st = &app.reports.diag.chg;
    RowCtx {
        key: "chg",
        expanded: st.expanded.contains(id),
        hide: app.prefs.hide_sensitive(),
        reverting: st.reverting,
        hover_field: st.hover_field.clone(),
        toggle: |a, id| {
            let e = &mut a.reports.diag.chg.expanded;
            if !e.remove(id) {
                e.insert(id.to_owned());
            }
        },
        confirm: |a, ev, f| a.reports.diag.chg.confirm = Some((ev, f)),
        hover: |a, f| a.reports.diag.chg.hover_field = f,
    }
}

/// The revert confirm (`confirmRevert`), mounted while one is asked.
pub fn mount_confirm(
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    pending: &Option<(Value, Option<Value>)>,
    ok: fn(&mut App, Value, Option<Value>),
    cancel: fn(&mut App),
) {
    let Some((ev, field)) = pending else { return };
    let label = match field {
        Some(f) => s(f, "label").to_owned(),
        None => t("transactions:history.confirmRevertAll"),
    };
    let (e2, f2) = (ev.clone(), field.clone());
    super::widgets::confirm(
        tk,
        layer,
        key,
        tr(
            "transactions:history.confirmRevertField",
            &[("label", &label)],
        ),
        move |a: &mut App, _tx: &mut Tx| ok(a, e2.clone(), f2.clone()),
        cancel,
    );
}

pub fn view(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let st = &app.reports.diag.chg;
    let n = st.events.len();
    let mut count = row().gap(tk.r(0.25)).center_y();
    if st.fetching && !st.loading {
        count = count.child(surfaces::spinner(tk, tk.r(0.75)));
    } else if n > 0 {
        count = count.child(
            text(tc(
                "assistant:panel.changes.count",
                n,
                &[("count", &n.to_string())],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        );
    }
    let toolbar = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .child(surfaces::segmented(
            tk,
            "chg-source",
            t("assistant:panel.changes.filter"),
            vec![
                (Source::All, None, t("assistant:panel.changes.all")),
                (
                    Source::Ai,
                    Some(&I::BOT),
                    t("assistant:panel.changes.byAssistant"),
                ),
            ],
            st.source(),
            |a, v| a.reports.diag.chg.source = Some(v),
        ))
        .child(rune::el().grow(1.0))
        .child(count);
    let scroll = |c: El| {
        col()
            .key("chg-scroll")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(rune::ScrollSpec {
                stick_bottom: false,
            })
            .child(c)
    };
    let body = if st.loading {
        scroll(
            col()
                .w_full()
                .min_h(tk.r(12.0))
                .grow(1.0)
                .center()
                .role("status")
                .label(t("assistant:panel.changes.loading"))
                .child(surfaces::spinner(tk, tk.r(1.0))),
        )
    } else if st.error && st.events.is_empty() {
        let again = button(
            tk,
            "chg-retry",
            Variant::Outline,
            Size::Sm,
            None,
            &t("assistant:panel.retry"),
            true,
        )
        .on_press(retry);
        scroll(surfaces::empty(
            tk,
            &I::TRIANGLE_ALERT,
            EmptyTone::Destructive,
            t("assistant:panel.changes.error"),
            None,
            Some(again),
        ))
    } else if st.events.is_empty() {
        let body = t(if st.source() == Source::Ai {
            "assistant:panel.changes.emptyAssistant"
        } else {
            "assistant:panel.changes.emptyBody"
        });
        scroll(surfaces::empty(
            tk,
            &I::GIT_COMPARE_ARROWS,
            EmptyTone::Muted,
            t("assistant:panel.changes.emptyTitle"),
            Some(body),
            None,
        ))
    } else {
        let mut list = col().w_full().padding(tk.r(0.5)).role("list");
        for ev in &st.events {
            list = list.child(change_row(tk, ev, &panel_ctx(app, s(ev, "id"))));
        }
        let mut c = col().w_full().child(list);
        if st.next_cursor.is_some() {
            let busy = st.fetching_next;
            let mut b = button(
                tk,
                "chg-more",
                Variant::Ghost,
                Size::Xs,
                None,
                &t(if busy {
                    "review:activity.loadingMore"
                } else {
                    "review:activity.loadMore"
                }),
                !busy,
            );
            if !busy {
                b = b.on_press(load_more);
            }
            c = c.child(
                row()
                    .w_full()
                    .px(tk.r(0.75))
                    .pb(tk.r(0.75))
                    .justify_center()
                    .child(b),
            );
        }
        scroll(c)
    };
    mount_confirm(
        tk,
        layer,
        "chg-confirm",
        &st.confirm,
        |a, ev, f| {
            a.reports.diag.chg.confirm = None;
            a.reports.diag.chg.reverting = true;
            revert(a, &ev, f.as_ref(), "reports.diag.chg.revert");
        },
        |a| a.reports.diag.chg.confirm = None,
    );
    surfaces::frame(
        tk,
        t("assistant:panel.surfaceChanges"),
        Some(toolbar),
        body,
        None,
    )
}

/// SYNTHETIC: the panel's list filled with the sample events (scenes only).
pub fn inject_sample(app: &mut App) {
    let s = &mut app.reports.diag.chg;
    s.events = sample_events();
    s.loading = false;
    s.fetching = false;
    s.error = false;
    s.expanded.insert("ev-1".into());
}

/// SYNTHETIC events (the demo book has no edit history): shared with the Activity page's
/// sample scene.
pub fn sample_events() -> Vec<Value> {
    let now = crate::format::now_secs();
    let at = |ago: f64| super::logstore::iso_from_secs(now - ago);
    vec![
        json!({"id": "ev-1", "transaction_id": "tx-1", "transaction_external_id": "f3a9c1e2b7d40c55e1a7c9d2f6b8a3e4d5c6b7a8", "wallet_label": "Regtest Cold Storage", "source": "gui", "source_label": "Desktop", "changed_at": at(3_600.0 * 3.0), "summary": "Changed label and note", "families": ["metadata"], "reason": "Cleanup before tax export", "fields": [
            {"id": "f-1", "field": "label", "label": "Label", "family": "metadata", "before_label": "", "after_label": "Salary", "diff": {}},
            {"id": "f-2", "field": "tags", "label": "Tags", "family": "metadata", "before_label": "income", "after_label": "income, payroll", "diff": {"added": ["payroll"], "removed": []}}
        ]}),
        json!({"id": "ev-2", "transaction_id": "tx-2", "source": "ai_tool", "source_label": "Assistant", "changed_at": at(86_400.0 * 1.2), "summary": "Set manual price", "families": ["pricing"], "report_anchor": {"stale_for_reports": true}, "fields": [
            {"id": "f-3", "field": "fiat_rate", "label": "Price", "family": "pricing", "before_label": "unpriced", "after_label": "€ 61,240.00", "diff": {}}
        ]}),
        json!({"id": "ev-3", "transaction_id": "tx-3", "source": "cli", "source_label": "CLI", "changed_at": at(86_400.0 * 6.0), "summary": "Marked as gift", "families": ["tax"], "fields": [
            {"id": "f-4", "field": "kind", "label": "Kind", "family": "tax", "before_label": "receive", "after_label": "gift", "diff": {}, "redacted": true}
        ]}),
    ]
}

#[cfg(test)]
mod close_tests {
    use super::*;

    fn reply(id: &str) -> Reply {
        Reply {
            kind: "ui.activity.history".into(),
            data: json!({ "events": [{ "id": id }] }),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        }
    }

    /// Closed while a next page was loading and opened again: that page is not taken for the
    /// reopened list's first page, which still lands.
    #[test]
    fn a_page_asked_before_closing_is_dropped() {
        let mut app = crate::app::tests::quiet_app();
        poll(&mut app);
        on_reply(&mut app, "reports.diag.chg.history", &reply("first"));
        let s = &mut app.reports.diag.chg;
        s.next_cursor = Some("c".into());
        load_more(&mut app);
        app.reports.diag.chg.close();
        poll(&mut app);
        on_reply(&mut app, "reports.diag.chg.history", &reply("old page"));
        assert!(app.reports.diag.chg.events.is_empty(), "the old page");
        assert!(app.reports.diag.chg.loading);
        on_reply(&mut app, "reports.diag.chg.history", &reply("new"));
        let s = &app.reports.diag.chg;
        assert_eq!(s.events, [json!({ "id": "new" })]);
        assert!(!s.loading && !s.fetching);
    }
}
