//! The footer under the chart (ChartRangeToolbar.tsx): range chips, the year menu, log time,
//! log/fit scale chips and the gear menu with the dot-minimum submenus.

use std::rc::Rc;

use rune::{El, row};
use rune_kit::Placement;

use super::data::{DEFAULT_INCOMING_MIN, DEFAULT_OUTGOING_MIN, Period, SLIDER_MARKS};
use super::{ChartPopup, Geo, View, strings};
use crate::app::App;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::kit_menu::{self, Entry, Item as MenuItem};
use crate::ui::popup::TriggerExt;
use crate::ui::{chip, hairline, ic};

fn min_label(v: f64) -> String {
    // The `≥ … BTC` form is hard-coded in ChartRangeToolbar.tsx; only "All dots" is translated.
    if v == 0.0 {
        t("overview:controls.allDots")
    } else {
        format!("≥ {} BTC", super::controls::editable(v))
    }
}

pub fn toolbar(app: &App, tk: &Tk, g: &Geo, view: View, layer: &mut Vec<El>) -> El {
    let ui = &app.chart;
    let v = view as usize;
    // Range chips: offered ranges plus the active resolved one.
    // `gap-0.5` between chips, `gap-x-3` between the two groups.
    // The chips swap at once with the range (the year chip's label, a chip coming or going), as
    // the web's do; their colours keep easing. Eased, the chips after the year menu slide.
    let mut left = row()
        .gap(tk.r(0.125))
        .center_y()
        .flex_wrap()
        .role("group")
        .label(t("overview:controls.timeRange"))
        .instant_changes();
    for p in Period::RANGES {
        if !g.selectable.contains(&p) && g.resolved != p {
            continue;
        }
        // After a manual window (navigator, zoom, pan) no chip is active: pressing one returns
        // to its range.
        let active = g.resolved == p && !g.zoomed;
        left = left.child(
            chip(tk, &format!("range-{}-{v}", p.id()), &p.short(), active)
                .label(p.long())
                .help(p.long())
                .on_press(move |a: &mut App| a.chart.set_period(p)),
        );
    }
    if !g.years.is_empty() {
        let year_label = match ui.period {
            Period::Year(y) => y.to_string(),
            _ => t("common:period.year"),
        };
        let open = ui.popup == Some(ChartPopup::Year(view));
        let active = matches!(ui.period, Period::Year(_)) && !g.zoomed;
        // PeriodYearMenu: `inline-flex gap-1`, the label then a `size-3.5` chevron at 70 %.
        let ink = if active || open { tk.fg } else { tk.muted_fg };
        let trigger = row()
            .key(format!("year-chip-{v}"))
            .px(tk.r(0.375))
            .py(tk.r(0.125))
            .gap(tk.r(0.25))
            .center_y()
            .radius(tk.r(0.375))
            .fill(if active || open {
                tk.muted
            } else {
                rune::Color::TRANSPARENT
            })
            .hover(tk.muted.alpha(0.45))
            .role("button")
            .label(t("common:period.yearMenuAria"))
            .focusable(true)
            .child(
                rune::text(year_label)
                    .font(tk.txs(500.0).features("'tnum' 1"))
                    .color(ink),
            )
            .child(ic(&I::CHEVRON_DOWN, tk.r(0.875), ink.alpha(ink.a * 0.7)))
            .menu_trigger(open, move |a: &mut App| {
                a.chart.popup = if a.chart.popup == Some(ChartPopup::Year(view)) {
                    None
                } else {
                    Some(ChartPopup::Year(view))
                }
            });
        if open {
            let this_year = crate::format::Day::from_epoch_days(
                (crate::format::now_secs() / 86_400.0).floor() as i64,
            )
            .y;
            let mut entries = vec![];
            // The menu does not grow to a row's natural width, and the badge keeps its size, so
            // the year itself was cut ("20…"). Size it from the widest badge: the year, the
            // badge (about 6.5 px a character at this size), the check and the paddings.
            let mut widest = 0usize;
            for (y, n) in &g.years {
                let y = *y;
                let count = tc(
                    if y == this_year {
                        "common:period.yearCountSoFar"
                    } else {
                        "common:period.yearCount"
                    },
                    *n,
                    &[],
                );
                widest = widest.max(count.chars().count());
                entries.push(Entry::Item(
                    MenuItem::new(y.to_string())
                        .badge(count)
                        .checked(ui.period == Period::Year(y))
                        .on_select(move |a: &mut App| {
                            a.chart.set_period(Period::Year(y));
                            a.chart.popup = None;
                        }),
                ));
            }
            kit_menu::mount(
                tk,
                layer,
                &format!("year-menu-{v}"),
                &format!("year-chip-{v}"),
                Placement::TopStart,
                6.0,
                year_menu_width(widest),
                entries,
                Rc::new(|a: &mut App| a.chart.popup = None),
            );
        }
        left = left.child(trigger);
    }
    left = left.child(
        chip(
            tk,
            &format!("logx-{v}"),
            &t("overview:controls.logChip"),
            ui.log_x,
        )
        .label(t("overview:controls.logTimeTitle"))
        .help(t("overview:controls.logTimeTitle"))
        .on_press(|a: &mut App| {
            let on = !a.chart.log_x;
            a.chart.enable_log_time(on);
        }),
    );

    // These two chips carry only a `title`, so their accessible name is their text.
    let mut right = row()
        .gap(tk.r(0.125))
        .center_y()
        .role("group")
        .label(t("overview:controls.scale"));
    right = right
        .child(
            chip(tk, &format!("pct-{v}"), &strings::percent_chip(), ui.pct)
                .label(strings::percent_title())
                .help(strings::percent_title())
                .on_press(|a: &mut App| {
                    let on = !a.chart.pct;
                    a.chart.set_pct(on);
                }),
        )
        .child(
            chip(
                tk,
                &format!("logy-{v}"),
                &t("overview:controls.logChip"),
                ui.log_y,
            )
            .help(t("overview:controls.logScaleTitle"))
            .on_press(|a: &mut App| {
                let on = !a.chart.log_y;
                a.chart.set_log_y(on);
            }),
        )
        .child(
            chip(
                tk,
                &format!("fit-{v}"),
                &t("overview:controls.autoChip"),
                ui.fit,
            )
            .help(t("overview:controls.autoFitTitle"))
            .on_press(|a: &mut App| a.chart.fit = !a.chart.fit),
        );
    right = right.child(gear(app, tk, g, view, layer));

    // `mt-2 border-t pt-2`; it sits inside the chart block, not in the card's gap.
    let footer = row()
        .w_full()
        .pt(tk.r(0.5))
        .gaps(tk.r(0.25), tk.r(0.75))
        .justify_between()
        .center_y()
        .flex_wrap()
        .child(left)
        .child(right);
    rune::col()
        .w_full()
        .mt(tk.r(0.5))
        .child(hairline(tk.border))
        .child(footer)
}

fn radio_list(current: f64, set: fn(&mut App, f64)) -> Vec<Entry> {
    SLIDER_MARKS
        .iter()
        .map(|&m| {
            Entry::Item(MenuItem::new(min_label(m)).checked(current == m).on_select(
                move |a: &mut App| {
                    set(a, m);
                    a.chart.popup = None;
                },
            ))
        })
        .collect()
}

fn gear(app: &App, tk: &Tk, g: &Geo, view: View, layer: &mut Vec<El>) -> El {
    let ui = &app.chart;
    let v = view as usize;
    let open = matches!(ui.popup, Some(ChartPopup::Gear(x) | ChartPopup::GearIncoming(x) | ChartPopup::GearOutgoing(x)) if x == view);
    // `chipClass` + `inline-flex`: px-1.5 py-0.5 around a size-3.5 icon.
    let trigger = row()
        .key(format!("gear-{v}"))
        .px(tk.r(0.375))
        .py(tk.r(0.125))
        .center()
        .fill(if open {
            tk.muted
        } else {
            rune::Color::TRANSPARENT
        })
        .radius(tk.r(0.375))
        .hover(tk.muted.alpha(0.45))
        .role("button")
        .label(t("overview:controls.scaleMenuAria"))
        .focusable(true)
        .child(ic(
            &I::SETTINGS,
            tk.r(0.875),
            if open { tk.fg } else { tk.muted_fg },
        ))
        .menu_trigger(open, move |a: &mut App| {
            a.chart.popup = if matches!(
                a.chart.popup,
                Some(
                    ChartPopup::Gear(_) | ChartPopup::GearIncoming(_) | ChartPopup::GearOutgoing(_)
                )
            ) {
                None
            } else {
                Some(ChartPopup::Gear(view))
            }
        });
    if !open {
        return trigger;
    }
    let power = ui.log_x && ui.log_y;
    // The side sheet's "shown of total" counts, kept on the two minimum items.
    let shown = |visible: usize, total: usize| {
        tr(
            "overview:controls.dotsShownOfTotal",
            &[
                ("visible", &crate::format::count(visible)),
                ("total", &crate::format::count(total)),
            ],
        )
    };
    let zoomed = g.zoomed;
    let back = move |a: &mut App| a.chart.popup = Some(ChartPopup::Gear(view));
    let entries = vec![
        Entry::Item(
            MenuItem::new(t("overview:controls.lastValueLabel"))
                .checked(ui.show_last)
                .on_select(|a: &mut App| a.chart.show_last = !a.chart.show_last),
        ),
        Entry::Item(
            MenuItem::new(t("overview:controls.groupDotsLabel"))
                .checked(ui.group)
                .on_select(|a: &mut App| a.chart.group = !a.chart.group),
        ),
        Entry::Item(
            MenuItem::new(t("overview:controls.powerLawLabel"))
                .checked(power)
                .on_select(move |a: &mut App| {
                    let on = !(a.chart.log_x && a.chart.log_y);
                    a.chart.set_log_y(on);
                    a.chart.enable_log_time(on);
                }),
        ),
        Entry::Sep,
        Entry::Label(t("overview:controls.minBtcSize")),
        Entry::Item(
            MenuItem::new(t("overview:controls.incomingPayments"))
                .badge(min_label(ui.inc_min))
                .description(shown(g.counts.inc_visible, g.counts.inc_total))
                .submenu(
                    radio_list(ui.inc_min, |a, m| a.chart.set_inc_min(m)),
                    ui.popup == Some(ChartPopup::GearIncoming(view)),
                    move |a: &mut App| a.chart.popup = Some(ChartPopup::GearIncoming(view)),
                    back,
                ),
        ),
        Entry::Item(
            MenuItem::new(t("overview:controls.outgoingActivity"))
                .badge(min_label(ui.out_min))
                .description(shown(g.counts.out_visible, g.counts.out_total))
                .submenu(
                    radio_list(ui.out_min, |a, m| a.chart.set_out_min(m)),
                    ui.popup == Some(ChartPopup::GearOutgoing(view)),
                    move |a: &mut App| a.chart.popup = Some(ChartPopup::GearOutgoing(view)),
                    back,
                ),
        ),
        Entry::Item(
            MenuItem::new(t("overview:controls.resetMarkerMinimums"))
                .icon(&I::REFRESH_CW)
                .disabled(ui.mins_default())
                .on_select(|a: &mut App| {
                    a.chart.set_inc_min(DEFAULT_INCOMING_MIN);
                    a.chart.set_out_min(DEFAULT_OUTGOING_MIN);
                }),
        ),
        Entry::Sep,
        Entry::Label(strings::tools_group()),
        Entry::Item(
            MenuItem::new(strings::crosshair())
                .checked(ui.crosshair)
                .on_select(|a: &mut App| a.chart.crosshair = !a.chart.crosshair),
        ),
        Entry::Item(
            MenuItem::new(strings::measure())
                .shortcut(strings::measure_shortcut())
                .checked(ui.measure_armed)
                .on_select(|a: &mut App| {
                    a.chart.measure_armed = !a.chart.measure_armed;
                    a.chart.popup = None;
                }),
        ),
        Entry::Item(
            MenuItem::new(strings::reset_zoom())
                .shortcut("0")
                .disabled(!zoomed)
                .on_select_tx(move |a: &mut App, tx: &mut rune::Tx| {
                    a.chart.reset_zoom(view);
                    a.chart.popup = None;
                    tx.instant();
                }),
        ),
    ];
    // German labels run ~30 % longer than English; widen so items don't truncate.
    let min_width = tk.r(if crate::i18n::lang() == crate::i18n::Lang::De {
        24.0
    } else {
        18.5
    });
    kit_menu::mount(
        tk,
        layer,
        &format!("gear-menu-{v}"),
        &format!("gear-{v}"),
        Placement::TopEnd,
        6.0,
        min_width,
        entries,
        Rc::new(|a: &mut App| a.chart.popup = None),
    );
    trigger
}

/// The year menu's width for its widest badge (in characters): the year (4 digits), the badge,
/// the check and the row's gaps and paddings fit without cutting the year.
fn year_menu_width(badge_chars: usize) -> f32 {
    (100.0 + 6.5 * badge_chars as f32).max(160.0)
}

#[cfg(test)]
mod year_menu_tests {
    /// "so far · 107 tx" and longer counts widen the menu; short ones keep the minimum.
    #[test]
    fn the_year_menu_fits_its_widest_badge() {
        assert_eq!(super::year_menu_width(5), 160.0);
        assert!(super::year_menu_width("so far · 107 tx".chars().count()) >= 195.0);
        assert!(super::year_menu_width(22) > super::year_menu_width(15));
    }
}
