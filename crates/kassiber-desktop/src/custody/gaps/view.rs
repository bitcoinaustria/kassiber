//! Rendering of the two tabs (`CustodyGapsContent`, `CustodyGapCard`, `GapReviewDetails` and
//! its preview panels, `ReviewHistoryPanel`, `CustodyCoverageTimeline`).

use rune::{Color, Cx, El, Icon, Shadow, col, edit_access, el, row, text, text_input};
use rune_kit::Placement;
use serde_json::Value;

use super::model::{self, ActionMode, Gap, fmt_date, fmt_msat};
use super::{Detail, Rd};
use crate::app::App;
use crate::custody::kit::{self, Entry, Var};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::popup::TriggerExt;
use crate::ui::{ic, sensitive};

// ---- Primitives --------------------------------------------------------------------------------

/// shadcn `Card` with the tab's `gap-4 py-5` (or `gap-2 py-5`).
fn card(tk: &Tk, gap: f32) -> El {
    crate::ui::surface(tk).w_full().py(tk.card_pad()).gap(gap)
}

#[derive(Clone, Copy)]
enum Badge {
    Default,
    Secondary,
    Destructive,
    Outline,
}

/// A status label (`crate::ui::tag`): destructive ink for problems, else the text or muted ink.
fn badge(tk: &Tk, v: Badge, label: String) -> El {
    let fg = match v {
        Badge::Default => tk.fg,
        Badge::Destructive => tk.destructive,
        Badge::Secondary | Badge::Outline => tk.muted_fg,
    };
    crate::ui::tag(tk, fg, None, label)
}

fn muted(tk: &Tk, s: String) -> El {
    text(s).font(tk.tsm(400.0)).color(tk.muted_fg).wrap()
}

fn muted_xs(tk: &Tk, s: String) -> El {
    text(s).font(tk.txs(400.0)).color(tk.muted_fg).wrap()
}

fn amber_ink(tk: &Tk) -> Color {
    kit::tw(if tk.dark { "amber-200" } else { "amber-900" })
}

/// `rounded-md border bg-background/60 p-3`.
fn inner_box(tk: &Tk) -> El {
    col()
        .w_full()
        .padding(tk.r(0.75))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.background.alpha(0.6))
}

/// shadcn `Checkbox`-like native checkbox (`size-4`).
fn checkbox(tk: &Tk, key: &str, checked: bool, label: String) -> El {
    let side = tk.r(1.0);
    let mut b = row()
        .key(key.to_owned())
        .size(side, side)
        .mt(tk.r(0.125))
        .center()
        .radius(3.0)
        .role("checkbox")
        .checked(checked)
        .label(label)
        .focusable(true);
    if checked {
        b = b
            .fill(tk.primary)
            .border(1.0, tk.primary)
            .border_layout()
            .child(ic(&I::CHECK, tk.r(0.75), tk.primary_fg));
    } else {
        b = b
            .fill(if tk.dark {
                tk.input.alpha(tk.input.a * 0.3)
            } else {
                tk.background
            })
            .border(1.0, tk.input)
            .border_layout();
    }
    b
}

/// `Textarea`: `min-h-16 rounded-md border border-input px-3 py-2 text-sm`, up to 500 chars.
fn textarea(
    tk: &Tk,
    key: &str,
    edit: &rune::TextEdit,
    placeholder: String,
    label: String,
    residual: bool,
) -> El {
    let input = text_input(edit, tk.tsm(400.0))
        .key(format!("{key}-input"))
        .w_full()
        .color(tk.fg)
        .placeholder(placeholder, tk.muted_fg)
        .caret(tk.fg, tk.primary.alpha(0.3))
        .label(label);
    let input = if residual {
        input
            .on_edit(edit_access(|s: &mut App| {
                &mut s.custody.gaps.detail.as_mut().expect("detail").residual
            }))
            .on_submit(|s: &mut App, _| {
                if let Some(d) = s.custody.gaps.detail.as_mut() {
                    d.residual.insert("\n");
                }
            })
            .on_input_change(|s: &mut App, _| edited(s, true))
    } else {
        input
            .on_edit(edit_access(|s: &mut App| {
                &mut s.custody.gaps.detail.as_mut().expect("detail").correction
            }))
            .on_submit(|s: &mut App, _| {
                if let Some(d) = s.custody.gaps.detail.as_mut() {
                    d.correction.insert("\n");
                }
            })
            .on_input_change(|s: &mut App, _| edited(s, false))
    };
    let mut b = col()
        .key(key.to_owned())
        .w_full()
        .min_h(tk.r(4.0))
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .radius(tk.r(0.375))
        .border(1.0, tk.input)
        .border_layout()
        .box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: Color::BLACK.alpha(0.05),
            ..Shadow::default()
        })
        .child(rune::auto_grow(input, 2, 10));
    if tk.dark {
        b = b.fill(tk.input.alpha(tk.input.a * 0.3));
    }
    b
}

/// A typed reason clears the shown plan (`setPendingPlan(null)`) and stays within 500 chars.
fn edited(s: &mut App, residual: bool) {
    if let Some(d) = s.custody.gaps.detail.as_mut() {
        d.pending = None;
        let e = if residual {
            &mut d.residual
        } else {
            &mut d.correction
        };
        if e.text.chars().count() > 500 {
            let cut: String = e.text.chars().take(500).collect();
            e.replace_text(cut);
        }
    }
}

// ---- Missing wallets ---------------------------------------------------------------------------

pub fn content(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    focus_gap: Option<&str>,
) -> El {
    let s = &app.custody.gaps;
    if s.list_rd != Rd::Ready {
        return skeleton(tk);
    }
    let snapshot = s.pages.first();
    if s.list_failed.is_some() || snapshot.is_none() {
        let body = s
            .list_failed
            .clone()
            .unwrap_or_else(|| t("custodyGaps:unavailable.body"));
        return card(tk, tk.r(0.5)).key("cg-unavailable").child(
            col()
                .w_full()
                .px(tk.card_pad())
                .gap(tk.r(0.375))
                .child(
                    text(t("custodyGaps:unavailable.title"))
                        .font(tk.tsm(600.0))
                        .color(tk.fg),
                )
                .child(muted(tk, body)),
        );
    }
    let snapshot = snapshot.unwrap();
    let summary = &snapshot["summary"];
    let gaps = model::collect_pages(&s.pages);
    let review: Vec<&Gap> = gaps.iter().filter(|g| g.in_queue()).collect();
    let canonical = summary["canonical_issue_count"]
        .as_i64()
        .unwrap_or(0)
        .max(0) as usize;
    let amounts: Vec<String> = summary["canonical_unresolved_by_asset"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| fmt_msat(&x["amount_msat"], x["asset"].as_str().unwrap_or("")))
                .collect()
        })
        .unwrap_or_default();
    let current = summary["derived_state_current"].as_bool() == Some(true);
    let complete = summary["search_complete"].as_bool() != Some(false);
    let clear = model::can_show_clear(summary, review.len());
    let needs = (summary["needs_review"].as_i64().unwrap_or(0).max(0) as usize).max(review.len());
    let hidden = app.prefs.hide_sensitive();

    let mut page = col().key("cg").w_full().gap(tk.r(0.75));
    page = page.child(
        text(t("custodyGaps:description"))
            .font(tk.tsm(400.0))
            .color(tk.muted_fg)
            .wrap()
            .max_w(tk.r(48.0)),
    );
    let verdict_el = if !current {
        Some(verdict(
            tk,
            false,
            &I::CLOCK_3,
            t("custodyGaps:processing.title"),
            t("custodyGaps:processing.body"),
            vec![],
            false,
        ))
    } else if needs > 0 {
        let body = if canonical > 0 {
            tc("custodyGaps:verdict.needsBlockingBody", canonical, &[])
        } else {
            t("custodyGaps:verdict.needsBody")
        };
        Some(verdict(
            tk,
            false,
            &I::TRIANGLE_ALERT,
            tc("custodyGaps:verdict.needs", needs, &[]),
            body,
            if canonical > 0 {
                amounts.clone()
            } else {
                vec![]
            },
            hidden,
        ))
    } else if canonical > 0 {
        Some(verdict(
            tk,
            false,
            &I::TRIANGLE_ALERT,
            t("custodyGaps:blocking.title"),
            tc("custodyGaps:blocking.body", canonical, &[]),
            amounts.clone(),
            hidden,
        ))
    } else if clear {
        Some(verdict(
            tk,
            true,
            &I::CIRCLE_CHECK,
            t("custodyGaps:empty.title"),
            t("custodyGaps:empty.body"),
            vec![],
            false,
        ))
    } else {
        None
    };
    if let Some(v) = verdict_el {
        page = page.child(v);
    }
    if !complete {
        page = page.child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .items_start()
                .child(
                    ic(&I::TRIANGLE_ALERT, tk.r(1.0), kit::tw("amber-600"))
                        .mt(tk.r(0.125))
                        .a11y_hidden(),
                )
                .child({
                    let head = format!("{}.", t("custodyGaps:searchIncomplete.title"));
                    let all = format!("{head} {}", t("custodyGaps:searchIncomplete.body"));
                    text(all)
                        .spans(vec![
                            rune::Span::new(0, head.len()).weight(500.0).color(tk.fg),
                        ])
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0)
                }),
        );
    }
    // A focused gap outside the queue (resolved or dismissed) shows its review on its own.
    if let Some(f) = focus_gap
        && !review.iter().any(|g| g.gap_id == f)
    {
        page = page.child(details(app, cx, tk, layer));
    }
    if !review.is_empty() {
        let mut sec = col()
            .key("cg-queue")
            .w_full()
            .gap(tk.r(0.75))
            .role("region")
            .label(t("custodyGaps:reviewQueue.title"))
            .child(
                col()
                    .w_full()
                    .child(
                        text(t("custodyGaps:reviewQueue.title"))
                            .font(tk.tbase(600.0))
                            .color(tk.fg)
                            .role("heading")
                            .wrap(),
                    )
                    .child(muted(tk, t("custodyGaps:reviewQueue.description"))),
            );
        for g in &review {
            let expanded = focus_gap == Some(g.gap_id.as_str());
            sec = sec.child(gap_card(app, cx, tk, layer, g, expanded));
        }
        page = page.child(sec);
    }
    if s.cursor.is_some() {
        let loaded = gaps.len().to_string();
        let total = model::num(&summary["total"]);
        page = page.child(
            col()
                .w_full()
                .gap(tk.r(0.5))
                .items_center()
                .child(muted_xs(
                    tk,
                    tr(
                        "custodyGaps:pagination.loaded",
                        &[("loaded", &loaded), ("total", &total)],
                    ),
                ))
                .child(
                    kit::btn_md(
                        tk,
                        "cg-more",
                        Var::Outline,
                        None,
                        &t(if s.loading_more {
                            "custodyGaps:pagination.loading"
                        } else {
                            "custodyGaps:pagination.loadMore"
                        }),
                        !s.loading_more,
                    )
                    .on_press(super::load_more),
                ),
        );
    }
    page
}

/// `ScreenSkeleton titleWidth="w-48"` (aria-busy).
fn skeleton(tk: &Tk) -> El {
    let sk = |w: f32, h: f32| kit::skeleton(tk, w, h);
    let mut metrics = row().w_full();
    for i in 0..4 {
        let mut m = col()
            .grow(1.0)
            .w(0.0)
            .padding(tk.r(1.0))
            .gap(tk.r(0.5))
            .child(sk(tk.r(6.0), tk.r(0.75)))
            .child(sk(tk.r(5.0), tk.r(1.5)))
            .child(sk(tk.r(8.0), tk.r(0.75)));
        if i < 3 {
            m = m.border_sides([0.0, 1.0, 0.0, 0.0], tk.border);
        }
        metrics = metrics.child(m);
    }
    let mut rows = col().w_full();
    for _ in 0..8 {
        rows = rows.child(
            row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .gap(tk.r(0.75))
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .child(
                    col()
                        .grow(1.0)
                        .gap(tk.r(0.5))
                        .child(sk(tk.r(11.0), tk.r(1.0)))
                        .child(sk(tk.r(16.0), tk.r(0.75))),
                )
                .child(
                    col()
                        .w(96.0)
                        .gap(tk.r(0.5))
                        .items_end()
                        .child(sk(tk.r(5.0), tk.r(1.0)))
                        .child(sk(tk.r(3.5), tk.r(0.75))),
                ),
        );
    }
    col()
        .key("cg-loading")
        .w_full()
        .padding(tk.r(1.25))
        .gap(tk.r(0.75))
        .busy(true)
        .role("status")
        .label(t("custodyGaps:review.loading"))
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(
                    col()
                        .gap(tk.r(0.5))
                        .child(sk(tk.r(7.0), tk.r(0.75)))
                        .child(sk(tk.r(12.0), tk.r(1.25))),
                )
                .child(
                    row()
                        .gap(tk.r(0.75))
                        .child(sk(tk.r(6.0), tk.r(2.0)))
                        .child(sk(tk.r(7.0), tk.r(2.0))),
                ),
        )
        .child(
            crate::ui::surface(tk)
                .w_full()
                .clip_rounded(tk.r(0.75))
                .child(metrics),
        )
        .child(
            crate::ui::surface(tk)
                .w_full()
                .child(
                    col()
                        .w_full()
                        .px(tk.r(1.0))
                        .py(tk.r(0.75))
                        .gap(tk.r(0.5))
                        .child(sk(tk.r(9.0), tk.r(1.0)))
                        .child(sk(tk.r(14.0), tk.r(0.75))),
                )
                .child(rows),
        )
}

/// `Verdict`: `kb-surface flex items-start gap-3 p-5` with an icon, title, body and amounts.
fn verdict(
    tk: &Tk,
    ok: bool,
    glyph: &'static Icon,
    title: String,
    body: String,
    amounts: Vec<String>,
    hidden: bool,
) -> El {
    let color = if ok {
        kit::emerald_icon(tk)
    } else {
        kit::amber_icon(tk)
    };
    let mut r = row()
        .w_full()
        .padding(tk.card_pad())
        .gap(tk.r(0.75))
        .items_start()
        .child(ic(glyph, tk.r(1.25), color).mt(tk.r(0.125)).a11y_hidden())
        .child(
            col()
                .min_w(0.0)
                .grow(1.0)
                .gap(tk.r(0.125))
                .child(text(title).font(tk.tsm(600.0)).color(tk.fg).wrap())
                .child(muted(tk, body)),
        );
    if !amounts.is_empty() {
        let mut a = col().items_end();
        for x in amounts {
            a = a.child(sensitive(
                x,
                tk.tsm(500.0).features("'tnum' 1"),
                tk.fg,
                hidden,
            ));
        }
        r = r.child(a);
    }
    crate::ui::surface(tk)
        .key("cg-verdict")
        .w_full()
        .role("status")
        .child(r)
}

/// `CustodyGapCard`.
fn gap_card(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    g: &Gap,
    expanded: bool,
) -> El {
    let hidden = app.prefs.hide_sensitive();
    let status = match g.status.as_str() {
        "conflicting" => Badge::Destructive,
        "needs_review" => Badge::Secondary,
        _ => Badge::Outline,
    };
    let title = row()
        .min_w(0.0)
        .gap(tk.r(0.25))
        .center_y()
        .label(format!("{} → {}", g.source_wallet_label, g.destinations()))
        .child(sensitive(
            g.source_wallet_label.clone(),
            tk.tbase(600.0).lh(tk.r(1.0)),
            tk.fg,
            hidden,
        ))
        .child(
            ic(&I::ARROW_RIGHT, tk.r(1.0), tk.fg)
                .mx(tk.r(0.25))
                .a11y_hidden(),
        )
        .child(sensitive(
            g.destinations(),
            tk.tbase(600.0).lh(tk.r(1.0)),
            tk.fg,
            hidden,
        ));
    let conf = badge(
        tk,
        Badge::Outline,
        t(&format!("custodyGaps:confidence.{}", g.confidence)),
    )
    .help(t(if g.promotion_eligible {
        "custodyGaps:candidate.promotionEligible"
    } else {
        "custodyGaps:candidate.searchHint"
    }));
    let left = col()
        .min_w(0.0)
        .grow(1.0)
        .gap(tk.r(0.375))
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .center_y()
                .flex_wrap()
                .child(title)
                .child(badge(
                    tk,
                    status,
                    t(&format!("custodyGaps:status.{}", g.status)),
                ))
                .child(conf),
        )
        .child(muted(
            tk,
            format!(
                "{} – {}",
                fmt_date(g.started_at.as_deref()),
                fmt_date(g.ended_at.as_deref())
            ),
        ));
    let id = g.gap_id.clone();
    let btn = kit::btn(
        tk,
        format!("cg-review-{}", g.gap_id),
        Var::Outline,
        None,
        &t(if expanded {
            "custodyGaps:card.hideReview"
        } else {
            "custodyGaps:card.review"
        }),
        true,
    )
    .expanded(expanded)
    .on_press(move |a: &mut App| super::toggle_focus(a, &id));
    // `CardHeader` keeps an empty second grid row: its `gap-3` row gap shows under the header.
    let header = row()
        .w_full()
        .px(tk.card_pad())
        .pb(tk.r(0.75))
        .gap(tk.r(0.75))
        .items_start()
        .child(left)
        .child(btn);
    let fig = |label: String, value: &Value, extra: Option<String>, amber: bool| {
        let mut b = col()
            .grow(1.0)
            .w(0.0)
            .min_w(0.0)
            .padding(tk.r(0.75))
            .radius(tk.r(0.5))
            .border(
                1.0,
                if amber {
                    kit::tw("amber-500").alpha(0.3)
                } else {
                    tk.border
                },
            )
            .border_layout()
            .child(text(label).font(tk.txs(400.0)).color(tk.muted_fg))
            .child(
                sensitive(fmt_msat(value, &g.asset), tk.tbase(500.0), tk.fg, hidden).mt(tk.r(0.25)),
            );
        if amber {
            b = b.fill(kit::tw("amber-500").alpha(0.05));
        }
        if let Some(e) = extra {
            b = b.child(muted_xs(tk, e).mt(tk.r(0.25)));
        }
        b
    };
    let debit = tr(
        "custodyGaps:card.walletDebit",
        &[("amount", &fmt_msat(&g.source_debit_msat, &g.asset))],
    );
    let figures = row()
        .w_full()
        .gap(tk.r(0.75))
        .items_stretch()
        .child(fig(
            t("custodyGaps:card.principal"),
            &g.source_total_msat,
            None,
            false,
        ))
        .child(fig(
            t("custodyGaps:card.knownFee"),
            &g.source_fee_msat,
            Some(debit),
            false,
        ))
        .child(fig(
            t("custodyGaps:card.mayHaveReturned"),
            &g.return_total_msat,
            None,
            false,
        ))
        .child(fig(
            t("custodyGaps:card.residual"),
            &g.residual_msat,
            None,
            g.has_residual(),
        ));
    let mut body = col()
        .w_full()
        .px(tk.card_pad())
        .gap(tk.r(1.0))
        .child(figures);
    if expanded {
        body = body.child(details(app, cx, tk, layer));
    }
    card(tk, tk.r(1.0))
        .key(format!("cg-card-{}", g.gap_id))
        .child(header)
        .child(body)
}

// ---- The guided review -------------------------------------------------------------------------

/// `GapReviewDetails` of the focused gap.
fn details(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let _ = cx;
    let Some(d) = app.custody.gaps.detail.as_ref() else {
        return el();
    };
    let ctx = match &d.ctx {
        None => return muted(tk, t("custodyGaps:review.loading")).role("status"),
        Some(Err(e)) => {
            return text(e.clone())
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .wrap()
                .role("alert");
        }
        Some(Ok(v)) => v,
    };
    let Some(gv) = ctx["gaps"].as_array().and_then(|a| a.first()) else {
        return muted(tk, t("custodyGaps:review.missing"));
    };
    let g = Gap::parse(gv);
    let mode = model::action_mode(&g);
    let planning = d.planning.is_some();
    let mut c = col()
        .key("cg-detail")
        .w_full()
        .padding(tk.r(1.0))
        .gap(tk.r(0.75))
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.muted.alpha(0.25));
    // "Ask about this": the gap becomes the assistant's pinned item (`assistant::ask_about_gap`).
    if app.prefs.ai_features {
        let (id, started) = (g.gap_id.clone(), g.started_at.clone());
        c = c.child(
            row().w_full().justify_end().child(
                kit::btn(
                    tk,
                    "cg-ask",
                    Var::Ghost,
                    Some(&I::SPARKLES),
                    &t("assistant:panel.askAbout"),
                    true,
                )
                .on_press(move |a: &mut App| {
                    crate::assistant::ask_about_gap(a, &id, started.as_deref())
                }),
            ),
        );
    }
    // Why.
    let mut why = col().w_full().gap(tk.r(0.25)).mt(tk.r(0.5));
    for r in &g.reason_codes {
        let key = format!("custodyGaps:reasons.{r}");
        let label = if crate::i18n::has(&key) {
            t(&key)
        } else {
            r.replace('_', " ")
        };
        why = why.child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .items_start()
                .child(
                    ic(&I::CIRCLE_CHECK, tk.r(1.0), kit::tw("emerald-600"))
                        .mt(tk.r(0.125))
                        .a11y_hidden(),
                )
                .child(muted(tk, label).grow(1.0).min_w(0.0)),
        );
    }
    c = c.child(
        col()
            .w_full()
            .child(
                text(t("custodyGaps:review.whyTitle"))
                    .font(tk.tsm(500.0))
                    .color(tk.fg),
            )
            .child(why),
    );
    let years = if g.affected_years.is_empty() {
        "—".to_owned()
    } else {
        g.affected_years
            .iter()
            .map(|y| y.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let kv = |k: String, v: String| {
        let head = format!("{k}:  ");
        let n = head.len();
        text(format!("{head}{v}"))
            .spans(vec![rune::Span::new(0, n).color(tk.muted_fg)])
            .font(tk.tsm(400.0))
            .color(tk.fg)
            .grow(1.0)
            .w(0.0)
            .wrap()
    };
    c = c.child(
        row()
            .w_full()
            .gap(tk.r(0.5))
            .child(kv(
                t("custodyGaps:review.affectedDisposals"),
                g.affected_disposals.to_string(),
            ))
            .child(kv(t("custodyGaps:review.affectedYears"), years)),
    );
    // History.
    c = c.child(match &d.history {
        Some(Ok(h)) => history(
            tk,
            h["history"].as_array().map(Vec::as_slice).unwrap_or(&[]),
            &g.asset,
        ),
        None => muted(tk, t("custodyGaps:reviewHistory.loading")),
        Some(Err(_)) => muted(tk, t("custodyGaps:reviewHistory.unavailable")),
    });
    if let Some(p) = d.pending.as_ref().filter(|p| p.action == "create") {
        c = c.child(bridge_preview(app, tk, d, &p.preview, &g.asset));
    }
    if let Some(e) = &d.error {
        c = c.child(
            text(e.clone())
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .wrap()
                .role("alert"),
        );
    }
    if mode == ActionMode::Create {
        c = c.child(
            row()
                .gap(tk.r(0.5))
                .flex_wrap()
                .child(
                    kit::btn(
                        tk,
                        "cg-preview",
                        Var::Default,
                        None,
                        &t(if planning {
                            "custodyGaps:actions.previewing"
                        } else {
                            "custodyGaps:actions.previewBridge"
                        }),
                        !planning,
                    )
                    .on_press(|a: &mut App| super::plan(a, "create")),
                )
                .child(
                    kit::btn(
                        tk,
                        "cg-dismiss",
                        Var::Outline,
                        None,
                        &t("custodyGaps:actions.dismiss"),
                        !planning && !d.applying,
                    )
                    .on_press(|a: &mut App| super::plan(a, "dismiss")),
                ),
        );
    }
    if matches!(mode, ActionMode::Reopen | ActionMode::Revise) {
        let m = if mode == ActionMode::Revise {
            "revise"
        } else {
            "reopen"
        };
        let mut b = inner_box(tk)
            .gap(tk.r(0.75))
            .child(
                col()
                    .w_full()
                    .child(
                        text(t(&format!("custodyGaps:correction.{m}.title")))
                            .font(tk.tsm(500.0))
                            .color(tk.fg),
                    )
                    .child(muted_xs(
                        tk,
                        t(&format!("custodyGaps:correction.{m}.description")),
                    )),
            )
            .child(
                col()
                    .w_full()
                    .gap(tk.r(0.375))
                    .child(
                        text(t("custodyGaps:correction.reasonLabel"))
                            .font(tk.txs(500.0))
                            .color(tk.fg),
                    )
                    .child(textarea(
                        tk,
                        "cg-correction",
                        &d.correction,
                        t("custodyGaps:correction.reasonPlaceholder"),
                        t("custodyGaps:correction.reasonLabel"),
                        false,
                    )),
            )
            .child(
                row().child(
                    kit::btn(
                        tk,
                        "cg-correction-preview-btn",
                        Var::Outline,
                        None,
                        &(if planning {
                            t("custodyGaps:correction.previewing")
                        } else {
                            t(&format!("custodyGaps:correction.{m}.preview"))
                        }),
                        !planning,
                    )
                    .on_press(move |a: &mut App| super::plan(a, m)),
                ),
            );
        if let Some(p) = d
            .pending
            .as_ref()
            .filter(|p| p.action == "reopen" || p.action == "revise")
        {
            b = b.child(correction_preview(
                app, tk, d, &p.action, &p.preview, &g.asset,
            ));
        }
        c = c.child(b);
    }
    if let Some((class, amount)) = &g.residual_classification {
        c = c.child(
            inner_box(tk)
                .child(
                    text(t("custodyGaps:residual.currentTitle"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(muted(
                    tk,
                    tr(
                        "custodyGaps:residual.current",
                        &[
                            ("amount", &fmt_msat(amount, &g.asset)),
                            (
                                "classification",
                                &t(&format!("custodyGaps:residual.options.{class}.label")),
                            ),
                        ],
                    ),
                ))
                .child(muted_xs(tk, t("custodyGaps:residual.taxMeaningUnassigned")).mt(tk.r(0.25))),
        );
    }
    if model::offer_residual(&g) {
        c = c.child(residual_form(app, tk, layer, d, &g, planning));
    }
    c.child(muted_xs(tk, t("custodyGaps:review.explicitReview")))
}

/// `ReviewHistoryPanel`.
fn history(tk: &Tk, entries: &[Value], asset: &str) -> El {
    if entries.is_empty() {
        return muted(tk, t("custodyGaps:reviewHistory.empty"));
    }
    let mut list = col()
        .w_full()
        .gap(tk.r(0.5))
        .pl(tk.r(1.0))
        .border_sides([0.0, 0.0, 0.0, 1.0], tk.border)
        .role("list");
    for e in entries {
        let rev = model::num(&e["revision"]);
        let mut item = col()
            .w_full()
            .gap(tk.r(0.25))
            .role("listitem")
            .child(
                row()
                    .gap(tk.r(0.5))
                    .center_y()
                    .flex_wrap()
                    .child(badge(
                        tk,
                        Badge::Outline,
                        tr("custodyGaps:reviewHistory.revision", &[("revision", &rev)]),
                    ))
                    .child(
                        text(t(&format!(
                            "custodyGaps:reviewHistory.events.{}",
                            e["event_kind"].as_str().unwrap_or("")
                        )))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                    )
                    .child(
                        text(fmt_date(e["created_at"].as_str()))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg),
                    ),
            )
            .child(muted_xs(
                tk,
                tr(
                    "custodyGaps:reviewHistory.amounts",
                    &[
                        ("retained", &fmt_msat(&e["retained_msat"], asset)),
                        ("residual", &fmt_msat(&e["residual_msat"], asset)),
                    ],
                ),
            ));
        if let Some(c) = e["residual_classification"].as_str() {
            item = item.child(muted_xs(
                tk,
                tr(
                    "custodyGaps:reviewHistory.residual",
                    &[(
                        "classification",
                        &t(&format!("custodyGaps:residual.options.{c}.label")),
                    )],
                ),
            ));
        }
        if let Some(r) = e["reason"].as_str().filter(|r| !r.is_empty()) {
            item = item.child(muted_xs(
                tk,
                tr("custodyGaps:reviewHistory.reason", &[("reason", r)]),
            ));
        }
        let n = e["filed_report_impact_count"].as_u64().unwrap_or(0) as usize;
        if n > 0 {
            item = item.child(
                text(tc("custodyGaps:reviewHistory.reportImpacts", n, &[]))
                    .font(tk.txs(400.0))
                    .color(kit::tw(if tk.dark { "amber-200" } else { "amber-800" }))
                    .wrap(),
            );
        }
        list = list.child(item);
    }
    col()
        .w_full()
        .gap(tk.r(0.5))
        .child(
            text(t("custodyGaps:reviewHistory.title"))
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .role("heading"),
        )
        .child(list)
}

/// `FiledReportImpactList`.
fn filed_impacts(tk: &Tk, impacts: &[Value]) -> Option<El> {
    if impacts.is_empty() {
        return None;
    }
    let mut c = col()
        .key("custody-filed-report-impacts")
        .w_full()
        .padding(tk.r(0.75))
        .gap(tk.r(0.5))
        .radius(tk.r(0.375))
        .border(1.0, kit::tw("amber-500").alpha(0.4))
        .border_layout()
        .fill(kit::tw("amber-500").alpha(0.1))
        .child(
            text(t("custodyGaps:filedImpact.title"))
                .font(tk.tsm(500.0))
                .color(amber_ink(tk)),
        );
    for i in impacts {
        let state = t(&format!(
            "custodyGaps:filedImpact.state.{}",
            i["report_state"].as_str().unwrap_or("")
        ));
        let kind = i["report_kind"].as_str().unwrap_or("").replace('_', " ");
        let mut item = col()
            .w_full()
            .gap(tk.r(0.25))
            .child(
                text(tr(
                    "custodyGaps:filedImpact.report",
                    &[
                        ("kind", &kind),
                        ("state", &state),
                        ("start", &model::num(&i["affected_period_start_year"])),
                        ("end", &model::num(&i["affected_period_end_year"])),
                    ],
                ))
                .font(tk.tsm(400.0))
                .color(tk.fg)
                .wrap(),
            )
            .child(muted(
                tk,
                i["amendment_warning"].as_str().unwrap_or("").to_owned(),
            ));
        if i["after_gain_summary"]["status"].as_str() == Some("pending_journal_rebuild") {
            item = item.child(
                text(t("custodyGaps:filedImpact.pendingRebuild"))
                    .font(tk.tsm(500.0))
                    .color(amber_ink(tk))
                    .wrap(),
            );
        }
        c = c.child(item);
    }
    Some(c)
}

/// The explicit confirmation row and the confirm button under a preview.
fn confirmation(
    tk: &Tk,
    d: &Detail,
    label: String,
    confirm: String,
    pending_label: String,
    enabled: bool,
) -> Vec<El> {
    let checked = d.confirmed;
    let line = row()
        .key("cg-confirm-row")
        .w_full()
        .padding(tk.r(0.75))
        .gap(tk.r(0.5))
        .items_start()
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.background.alpha(0.6))
        // The `<label>` around the checkbox is the one control: a press anywhere toggles it.
        .role("checkbox")
        .checked(checked)
        .label(label.clone())
        .focusable(true)
        .on_press(|a: &mut App| {
            if let Some(d) = a.custody.gaps.detail.as_mut() {
                d.confirmed = !d.confirmed;
            }
        })
        .child(
            checkbox(tk, "cg-confirm", checked, label.clone())
                .focusable(false)
                .a11y_hidden(),
        )
        .child(
            text(label)
                .font(tk.tsm(400.0))
                .color(tk.fg)
                .wrap()
                .grow(1.0)
                .min_w(0.0)
                .a11y_hidden(),
        );
    let btn = kit::btn(
        tk,
        "cg-confirm-apply",
        Var::Default,
        None,
        &(if d.applying { pending_label } else { confirm }),
        !d.applying && enabled && checked,
    )
    .on_press(super::confirm);
    vec![line, row().child(btn)]
}

/// `BridgePreviewPanel`.
fn bridge_preview(app: &App, tk: &Tk, d: &Detail, p: &Value, asset: &str) -> El {
    let _ = app;
    let manual = p["review_mode"].as_str() == Some("manual_weak_hint");
    let (border, fill) = if manual {
        (
            kit::tw("amber-500").alpha(0.4),
            kit::tw("amber-500").alpha(0.1),
        )
    } else {
        (
            kit::tw("emerald-600").alpha(0.3),
            kit::tw("emerald-600").alpha(0.05),
        )
    };
    let mut c = col()
        .key("cg-bridge-preview")
        .w_full()
        .padding(tk.r(0.75))
        .gap(tk.r(0.75))
        .radius(tk.r(0.375))
        .border(1.0, border)
        .border_layout()
        .fill(fill)
        .child(
            text(t("custodyGaps:actions.previewReady"))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        )
        .child(muted(
            tk,
            tr(
                "custodyGaps:actions.previewAmounts",
                &[
                    ("retained", &fmt_msat(&p["retained_msat"], asset)),
                    ("residual", &fmt_msat(&p["residual_msat"], asset)),
                    ("fee", &fmt_msat(&p["fee_msat"], asset)),
                ],
            ),
        ));
    let warnings: Vec<&str> = p["warnings"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if !warnings.is_empty() {
        let mut list = col().w_full().gap(tk.r(0.25)).pl(tk.r(0.5)).role("list");
        for w in warnings {
            let key = format!("custodyGaps:warnings.{w}");
            let label = if crate::i18n::has(&key) {
                t(&key)
            } else {
                w.replace('_', " ")
            };
            list = list.child(
                row()
                    .w_full()
                    .gap(tk.r(0.5))
                    .items_start()
                    .role("listitem")
                    .child(
                        text("•")
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg)
                            .a11y_hidden(),
                    )
                    .child(muted(tk, label).grow(1.0).min_w(0.0)),
            );
        }
        c = c.child(
            col()
                .w_full()
                .padding(tk.r(0.75))
                .gap(tk.r(0.5))
                .radius(tk.r(0.375))
                .border(1.0, kit::tw("amber-500").alpha(0.4))
                .border_layout()
                .fill(tk.background.alpha(0.6))
                .child(
                    text(t("custodyGaps:actions.reviewWarnings"))
                        .font(tk.tsm(500.0))
                        .color(amber_ink(tk)),
                )
                .child(list),
        );
    }
    if let Some(f) = filed_impacts(
        tk,
        p["filed_report_impacts"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]),
    ) {
        c = c.child(f);
    }
    let activatable = p["activatable"].as_bool() == Some(true);
    c.children(confirmation(
        tk,
        d,
        t("custodyGaps:actions.explicitOwnershipConfirmation"),
        t("custodyGaps:actions.confirmBridge"),
        t("custodyGaps:actions.creating"),
        activatable,
    ))
}

/// `ExplicitPreviewConfirmation`'s green frame.
fn explicit_frame(tk: &Tk, key: &str) -> El {
    col()
        .key(key.to_owned())
        .w_full()
        .padding(tk.r(0.75))
        .gap(tk.r(0.75))
        .radius(tk.r(0.375))
        .border(1.0, kit::tw("emerald-600").alpha(0.3))
        .border_layout()
        .fill(kit::tw("emerald-600").alpha(0.05))
}

/// `CorrectionPreviewPanel`.
fn correction_preview(app: &App, tk: &Tk, d: &Detail, mode: &str, p: &Value, asset: &str) -> El {
    let _ = app;
    let current = model::num(&p["current_component_revision"]);
    let next = if p["new_component_revision"].is_null() {
        "—".into()
    } else {
        model::num(&p["new_component_revision"])
    };
    let mut c = explicit_frame(tk, "cg-correction-preview")
        .child(
            text(t(&format!("custodyGaps:correction.{mode}.previewReady")))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        )
        .child(muted(
            tk,
            tr(
                &format!("custodyGaps:correction.{mode}.revision"),
                &[("current", &current), ("next", &next)],
            ),
        ));
    if !p["retained_msat"].is_null()
        && !p["residual_msat"].is_null()
        && p.get("retained_msat").is_some()
        && p.get("residual_msat").is_some()
    {
        c = c.child(muted(
            tk,
            tr(
                "custodyGaps:correction.revise.amounts",
                &[
                    ("retained", &fmt_msat(&p["retained_msat"], asset)),
                    ("residual", &fmt_msat(&p["residual_msat"], asset)),
                ],
            ),
        ));
    }
    if let Some(f) = filed_impacts(
        tk,
        p["filed_report_impacts"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]),
    ) {
        c = c.child(f);
    }
    let enabled = model::confirmable(mode, p);
    c.children(confirmation(
        tk,
        d,
        t(&format!("custodyGaps:correction.{mode}.confirmation")),
        t(&format!("custodyGaps:correction.{mode}.confirm")),
        t(&format!("custodyGaps:correction.{mode}.confirming")),
        enabled,
    ))
}

/// The residual classification form and its preview.
fn residual_form(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    d: &Detail,
    g: &Gap,
    planning: bool,
) -> El {
    let label = t(&format!(
        "custodyGaps:residual.options.{}.label",
        d.residual_class
    ));
    let select = row()
        .key("cg-residual-class")
        .w_full()
        .h(tk.r(2.25))
        .px(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.375))
        .border(1.0, tk.input)
        .border_layout()
        .fill(tk.background)
        .role("combobox")
        .label(t("custodyGaps:residual.classificationLabel"))
        .a11y_value(label.clone())
        .focusable(true)
        .select_trigger(d.select_open, |a: &mut App| {
            if let Some(d) = a.custody.gaps.detail.as_mut() {
                d.select_open = !d.select_open;
            }
        })
        .child(text(label).font(tk.tsm(400.0)).color(tk.fg).grow(1.0))
        .child(ic(&I::CHEVRONS_UP_DOWN, tk.r(1.0), tk.muted_fg));
    if d.select_open {
        let entries = model::RESIDUAL_CLASSIFICATIONS
            .iter()
            .map(|c| {
                let c = *c;
                Entry::Radio {
                    key: format!("cg-residual-{c}"),
                    label: t(&format!("custodyGaps:residual.options.{c}.label")),
                    trailing: None,
                    checked: d.residual_class == c,
                    disabled: false,
                    on: std::rc::Rc::new(move |a: &mut App| {
                        if let Some(d) = a.custody.gaps.detail.as_mut() {
                            d.residual_class = c.to_owned();
                            d.pending = None;
                        }
                    }),
                }
            })
            .collect();
        let w = app.custody.gaps.detail.as_ref().map(|_| tk.r(24.0));
        kit::select_list(
            tk,
            layer,
            "cg-residual-menu",
            "cg-residual-class",
            Placement::BottomStart,
            w,
            entries,
            |a: &mut App| {
                if let Some(d) = a.custody.gaps.detail.as_mut() {
                    d.select_open = false;
                }
            },
        );
    }
    let mut b = inner_box(tk)
        .gap(tk.r(0.75))
        .child(
            col()
                .w_full()
                .child(
                    text(t("custodyGaps:residual.title"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(muted_xs(
                    tk,
                    tr(
                        "custodyGaps:residual.description",
                        &[("amount", &fmt_msat(&g.residual_msat, &g.asset))],
                    ),
                )),
        )
        .child(
            col()
                .w_full()
                .gap(tk.r(0.375))
                .child(
                    text(t("custodyGaps:residual.classificationLabel"))
                        .font(tk.txs(500.0))
                        .color(tk.fg),
                )
                .child(select)
                .child(muted_xs(
                    tk,
                    t(&format!(
                        "custodyGaps:residual.options.{}.description",
                        d.residual_class
                    )),
                )),
        )
        .child(
            col()
                .w_full()
                .gap(tk.r(0.375))
                .child(
                    text(t("custodyGaps:residual.reasonLabel"))
                        .font(tk.txs(500.0))
                        .color(tk.fg),
                )
                .child(textarea(
                    tk,
                    "cg-residual-reason",
                    &d.residual,
                    t("custodyGaps:residual.reasonPlaceholder"),
                    t("custodyGaps:residual.reasonLabel"),
                    true,
                )),
        )
        .child(
            col()
                .w_full()
                .padding(tk.r(0.75))
                .radius(tk.r(0.375))
                .border(1.0, kit::tw("amber-500").alpha(0.3))
                .border_layout()
                .fill(kit::tw("amber-500").alpha(0.05))
                .child(muted_xs(tk, t("custodyGaps:residual.taxBoundary"))),
        )
        .child(
            row().child(
                kit::btn(
                    tk,
                    "cg-residual-preview-btn",
                    Var::Outline,
                    None,
                    &t(if planning {
                        "custodyGaps:residual.previewing"
                    } else {
                        "custodyGaps:residual.preview"
                    }),
                    !planning,
                )
                .on_press(|a: &mut App| super::plan(a, "classify_residual")),
            ),
        );
    if let Some(p) = d
        .pending
        .as_ref()
        .filter(|p| p.action == "classify_residual")
    {
        b = b.child(residual_preview(tk, d, &p.preview, &g.asset));
    }
    b
}

/// `ResidualPreviewPanel`.
fn residual_preview(tk: &Tk, d: &Detail, p: &Value, asset: &str) -> El {
    let class = p["classification"].as_str().unwrap_or("");
    let needs_tax = class == "external_gift" || class == "external_loss";
    let mut result = inner_box(tk)
        .child(
            text(t("custodyGaps:residual.custodyResultTitle"))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        )
        .child(muted(
            tk,
            t(&format!(
                "custodyGaps:residual.custodyState.{}",
                p["custody_state"].as_str().unwrap_or("")
            )),
        ))
        .child(
            text(t("custodyGaps:residual.taxMeaningTitle"))
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .mt(tk.r(0.5)),
        )
        .child(muted(tk, t("custodyGaps:residual.taxMeaningUnassigned")));
    if needs_tax {
        result = result.child(
            text(t("custodyGaps:residual.giftLossTaxReview"))
                .font(tk.tsm(400.0))
                .color(kit::tw(if tk.dark { "amber-200" } else { "amber-800" }))
                .wrap()
                .mt(tk.r(0.5)),
        );
    }
    let mut c = explicit_frame(tk, "cg-residual-preview")
        .child(
            text(t("custodyGaps:residual.previewReady"))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        )
        .child(
            text(tr(
                "custodyGaps:residual.previewAmount",
                &[
                    ("amount", &fmt_msat(&p["residual_msat"], asset)),
                    (
                        "classification",
                        &t(&format!("custodyGaps:residual.options.{class}.label")),
                    ),
                ],
            ))
            .font(tk.tsm(400.0))
            .color(tk.fg)
            .wrap(),
        )
        .child(result);
    if let Some(f) = filed_impacts(
        tk,
        p["filed_report_impacts"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]),
    ) {
        c = c.child(f);
    }
    c.children(confirmation(
        tk,
        d,
        t("custodyGaps:residual.confirmation"),
        t("custodyGaps:residual.confirm"),
        t("custodyGaps:residual.confirming"),
        model::confirmable("classify_residual", p),
    ))
}

/// The browser confirm of a planned dismissal (`window.confirm(actions.dismissConfirm)`).
pub fn dismiss_dialog(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    if app
        .custody
        .gaps
        .detail
        .as_ref()
        .is_none_or(|d| d.dismiss.is_none())
    {
        return;
    }
    let close = |a: &mut App| {
        if let Some(d) = a.custody.gaps.detail.as_mut() {
            d.dismiss = None;
        }
    };
    let content = col()
        .w_full()
        .gap(tk.r(1.0))
        .child(
            text(t("custodyGaps:actions.dismissConfirm"))
                .font(tk.tsm(400.0))
                .color(tk.fg)
                .wrap()
                .pr(tk.r(1.5)),
        )
        .child(
            kit::dialog_footer(tk)
                .child(
                    kit::btn_md(
                        tk,
                        "cg-dismiss-cancel",
                        Var::Outline,
                        None,
                        &t("common:actions.cancel"),
                        true,
                    )
                    .on_press(close),
                )
                .child(
                    kit::btn_md(tk, "cg-dismiss-ok", Var::Default, None, "OK", true)
                        .autofocus()
                        .on_press(super::confirm_dismiss),
                ),
        );
    kit::dialog(
        app,
        tk,
        layer,
        "cg-dismiss-dialog",
        28.0,
        content.label(t("custodyGaps:actions.dismiss")),
        close,
    );
}

// ---- Scan coverage -----------------------------------------------------------------------------

/// `CustodyCoverageSection` / `CustodyCoverageTimeline`.
pub fn coverage(app: &App, tk: &Tk) -> El {
    let s = &app.custody.gaps;
    let Some(snap) = s.coverage.as_ref() else {
        return text(t(if s.cov_rd != Rd::Ready {
            "custodyGaps:coverage.loading"
        } else {
            "custodyGaps:coverage.unavailable"
        }))
        .font(tk.tsm(400.0))
        .color(tk.muted_fg)
        .role("status")
        .wrap();
    };
    let head = col()
        .w_full()
        .child(
            text(t("custodyGaps:coverage.title"))
                .font(tk.tbase(600.0))
                .color(tk.fg)
                .role("heading"),
        )
        .child(
            muted(tk, t("custodyGaps:coverage.description"))
                .mt(tk.r(0.25))
                .max_w(tk.r(48.0)),
        );
    let wallets = snap["wallets"].as_array().cloned().unwrap_or_default();
    let mut sec = col()
        .key("cg-coverage")
        .w_full()
        .gap(tk.r(0.75))
        .role("region")
        .label(t("custodyGaps:coverage.title"))
        .child(head);
    if wallets.is_empty() {
        return sec.child(
            card(tk, tk.r(0.5)).child(
                col()
                    .w_full()
                    .px(tk.card_pad())
                    .child(muted(tk, t("custodyGaps:coverage.empty"))),
            ),
        );
    }
    let mut list = col().w_full().gap(tk.r(1.0));
    for (wi, w) in wallets.iter().enumerate() {
        let epochs = w["epochs"].as_array().cloned().unwrap_or_default();
        let header = col()
            .w_full()
            .px(tk.card_pad())
            .gap(tk.r(0.375))
            .child(
                row()
                    .gap(tk.r(0.5))
                    .center_y()
                    .child(ic(&I::WALLET_CARDS, tk.r(1.0), tk.fg))
                    .child(
                        text(w["wallet_label"].as_str().unwrap_or("").to_owned())
                            .font(tk.tbase(600.0).lh(tk.r(1.0)))
                            .color(tk.fg),
                    ),
            )
            .child(muted(
                tk,
                tc("custodyGaps:coverage.epochCount", epochs.len(), &[]),
            ));
        let mut ol = col()
            .w_full()
            .gap(tk.r(1.0))
            .pl(tk.r(1.25))
            .border_sides([0.0, 0.0, 0.0, 1.0], tk.border)
            .role("list");
        for e in &epochs {
            let status = e["status"].as_str().unwrap_or("");
            let mut dates = tr(
                "custodyGaps:coverage.started",
                &[("date", &fmt_date(e["created_at"].as_str()))],
            );
            if let Some(r) = e["retired_at"].as_str() {
                dates.push_str(&format!(
                    " · {}",
                    tr(
                        "custodyGaps:coverage.retiredAt",
                        &[("date", &fmt_date(Some(r)))]
                    )
                ));
            }
            let mut li = col()
                .w_full()
                .gap(tk.r(0.75))
                .role("listitem")
                .child(
                    el().absolute(-tk.r(1.55), tk.r(0.25))
                        .size(tk.r(0.75), tk.r(0.75))
                        .radius(tk.r(0.375))
                        .fill(tk.background)
                        .border(1.0, tk.border)
                        .border_layout()
                        .a11y_hidden(),
                )
                .child(
                    row()
                        .gap(tk.r(0.5))
                        .center_y()
                        .flex_wrap()
                        .child(badge(
                            tk,
                            if status == "active" {
                                Badge::Default
                            } else {
                                Badge::Outline
                            },
                            t(&format!("custodyGaps:coverage.{status}")),
                        ))
                        .child(muted_xs(
                            tk,
                            format!(
                                "{} · {}",
                                model::num(&e["chain"]),
                                model::num(&e["network"])
                            ),
                        )),
                )
                .child(muted_xs(tk, dates));
            let sources = e["sources"].as_array().cloned().unwrap_or_default();
            if sources.is_empty() {
                li = li.child(muted(tk, t("custodyGaps:coverage.noSources")));
            } else {
                // `grid gap-3 lg:grid-cols-2`.
                let mut grid = col().w_full().gap(tk.r(0.75));
                for pair in sources.chunks(2) {
                    let mut r = row().w_full().gap(tk.r(0.75)).items_start();
                    for src in pair {
                        r = r.child(source_box(tk, src));
                    }
                    if pair.len() == 1 {
                        r = r.child(el().grow(1.0).w(0.0));
                    }
                    grid = grid.child(r);
                }
                li = li.child(grid);
            }
            ol = ol.child(li);
        }
        list = list.child(
            card(tk, tk.r(1.0))
                .key(format!("cg-cov-wallet-{wi}"))
                .child(header)
                .child(col().w_full().px(tk.card_pad()).child(ol)),
        );
    }
    sec = sec.child(list);
    sec
}

fn source_box(tk: &Tk, src: &Value) -> El {
    let mut b = col()
        .grow(1.0)
        .w(0.0)
        .min_w(0.0)
        .padding(tk.r(0.75))
        .gap(tk.r(0.5))
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout()
        .child(
            text(format!(
                "{} · {}",
                model::num(&src["observer_kind"]),
                model::num(&src["source"])
            ))
            .font(tk.tsm(500.0))
            .color(tk.fg)
            .wrap(),
        );
    let branches = src["branches"].as_array().cloned().unwrap_or_default();
    if branches.is_empty() {
        return b.child(muted_xs(tk, t("custodyGaps:coverage.noBranches")));
    }
    let mut list = col().w_full().gap(tk.r(0.5));
    for br in &branches {
        let line = |s: String| text(s).font(tk.txs(400.0)).color(tk.fg).wrap();
        let mut c = col()
            .w_full()
            .padding(tk.r(0.5))
            .radius(tk.r(0.375))
            .fill(tk.muted.alpha(0.4))
            .child(
                text(t(&format!(
                    "custodyGaps:coverage.branch.{}",
                    br["branch"].as_str().unwrap_or("")
                )))
                .font(tk.txs(500.0))
                .color(tk.fg),
            );
        if br["scanned_to_exclusive"].is_null() {
            c = c.child(muted_xs(tk, t("custodyGaps:coverage.notObserved")));
        } else {
            c = c
                .child(line(tr(
                    "custodyGaps:coverage.exclusiveBound",
                    &[("index", &model::num(&br["scanned_to_exclusive"]))],
                )))
                .child(line(if br["highest_used"].is_null() {
                    t("custodyGaps:coverage.unused")
                } else {
                    tr(
                        "custodyGaps:coverage.highestUsed",
                        &[("index", &model::num(&br["highest_used"]))],
                    )
                }))
                .child(muted_xs(
                    tk,
                    tr(
                        "custodyGaps:coverage.observed",
                        &[("date", &fmt_date(br["observed_at"].as_str()))],
                    ),
                ));
        }
        list = list.child(c);
    }
    b = b.child(list);
    b
}
