//! The suggestions card's view (`ReviewTransfers` with `framed`): the titled header with its
//! count, the info note, the saved view's chip and the bulk actions; the live status line; one
//! row per candidate laid out by the list's width (`@min-[60rem]/moves`); "Show more"; the bulk
//! confirmation with the core's plan.

use rune::{Color, Cx, El, Shadow, col, el, row, text};
use rune_kit::Placement;
use serde_json::Value;

use super::model::{self, candidate_key, s};
use super::{Popup, SHOWN};
use crate::app::{App, ThemeMode};
use crate::custody::kit::{self, Entry, Var};
use crate::i18n::{Lang, t, tc, tr};
use crate::icons as I;
use crate::theme::{MASK, Tk};
use crate::ui::{ic, sensitive};

/// Tailwind v4 colours the rows' chips use.
fn tw(name: &str) -> Color {
    let (l, c, h) = match name {
        "sky-50" => (0.977, 0.013, 236.62),
        "sky-100" => (0.951, 0.026, 236.824),
        "sky-200" => (0.901, 0.058, 230.902),
        "sky-400" => (0.746, 0.16, 232.661),
        "sky-500" => (0.685, 0.169, 237.323),
        "sky-800" => (0.443, 0.11, 240.79),
        "sky-950" => (0.293, 0.066, 243.157),
        "amber-50" => (0.987, 0.022, 95.277),
        "amber-100" => (0.962, 0.059, 95.617),
        "amber-200" => (0.924, 0.12, 95.746),
        "amber-400" => (0.828, 0.189, 84.429),
        "amber-500" => (0.769, 0.188, 70.08),
        "amber-800" => (0.473, 0.137, 46.201),
        "amber-950" => (0.279, 0.077, 45.635),
        "emerald-50" => (0.979, 0.021, 166.113),
        "emerald-100" => (0.95, 0.052, 163.051),
        "emerald-200" => (0.905, 0.093, 164.15),
        "emerald-400" => (0.765, 0.177, 163.223),
        "emerald-500" => (0.696, 0.17, 162.48),
        "emerald-800" => (0.432, 0.095, 166.913),
        "emerald-950" => (0.262, 0.051, 172.552),
        "red-50" => (0.971, 0.013, 17.38),
        "red-100" => (0.936, 0.032, 17.717),
        "red-200" => (0.885, 0.062, 18.334),
        "red-400" => (0.704, 0.191, 22.216),
        "red-800" => (0.444, 0.177, 26.899),
        "red-950" => (0.258, 0.092, 26.042),
        _ => (0.5, 0.0, 0.0),
    };
    rune::oklch(l, c, h)
}

/// A soft-tint chip (`rounded-full border px-1.5 leading-4 font-medium`) in a hue:
/// `border-<h>-200 bg-<h>-50 text-<h>-800`, dark `border-<h>-400/30 bg-<h>-950/40 text-<h>-100`.
fn tint_chip(
    tk: &Tk,
    hue: &str,
    glyph: Option<&'static rune::Icon>,
    label: String,
    help: Option<String>,
) -> El {
    let (border, fill, ink) = if tk.dark {
        (
            tw(&format!("{hue}-400")).alpha(0.3),
            tw(&format!("{hue}-950")).alpha(0.4),
            tw(&format!("{hue}-100")),
        )
    } else {
        (
            tw(&format!("{hue}-200")),
            tw(&format!("{hue}-50")),
            tw(&format!("{hue}-800")),
        )
    };
    let _ = (fill, border);
    let mut r = row().gap(tk.r(0.25)).center_y();
    if let Some(g) = glyph {
        r = r.child(ic(g, tk.r(0.75), ink));
    }
    r = r.child(text(label).font(tk.t2xs(500.0).lh(tk.r(1.0))).color(ink));
    if let Some(h) = help {
        r = r.help(h);
    }
    r
}

/// shadcn `Checkbox`.
fn checkbox(tk: &Tk, key: String, checked: bool, label: String) -> El {
    let side = tk.r(1.0);
    let mut b = row()
        .key(key)
        .size(side, side)
        .center()
        .radius(4.0)
        .role("checkbox")
        .checked(checked)
        .label(label)
        .focusable(true);
    if checked {
        b = b.fill(tk.primary).border(1.0, tk.primary).child(ic(
            &I::CHECK,
            tk.r(0.875),
            tk.primary_fg,
        ));
    } else {
        b = b
            .fill(if tk.dark {
                tk.input.alpha(tk.input.a * 0.3)
            } else {
                Color::TRANSPARENT
            })
            .border(1.0, tk.input);
    }
    b.box_shadow(Shadow {
        y: 1.0,
        blur: 2.0,
        color: Color::BLACK.alpha(0.05),
        ..Shadow::default()
    })
}

/// `InfoNote`: an info button whose notes open in a popover (`w-72 p-3 text-xs space-y-1`).
fn info_note(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let key = "tp-sugg-about";
    let open = app.custody.sugg.popup == Some(Popup::Info);
    let button = row()
        .key(key)
        .center()
        .radius(tk.r(0.125))
        .role("button")
        .label(t("review:inbox.transfers.aboutLabel"))
        .expanded(open)
        .focusable(true)
        .hover_group()
        .on_press(|a: &mut App| super::toggle_popup(a, Popup::Info))
        .child(ic(&I::INFO, tk.r(0.875), tk.muted_fg).group_color(tk.fg));
    if open {
        let mut body = col().w(tk.r(18.0)).padding(tk.r(0.75)).gap(tk.r(0.25));
        for n in [
            t("review:inbox.transfers.why"),
            t("review:inbox.transfers.policyNote"),
        ] {
            body = body.child(text(n).font(tk.txs(400.0)).color(tk.fg).wrap());
        }
        let surface = crate::shell::glass(tk, body)
            .modal(true, super::close_popup)
            .focus_return(key.to_owned());
        layer.push(crate::shell::anchored(
            tk,
            surface,
            key,
            Placement::BottomStart,
            4.0,
        ));
    }
    button
}

pub fn card(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    filter_label: Option<&str>,
) -> El {
    let st = &app.custody.sugg;
    let hidden = app.prefs.hide_sensitive();
    // `if (suggest.isLoading && framed) return null`.
    if st.loading && st.live.is_none() {
        return el();
    }
    if st.failed && st.live.is_none() {
        return kit::surface(tk)
            .key("review-transfers")
            .w_full()
            .px(tk.card_pad())
            .py(tk.r(0.875))
            .role("alert")
            .child(
                text(t("review:inbox.transfers.title"))
                    .font(tk.tsm(600.0))
                    .color(tk.fg)
                    .role("heading"),
            )
            .child(
                text(t("review:inbox.transfers.unavailable"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .mt(tk.r(0.25)),
            );
    }
    let cands = st.candidates();
    if cands.is_empty() && st.undo.is_none() && st.error.is_none() && filter_label.is_none() {
        return el();
    }
    let exact = model::bulk_pairable(&cands);
    let chosen = cands
        .iter()
        .filter(|c| st.selected.contains(&candidate_key(c)))
        .count();
    let bulk_busy = st.bulk.as_ref().is_some_and(|b| b.busy);
    let refreshing = crate::custody::updating(app);

    // Header actions.
    let mut actions = row().gap(tk.r(0.5)).center_y().flex_wrap();
    if chosen > 0 {
        let enabled = st.saving == 0 && !bulk_busy;
        let mut b = kit::btn(
            tk,
            "tp-sugg-dismiss-selected",
            Var::Outline,
            None,
            &tc("review:inbox.transfers.dismissSelected", chosen, &[]),
            enabled,
        );
        if enabled {
            b = b.on_press(super::dismiss_selected);
        }
        actions = actions.child(b);
    }
    if !exact.is_empty() {
        let enabled = !refreshing && st.saving == 0 && !bulk_busy;
        let mut b = kit::btn(
            tk,
            "tp-sugg-pair-exact",
            Var::Default,
            None,
            &tc("review:inbox.transfers.pairExact", exact.len(), &[]),
            enabled,
        );
        if enabled {
            b = b.on_press(super::prepare_bulk);
        }
        actions = actions.child(b);
    }
    let mut extras = vec![info_note(app, tk, layer)];
    if let Some(name) = filter_label {
        extras.push(
            kit::filter_chip(
                tk,
                "tp-sugg-view-chip",
                &tr("review:inbox.transfers.view", &[("name", name)]),
                tr("review:inbox.transfers.clearView", &[("name", name)]),
            )
            .on_press(crate::custody::page::clear_saved_view),
        );
    }
    let count = (!cands.is_empty()).then(|| cands.len().to_string());
    let has_actions = chosen > 0 || !exact.is_empty();
    let mut c = kit::surface(tk)
        .key("review-transfers")
        .w_full()
        .role("region")
        .label(t("review:inbox.transfers.title"));
    c = c.child(kit::card_header(
        tk,
        t("review:inbox.transfers.title"),
        count,
        extras,
        has_actions.then_some(actions),
    ));
    if filter_label.is_some() && cands.is_empty() {
        c = c.child(kit::rule(tk)).child(
            row().w_full().px(tk.card_pad()).py(tk.r(0.75)).child(
                text(t("review:inbox.transfers.viewEmpty"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg),
            ),
        );
    }
    // The status line (saving / the undo offer) and the error, both live regions.
    if st.saving > 0 {
        c = c.child(
            row()
                .key("tp-sugg-status")
                .w_full()
                .px(tk.card_pad())
                .pb(tk.r(0.75))
                .gap(tk.r(0.375))
                .center_y()
                .role("status")
                .child(kit::spinner(tk, tk.r(0.75), tk.muted_fg))
                .child(
                    text(t("review:inbox.transfers.saving"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                ),
        );
    } else if let Some(u) = &st.undo {
        c = c.child(
            row()
                .key("tp-sugg-status")
                .w_full()
                .px(tk.card_pad())
                .pb(tk.r(0.75))
                .gap(tk.r(0.5))
                .center_y()
                .flex_wrap()
                .role("status")
                .child(sensitive(
                    u.label.clone(),
                    tk.txs(400.0),
                    tk.muted_fg,
                    hidden,
                ))
                .child(
                    row()
                        .key("tp-sugg-undo")
                        .role("button")
                        .label(t("review:inbox.transfers.undo"))
                        .focusable(true)
                        .hover_group()
                        .on_press(super::run_undo)
                        .child(
                            text(t("review:inbox.transfers.undo"))
                                .font(tk.txs(500.0))
                                .color(tk.primary)
                                .group_opacity(1.0, 0.8),
                        ),
                ),
        );
    }
    if let Some(e) = &st.error {
        c = c.child(
            row().w_full().px(tk.card_pad()).pb(tk.r(0.75)).child(
                text(e.clone())
                    .font(tk.txs(400.0))
                    .color(tk.destructive)
                    .role("alert")
                    .wrap(),
            ),
        );
    }
    if !cands.is_empty() {
        let lw = cx
            .layout_size("review-transfers")
            .map(|[w, _]| w)
            .filter(|w| *w > 0.0)
            .unwrap_or_else(|| kit::page_w(app, cx, tk));
        let wide = lw >= tk.r(60.0);
        let shown: Vec<&Value> = if st.expanded {
            cands.iter().collect()
        } else {
            cands.iter().take(SHOWN).collect()
        };
        // Auto columns are as wide as their widest cell, so every row's edges line up.
        let act_w = shown
            .iter()
            .map(|c| action_width(tk, c))
            .fold(0.0f32, f32::max);
        let mut list = col().key("review-transfers-list").w_full().role("list");
        for cnd in shown {
            list = list
                .child(kit::rule(tk))
                .child(candidate_row(app, cx, tk, layer, cnd, wide, act_w));
        }
        c = c.child(list);
        if cands.len() > SHOWN {
            let label = if st.expanded {
                t("review:inbox.transfers.showFewer")
            } else {
                tc("review:inbox.transfers.showMore", cands.len() - SHOWN, &[])
            };
            c = c.child(
                row().w_full().child(
                    kit::btn(tk, "tp-sugg-more", Var::Ghost, None, &label, true)
                        .mx(tk.r(0.5))
                        .my(tk.r(0.5))
                        .expanded(st.expanded)
                        .on_press(|a: &mut App| a.custody.sugg.expanded = !a.custody.sugg.expanded),
                ),
            );
        }
    }
    if st.bulk.is_some() {
        bulk_dialog(app, tk, layer, exact.len());
    }
    c
}

/// The actions cell's width: the Link or Review button and the menu (`gap-0.5`).
fn action_width(tk: &Tk, c: &Value) -> f32 {
    let review = model::needs_review(c);
    let label = t(if review {
        "review:inbox.transfers.review"
    } else {
        "review:inbox.transfers.pair"
    });
    let text_w = label.chars().count() as f32 * tk.r(0.75) * 0.56;
    let icon = if review {
        0.0
    } else {
        tk.r(0.875) + tk.r(0.25)
    };
    tk.r(0.625) * 2.0 + text_w + icon + tk.r(0.125) + tk.r(1.75)
}

#[allow(clippy::too_many_arguments)]
fn candidate_row(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    c: &Value,
    wide: bool,
    act_w: f32,
) -> El {
    let hidden = app.prefs.hide_sensitive();
    let key = candidate_key(c);
    let route_label = tr(
        "review:inbox.transfers.route",
        &[
            ("from", &s(c, "out_wallet_label")),
            ("to", &s(c, "in_wallet_label")),
        ],
    );
    let ty = model::move_type(c);
    let (_, policy_raw) = model::candidate_terms(c);
    let policy = match policy_raw.as_str() {
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
    let out_asset = s(c, "out_asset");
    let in_asset = s(c, "in_asset");
    let fee = c["swap_fee_msat"].as_f64().unwrap_or(0.0);
    let same_asset = model::is_bitcoin(&out_asset) == model::is_bitcoin(&in_asset)
        && (model::is_bitcoin(&out_asset) || out_asset.eq_ignore_ascii_case(&in_asset));
    let show_received = !same_asset || fee <= 0.0;
    let out_rail =
        crate::custody::model::rail_for_leg(Some(&out_asset), c["out_wallet_kind"].as_str());
    let in_rail =
        crate::custody::model::rail_for_leg(Some(&in_asset), c["in_wallet_kind"].as_str());

    // Checkbox and date.
    let sel = app.custody.sugg.selected.contains(&key);
    let k1 = key.clone();
    let check = checkbox(
        tk,
        format!("tp-sugg-select-{key}"),
        sel,
        tr(
            "review:inbox.transfers.selectLabel",
            &[("route", &route_label)],
        ),
    )
    .on_press(move |a: &mut App| super::toggle_selected(a, &k1));
    let date_style = if wide { tk.txs(400.0) } else { tk.t2xs(400.0) };
    let date = row()
        .gap(tk.r(0.625))
        .center_y()
        .child(check)
        .child(sensitive(
            model::short_date(&s(c, "out_occurred_at")),
            date_style.features("'tnum' 1"),
            tk.muted_fg,
            hidden,
        ));
    // Route.
    // From over to (the table kit's stacked route), fitted to the cell's last laid-out width.
    let route_key = format!("tp-sugg-route-{key}");
    let route_w = cx
        .layout_size(&route_key)
        .map(|[w, _]| w)
        .filter(|w| *w > 0.0)
        .unwrap_or(tk.r(16.0));
    let route = row()
        .key(route_key)
        .min_w(0.0)
        .grow(1.0)
        .w(0.0)
        .child(kit::route_stack(
            app,
            cx,
            tk,
            &s(c, "out_wallet_label"),
            &s(c, "in_wallet_label"),
            out_rail,
            in_rail,
            false,
            route_w,
        ));
    // Amount: what left, then what arrived or the fee.
    let mut amount = col().w(tk.r(9.0)).items_end().child(crate::ui::table::num(
        tk,
        model::leg_amount(c["out_amount_msat"].as_f64().unwrap_or(0.0), &out_asset),
        tk.fg,
        hidden,
    ));
    if show_received {
        let to = if hidden {
            MASK.to_owned()
        } else {
            model::leg_amount(c["in_amount_msat"].as_f64().unwrap_or(0.0), &in_asset)
        };
        amount = amount.child(
            row()
                .label(format!("{} {to}", t("review:inbox.transfers.to")))
                .child(
                    text("→ ")
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .a11y_hidden(),
                )
                .child(
                    sensitive(
                        model::leg_amount(c["in_amount_msat"].as_f64().unwrap_or(0.0), &in_asset),
                        crate::ui::table::sub_style(tk),
                        tk.muted_fg,
                        hidden,
                    )
                    .a11y_hidden(),
                ),
        );
    } else {
        let f = if model::is_bitcoin(&out_asset) {
            model::sats(fee)
        } else {
            model::leg_amount(fee, &out_asset)
        };
        amount = amount.child(crate::ui::table::num_sub(
            tk,
            tr("review:inbox.transfers.fee", &[("amount", &f)]),
            hidden,
        ));
    }
    // What linking records and how sure the match is.
    let mut kind = row()
        .w_full()
        .min_w(0.0)
        .gaps(tk.r(0.125), tk.r(0.5))
        .center_y()
        .flex_wrap();
    if ty == "transfer" {
        kind = kind.child(
            row()
                .gap(tk.r(0.25))
                .center_y()
                .child(ic(&I::ARROW_LEFT_RIGHT, tk.r(0.75), tk.muted_fg))
                .child(
                    text(t("review:inbox.transfers.type.transfer"))
                        .font(tk.t2xs(400.0))
                        .color(tk.muted_fg),
                ),
        );
    } else {
        let (hue, glyph) = if ty == "bitcoinSwap" {
            ("sky", &I::REPEAT)
        } else {
            ("amber", &I::SHUFFLE)
        };
        kind = kind.child(tint_chip(
            tk,
            hue,
            Some(glyph),
            t(&format!("review:inbox.transfers.type.{ty}")),
            None,
        ));
        if let Some(carries) = policy {
            kind = kind.child(if carries {
                tint_chip(
                    tk,
                    "emerald",
                    None,
                    t("review:inbox.transfers.policy.carrying"),
                    Some(t("review:inbox.transfers.policy.carryingHint")),
                )
            } else {
                tint_chip(
                    tk,
                    "red",
                    None,
                    t("review:inbox.transfers.policy.taxable"),
                    Some(t("review:inbox.transfers.policy.taxableHint")),
                )
            });
        }
    }
    let dot = match evidence {
        "likely" => tw("amber-500"),
        "exact" => tw("emerald-500"),
        _ => tw("sky-500"),
    };
    kind = kind.child(
        row()
            .gap(tk.r(0.25))
            .center_y()
            .child(crate::ui::dot(tk.r(0.375), dot).a11y_hidden())
            .child(
                text(t(&format!(
                    "review:inbox.transfers.evidenceShort.{evidence}"
                )))
                .font(tk.t2xs(400.0))
                .color(tk.muted_fg),
            ),
    );
    kind = kind.child(
        text(model::reason(c))
            .font(tk.t2xs(400.0))
            .color(tk.muted_fg)
            .wrap()
            .max_w(tk.r(10.0)),
    );
    if c["rule_match"].is_object() {
        let name = c["rule_match"]["rule_name"]
            .as_str()
            .map(str::to_owned)
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
    let conflict = model::conflict_size(c);
    if conflict > 1 {
        kind = kind.child(
            text(tc(
                "review:inbox.transfers.competing",
                (conflict - 1) as usize,
                &[],
            ))
            .font(tk.t2xs(400.0))
            .color(kit::amber_text(tk))
            .wrap(),
        );
    }
    // Link (exact, uncontested) or Review (opens Link manually on the send), then the menu.
    let out_id = s(c, "out_id");
    let in_id = s(c, "in_id");
    let primary = if review {
        let o = out_id.clone();
        kit::btn_xs(
            tk,
            format!("tp-sugg-review-{key}"),
            Var::Outline,
            None,
            &t("review:inbox.transfers.review"),
            true,
        )
        .gap(tk.r(0.25))
        .label(tr(
            "review:inbox.transfers.reviewLabel",
            &[("route", &route_label)],
        ))
        .on_press(move |a: &mut App| crate::custody::page::open_link(a, Some(o.clone())))
    } else {
        let cc = c.clone();
        kit::btn_xs(
            tk,
            format!("tp-sugg-pair-{key}"),
            Var::Default,
            Some(&I::CHECK),
            &t("review:inbox.transfers.pair"),
            true,
        )
        .gap(tk.r(0.25))
        .label(tr(
            "review:inbox.transfers.pairLabel",
            &[("route", &route_label)],
        ))
        .on_press(move |a: &mut App| super::decide(a, &cc, true))
    };
    let menu_key = format!("tp-sugg-menu-{key}");
    let open = app.custody.sugg.popup == Some(Popup::Row(key.clone()));
    if open {
        let (cc, o, o2, i2) = (c.clone(), out_id.clone(), out_id.clone(), in_id.clone());
        let entries = vec![
            Entry::Item {
                key: format!("tp-sugg-dismiss-{key}"),
                glyph: Some(&I::X),
                label: t("review:inbox.transfers.dismiss"),
                hint: None,
                badge: false,
                disabled: false,
                title: None,
                on: std::rc::Rc::new(move |a: &mut App| super::decide(a, &cc, false)),
            },
            Entry::item(
                format!("tp-sugg-diff-{key}"),
                Some(&I::LINK_2),
                t("review:inbox.transfers.linkDifferently"),
                move |a: &mut App| crate::custody::page::open_link(a, Some(o.clone())),
            ),
            Entry::Sep,
            Entry::item(
                format!("tp-sugg-sent-{key}"),
                Some(&I::ARROW_UP_RIGHT),
                t("review:transfersPage.rowMenu.showSent"),
                move |a: &mut App| crate::custody::show_transaction(a, &o2, None),
            ),
            Entry::item(
                format!("tp-sugg-recv-{key}"),
                Some(&I::ARROW_UP_RIGHT),
                t("review:transfersPage.rowMenu.showReceived"),
                move |a: &mut App| crate::custody::show_transaction(a, &i2, None),
            ),
        ];
        kit::menu(
            tk,
            layer,
            &format!("{menu_key}-content"),
            &menu_key,
            Placement::BottomEnd,
            None,
            entries,
            super::close_popup,
        );
    }
    let k2 = key.clone();
    let menu = kit::icon_btn(
        tk,
        menu_key,
        Var::Ghost,
        tk.r(1.75),
        &I::ELLIPSIS,
        tk.r(0.875),
        tr(
            "review:inbox.transfers.moreLabel",
            &[("route", &route_label)],
        ),
    )
    .expanded(open)
    .on_press(move |a: &mut App| super::toggle_popup(a, Popup::Row(k2.clone())));
    let actions = row()
        .w(act_w)
        .gap(tk.r(0.125))
        .center_y()
        .justify_end()
        .child(primary)
        .child(menu);
    let base = col()
        .key(format!("tp-sugg-row-{key}"))
        .w_full()
        .px(tk.r(crate::ui::table::EDGE))
        .py(tk.r(crate::ui::table::PY))
        .hover(tk.muted.alpha(0.35))
        .role("listitem");
    if wide {
        base.child(
            row()
                .w_full()
                .gap(tk.r(1.0))
                .center_y()
                .child(date)
                .child(route)
                .child(amount)
                .child(col().w(tk.r(10.0)).min_w(tk.r(10.0)).child(kind))
                .child(actions),
        )
    } else {
        base.gap(tk.r(0.25))
            .child(
                row()
                    .w_full()
                    .gap(tk.r(0.75))
                    .center_y()
                    .child(route)
                    .child(amount),
            )
            .child(
                row()
                    .w_full()
                    .gap(tk.r(0.75))
                    .center_y()
                    .child(date)
                    .child(row().min_w(0.0).grow(1.0).w(0.0).child(kind))
                    .child(
                        row()
                            .w(tk.r(9.0))
                            .justify_end()
                            .child(actions.w(act_w.min(tk.r(9.0)))),
                    ),
            )
    }
}

/// `ReviewTaxDeltaView`: the previewed tax change by year.
fn tax_delta(tk: &Tk, artifact: &Value, hidden: bool) -> Option<El> {
    let (tax, years) = model::tax_delta(artifact)?;
    let cur = s(&tax, "fiat_currency");
    let money = |v: f64| {
        if hidden {
            MASK.to_owned()
        } else {
            format!(
                "{}{}",
                if v > 0.0 { "+" } else { "" },
                crate::format::fiat(v, &cur)
            )
        }
    };
    let mut c = col()
        .w_full()
        .gap(tk.r(0.25))
        .label(t("assistant:review.tax.title"))
        .child(
            text(t("assistant:review.tax.title"))
                .font(tk.txs(500.0))
                .color(tk.muted_fg),
        );
    if years.is_empty() {
        return Some(
            c.child(
                text(t("assistant:review.tax.unchanged"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            ),
        );
    }
    let at = s(&tax, "jurisdiction") == "AT";
    for y in &years {
        let num = |k: &str| s(y, k).parse::<f64>().unwrap_or(0.0);
        let mut parts = vec![
            format!(
                "{} {}",
                t("assistant:review.tax.realized"),
                money(num("realized_gain_loss"))
            ),
            format!(
                "{} {}",
                t("assistant:review.tax.income"),
                money(num("income"))
            ),
        ];
        if at {
            let year = y["year"].as_i64().unwrap_or(0);
            parts.push(if let Some(cents) = y["estimated_tax_eur_cents"].as_f64() {
                format!(
                    "{} {}",
                    t("assistant:review.tax.indicative"),
                    money(cents / 100.0)
                )
            } else if let Some(after) = tax["years"]
                .as_array()
                .and_then(|a| a.iter().find(|r| r["year"].as_i64() == Some(year)))
                .and_then(|r| r["indicative_tax"]["estimated_tax_eur_cents"].as_f64())
            {
                let amount = if hidden {
                    MASK.to_owned()
                } else {
                    crate::format::fiat(after / 100.0, &cur)
                };
                tr(
                    "assistant:review.tax.indicativeAfter",
                    &[("amount", &amount)],
                )
            } else {
                t("assistant:review.tax.withheld")
            });
        }
        c = c.child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .items_end()
                .flex_wrap()
                .child(
                    text(y["year"].to_string())
                        .font(tk.txs(500.0).features("'tnum' 1"))
                        .color(tk.fg),
                )
                .child(
                    text(parts.join(" · "))
                        .font(tk.txs(400.0).features("'tnum' 1"))
                        .color(tk.fg)
                        .wrap(),
                ),
        );
    }
    Some(c)
}

/// The bulk confirmation: the core's plan, then one confirmation that applies exactly it.
fn bulk_dialog(app: &App, tk: &Tk, layer: &mut Vec<El>, exact: usize) {
    let st = &app.custody.sugg;
    let b = st.bulk.as_ref().unwrap();
    let hidden = app.prefs.hide_sensitive();
    let count = b
        .preview
        .as_ref()
        .and_then(|(a, _)| a["operations"].as_array().map(Vec::len))
        .unwrap_or(if b.count > 0 { b.count } else { exact });
    let title = tc("review:inbox.transfers.bulk.title", count, &[]);
    let mut c = col()
        .w_full()
        .gap(tk.r(1.0))
        .label(title.clone())
        .child(kit::dialog_header(
            tk,
            title,
            Some(t("review:inbox.transfers.bulk.body")),
        ));
    if let Some(e) = &st.error {
        c = c.child(
            text(e.clone())
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .role("alert")
                .wrap(),
        );
    }
    match &b.preview {
        Some((a, _)) => {
            if let Some(d) = tax_delta(tk, a, hidden) {
                c = c.child(d);
            }
        }
        None => {
            c = c.child(
                text(t("review:inbox.transfers.saving"))
                    .font(tk.tsm(400.0))
                    .color(tk.fg)
                    .role("status"),
            )
        }
    }
    let enabled = !b.busy && b.preview.is_some();
    let label = tc("review:inbox.transfers.bulk.confirm", count, &[]);
    let mut confirm = if b.applying {
        kit::btn_busy(
            tk,
            "tp-sugg-bulk-confirm",
            Var::Default,
            true,
            None,
            &label,
            false,
        )
        .h(tk.r(2.25))
        .px(tk.r(0.75))
    } else {
        kit::btn_md(
            tk,
            "tp-sugg-bulk-confirm",
            Var::Default,
            None,
            &label,
            enabled,
        )
    };
    if enabled {
        confirm = confirm.on_press(super::pair_all_exact);
    }
    let cancel = kit::btn_md(
        tk,
        "tp-sugg-bulk-cancel",
        Var::Outline,
        None,
        &t("review:inbox.transfers.bulk.cancel"),
        !b.busy,
    )
    .on_press(super::close_bulk);
    c = c.child(kit::dialog_footer(tk).child(cancel).child(confirm));
    kit::dialog(app, tk, layer, "tp-sugg-bulk", 32.0, c, super::close_bulk);
}

// ---- Scenes ------------------------------------------------------------------------------------

/// The page on its Needs-you list with the suggestions read.
fn needs(a: &mut App) {
    a.navigate("/transfers");
    crate::custody::prime(a);
    a.custody.page.tab = Some(crate::custody::page::StatusTab::Needs);
    // The card asks for its read while drawing; start it as the first frame would.
    *a.custody.sugg.want.borrow_mut() = Some(Default::default());
    prime_card(a);
}

/// Runs the card's first read and settles it (headless).
fn prime_card(a: &mut App) {
    let mut tx = rune::Tx::default();
    super::poll(a, &mut tx);
    crate::custody::settle(a);
}

fn first_key(a: &App) -> Option<String> {
    a.custody.sugg.candidates().first().map(candidate_key)
}

/// Hides the first `n` suggestions as decided and offers their Undo, as a dismissal's reply does.
fn fake_undo(a: &mut App, n: usize) {
    let cands: Vec<Value> = a.custody.sugg.candidates().into_iter().take(n).collect();
    // A book without suggestions has nothing to undo.
    if cands.is_empty() {
        return;
    }
    let keys: Vec<String> = cands.iter().map(candidate_key).collect();
    super::hide_for_scene(a, &keys);
    let label = if n == 1 {
        tr(
            "review:inbox.transfers.dismissed",
            &[(
                "route",
                &tr(
                    "review:inbox.transfers.route",
                    &[
                        ("from", &s(&cands[0], "out_wallet_label")),
                        ("to", &s(&cands[0], "in_wallet_label")),
                    ],
                ),
            )],
        )
    } else {
        tc("review:inbox.transfers.dismissedMany", n, &[])
    };
    a.custody.sugg.undo = Some(super::Undo {
        decisions: vec![],
        label,
        deadline: a.clock + 3600.0,
    });
}

/// Opens the page on a saved view, by name (`savedView=<id>`), with the card read for its filter.
fn open_view(a: &mut App, name: &str) {
    crate::custody::rules::open(a);
    crate::custody::settle(a);
    let views: Vec<Value> = a
        .custody
        .rules
        .transfer_views
        .iter()
        .chain(a.custody.rules.swap_views.iter())
        .cloned()
        .collect();
    crate::custody::rules::close(a);
    let Some(v) = views.into_iter().find(|v| v["name"] == name) else {
        return needs(a);
    };
    a.navigate(&format!("/transfers?savedView={}", s(&v, "id")));
    crate::custody::prime(a);
    // The page's `viewFilter`: the matcher's own filters, the surface's candidate type.
    let get = |k: &str| {
        v["filter"][k]
            .as_str()
            .filter(|x| !x.is_empty() && *x != "all")
            .map(str::to_owned)
    };
    let mut f = super::Filter {
        confidence: get("confidence"),
        method: get("method"),
        asset_pair: get("asset_pair"),
        route_pair: get("route_pair"),
        candidate_type: get("candidate_type"),
    };
    if f.candidate_type.is_none() {
        f.candidate_type = Some(
            if v["surface"] == "swap_candidates" {
                "swap"
            } else {
                "transfer"
            }
            .into(),
        );
    }
    *a.custody.sugg.want.borrow_mut() = Some(f);
    prime_card(a);
}

pub const SCENES: &[crate::app::Scene] = &[
    ("custody-sugg", needs),
    ("custody-sugg-light", |a| {
        a.prefs.theme = ThemeMode::Light;
        needs(a);
    }),
    ("custody-sugg-de", |a| {
        a.prefs.lang = Lang::De;
        crate::i18n::set_lang(Lang::De);
        needs(a);
    }),
    ("custody-sugg-expanded", |a| {
        needs(a);
        a.custody.sugg.expanded = true;
    }),
    ("custody-sugg-selected", |a| {
        needs(a);
        let keys: Vec<String> = a
            .custody
            .sugg
            .candidates()
            .iter()
            .take(2)
            .map(candidate_key)
            .collect();
        a.custody.sugg.selected.extend(keys);
    }),
    ("custody-sugg-menu", |a| {
        needs(a);
        if let Some(k) = first_key(a) {
            a.custody.sugg.popup = Some(Popup::Row(k));
        }
    }),
    ("custody-sugg-about", |a| {
        needs(a);
        a.custody.sugg.popup = Some(Popup::Info);
    }),
    // The bulk confirmation with the core's plan (cases + plan: reads, nothing applied).
    ("custody-sugg-bulk", |a| {
        needs(a);
        super::prepare_bulk(a);
        crate::custody::settle(a);
    }),
    // The undo offer after setting one aside (shown as the reply leaves it; nothing is sent).
    ("custody-sugg-undo", |a| {
        needs(a);
        fake_undo(a, 1);
    }),
    // Every suggestion decided, the Undo still on offer: the card stays with only the offer.
    ("custody-sugg-empty-undo", |a| {
        needs(a);
        let n = a.custody.sugg.candidates().len();
        fake_undo(a, n);
    }),
    // A saved view's filter: the chip that clears it.
    ("custody-sugg-view", |a| open_view(a, "Likely transfers")),
    // A saved view nothing matches.
    ("custody-sugg-view-empty", |a| open_view(a, "Exact swaps")),
    ("custody-sugg-narrow", |a| {
        needs(a);
        a.prefs.nav_collapsed = false;
    }),
];
