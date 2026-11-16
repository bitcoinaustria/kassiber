//! The Reports page (`ReportOverview` / `ReportsView`): the readiness header with the tax-year
//! select, the preview-country notice, the metric strip, then the breakdown and rules cards
//! beside the Files panel, and the Lightning panel under them.

use rune::{Color, Cx, El, col, row, text};
use serde_json::Value;

use super::model::{self, Completeness, Glyph, Region, Report, Tone};
use super::widgets::{arrow_link, card, info_note, select_menu, select_trigger, tnum, toggle};
use super::{Popup, files};
use crate::app::{App, Load};
use crate::format;
use crate::i18n::t;
use crate::icons as I;
use crate::reports::ledger::common::{self as lc, Col, ColW, avail, column_widths, tw};
use crate::theme::Tk;
use crate::ui::table::{EDGE, PX};
use crate::ui::{ic, sensitive, surface};

pub(super) fn hexc(s: &str) -> Color {
    rune::color::hex(s).unwrap_or(Color::WHITE)
}

/// emerald / red text (`600`/`400` on the metric, `700`/`300` in the breakdown).
pub(super) fn gain_ink(tk: &Tk, v: f64, strong: bool) -> Color {
    let (pos, neg) = match (strong, tk.dark) {
        (true, false) => ("#059669", "#dc2626"),
        (true, true) => ("#34d399", "#f87171"),
        (false, false) => ("#047857", "#b91c1c"),
        (false, true) => ("#6ee7b7", "#fca5a5"),
    };
    if v > 0.0 {
        hexc(pos)
    } else if v < 0.0 {
        hexc(neg)
    } else if strong {
        tk.fg
    } else {
        tk.muted_fg
    }
}

/// `completeness` from the overview read; None (unknown) when it failed, `pending` while loading.
fn completeness(app: &App) -> (Option<Completeness>, bool) {
    match &app.overview {
        Load::Ready(s) => {
            let c = &s.fiat.completeness;
            (
                Some(Completeness {
                    cost_basis_complete: c.cost_basis_complete,
                    reasons: c.reasons.clone(),
                    missing_prices: c.missing_price_count.max(0.0) as usize,
                }),
                false,
            )
        }
        Load::Loading | Load::Idle => (None, true),
        Load::Failed(_) => (None, false),
    }
}

/// The page's content width (`@container/reports`).
fn page_width(app: &App, cx: &Cx<App>, tk: &Tk) -> f32 {
    avail(
        cx,
        "rep-page",
        lc::page_w(app, cx, tk) + 2.0 * tk.page_pad(),
    ) - 2.0 * tk.page_pad()
}

pub fn page(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let ui = &app.reports.report;
    let data = match ui.current() {
        None | Some(Load::Idle) | Some(Load::Loading) => {
            return lc::screen_skeleton(tk, tk.r(12.0), false).key("rep-page");
        }
        Some(Load::Failed(e)) => return unavailable(tk, e),
        Some(Load::Ready(d)) => d,
    };
    let report = Report::parse(data);
    if let Some(y) = ui.year
        && report.year != y
    {
        return unavailable(
            tk,
            &format!(
                "The daemon returned {} while the selected tax year is {y}.",
                report.year
            ),
        );
    }
    let year = ui.year.unwrap_or(report.year);
    let width = page_width(app, cx, tk);
    let hidden = app.prefs.hide_sensitive();
    let (comp, pending) = completeness(app);
    let readiness = model::readiness(
        &report,
        report.row_count,
        year,
        comp.as_ref(),
        pending,
        blocking_source_count(app),
    );
    let provisional = model::provisional(&report, comp.as_ref(), pending);
    let mut page = lc::shell(tk)
        .key("rep-page")
        .child(header(app, cx, tk, layer, &report, year));
    if let Some(n) = preview_notice(tk, &report.jurisdiction) {
        page = page.child(n);
    }
    page = page.child(metrics(
        app,
        cx,
        tk,
        &report,
        &readiness,
        hidden,
        provisional,
    ));
    let left = col()
        .w_full()
        .min_w(0.0)
        .gap(tk.card_gap())
        .child(breakdown(app, cx, tk, layer, &report, year, hidden, width))
        .child(rules(tk, &report, year));
    let right = files::panel(app, cx, tk, layer, &report, year);
    // `@5xl/reports:grid-cols-[minmax(0,1fr)_22rem]`, `items-start`.
    page = if width >= tk.r(64.0) {
        page.child(
            row()
                .w_full()
                .gap(tk.card_gap())
                .items_start()
                .child(left.grow(1.0).w(0.0))
                .child(col().w(tk.r(22.0)).min_w(tk.r(22.0)).child(right)),
        )
    } else {
        page.child(left).child(right)
    };
    if let Some(ln) = super::extra::lightning_panel(app, cx, tk, layer) {
        page = page.child(ln);
    }
    page
}

fn unavailable(tk: &Tk, message: &str) -> El {
    // `screenPanelClassName` + `kb-surface p-(--kb-card-padding)`.
    col()
        .key("rep-page")
        .w_full()
        .padding(tk.page_pad())
        .fill(tk.background)
        .child(
            surface(tk)
                .w_full()
                .padding(tk.card_pad())
                .gap(tk.r(0.25))
                .role("alert")
                .child(
                    text("Reports unavailable")
                        .font(tk.tbase(600.0))
                        .color(tk.fg),
                )
                .child(
                    text(message.to_owned())
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .w_full(),
                ),
        )
}

// ---- Header ---------------------------------------------------------------------------------

/// The readiness state's glyph and signal colour (None while the book is ready: the page then
/// says nothing about it, as the sibling pages do).
fn readiness_signal(tk: &Tk, r: &model::Readiness) -> Option<(&'static rune::Icon, Option<Color>)> {
    let glyph = match r.glyph {
        Glyph::WalletCards => &I::WALLET_CARDS,
        Glyph::RefreshCw => &I::REFRESH_CW,
        Glyph::ShieldAlert => &I::SHIELD_ALERT,
        Glyph::Loader => &I::LOADER_CIRCLE,
        Glyph::Alert => &I::TRIANGLE_ALERT,
        Glyph::Check => &I::CIRCLE_CHECK,
    };
    match r.tone {
        Tone::Good => None,
        Tone::Warning => Some((glyph, Some(tk.warn.solid))),
        Tone::Alert => Some((glyph, Some(tk.alert.solid))),
        Tone::Neutral => Some((glyph, None)),
    }
}

/// The page header in the sibling pages' pattern (Journals, Transfers): the period select on
/// the left, the running export's status beside it; the page's actions live in the Files card.
fn header(app: &App, cx: &Cx<App>, tk: &Tk, layer: &mut Vec<El>, report: &Report, year: i32) -> El {
    let locked = app.reports.report.active.is_some();
    let open = app.reports.report.popup == Some(Popup::Year);
    let trigger = select_trigger(
        tk,
        "rep-year",
        Some("Tax year"),
        &report.period_of(year),
        false,
        false,
        !locked,
        open,
        false,
    );
    let trigger = if locked {
        trigger
    } else {
        crate::ui::popup::trigger(
            trigger,
            crate::ui::popup::Kind::Listbox,
            open,
            |a: &mut App| toggle(a, Popup::Year),
        )
    };
    if open {
        let w = cx
            .layout_size("rep-year")
            .map(|[w, _]| w)
            .unwrap_or(tk.r(9.0));
        let years = report
            .years(year)
            .into_iter()
            .map(|y| (y.to_string(), report.year_label(y), false))
            .collect();
        select_menu(
            tk,
            layer,
            "rep-year",
            w,
            years,
            &year.to_string(),
            true,
            |a: &mut App, v: &str| {
                if let Ok(y) = v.parse::<i32>() {
                    super::set_year(a, y);
                }
            },
        );
    }
    let mut left = row().min_w(0.0).gap(tk.r(0.75)).center_y().child(trigger);
    if locked {
        left = left.child(lc::inline_status(tk, "Exporting…".into()));
    }
    lc::page_header(tk, left, None)
}

/// `TaxCountryPreviewNotice`: `rounded-md border px-3 py-2 text-xs leading-5`.
fn preview_notice(tk: &Tk, country: &str) -> Option<El> {
    if !crate::reports::books::model::is_preview(country) {
        return None;
    }
    let body = format!(
        "{}: {}",
        t("common:taxCountry.previewBadge"),
        t("common:taxCountry.previewNotice")
    );
    Some(
        col()
            .w_full()
            .px(tk.r(0.75))
            .py(tk.r(0.5))
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .border_layout()
            .role("note")
            .label(body)
            .child({
                let badge = format!("{}: ", t("common:taxCountry.previewBadge"));
                let n = badge.len();
                rune::rich(
                    format!("{badge}{}", t("common:taxCountry.previewNotice")),
                    vec![rune::Span::new(0, n).weight(500.0)],
                )
                .font(tk.txs(400.0).lh(tk.r(1.25)))
                .color(tk.fg)
                .w_full()
            }),
    )
}

// ---- Stat tiles -----------------------------------------------------------------------------

const PROVISIONAL_HELP: &str =
    "Quarantined rows, custody gaps, missing prices, or stale journals are not reflected yet.";

/// The headline figures as the home screen's stat tiles (an owner request, as on Wallets,
/// Transactions, Transfers and Journals): the readiness tile only while something stands
/// between the book and its reports (it links to the fix), then proceeds, cost basis, the
/// gain or loss and, for Austrian books, the indicative tax. "Provisional" marks the figures a
/// held row or a gap can still move.
fn metrics(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    r: &Report,
    readiness: &model::Readiness,
    hidden: bool,
    provisional: bool,
) -> El {
    use crate::overview::grid::stat::{Stat, row_of};
    let cur = &r.fiat;
    let money = |v: f64| format::fiat(v, cur);
    let amber = hexc(if tk.dark { "#fcd34d" } else { "#b45309" });
    let mut stats = Vec::new();
    if let Some((glyph, ink)) = readiness_signal(tk, readiness) {
        let mut s = Stat::new("rep-readiness", glyph, "Readiness", readiness.title.clone())
            .line(readiness.detail.clone())
            .help(readiness.detail.clone());
        if let Some(c) = ink {
            s = s.ink(c).glyph_ink(c);
        }
        if let Some(href) = readiness.href {
            s = s.link(
                format!("{}: {}", readiness.title, readiness.detail),
                move |a: &mut App| a.navigate(href),
            );
        }
        stats.push(s);
    }
    // The line under a basis figure: "Provisional" (a signal) or what the figure covers.
    let basis_line = |s: Stat, quiet: String| {
        if provisional {
            s.line_with("Provisional", Some(amber), false)
                .help(PROVISIONAL_HELP)
        } else {
            s.line(quiet)
        }
    };
    stats.push(
        Stat::new("rep-metric-0", &I::BANKNOTE, "Proceeds", money(r.proceeds))
            .sensitive(true)
            .line(format!("Disposals in {}", r.year)),
    );
    stats.push(basis_line(
        Stat::new("rep-metric-1", &I::COINS, "Cost basis", money(r.cost)).sensitive(true),
        model::method_short(&r.method),
    ));
    let mut parts = vec![];
    if r.income > 0.0 {
        parts.push(format!(
            "{} income",
            if hidden {
                "—".into()
            } else {
                money(r.income)
            }
        ));
    }
    let tf = r.tax_free_gain();
    if tf.abs() >= 0.005 {
        parts.push(format!(
            "{} tax-free (Altbestand)",
            if hidden {
                "—".into()
            } else {
                model::signed_fiat(tf, cur)
            }
        ));
    }
    let includes = if parts.is_empty() {
        "Realised in the period".to_owned()
    } else {
        format!("Includes {}", parts.join(" · "))
    };
    let mut gain = Stat::new(
        "rep-metric-2",
        &I::TRENDING_UP,
        "Gain / loss",
        model::signed_fiat(r.gain, cur),
    )
    .sensitive(true);
    if r.gain.abs() >= 0.005 {
        gain = gain.ink(gain_ink(tk, r.gain, true));
    }
    gain = if provisional {
        gain.line_with("Provisional", Some(amber), false)
            .help(format!("{PROVISIONAL_HELP} {includes}."))
    } else {
        gain.line(includes)
    };
    stats.push(gain);
    if let Some((amount, progressive, notes)) = &r.tax {
        let value = amount
            .map(|c| money(c / 100.0))
            .unwrap_or_else(|| "—".into());
        let sub = if (progressive / 100.0).abs() >= 0.005 {
            "KZ 172 + 174 − 176, without KZ 801"
        } else {
            "KZ 172 + 174 − 176"
        };
        let mut s = Stat::new(
            "rep-metric-3",
            &I::LANDMARK,
            {
                let rate = model::tax_rate(r.tax_rate.as_deref());
                if rate.is_empty() {
                    "Indicative tax".to_owned()
                } else {
                    format!("Indicative tax · {rate}")
                }
            },
            value,
        )
        .sensitive(amount.is_some());
        s = if provisional {
            s.line_with("Provisional", Some(amber), false)
        } else {
            s.line(sub)
        };
        let mut help = vec![sub.to_owned()];
        if provisional {
            help.push(PROVISIONAL_HELP.to_owned());
        }
        help.extend(notes.iter().cloned());
        stats.push(s.help(help.join("\n")));
    }
    row_of(app, cx, tk, "rep-metrics", "Report figures".into(), stats)
}

// ---- Breakdown ------------------------------------------------------------------------------

fn transactions_link(tk: &Tk, key: String, row_: &model::BreakdownRow) -> El {
    let count = format::count(row_.transaction_count);
    let Some(href) = row_.href.clone() else {
        return text(count).font(tnum(tk.tsm(400.0))).color(tk.muted_fg);
    };
    let label = if row_.exact {
        format!("Show these {count} in Transactions")
    } else {
        format!("Open the year in Transactions; {count} is too many to list one by one")
    };
    let name = format!(
        "{}{}: {label}",
        row_.code
            .as_ref()
            .map(|c| format!("{c} "))
            .unwrap_or_default(),
        row_.label
    );
    row()
        .key(key)
        .gap(tk.r(0.125))
        .center_y()
        .radius(tk.r(0.125))
        .role("link")
        .label(name)
        .help(label)
        .focusable(true)
        .on_press(move |a: &mut App| a.navigate(&href))
        .child(text(count).font(tnum(tk.tsm(400.0))).color(tk.fg))
        .child(ic(&I::ARROW_UP_RIGHT, tk.r(0.75), tk.muted_fg))
}

#[allow(clippy::too_many_arguments)]
fn breakdown(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    r: &Report,
    year: i32,
    hidden: bool,
    width: f32,
) -> El {
    let austrian = r.region() == Region::At;
    let groups = model::breakdown(r);
    let cur = &r.fiat;
    let title = if austrian {
        "Tax fields"
    } else {
        "Gains and income"
    };
    let head = lc::card_head(
        tk,
        title.into(),
        None,
        Some(arrow_link(
            tk,
            "rep-year-link",
            &format!("All of {year} in Transactions"),
            model::transactions_href(year, &[]),
        )),
    );
    let card = card(tk, "rep-breakdown")
        .role("region")
        .label(if austrian {
            "Tax fields"
        } else {
            "Gains and income"
        })
        .child(head);
    if groups.is_empty() {
        return card.child(
            row()
                .w_full()
                .px(tk.r(EDGE))
                .py(tk.r(2.0))
                .justify_center()
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .border_layout()
                .child(
                    text(format!("No disposals or income in {year}."))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                ),
        );
    }
    let sm = |s: &str| tw(s, tnum(tk.tsm(400.0)));
    let smm = |s: &str| tw(s, tnum(tk.tsm(500.0)));
    let wide_cols = width >= tk.r(42.0);
    // Columns and the widest cell per column (padding added below).
    let (cols, heads): (Vec<Col>, Vec<&str>) = if austrian {
        (
            vec![
                Col {
                    w: ColW::Fixed(0.0),
                    right: false,
                },
                Col {
                    w: ColW::Auto(0.0),
                    right: false,
                },
                Col {
                    w: ColW::Auto(0.0),
                    right: true,
                },
                Col {
                    w: ColW::Auto(0.0),
                    right: true,
                },
            ],
            vec!["Field", "Description", "Transactions", "Amount"],
        )
    } else if wide_cols {
        (
            vec![
                Col {
                    w: ColW::Auto(0.0),
                    right: false,
                },
                Col {
                    w: ColW::Auto(0.0),
                    right: true,
                },
                Col {
                    w: ColW::Auto(0.0),
                    right: true,
                },
                Col {
                    w: ColW::Auto(0.0),
                    right: true,
                },
                Col {
                    w: ColW::Auto(0.0),
                    right: true,
                },
            ],
            vec![
                "Kind",
                "Transactions",
                "Proceeds",
                "Cost basis",
                "Gain / loss",
            ],
        )
    } else {
        (
            vec![
                Col {
                    w: ColW::Auto(0.0),
                    right: false,
                },
                Col {
                    w: ColW::Auto(0.0),
                    right: true,
                },
                Col {
                    w: ColW::Auto(0.0),
                    right: true,
                },
            ],
            vec!["Kind", "Transactions", "Gain / loss"],
        )
    };
    let n = cols.len();
    let pad = |i: usize| {
        if i == 0 || i == n - 1 {
            tk.r(PX) + tk.r(EDGE)
        } else {
            2.0 * tk.r(PX)
        }
    };
    let mut content: Vec<f32> = heads.iter().map(|h| tw(h, tk.tsm(500.0))).collect();
    let amount_s = |row_: &model::BreakdownRow| {
        row_.amount
            .map(|a| format::fiat(a, cur))
            .unwrap_or_else(|| "—".into())
    };
    for g in &groups {
        for row_ in &g.rows {
            let link_w = sm(&format::count(row_.transaction_count))
                + if row_.href.is_some() {
                    tk.r(0.875)
                } else {
                    0.0
                };
            let cells: Vec<f32> = if austrian {
                vec![
                    smm(row_.code.as_deref().unwrap_or("")),
                    0.0,
                    link_w,
                    smm(&amount_s(row_)),
                ]
            } else if wide_cols {
                let inc = row_.key == "income";
                vec![
                    sm(&row_.label),
                    link_w,
                    sm(&if inc {
                        "—".into()
                    } else {
                        format::fiat(row_.proceeds, cur)
                    }),
                    sm(&if inc {
                        "—".into()
                    } else {
                        format::fiat(row_.cost, cur)
                    }),
                    smm(&model::signed_fiat(row_.amount.unwrap_or(0.0), cur)),
                ]
            } else {
                vec![
                    sm(&row_.label),
                    link_w,
                    smm(&model::signed_fiat(row_.amount.unwrap_or(0.0), cur)),
                ]
            };
            for (i, w) in cells.into_iter().enumerate() {
                content[i] = content[i].max(w);
            }
        }
    }
    let padded: Vec<f32> = content
        .iter()
        .enumerate()
        .map(|(i, w)| w + pad(i))
        .collect();
    // shadcn's `overflow-x-auto` container: the columns share the card's width, and below their
    // widest cells the table keeps them and scrolls sideways.
    let least: f32 = padded
        .iter()
        .enumerate()
        .filter(|(i, _)| !(austrian && *i == 1))
        .map(|(_, w)| *w)
        .sum::<f32>()
        + if austrian { 2.0 * tk.r(PX) } else { 0.0 };
    let (table_w, overflow) = crate::ui::table::fit(avail(cx, "rep-breakdown", width) - 2.0, least);
    let mut widths = column_widths(&cols, &padded, table_w);
    if austrian {
        // `w-full max-w-0` on the description: it takes the rest.
        let others: f32 = widths
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 1)
            .map(|(_, w)| *w)
            .sum();
        widths[1] = (table_w - others).max(0.0);
    }
    let cell = |i: usize, h: f32, content: El| {
        let mut b = row()
            .w(widths[i])
            .min_w(widths[i])
            .h(h)
            .center_y()
            .pl(if i == 0 { tk.r(EDGE) } else { tk.r(PX) })
            .pr(if i == n - 1 { tk.r(EDGE) } else { tk.r(PX) });
        if cols[i].right {
            b = b.justify_end();
        }
        b.child(content)
    };
    let mut head_row = crate::ui::table::head(tk);
    for (i, h) in heads.iter().enumerate() {
        head_row = head_row.child(cell(
            i,
            tk.r(crate::ui::table::HEAD_H),
            crate::ui::table::th_text(tk, (*h).to_owned()),
        ));
    }
    let mut body = col().w_full();
    let total: usize = groups
        .iter()
        .map(|g| g.rows.len() + usize::from(!g.title.is_empty()))
        .sum();
    let mut idx = 0;
    let row_h = tk.r(1.25) + tk.r(1.0) + 1.0;
    let border = |r: El, idx: usize| {
        if idx + 1 < total {
            r.border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .border_layout()
        } else {
            r
        }
    };
    for g in &groups {
        if !g.title.is_empty() {
            let r = row()
                .w_full()
                .px(tk.r(EDGE))
                .py(tk.r(0.375))
                .fill(tk.muted.alpha(0.4))
                .role("rowheader")
                .child(text(g.title.clone()).font(tk.txs(500.0)).color(tk.muted_fg));
            body = body.child(border(r, idx));
            idx += 1;
        }
        for row_ in &g.rows {
            let key = format!("rep-row-{}", row_.key);
            let mut r = row().w_full().hover(tk.muted.alpha(0.35));
            if austrian {
                let empty = row_.transaction_count == 0 && row_.amount.unwrap_or(0.0).abs() < 0.005;
                let ink = if row_.attention && !empty {
                    hexc(if tk.dark { "#fbbf24" } else { "#b45309" })
                } else if empty || !g.reported {
                    tk.muted_fg
                } else {
                    tk.fg
                };
                let mut desc = row().min_w(0.0).gap(tk.r(0.375)).center_y().child(
                    text(row_.label.clone())
                        .font(tk.tsm(400.0))
                        .color(ink)
                        .truncate(),
                );
                if let Some(note) = &row_.note
                    && let Some(n) = info_note(
                        app,
                        tk,
                        layer,
                        &format!("{key}-note"),
                        &format!("About {}", row_.label),
                        std::slice::from_ref(note),
                    )
                {
                    desc = desc.child(n);
                }
                let amount = amount_s(row_);
                let amount_font = tnum(tk.tsm(if g.reported { 500.0 } else { 400.0 }));
                r = r
                    .child(cell(
                        0,
                        row_h,
                        text(row_.code.clone().unwrap_or_default())
                            .font(tnum(tk.tsm(500.0)))
                            .color(ink),
                    ))
                    .child(cell(1, row_h, desc))
                    .child(cell(
                        2,
                        row_h,
                        transactions_link(tk, format!("{key}-link"), row_),
                    ))
                    .child(cell(
                        3,
                        row_h,
                        if row_.amount.is_none() {
                            text(amount).font(amount_font).color(ink)
                        } else {
                            sensitive(amount, amount_font, ink, hidden)
                        },
                    ));
            } else {
                let inc = row_.key == "income";
                let gain = row_.amount.unwrap_or(0.0);
                let money = |v: f64| {
                    if inc {
                        "—".to_owned()
                    } else {
                        format::fiat(v, cur)
                    }
                };
                let muted = |s: String| {
                    if inc {
                        text(s).font(tnum(tk.tsm(400.0))).color(tk.muted_fg)
                    } else {
                        sensitive(s, tnum(tk.tsm(400.0)), tk.muted_fg, hidden)
                    }
                };
                let mut i = 0;
                r = r.child(cell(
                    i,
                    row_h,
                    text(row_.label.clone()).font(tk.tsm(400.0)).color(tk.fg),
                ));
                i += 1;
                r = r.child(cell(
                    i,
                    row_h,
                    transactions_link(tk, format!("{key}-link"), row_),
                ));
                i += 1;
                if wide_cols {
                    r = r.child(cell(i, row_h, muted(money(row_.proceeds))));
                    i += 1;
                    r = r.child(cell(i, row_h, muted(money(row_.cost))));
                    i += 1;
                }
                r = r.child(cell(
                    i,
                    row_h,
                    sensitive(
                        model::signed_fiat(gain, cur),
                        tnum(tk.tsm(500.0)),
                        gain_ink(tk, gain, false),
                        hidden,
                    ),
                ));
            }
            body = body.child(border(r.key(key), idx));
            idx += 1;
        }
    }
    let grid = col().w(table_w).role("table").child(head_row).child(body);
    card.child(crate::ui::table::hscroll_if(
        "rep-breakdown-scroll",
        title.to_owned(),
        overflow,
        grid,
    ))
}

// ---- Rules ----------------------------------------------------------------------------------

fn rules(tk: &Tk, r: &Report, year: i32) -> El {
    let _ = year;
    let rate = model::tax_rate(r.tax_rate.as_deref());
    let facts: Vec<(&str, String)> = if r.region() == Region::At {
        vec![
            ("Region", "Austria".into()),
            ("Method", model::method_label(&r.method)),
            (
                "Tax",
                if r.e1kv_applies() {
                    format!(
                        "E 1kv{} · KZ 801 income-tax rate",
                        if rate.is_empty() {
                            String::new()
                        } else {
                            format!(" {rate}")
                        }
                    )
                } else {
                    "KZ 801 income-tax rate · no crypto Kennzahlen before 2022".into()
                },
            ),
            (
                "Altbestand",
                "Bought before 1 Mar 2021 · tax-free after a year".into(),
            ),
            ("Tax year", "Calendar year, Vienna time".into()),
        ]
    } else {
        let mut v = vec![
            ("Region", "Generic".to_owned()),
            ("Method", model::method_label(&r.method)),
        ];
        if let Some(days) = r.long_term_days {
            v.push((
                "Long-term after",
                format!(
                    "{} day{}",
                    format::count(days.max(0.0) as usize),
                    if days == 1.0 { "" } else { "s" }
                ),
            ));
        }
        v.push(("Tax", "No rate applied".into()));
        v.push(("Tax year", r.tax_year_rule()));
        v
    };
    // The indicative tax's notes (its stat tile shows them on hover; here they stay readable).
    let notes: Vec<String> = r
        .tax
        .as_ref()
        .map(|(_, _, n)| n.clone())
        .unwrap_or_default();
    let facts: Vec<(&str, String)> = facts
        .into_iter()
        .chain(notes.into_iter().map(|n| ("Note", n)))
        .collect();
    // `grid-cols-[5.5rem_minmax(0,1fr)] gap-x-3 gap-y-2 px-3 py-3 sm:px-4 text-xs`.
    let mut dl = col()
        .w_full()
        .px(tk.r(EDGE))
        .pt(tk.r(0.25))
        .pb(tk.r(1.0))
        .gap(tk.r(0.5));
    for (label, value) in facts {
        dl = dl.child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .items_start()
                .child(
                    text(label.to_owned())
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .w(tk.r(7.5))
                        .min_w(tk.r(7.5)),
                )
                .child(
                    text(value)
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .wrap()
                        .grow(1.0)
                        .w(0.0)
                        .min_w(0.0),
                ),
        );
    }
    card(tk, "rep-rules")
        .role("region")
        .label("Rules")
        .child(lc::card_head(
            tk,
            "Rules".into(),
            None,
            Some(arrow_link(
                tk,
                "rep-books-link",
                "Change in Books",
                "/books".into(),
            )),
        ))
        .child(dl)
}

/// The `ui.overview.snapshot` connections the Lightning panel lists.
pub(super) fn connections(app: &App) -> Vec<Value> {
    app.read("ui.overview.snapshot")
        .and_then(|d| d["connections"].as_array().cloned())
        .unwrap_or_default()
}

/// `useBlockingSourceCount` (lib/reportBlockers.ts): the bell's `source.refresh-failed`
/// conditions of blocking priority, i.e. the sources in the freshness snapshot that block
/// reports, one per source identity (`source_key`, else its label).
fn blocking_source_count(app: &App) -> usize {
    let Some(sources) = app
        .read("ui.freshness.status")
        .and_then(|f| f["sources"].as_array())
    else {
        return 0;
    };
    let mut seen = std::collections::BTreeSet::new();
    for s in sources {
        let identity = s["source_key"]
            .as_str()
            .filter(|k| !k.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                s["source_label"]
                    .as_str()
                    .filter(|l| !l.is_empty())
                    .map(|l| format!("label:{l}"))
            });
        if let Some(id) = identity.filter(|_| s["blocking_reports"] == true) {
            seen.insert(id);
        }
    }
    seen.len()
}
