//! The Files panel (ReportFilesPanel.tsx): one row per file with a button per format, the
//! summary's and audit package's folded options, and the last export's outcome on top
//! (ExportStatus, QuarantineExportBlockNotice).

use rune::{Cx, El, col, el, row, text};

use super::model::{Region, Report};
use super::view::hexc;
use super::widgets::{card, checkbox, select_menu, select_trigger, toggle};
use super::{AUDIT_PARTS, Fmt, Fold, Popup, Status, StatusTone};
use crate::app::App;
use crate::format;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::reports::ledger::common as lc;
use crate::theme::Tk;
use crate::txdetail::widgets::{self as w, Size, Variant};
use crate::ui::ic;
use crate::ui::table::EDGE;
use rune_kit::Placement;

/// A file row's one action (owner rule: one visible action per row): an outline `h-8` Export
/// button, its glyph the spinner while one of `fmts` runs and a green check for 5 s after it
/// finished. `menu` makes it the trigger of the formats menu (a chevron after the label).
#[allow(clippy::too_many_arguments)]
fn export_button(
    app: &App,
    tk: &Tk,
    key: &str,
    fmts: &[Fmt],
    name: String,
    disabled: bool,
    year: i32,
    menu: Option<(bool, Popup)>,
) -> El {
    let ui = &app.reports.report;
    let loading = ui.active.is_some_and(|f| fmts.contains(&f));
    let success = ui
        .done
        .is_some_and(|(d, y, _)| fmts.contains(&d) && y == year);
    let enabled = ui.active.is_none() && !disabled;
    let glyph = if loading {
        ic(&I::LOADER_CIRCLE, tk.r(0.875), tk.fg).looping(rune::LoopSpec::rotation(1.0))
    } else if success {
        ic(
            &I::CHECK,
            tk.r(0.875),
            hexc(if tk.dark { "#34d399" } else { "#059669" }),
        )
    } else {
        ic(&I::DOWNLOAD, tk.r(0.875), tk.fg)
    };
    let mut b = w::button_surface(
        tk,
        row()
            .key(key.to_owned())
            .h(tk.r(2.0))
            .px(tk.r(0.625))
            .gap(tk.r(0.375))
            .center(),
        Variant::Outline,
    )
    .role("button")
    .label(name)
    .busy(loading)
    .focusable(enabled)
    .disabled(!enabled)
    .child(glyph)
    .child(
        text(t("reports:files.export"))
            .font(tk.tsm(500.0))
            .color(tk.fg)
            .single_line(),
    );
    if menu.is_some() {
        b = b.child(ic(&I::CHEVRON_DOWN, tk.r(0.875), tk.muted_fg));
    }
    if !enabled {
        b = b.opacity(0.5);
    }
    match (menu, fmts) {
        (Some((open, popup)), _) if enabled => {
            crate::ui::popup::trigger(b, crate::ui::popup::Kind::Menu, open, move |a: &mut App| {
                toggle(a, popup.clone())
            })
        }
        (None, [f]) if enabled => {
            let f = *f;
            b.on_press(move |a: &mut App| super::export(a, f))
        }
        _ => b,
    }
}

/// The tax report's formats menu under its Export button.
fn tax_formats_menu(tk: &Tk, layer: &mut Vec<El>) {
    use crate::ui::kit_menu::{Entry, Item};
    let item = |label: &str, desc: &str, glyph: &'static rune::Icon, f: Fmt| {
        Entry::Item(Item::new(label).description(t(desc)).icon(glyph).on_select(
            move |a: &mut App| {
                a.reports.report.popup = None;
                super::export(a, f);
            },
        ))
    };
    let entries = vec![
        item("PDF", "reports:files.formats.pdf", &I::FILE_TEXT, Fmt::Pdf),
        item(
            "XLSX",
            "reports:files.formats.xlsx",
            &I::FILE_SPREADSHEET,
            Fmt::Xlsx,
        ),
        item("CSV", "reports:files.formats.csv", &I::TABLE_2, Fmt::Csv),
    ];
    crate::ui::kit_menu::mount(
        tk,
        layer,
        "rep-fmt-tax-menu",
        "rep-fmt-tax",
        Placement::BottomEnd,
        4.0,
        tk.r(15.0),
        entries,
        std::rc::Rc::new(|a: &mut App| a.reports.report.popup = None),
    );
}

/// The wealth statement's formats menu under its Export button.
fn wealth_formats_menu(tk: &Tk, layer: &mut Vec<El>) {
    use crate::ui::kit_menu::{Entry, Item};
    let item = |label: &str, desc: &str, glyph: &'static rune::Icon, f: Fmt| {
        Entry::Item(Item::new(label).description(t(desc)).icon(glyph).on_select(
            move |a: &mut App| {
                a.reports.report.popup = None;
                super::export(a, f);
            },
        ))
    };
    let entries = vec![
        item(
            "PDF",
            "reports:files.formats.pdf",
            &I::FILE_TEXT,
            Fmt::WealthPdf,
        ),
        item(
            "XLSX",
            "reports:files.formats.xlsx",
            &I::FILE_SPREADSHEET,
            Fmt::WealthXlsx,
        ),
        item(
            "CSV",
            "reports:files.formats.csv",
            &I::TABLE_2,
            Fmt::WealthCsv,
        ),
    ];
    crate::ui::kit_menu::mount(
        tk,
        layer,
        "rep-fmt-wealth-menu",
        "rep-fmt-wealth",
        Placement::BottomEnd,
        4.0,
        tk.r(15.0),
        entries,
        std::rc::Rc::new(|a: &mut App| a.reports.report.popup = None),
    );
}

/// `FileRow` on the table kit's inset: the title and its detail on the left, the row's one
/// action on the right, and the options as a disclosure line under them.
fn file_row(
    tk: &Tk,
    key: &str,
    title: &str,
    detail: String,
    action: El,
    options: Option<(String, bool, Fold, El)>,
    last: bool,
) -> El {
    let head = row()
        .w_full()
        .gap(tk.r(0.75))
        .center_y()
        .child(
            col()
                .grow(1.0)
                .w(0.0)
                .min_w(0.0)
                .gap(tk.r(0.125))
                .child(
                    text(title.to_owned())
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .single_line()
                        .truncate()
                        .min_w(0.0),
                )
                .child(
                    text(detail)
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .w_full(),
                ),
        )
        .child(action);
    let mut li = col()
        .key(key.to_owned())
        .w_full()
        .px(tk.r(EDGE))
        .py(tk.r(0.75))
        .role("listitem")
        .child(head);
    if let Some((summary, open, fold, body)) = options {
        li = li.child(
            row()
                .key(format!("{key}-options"))
                .mt(tk.r(0.375))
                .ml(-tk.r(0.375))
                .px(tk.r(0.375))
                .h(tk.r(1.5))
                .min_w(0.0)
                .max_w(tk.r(20.0))
                .gap(tk.r(0.25))
                .center_y()
                .radius(tk.r(0.375))
                .hover(tk.muted.alpha(0.6))
                .cursor(rune::Cursor::Pointer)
                .role("button")
                .label(summary.clone())
                .expanded(open)
                .focusable(true)
                .on_press(move |a: &mut App| {
                    let ui = &mut a.reports.report;
                    ui.fold = if ui.fold == Some(fold) {
                        None
                    } else {
                        Some(fold)
                    };
                })
                .child(ic(
                    if open {
                        &I::CHEVRON_DOWN
                    } else {
                        &I::CHEVRON_RIGHT
                    },
                    tk.r(0.875),
                    tk.muted_fg,
                ))
                .child(
                    text(summary)
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .truncate()
                        .min_w(0.0)
                        .shrink(),
                ),
        );
        if open {
            li = li.child(
                col()
                    .w_full()
                    .mt(tk.r(0.5))
                    .padding(tk.r(0.75))
                    .radius(tk.r(0.5))
                    .fill(tk.muted.alpha(0.4))
                    .child(body),
            );
        }
    }
    if !last {
        li = li
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .border_layout();
    }
    li
}

/// A checkbox row: `flex cursor-pointer items-center gap-2 rounded-md px-1 py-1 text-xs
/// hover:bg-muted`.
fn check_row(
    tk: &Tk,
    key: String,
    label: &str,
    trailing: Option<&str>,
    on: bool,
    toggle_: impl Fn(&mut App) + 'static,
) -> El {
    let mut r = row()
        .key(key)
        .w_full()
        .px(tk.r(0.25))
        .py(tk.r(0.25))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.375))
        .hover(tk.muted)
        .role("checkbox")
        .checked(on)
        .label(label.to_owned())
        .focusable(true)
        .on_press(toggle_)
        .child(checkbox(tk, on))
        .child(
            text(label.to_owned())
                .font(tk.txs(400.0))
                .color(tk.fg)
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        );
    if let Some(tr) = trailing.filter(|s| !s.is_empty()) {
        r = r.child(text(tr.to_owned()).font(tk.t2xs(400.0)).color(tk.muted_fg));
    }
    r
}

/// The options' select-style dropdown trigger (`SelectTrigger` at `h-8`, `text-xs`): a leading
/// glyph, the value and a muted chevron, at one fixed width so the stacked fields line up.
fn field_trigger(
    tk: &Tk,
    key: &str,
    glyph: &'static rune::Icon,
    value: String,
    label: String,
    open: bool,
) -> El {
    let (fill, hover) = if tk.dark {
        (
            tk.input.alpha(tk.input.a * 0.3),
            tk.input.alpha(tk.input.a * 0.5),
        )
    } else {
        (tk.background, tk.accent)
    };
    row()
        .key(key.to_owned())
        .w(tk.r(9.5))
        .h(tk.r(2.0))
        .pl(tk.r(0.625))
        .pr(tk.r(0.5))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.375))
        .border(1.0, if open { tk.ring } else { tk.input })
        .border_layout()
        .fill(fill)
        .hover(hover)
        .cursor(rune::Cursor::Pointer)
        .role("button")
        .label(label)
        .a11y_value(value.clone())
        .focusable(true)
        .child(ic(glyph, tk.r(0.875), tk.muted_fg))
        .child(
            text(value)
                .font(super::widgets::tnum(tk.txs(500.0)))
                .color(tk.fg)
                .single_line()
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        )
        .child(ic(
            if open {
                &I::CHEVRON_UP
            } else {
                &I::CHEVRON_DOWN
            },
            tk.r(0.875),
            tk.muted_fg,
        ))
}

/// A field line in the options: its name on the left, its dropdown on the right.
fn field_row(tk: &Tk, label: String, control: El) -> El {
    row()
        .w_full()
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .child(
            text(label)
                .font(tk.txs(500.0))
                .color(tk.fg)
                .single_line()
                .truncate()
                .shrink()
                .min_w(0.0),
        )
        .child(control)
}

/// A switch line: a short name over a muted detail line, the switch on the right.
fn switch_row(
    tk: &Tk,
    key: &str,
    label: String,
    detail: String,
    on: bool,
    flip: impl Fn(&mut App) + 'static,
) -> El {
    row()
        .w_full()
        .gap(tk.r(0.75))
        .center_y()
        .child(
            col()
                .grow(1.0)
                .w(0.0)
                .min_w(0.0)
                .gap(tk.r(0.0625))
                .child(
                    text(label.clone())
                        .font(tk.txs(500.0))
                        .color(tk.fg)
                        .single_line()
                        .truncate()
                        .min_w(0.0),
                )
                .child(
                    text(detail.clone())
                        .font(tk.t2xs(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .w_full(),
                ),
        )
        .child(
            crate::ui::switch(tk, key, on, &label)
                .description(detail)
                .on_press(flip),
        )
}

/// Which wallet selection a checklist edits.
#[derive(Clone, Copy)]
enum Pick {
    Wealth,
    Summary,
}

impl Pick {
    fn get(self, a: &App) -> &Vec<String> {
        match self {
            Pick::Wealth => &a.reports.report.wealth.wallets,
            Pick::Summary => &a.reports.report.summary_wallets,
        }
    }
    fn get_mut(self, a: &mut App) -> &mut Vec<String> {
        match self {
            Pick::Wealth => &mut a.reports.report.wealth.wallets,
            Pick::Summary => &mut a.reports.report.summary_wallets,
        }
    }
    fn popup(self) -> Popup {
        match self {
            Pick::Wealth => Popup::WealthWallets,
            Pick::Summary => Popup::SummaryWallets,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Pick::Wealth => "rep-wealth-wallets",
            Pick::Summary => "rep-wallets",
        }
    }
}

/// The wallet selection as one compact dropdown (`Alle (6) ▾`, `2 von 6 ▾`) whose menu is a
/// checklist with "Select all" on top; the selection never goes empty.
fn wallet_dropdown(app: &App, tk: &Tk, layer: &mut Vec<El>, pick: Pick, label: String) -> El {
    use crate::ui::kit_menu::{Entry, Item};
    let ui = &app.reports.report;
    let wallets = ui.wallets.as_deref().unwrap_or(&[]);
    let sel = pick.get(app);
    let chosen = wallets.iter().filter(|w| sel.contains(&w.id)).count();
    let total = wallets.len();
    let value = if chosen >= total {
        tr(
            "reports:files.walletPicker.all",
            &[("count", &format::count(total))],
        )
    } else {
        tr(
            "reports:files.walletPicker.some",
            &[
                ("selected", &format::count(chosen)),
                ("total", &format::count(total)),
            ],
        )
    };
    let open = ui.popup == Some(pick.popup());
    let key = pick.key();
    let trigger = crate::ui::popup::trigger(
        field_trigger(
            tk,
            key,
            &I::WALLET,
            value,
            t("reports:files.walletPicker.choose"),
            open,
        ),
        crate::ui::popup::Kind::Menu,
        open,
        move |a: &mut App| toggle(a, pick.popup()),
    );
    if open {
        let all_ids: Vec<String> = wallets.iter().map(|w| w.id.clone()).collect();
        let mut entries = vec![Entry::Item(
            Item::new(t("reports:files.wealth.selectAll"))
                .checked(chosen >= total)
                .on_select(move |a: &mut App| *pick.get_mut(a) = all_ids.clone()),
        )];
        entries.push(Entry::Sep);
        for wc in wallets {
            let on = sel.contains(&wc.id);
            let id = wc.id.clone();
            let mut item = Item::new(wc.label.clone())
                .checked(on)
                // The last selected wallet stays: an empty selection exports nothing.
                .disabled(on && chosen <= 1)
                .on_select(move |a: &mut App| {
                    let sel = pick.get_mut(a);
                    if let Some(i) = sel.iter().position(|x| *x == id) {
                        if sel.len() > 1 {
                            sel.remove(i);
                        }
                    } else {
                        sel.push(id.clone());
                    }
                });
            if !wc.chain.is_empty() {
                item = item.badge(wc.chain.clone());
            }
            entries.push(Entry::Item(item));
        }
        crate::ui::kit_menu::mount(
            tk,
            layer,
            &format!("{key}-menu"),
            key,
            Placement::BottomEnd,
            4.0,
            tk.r(20.0),
            entries,
            std::rc::Rc::new(|a: &mut App| a.reports.report.popup = None),
        );
    }
    field_row(tk, label, trigger)
}

/// Open or close the cutoff calendar, starting on the selected date's month.
fn toggle_calendar(a: &mut App, year: i32) {
    let ui = &mut a.reports.report;
    if ui.popup == Some(Popup::WealthDate) {
        ui.popup = None;
    } else {
        let (y, m, _) =
            crate::reports::ledger::exit_tax::parse_iso(&super::resolved_wealth_date(ui, year));
        ui.wealth.month = (y, m);
        ui.wealth.cal_view = super::CalView::Days;
        ui.popup = Some(Popup::WealthDate);
    }
}

/// A 12-year page `[start, end]` covering `viewed` without dipping below Bitcoin's 2009 genesis
/// year unless the user stepped there explicitly.
fn year_page(viewed: i32, anchor: i32) -> (i32, i32) {
    let top = anchor.max(viewed);
    let offset = (top - viewed).div_euclid(12) * 12;
    let end = top - offset;
    let start = (end - 11).max(2009);
    (start, start + 11)
}

/// The cutoff calendar dropdown: scaling quick picks (`Heute` + `31. Dez. <shown year>`) on top,
/// independent Month ▾ and Year ▾ triggers in the header, and a fixed-height body switching
/// cleanly between days (6 weeks), 12 months, and 12 years at once.
fn wealth_calendar(app: &App, tk: &Tk, year: i32) -> El {
    use super::CalView;
    use crate::i18n::lang;
    use crate::reports::ledger::exit_tax::{
        days_in_month, first_weekday, format_display, months, months_short, parse_iso, weekdays,
    };
    let ui = &app.reports.report;
    let (vy, vm) = ui.wealth.month;
    let view = ui.wealth.cal_view;
    let resolved = super::resolved_wealth_date(ui, year);
    let selected = parse_iso(&resolved);
    let today = parse_iso("");
    let (yr_start, yr_end) = year_page(vy, today.0.max(year));
    let cell_w = tk.r(2.25);
    let gap = tk.r(0.25);
    let grid_w = cell_w * 7.0 + gap * 6.0;

    // Quick picks: two universal actions that scale to any multi-year history — Today, plus
    // 31 Dec of whichever year is currently viewed in the calendar.
    let chip_w = (grid_w - gap) / 2.0;
    let chip = |key: &str, label: String, iso: String| {
        let active = resolved == iso;
        let (fill, ink) = if active {
            (tk.primary, tk.primary_fg)
        } else {
            (tk.muted.alpha(0.6), tk.fg)
        };
        let mut c = row()
            .key(key.to_owned())
            .w(chip_w)
            .h(tk.r(1.75))
            .center()
            .radius(tk.r(0.375))
            .fill(fill)
            .cursor(rune::Cursor::Pointer)
            .role("button")
            .label(label.clone())
            .checked(active)
            .focusable(true)
            .on_press(move |a: &mut App| {
                a.reports.report.wealth.as_of = iso.clone();
                a.reports.report.popup = None;
            })
            .child(
                text(label)
                    .font(super::widgets::tnum(tk.txs(500.0)))
                    .color(ink)
                    .single_line(),
            );
        if !active {
            c = c.hover(tk.accent);
        }
        c
    };
    let today_iso = super::today_iso();
    let viewed_ye = format!("{vy:04}-12-31");
    let presets = row()
        .w(grid_w)
        .gap(gap)
        .role("group")
        .label(t("reports:files.wealth.calendar.quickPicks"))
        .child(chip(
            "rep-wealth-preset-today",
            t("reports:files.wealth.today"),
            today_iso,
        ))
        .child(chip(
            "rep-wealth-preset-ye",
            tr("reports:files.wealth.yearEnd", &[("year", &vy.to_string())]),
            viewed_ye,
        ));

    // Header: ‹ [Month ▾] [Year ▾] › (or ‹ [2015–2026 ▾] › in the 12-year view).
    let step = move |a: &mut App, delta: i32| {
        let w = &mut a.reports.report.wealth;
        let (y, m) = w.month;
        match w.cal_view {
            CalView::Days => {
                let total = (y * 12 + m as i32 + delta).max(2009 * 12);
                w.month = (total.div_euclid(12), total.rem_euclid(12) as u32);
            }
            CalView::Months => {
                w.month = ((y + delta).max(2009), m);
            }
            CalView::Years => {
                w.month = ((y + delta * 12).max(2009), m);
            }
        }
    };
    let nav = |key: &str, glyph: &'static rune::Icon, label: String, delta: i32| {
        w::icon_button(
            tk,
            key.to_owned(),
            Variant::Ghost,
            tk.r(2.0),
            glyph,
            tk.r(1.0),
            label,
        )
        .on_press(move |a: &mut App| step(a, delta))
    };
    let (prev_label, next_label) = match view {
        CalView::Days => (
            t("reports:files.wealth.calendar.previousMonth"),
            t("reports:files.wealth.calendar.nextMonth"),
        ),
        CalView::Months => (
            t("reports:files.wealth.calendar.previousYear"),
            t("reports:files.wealth.calendar.nextYear"),
        ),
        CalView::Years => (
            t("reports:files.wealth.calendar.previousYears"),
            t("reports:files.wealth.calendar.nextYears"),
        ),
    };
    let cap_pill =
        |key: &str, text_val: String, aria_label: String, active: bool, on_click: fn(&mut App)| {
            let mut p = row()
                .key(key.to_owned())
                .h(tk.r(2.0))
                .px(tk.r(0.5))
                .gap(tk.r(0.25))
                .center()
                .radius(tk.r(0.375))
                .cursor(rune::Cursor::Pointer)
                .role("button")
                .label(aria_label)
                .a11y_value(text_val.clone())
                .expanded(active)
                .focusable(true)
                .on_press(on_click)
                .child(
                    text(text_val)
                        .font(super::widgets::tnum(tk.tsm(600.0)))
                        .color(tk.fg)
                        .single_line(),
                )
                .child(ic(
                    if active {
                        &I::CHEVRON_UP
                    } else {
                        &I::CHEVRON_DOWN
                    },
                    tk.r(0.75),
                    tk.muted_fg,
                ));
            p = if active {
                p.fill(tk.accent)
            } else {
                p.hover(tk.accent)
            };
            p
        };
    let center_ctrl = match view {
        CalView::Years => cap_pill(
            "rep-wealth-cal-year-range",
            format!("{yr_start}–{yr_end}"),
            t("reports:files.wealth.calendar.backToDays"),
            true,
            |a: &mut App| a.reports.report.wealth.cal_view = CalView::Days,
        ),
        CalView::Days | CalView::Months => row()
            .gap(tk.r(0.25))
            .center_y()
            .child(cap_pill(
                "rep-wealth-cal-month",
                months(lang())[vm as usize].to_owned(),
                if view == CalView::Months {
                    t("reports:files.wealth.calendar.backToDays")
                } else {
                    t("reports:files.wealth.calendar.chooseMonth")
                },
                view == CalView::Months,
                |a: &mut App| {
                    let w = &mut a.reports.report.wealth;
                    w.cal_view = if w.cal_view == CalView::Months {
                        CalView::Days
                    } else {
                        CalView::Months
                    };
                },
            ))
            .child(cap_pill(
                "rep-wealth-cal-year",
                vy.to_string(),
                t("reports:files.wealth.calendar.chooseYear"),
                false,
                |a: &mut App| a.reports.report.wealth.cal_view = CalView::Years,
            )),
    };
    let header = row()
        .w(grid_w)
        .center_y()
        .justify_between()
        .child(nav("rep-wealth-cal-prev", &I::CHEVRON_LEFT, prev_label, -1))
        .child(center_ctrl)
        .child(nav("rep-wealth-cal-next", &I::CHEVRON_RIGHT, next_label, 1));

    // The weekday row plus six weeks: the month and 12-year grids fill the exact same box.
    let weekday_h = tk.r(1.5);
    let body_h = weekday_h + cell_w * 6.0 + gap * 5.0;
    let body = match view {
        CalView::Months => {
            let mcell_w = (grid_w - gap * 2.0) / 3.0;
            let mcell_h = (body_h - gap * 3.0) / 4.0;
            let mut grid = col()
                .w(grid_w)
                .h(body_h)
                .gap(gap)
                .role("grid")
                .label(vy.to_string())
                .focus_group();
            for r in 0..4u32 {
                let mut line = row().gap(gap);
                for c in 0..3u32 {
                    let m = r * 3 + c;
                    let is_sel = (vy, m) == (selected.0, selected.1);
                    let is_now = (vy, m) == (today.0, today.1);
                    let mut b = row()
                        .key(format!("rep-wealth-month-{m}"))
                        .size(mcell_w, mcell_h)
                        .center()
                        .radius(tk.r(0.375))
                        .cursor(rune::Cursor::Pointer)
                        .role("button")
                        .label(format!("{} {vy}", months(lang())[m as usize]))
                        .checked(is_sel)
                        .focusable(true)
                        .focus_selected(is_sel || (!is_sel && m == vm))
                        .on_press(move |a: &mut App| {
                            let w = &mut a.reports.report.wealth;
                            w.month = (w.month.0, m);
                            w.cal_view = CalView::Days;
                        });
                    b = if is_sel {
                        b.fill(tk.primary)
                    } else if is_now {
                        b.hover(tk.accent).border(1.0, tk.ring).border_layout()
                    } else {
                        b.hover(tk.accent)
                    };
                    line = line.child(
                        b.child(
                            text(months_short(lang())[m as usize])
                                .font(tk.tsm(if is_sel { 600.0 } else { 400.0 }))
                                .color(if is_sel { tk.primary_fg } else { tk.fg }),
                        ),
                    );
                }
                grid = grid.child(line);
            }
            grid
        }
        CalView::Years => {
            let ycell_w = (grid_w - gap * 2.0) / 3.0;
            let ycell_h = (body_h - gap * 3.0) / 4.0;
            let mut grid = col()
                .w(grid_w)
                .h(body_h)
                .gap(gap)
                .role("grid")
                .label(format!("{yr_start}–{yr_end}"))
                .focus_group();
            for r in 0..4i32 {
                let mut line = row().gap(gap);
                for c in 0..3i32 {
                    let y = yr_start + r * 3 + c;
                    let is_sel = y == selected.0;
                    let is_now = y == today.0;
                    let mut b = row()
                        .key(format!("rep-wealth-year-{y}"))
                        .size(ycell_w, ycell_h)
                        .center()
                        .radius(tk.r(0.375))
                        .cursor(rune::Cursor::Pointer)
                        .role("button")
                        .label(y.to_string())
                        .checked(is_sel)
                        .focusable(true)
                        .focus_selected(is_sel || (!is_sel && y == vy))
                        .on_press(move |a: &mut App| {
                            let w = &mut a.reports.report.wealth;
                            w.month = (y, w.month.1);
                            w.cal_view = CalView::Days;
                        });
                    b = if is_sel {
                        b.fill(tk.primary)
                    } else if is_now {
                        b.hover(tk.accent).border(1.0, tk.ring).border_layout()
                    } else {
                        b.hover(tk.accent)
                    };
                    line = line.child(
                        b.child(
                            text(y.to_string())
                                .font(super::widgets::tnum(tk.tsm(if is_sel || is_now {
                                    600.0
                                } else {
                                    400.0
                                })))
                                .color(if is_sel { tk.primary_fg } else { tk.fg }),
                        ),
                    );
                }
                grid = grid.child(line);
            }
            grid
        }
        CalView::Days => {
            let weekday_row = row()
                .w(grid_w)
                .h(weekday_h)
                .gap(gap)
                .a11y_hidden()
                .children(weekdays(lang()).map(|d| {
                    row()
                        .w(cell_w)
                        .h(weekday_h)
                        .center()
                        .child(text(d).font(tk.txs(500.0)).color(tk.muted_fg))
                }));
            // Six full weeks: the neighbouring months' days fill the edges, muted but pickable.
            let first = first_weekday(vy, vm) as i32;
            let n = days_in_month(vy, vm) as i32;
            let (py, pm) = if vm == 0 { (vy - 1, 11) } else { (vy, vm - 1) };
            let (ny, nm) = if vm == 11 { (vy + 1, 0) } else { (vy, vm + 1) };
            let pn = days_in_month(py, pm) as i32;
            let mut weeks = col()
                .gap(gap)
                .role("grid")
                .label(format!("{} {vy}", months(lang())[vm as usize]))
                .focus_group();
            for wk in 0..6 {
                let mut week = row().gap(gap);
                for wd in 0..7 {
                    let off = wk * 7 + wd - first;
                    let (y, m, d, inside) = if off < 0 {
                        (py, pm, (pn + off + 1) as u32, false)
                    } else if off >= n {
                        (ny, nm, (off - n + 1) as u32, false)
                    } else {
                        (vy, vm, (off + 1) as u32, true)
                    };
                    let before_floor = y < 2009;
                    let iso = format!("{y:04}-{:02}-{d:02}", m + 1);
                    let is_sel = !before_floor && (y, m, d) == selected;
                    let is_today = !before_floor && (y, m, d) == today;
                    let ink = if is_sel {
                        tk.primary_fg
                    } else if inside {
                        tk.fg
                    } else if before_floor {
                        tk.muted_fg.alpha(tk.muted_fg.a * 0.25)
                    } else {
                        tk.muted_fg.alpha(tk.muted_fg.a * 0.55)
                    };
                    let mut b = row()
                        .key(if inside {
                            format!("rep-wealth-day-{d}")
                        } else {
                            format!("rep-wealth-day-{iso}")
                        })
                        .size(cell_w, cell_w)
                        .center()
                        .radius(tk.r(0.375));
                    if before_floor {
                        b = b.a11y_hidden();
                    } else {
                        b = b
                            .cursor(rune::Cursor::Pointer)
                            .role("button")
                            .label(format_display(&iso, lang()))
                            .checked(is_sel)
                            .focusable(true)
                            .focus_selected(is_sel)
                            .on_press(move |a: &mut App| {
                                let ui = &mut a.reports.report;
                                ui.wealth.as_of = iso.clone();
                                ui.popup = None;
                            });
                        b = if is_sel {
                            b.fill(tk.primary)
                        } else if is_today {
                            b.hover(tk.accent).border(1.0, tk.ring).border_layout()
                        } else {
                            b.hover(tk.accent)
                        };
                    }
                    week = week.child(
                        b.child(
                            text(d.to_string())
                                .font(super::widgets::tnum(tk.tsm(if is_sel || is_today {
                                    600.0
                                } else {
                                    400.0
                                })))
                                .color(ink),
                        ),
                    );
                }
                weeks = weeks.child(week);
            }
            col().w(grid_w).h(body_h).child(weekday_row).child(weeks)
        }
    };
    crate::shell::glass(
        tk,
        col()
            .key("rep-wealth-cal")
            .role("dialog")
            .label(t("reports:files.wealth.pickDate"))
            .padding(tk.r(0.75))
            .gap(tk.r(0.5))
            .child(presets)
            .child(el().w(grid_w).h(1.0).fill(tk.border).a11y_hidden())
            .child(header)
            .child(body),
    )
}

fn wealth_options(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    region: Region,
    year: i32,
    picker: bool,
) -> El {
    let ui = &app.reports.report;
    let resolved = super::resolved_wealth_date(ui, year);
    let shown_date =
        crate::reports::ledger::exit_tax::format_display(&resolved, crate::i18n::lang());
    let open = ui.popup == Some(Popup::WealthDate);
    let date_btn = field_trigger(
        tk,
        "rep-wealth-date",
        &I::CALENDAR_DAYS,
        shown_date,
        t("reports:files.wealth.pickDate"),
        open,
    )
    .expanded(open)
    .on_press(move |a: &mut App| toggle_calendar(a, year))
    .on_key(
        move |a: &mut App, ev: &rune::KeyEvent, _tx: &mut rune::Tx| {
            if open || ev.mods.cmd || ev.mods.alt || ev.mods.ctrl {
                return false;
            }
            if matches!(ev.key, rune::Key::Down) {
                toggle_calendar(a, year);
                return true;
            }
            false
        },
    );
    if open {
        layer.push(rune_kit::scrim("rep-wealth-date-scrim", |a: &mut App| {
            a.reports.report.popup = None
        }));
        let surf = wealth_calendar(app, tk, year)
            .modal(true, |a: &mut App| a.reports.report.popup = None)
            .focus_return("rep-wealth-date");
        layer.push(crate::shell::anchored(
            tk,
            surf,
            "rep-wealth-date",
            Placement::BottomEnd,
            4.0,
        ));
    }
    let mut fields = col().w_full().gap(tk.r(0.5)).child(field_row(
        tk,
        t("reports:files.wealth.asOf"),
        date_btn,
    ));
    if picker {
        fields = fields.child(wallet_dropdown(
            app,
            tk,
            layer,
            Pick::Wealth,
            t("reports:files.wealth.wallets"),
        ));
    }
    let mut switches = col().w_full().gap(tk.r(0.625));
    if region == Region::At {
        switches = switches.child(switch_row(
            tk,
            "rep-wealth-regime",
            t("reports:files.wealth.includeRegime"),
            t("reports:files.wealth.includeRegimeDetail"),
            ui.wealth.include_regime,
            |a: &mut App| {
                a.reports.report.wealth.include_regime = !a.reports.report.wealth.include_regime
            },
        ));
    }
    switches = switches
        .child(switch_row(
            tk,
            "rep-wealth-valuation",
            t("reports:files.wealth.includeValuation"),
            t("reports:files.wealth.includeValuationDetail"),
            ui.wealth.include_valuation,
            |a: &mut App| {
                a.reports.report.wealth.include_valuation =
                    !a.reports.report.wealth.include_valuation
            },
        ))
        .child(switch_row(
            tk,
            "rep-wealth-breakdown",
            t("reports:files.wealth.includeWallets"),
            t(if ui.privacy_mode && ui.wealth.include_wallets {
                "reports:files.wealth.includeWalletsPrivate"
            } else {
                "reports:files.wealth.includeWalletsDetail"
            }),
            ui.wealth.include_wallets,
            |a: &mut App| {
                a.reports.report.wealth.include_wallets = !a.reports.report.wealth.include_wallets
            },
        ));
    let rule = || el().w_full().h(1.0).fill(tk.border).a11y_hidden();
    col()
        .w_full()
        .gap(tk.r(0.75))
        .child(fields)
        .child(rule())
        .child(switches)
        .child(rule())
        .child(
            text(t("reports:files.wealth.neverBlockedNote"))
                .font(tk.t2xs(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        )
}

fn summary_options(app: &App, tk: &Tk, layer: &mut Vec<El>, picker: bool) -> El {
    let ui = &app.reports.report;
    let cover = !ui.no_cover;
    let label = t("reports:files.summary.coverToggle");
    let mut body = col().w_full().gap(tk.r(0.75)).child(
        row()
            .w_full()
            .gap(tk.r(0.75))
            .center_y()
            .justify_between()
            .child(
                text(label.clone())
                    .font(tk.txs(500.0))
                    .color(tk.fg)
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0),
            )
            .child(
                crate::ui::switch(tk, "rep-cover", cover, &label)
                    .on_press(|a: &mut App| a.reports.report.no_cover = !a.reports.report.no_cover),
            ),
    );
    if picker {
        body = body.child(wallet_dropdown(
            app,
            tk,
            layer,
            Pick::Summary,
            t("reports:files.summary.wallets"),
        ));
    }
    body
}

fn audit_options(app: &App, cx: &Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let ui = &app.reports.report;
    let whole_book = t("reports:files.audit.wholeBook");
    let saved_case = t("reports:files.audit.savedCase");
    let scope_label = if ui.audit.case_scope {
        saved_case.as_str()
    } else {
        whole_book.as_str()
    };
    let open = app.reports.report.popup == Some(Popup::AuditScope);
    let scope = crate::ui::popup::trigger(
        select_trigger(
            tk,
            "rep-audit-scope",
            Some(&t("reports:files.audit.scopeLabel")),
            scope_label,
            false,
            true,
            true,
            open,
            false,
        ),
        crate::ui::popup::Kind::Listbox,
        open,
        |a: &mut App| toggle(a, Popup::AuditScope),
    );
    if open {
        let w = cx
            .layout_size("rep-audit-scope")
            .map(|[w, _]| w)
            .unwrap_or(tk.r(8.0));
        let options = vec![
            ("active_profile".to_owned(), whole_book.clone(), false),
            (
                "source_funds_case".to_owned(),
                saved_case.clone(),
                ui.cases.is_empty(),
            ),
        ];
        let current = if ui.audit.case_scope {
            "source_funds_case"
        } else {
            "active_profile"
        };
        select_menu(
            tk,
            layer,
            "rep-audit-scope",
            w,
            options,
            current,
            false,
            |a: &mut App, v: &str| super::set_case_scope(a, v == "source_funds_case"),
        );
    }
    let mut scope_box = col()
        .w_full()
        .gap(tk.r(0.375))
        .child(
            text(t("reports:files.audit.scope"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        )
        .child(row().child(scope));
    if ui.audit.case_scope {
        let open = app.reports.report.popup == Some(Popup::AuditCase);
        let current = ui
            .cases
            .iter()
            .find(|(id, _)| *id == ui.audit.case_id)
            .map(|(_, l)| l.clone());
        let trigger = select_trigger(
            tk,
            "rep-audit-case",
            Some(&t("reports:files.audit.case")),
            &current
                .clone()
                .unwrap_or_else(|| t("reports:files.audit.chooseCase")),
            current.is_none(),
            true,
            true,
            open,
            false,
        );
        let trigger = crate::ui::popup::trigger(
            trigger,
            crate::ui::popup::Kind::Listbox,
            open,
            |a: &mut App| toggle(a, Popup::AuditCase),
        );
        if open {
            let w = cx
                .layout_size("rep-audit-case")
                .map(|[w, _]| w)
                .unwrap_or(tk.r(8.0));
            let options = ui
                .cases
                .iter()
                .map(|(id, l)| (id.clone(), l.clone(), false))
                .collect();
            select_menu(
                tk,
                layer,
                "rep-audit-case",
                w,
                options,
                &ui.audit.case_id,
                false,
                |a: &mut App, v: &str| a.reports.report.audit.case_id = v.to_owned(),
            );
        }
        scope_box = scope_box.child(row().child(trigger));
    }
    let mut list = col().w_full().gap(tk.r(0.125)).role("list");
    for (i, part) in AUDIT_PARTS.iter().enumerate() {
        let on = ui.audit.include[i];
        list = list.child(check_row(
            tk,
            format!("rep-audit-part-{i}"),
            &t(part),
            None,
            on,
            move |a: &mut App| {
                a.reports.report.audit.include[i] = !a.reports.report.audit.include[i]
            },
        ));
    }
    col().w_full().gap(tk.r(0.75)).child(scope_box).child(
        col()
            .w_full()
            .gap(tk.r(0.25))
            .child(
                text(t("reports:files.audit.include"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            )
            .child(list),
    )
}

/// `Intl.DateTimeFormat(locale, {dateStyle: "medium"})` for an ISO timestamp.
fn medium_date(s: &str) -> String {
    let Some(day) = format::Day::parse(s) else {
        return s.to_owned();
    };
    match crate::i18n::lang() {
        crate::i18n::Lang::De => {
            let iso = day.iso();
            format!("{}.{}.{}", &iso[8..10], &iso[5..7], &iso[0..4])
        }
        crate::i18n::Lang::En => day.long(),
    }
}

fn short_txid(v: &str) -> String {
    if v.chars().count() > 20 {
        let c: Vec<char> = v.chars().collect();
        format!(
            "{}…{}",
            c[..10].iter().collect::<String>(),
            c[c.len() - 6..].iter().collect::<String>()
        )
    } else {
        v.to_owned()
    }
}

/// `QuarantineExportBlockNotice`.
fn quarantine_notice(tk: &Tk, q: &super::Quarantine, draft_fmt: Option<Fmt>) -> El {
    let amber = hexc(if tk.dark { "#fbbf24" } else { "#d97706" });
    let mut c = col()
        .w_full()
        .padding(tk.r(1.0))
        .gap(tk.r(0.75))
        .radius(tk.r(0.5))
        .border(1.0, hexc("#f59e0b").alpha(0.4))
        .border_layout()
        .fill(hexc("#f59e0b").alpha(0.1))
        .role("alert")
        .child(
            row()
                .w_full()
                .gap(tk.r(0.625))
                .items_start()
                .child(ic(&I::SHIELD_ALERT, tk.r(1.0), amber).mt(tk.r(0.125)))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(
                            text(tc(
                                "transactions:dashboard.export.blocked.title",
                                q.count,
                                &[],
                            ))
                            .font(tk.tsm(500.0))
                            .color(tk.fg)
                            .wrap()
                            .w_full(),
                        )
                        .child(
                            text(t("transactions:dashboard.export.blocked.body"))
                                .font(tk.txs(400.0).lh(tk.r(1.25)))
                                .color(tk.muted_fg)
                                .wrap()
                                .w_full()
                                .mt(tk.r(0.125)),
                        ),
                )
                .child(crate::shell::transient::dismiss(
                    tk,
                    "rep-quarantine-dismiss",
                    tk.muted_fg,
                    |a: &mut App| a.reports.report.status = None,
                )),
        );
    let mut list = col().w_full().gap(tk.r(0.25));
    for tx in q.rows.iter().take(5) {
        let id = tx["transaction_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| tx["transaction_ref"].as_str())
            .unwrap_or("");
        list = list.child(
            row()
                .w_full()
                .px(tk.r(0.5))
                .py(tk.r(0.375))
                .gap(tk.r(0.75))
                .center_y()
                .radius(tk.r(0.375))
                .fill(tk.background.alpha(0.6))
                .child(
                    text(medium_date(tx["occurred_at"].as_str().unwrap_or("")))
                        .font(super::widgets::tnum(tk.txs(400.0)))
                        .color(tk.muted_fg),
                )
                .child(
                    row()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(tk.r(0.5))
                        .child(
                            text(tx["wallet"].as_str().unwrap_or("").to_owned())
                                .font(tk.txs(500.0))
                                .color(tk.fg)
                                .truncate(),
                        )
                        .child(
                            text(short_txid(id))
                                .font(rune::TextStyle::new(tk.r(0.75), 400.0).mono())
                                .color(tk.muted_fg)
                                .truncate(),
                        ),
                )
                .child(
                    text(tx["reason"].as_str().unwrap_or("").to_owned())
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                ),
        );
    }
    c = c.child(list);
    if q.count > 5 {
        c = c.child(
            text(tc(
                "transactions:dashboard.export.blocked.more",
                q.count - 5,
                &[],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        );
    }
    let mut actions = col().w_full().gap(tk.r(0.5)).child(
        row().child(
            w::button(
                tk,
                "rep-review-quarantine",
                Variant::Outline,
                Size::Sm,
                Some(&I::ARROW_UP_RIGHT),
                &t("transactions:dashboard.export.blocked.review"),
                true,
            )
            .on_press(|a: &mut App| a.navigate("/review")),
        ),
    );
    if let Some(fmt) = draft_fmt {
        actions = actions.child(
            row().child(
                w::button(
                    tk,
                    "rep-export-draft-quarantine",
                    Variant::Outline,
                    Size::Sm,
                    Some(&I::DOWNLOAD),
                    &t("reports:files.exportDraft"),
                    true,
                )
                .help(t("reports:files.draftHint"))
                .on_press(move |a: &mut App| super::export_draft(a, fmt)),
            ),
        );
    }
    c.child(actions)
}

/// `ExportStatus`: the outcome strip under the header.
fn export_status(app: &App, tk: &Tk, s: &Status) -> El {
    let (fill, ink) = match s.tone {
        StatusTone::Error => (tk.destructive.alpha(0.1), tk.destructive),
        StatusTone::Warning => (
            hexc("#f59e0b").alpha(0.1),
            hexc(if tk.dark { "#fde68a" } else { "#92400e" }),
        ),
        StatusTone::Success => (
            hexc("#10b981").alpha(0.1),
            hexc(if tk.dark { "#a7f3d0" } else { "#065f46" }),
        ),
    };
    let glyph = if s.tone == StatusTone::Success {
        &I::CIRCLE_CHECK
    } else {
        &I::TRIANGLE_ALERT
    };
    let mut body = col().grow(1.0).w(0.0).min_w(0.0).child(
        text(s.message.clone())
            .font(tk.tsm(400.0))
            .color(ink)
            .wrap()
            .w_full(),
    );
    if !s.notes.is_empty() {
        body = body.child(col().w_full().mt(tk.r(0.25)).gap(tk.r(0.125)).children(
            s.notes.iter().map(|n| {
                text(n.clone())
                    .font(tk.txs(400.0))
                    .color(ink)
                    .wrap()
                    .w_full()
            }),
        ));
    }
    if let Some(p) = &s.path {
        body = body.child(
            text(p.clone())
                .font(rune::TextStyle::new(tk.r(0.75), 400.0).mono().lh(tk.r(1.0)))
                .color(ink.alpha(ink.a * 0.8))
                .truncate()
                .w_full()
                .mt(tk.r(0.25))
                .help(p.clone()),
        );
    }
    if let Some(fmt) = s.can_export_draft {
        body = body.child(
            row().mt(tk.r(0.5)).child(
                w::button(
                    tk,
                    "rep-export-draft",
                    Variant::Outline,
                    Size::Sm,
                    Some(&I::DOWNLOAD),
                    &t("reports:files.exportDraft"),
                    true,
                )
                .help(t("reports:files.draftHint"))
                .on_press(move |a: &mut App| super::export_draft(a, fmt)),
            ),
        );
    }
    // `toastopen`: Show in Finder (and Open for a saved copy) as quiet text actions under the
    // path, so the narrow Files column keeps the message's width.
    if s.tone != StatusTone::Error && !s.actions.is_empty() {
        body = body.child(
            crate::toastopen::links(
                tk,
                &s.actions,
                |a| format!("rep-export-{}", a.id),
                ink.alpha(ink.a * 0.8),
                ink,
            )
            .mt(tk.r(0.375)),
        );
    }
    // Open: only for a desktop-local path while the status is not an error.
    let openable = s.open_path.clone().filter(|p| {
        s.tone != StatusTone::Error
            && (p.starts_with('/') || p.get(1..3).is_some_and(|x| x == ":\\" || x == ":/"))
    });
    let mut strip = row()
        .key("rep-export-status")
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(0.625))
        .gap(tk.r(0.75))
        .items_start()
        .fill(fill)
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .role(if s.tone == StatusTone::Error {
            "alert"
        } else {
            "status"
        })
        .child(ic(glyph, tk.r(1.0), ink).mt(tk.r(0.125)))
        .child(body);
    if let Some(path) = openable {
        let b = w::button_surface(
            tk,
            row()
                .key("rep-export-open")
                .h(tk.r(1.75))
                .px(tk.r(0.5))
                .gap(tk.r(0.375))
                .center(),
            Variant::Outline,
        )
        .fill(tk.background)
        .role("button")
        .label(t("common:actions.open"))
        .focusable(true)
        .on_press(move |a: &mut App| super::open_export(a, &path))
        .child(ic(&I::EXTERNAL_LINK, tk.r(0.875), tk.fg))
        .child(
            text(t("common:actions.open"))
                .font(tk.txs(500.0))
                .color(tk.fg),
        );
        strip = strip.child(b);
    }
    let _ = app;
    // Owner request over the web: the strip can be dismissed (the next export or leaving the
    // page clears it too).
    strip.child(crate::shell::transient::dismiss(
        tk,
        "rep-export-status-dismiss",
        ink,
        |a: &mut App| a.reports.report.status = None,
    ))
}

pub fn panel(app: &App, cx: &Cx<App>, tk: &Tk, layer: &mut Vec<El>, r: &Report, year: i32) -> El {
    let ui = &app.reports.report;
    let region = r.region();
    // While another year loads, the report still describes the previous one.
    let e1kv = if r.year == year {
        r.e1kv_applies()
    } else {
        year >= super::model::E1KV_FIRST_TAX_YEAR
    };
    let before = if r.year == year {
        r.rows_before_fields
    } else {
        0.0
    };
    let refused = region == Region::At && (!e1kv || before > 0.0);
    let wallets = ui.wallets.as_deref().unwrap_or(&[]);
    let picker = wallets.len() > 1;
    let selected = if picker {
        ui.summary_wallets.len()
    } else {
        wallets.len()
    };
    let included = ui.audit.include.iter().filter(|x| **x).count();
    let case_missing = ui.audit.case_scope && ui.audit.case_id.is_empty();

    // What no file holds: the card's one muted line (its hover lists every exclusion).
    let title = t("reports:files.title");
    let head = lc::card_head(tk, title.clone(), Some(t("reports:files.noSecrets")), None)
        .help(t("reports:files.exclusions"));
    let privacy_label = t("reports:files.privacyMode");
    let privacy_detail = t("reports:files.privacyModeDetail");
    let on = ui.privacy_mode;
    let privacy_bar = row()
        .key("rep-privacy-bar")
        .w_full()
        .px(tk.r(EDGE))
        .py(tk.r(0.625))
        .gap(tk.r(0.75))
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout()
        .fill(if on {
            tk.accent.alpha(0.5)
        } else {
            tk.muted.alpha(0.25)
        })
        .help(t("reports:files.privacyModeHint"))
        .child(
            row()
                .size(tk.r(1.75), tk.r(1.75))
                .center()
                .radius(tk.r(0.375))
                .fill(if on {
                    tk.primary.alpha(0.12)
                } else {
                    tk.muted.alpha(0.6)
                })
                .a11y_hidden()
                .child(ic(
                    &I::SHIELD_CHECK,
                    tk.r(0.875),
                    if on { tk.fg } else { tk.muted_fg },
                )),
        )
        .child(
            col()
                .grow(1.0)
                .w(0.0)
                .min_w(0.0)
                .gap(tk.r(0.0625))
                .child(
                    text(privacy_label.clone())
                        .font(tk.txs(500.0))
                        .color(if on { tk.fg } else { tk.muted_fg })
                        .single_line()
                        .truncate()
                        .min_w(0.0),
                )
                .child(
                    text(privacy_detail.clone())
                        .font(tk.t2xs(400.0))
                        .color(tk.muted_fg)
                        .single_line()
                        .truncate()
                        .min_w(0.0),
                ),
        )
        .child(
            crate::ui::switch(tk, "rep-privacy-mode", on, &privacy_label)
                .description(privacy_detail)
                .on_press(|a: &mut App| {
                    a.reports.report.privacy_mode = !a.reports.report.privacy_mode
                }),
        );
    let mut card = card(tk, "rep-files")
        .role("region")
        .label(title)
        .child(head)
        .child(privacy_bar);
    match &ui.status {
        Some(s) if s.year == year => {
            card = match &s.quarantine {
                Some(q) => card.child(
                    col()
                        .w_full()
                        .padding(tk.r(0.75))
                        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                        .border_layout()
                        .child(quarantine_notice(tk, q, s.can_export_draft)),
                ),
                None => card.child(export_status(app, tk, s)),
            };
        }
        _ => {}
    }
    let y = year.to_string();
    let tax_detail = if region != Region::At {
        t("reports:files.tax.everyYear")
    } else if !e1kv {
        tr(
            "reports:files.tax.noKennzahlen",
            &[("year", &super::model::E1KV_FIRST_TAX_YEAR.to_string())],
        )
    } else if before > 0.0 {
        tr("reports:files.tax.rowsBefore", &[("year", &y)])
    } else {
        tr("reports:files.tax.e1kv", &[("year", &y)])
    };
    let fold = ui.fold;
    let summary_label = format!(
        "{} · {}",
        if picker && selected < wallets.len() {
            tr(
                "reports:files.summary.someWallets",
                &[
                    ("selected", &format::count(selected)),
                    ("total", &format::count(wallets.len())),
                ],
            )
        } else if wallets.len() == 1 {
            t("reports:files.summary.oneWallet")
        } else {
            t("reports:files.summary.allWallets")
        },
        t(if ui.no_cover {
            "reports:files.summary.noCover"
        } else {
            "reports:files.summary.cover"
        })
    );
    let audit_label = format!(
        "{} · {}",
        t(if ui.audit.case_scope {
            "reports:files.audit.oneCase"
        } else {
            "reports:files.audit.wholeBook"
        }),
        tr(
            "reports:files.audit.parts",
            &[
                ("included", &format::count(included)),
                ("total", &format::count(AUDIT_PARTS.len())),
            ],
        )
    );
    let wealth_date_iso = super::resolved_wealth_date(ui, year);
    let wealth_date_disp =
        crate::reports::ledger::exit_tax::format_display(&wealth_date_iso, crate::i18n::lang());
    let mut wealth_parts = vec![wealth_date_disp.clone()];
    if region == Region::At {
        wealth_parts.push(t(if ui.wealth.include_regime {
            "reports:files.wealth.regimeOn"
        } else {
            "reports:files.wealth.regimeOff"
        }));
    }
    wealth_parts.push(t(if ui.wealth.include_valuation {
        "reports:files.wealth.valuationOn"
    } else {
        "reports:files.wealth.valuationOff"
    }));
    wealth_parts.push(t(if ui.wealth.include_wallets {
        "reports:files.wealth.walletsOn"
    } else {
        "reports:files.wealth.walletsOff"
    }));
    let wealth_label = wealth_parts.join(" · ");
    let wealth_body = if fold == Some(Fold::Wealth) {
        wealth_options(app, tk, layer, region, year, picker)
    } else {
        el()
    };
    let summary_body = if fold == Some(Fold::Summary) {
        summary_options(app, tk, layer, picker)
    } else {
        el()
    };
    let audit_body = if fold == Some(Fold::Audit) {
        audit_options(app, cx, tk, layer)
    } else {
        el()
    };
    if ui.popup == Some(Popup::TaxFormats) && ui.active.is_none() && !refused {
        tax_formats_menu(tk, layer);
    }
    if ui.popup == Some(Popup::WealthFormats) && ui.active.is_none() {
        wealth_formats_menu(tk, layer);
    }
    let tax_open = ui.popup == Some(Popup::TaxFormats);
    let wealth_open = ui.popup == Some(Popup::WealthFormats);
    let export_as = |file: &str, format: &str| {
        tr(
            "reports:files.exportFileAs",
            &[("file", &t(file)), ("format", format)],
        )
    };
    let list = col()
        .w_full()
        .role("list")
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout()
        .child(file_row(
            tk,
            "rep-file-tax",
            &t("reports:files.tax.title"),
            tax_detail,
            export_button(
                app,
                tk,
                "rep-fmt-tax",
                &[Fmt::Pdf, Fmt::Xlsx, Fmt::Csv],
                tr(
                    "reports:files.exportFile",
                    &[("file", &t("reports:files.tax.title"))],
                ),
                refused,
                year,
                Some((tax_open, Popup::TaxFormats)),
            ),
            None,
            false,
        ))
        .child(file_row(
            tk,
            "rep-file-handoff",
            &t("reports:files.handoff.title"),
            tr("reports:files.handoff.detail", &[("year", &y)]),
            export_button(
                app,
                tk,
                "rep-fmt-year_handoff",
                &[Fmt::YearHandoff],
                export_as("reports:files.handoff.title", "XLSX"),
                false,
                year,
                None,
            ),
            None,
            false,
        ))
        .child(file_row(
            tk,
            "rep-file-wealth",
            &t("reports:files.wealth.title"),
            tr(
                "reports:files.wealth.detail",
                &[("date", &wealth_date_disp)],
            ),
            export_button(
                app,
                tk,
                "rep-fmt-wealth",
                &[Fmt::WealthPdf, Fmt::WealthXlsx, Fmt::WealthCsv],
                tr(
                    "reports:files.exportFile",
                    &[("file", &t("reports:files.wealth.title"))],
                ),
                picker && ui.wealth.wallets.is_empty(),
                year,
                Some((wealth_open, Popup::WealthFormats)),
            ),
            Some((
                wealth_label,
                fold == Some(Fold::Wealth),
                Fold::Wealth,
                wealth_body,
            )),
            false,
        ))
        .child(file_row(
            tk,
            "rep-file-summary",
            &t("reports:files.summary.title"),
            tr("reports:files.summary.detail", &[("year", &y)]),
            export_button(
                app,
                tk,
                "rep-fmt-summary_pdf",
                &[Fmt::SummaryPdf],
                export_as("reports:files.summary.title", "PDF"),
                picker && ui.summary_wallets.is_empty(),
                year,
                None,
            ),
            Some((
                summary_label,
                fold == Some(Fold::Summary),
                Fold::Summary,
                summary_body,
            )),
            false,
        ))
        .child(file_row(
            tk,
            "rep-file-audit",
            &t("reports:files.audit.title"),
            t("reports:files.audit.detail"),
            export_button(
                app,
                tk,
                "rep-fmt-audit_package",
                &[Fmt::AuditPackage],
                tr(
                    "reports:files.exportFile",
                    &[("file", &t("reports:files.audit.title"))],
                ),
                case_missing,
                year,
                None,
            ),
            Some((
                audit_label,
                fold == Some(Fold::Audit),
                Fold::Audit,
                audit_body,
            )),
            true,
        ));
    card.child(list)
}
