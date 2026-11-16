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
use crate::i18n::{t, tc};
use crate::icons as I;
use crate::reports::ledger::common as lc;
use crate::theme::Tk;
use crate::txdetail::widgets::{self as w, Size, Variant};
use crate::ui::ic;
use crate::ui::table::EDGE;
use rune_kit::Placement;

/// `NORMAL_HANDOFF_EXCLUSIONS`, joined with "or".
const EXCLUSIONS: &str = "No file includes wallet descriptors, xpubs, backend credentials, raw wallet config, AI settings, logs, or unrelated books.";

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
    menu: Option<bool>,
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
        text("Export")
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
        (Some(open), _) if enabled => {
            crate::ui::popup::trigger(b, crate::ui::popup::Kind::Menu, open, |a: &mut App| {
                toggle(a, Popup::TaxFormats)
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
    let item =
        |label: &str, desc: &str, glyph: &'static rune::Icon, f: Fmt| {
            Entry::Item(Item::new(label).description(desc).icon(glyph).on_select(
                move |a: &mut App| {
                    a.reports.report.popup = None;
                    super::export(a, f);
                },
            ))
        };
    let entries = vec![
        item(
            "PDF",
            "The report to file or print",
            &I::FILE_TEXT,
            Fmt::Pdf,
        ),
        item(
            "XLSX",
            "Every row, for a spreadsheet",
            &I::FILE_SPREADSHEET,
            Fmt::Xlsx,
        ),
        item("CSV", "Every row, plain text", &I::TABLE_2, Fmt::Csv),
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

fn summary_options(app: &App, tk: &Tk, picker: bool, all_selected: bool) -> El {
    let ui = &app.reports.report;
    let cover = !ui.no_cover;
    let label = "Cover page with today's balances";
    let mut body = col().w_full().gap(tk.r(0.75)).child(
        row()
            .w_full()
            .gap(tk.r(0.75))
            .center_y()
            .justify_between()
            .child(text(label).font(tk.txs(400.0)).color(tk.fg))
            .child(
                crate::ui::switch(tk, "rep-cover", cover, label)
                    .on_press(|a: &mut App| a.reports.report.no_cover = !a.reports.report.no_cover),
            ),
    );
    if picker {
        let mut headr = row()
            .w_full()
            .gap(tk.r(0.75))
            .center_y()
            .justify_between()
            .child(text("Wallets").font(tk.txs(400.0)).color(tk.muted_fg));
        if !all_selected {
            headr = headr.child(
                row()
                    .key("rep-wallets-all")
                    .role("button")
                    .label("Select all")
                    .focusable(true)
                    .on_press(|a: &mut App| {
                        let all: Vec<String> = a
                            .reports
                            .report
                            .wallets
                            .iter()
                            .flatten()
                            .map(|w| w.id.clone())
                            .collect();
                        a.reports.report.summary_wallets = all;
                    })
                    .child(text("Select all").font(tk.txs(400.0)).color(tk.fg)),
            );
        }
        let mut list = col().w_full().gap(tk.r(0.125)).role("list");
        for wc in ui.wallets.iter().flatten() {
            let on = ui.summary_wallets.contains(&wc.id);
            let id = wc.id.clone();
            list = list.child(check_row(
                tk,
                format!("rep-wallet-{}", wc.id),
                &wc.label,
                Some(&wc.chain),
                on,
                move |a: &mut App| {
                    // Never leaves the selection empty.
                    let sel = &mut a.reports.report.summary_wallets;
                    if let Some(i) = sel.iter().position(|x| *x == id) {
                        if sel.len() > 1 {
                            sel.remove(i);
                        }
                    } else {
                        sel.push(id.clone());
                    }
                },
            ));
        }
        body = body.child(col().w_full().gap(tk.r(0.25)).child(headr).child(list));
    }
    body
}

fn audit_options(app: &App, cx: &Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let ui = &app.reports.report;
    let scope_label = if ui.audit.case_scope {
        "A saved source-of-funds case"
    } else {
        "Whole book"
    };
    let open = app.reports.report.popup == Some(Popup::AuditScope);
    let scope = crate::ui::popup::trigger(
        select_trigger(
            tk,
            "rep-audit-scope",
            Some("Audit package scope"),
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
            ("active_profile".to_owned(), "Whole book".to_owned(), false),
            (
                "source_funds_case".to_owned(),
                "A saved source-of-funds case".to_owned(),
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
        .child(text("Scope").font(tk.txs(400.0)).color(tk.muted_fg))
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
            Some("Source-of-funds case"),
            current.as_deref().unwrap_or("Choose a saved case"),
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
            part,
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
            .child(text("Include").font(tk.txs(400.0)).color(tk.muted_fg))
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
        crate::i18n::Lang::En => {
            const M: [&str; 12] = [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ];
            let iso = day.iso();
            let m: usize = iso[5..7].parse().unwrap_or(1);
            let d: u32 = iso[8..10].parse().unwrap_or(1);
            format!("{} {d}, {}", M[m.clamp(1, 12) - 1], &iso[0..4])
        }
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
fn quarantine_notice(tk: &Tk, q: &super::Quarantine) -> El {
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
    c.child(
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
    )
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
        .label("Open")
        .focusable(true)
        .on_press(move |a: &mut App| super::open_export(a, &path))
        .child(ic(&I::EXTERNAL_LINK, tk.r(0.875), tk.fg))
        .child(text("Open").font(tk.txs(500.0)).color(tk.fg));
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
    let head = lc::card_head(
        tk,
        "Files".into(),
        Some("No keys or credentials in any file".into()),
        None,
    )
    .help(EXCLUSIONS.to_owned());
    let mut card = card(tk, "rep-files")
        .role("region")
        .label("Files")
        .child(head);
    match &ui.status {
        Some(s) if s.year == year => {
            card = match &s.quarantine {
                Some(q) => card.child(
                    col()
                        .w_full()
                        .padding(tk.r(0.75))
                        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                        .border_layout()
                        .child(quarantine_notice(tk, q)),
                ),
                None => card.child(export_status(app, tk, s)),
            };
        }
        _ => {}
    }
    let tax_detail = if region != Region::At {
        "Every year in the book".to_owned()
    } else if !e1kv {
        "No crypto Kennzahlen before 2022 · the bookkeeping handoff lists every row".to_owned()
    } else if before > 0.0 {
        format!("No E 1kv {year}: rows before 1 March · the bookkeeping handoff lists every row")
    } else {
        format!("E 1kv {year} with every row")
    };
    let fold = ui.fold;
    let summary_label = format!(
        "{} · {}",
        if picker && selected < wallets.len() {
            format!(
                "{} of {} wallets",
                format::count(selected),
                format::count(wallets.len())
            )
        } else if wallets.len() == 1 {
            "1 wallet".to_owned()
        } else {
            "All wallets".to_owned()
        },
        if ui.no_cover {
            "no cover page"
        } else {
            "balance cover page"
        }
    );
    let audit_label = format!(
        "{} · {} of {} parts",
        if ui.audit.case_scope {
            "One source-of-funds case"
        } else {
            "Whole book"
        },
        format::count(included),
        format::count(AUDIT_PARTS.len())
    );
    let summary_body = if fold == Some(Fold::Summary) {
        summary_options(app, tk, picker, selected >= wallets.len())
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
    let tax_open = ui.popup == Some(Popup::TaxFormats);
    let list = col()
        .w_full()
        .role("list")
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout()
        .child(file_row(
            tk,
            "rep-file-tax",
            "Tax report",
            tax_detail,
            export_button(
                app,
                tk,
                "rep-fmt-tax",
                &[Fmt::Pdf, Fmt::Xlsx, Fmt::Csv],
                "Export Tax report".into(),
                refused,
                year,
                Some(tax_open),
            ),
            None,
            false,
        ))
        .child(file_row(
            tk,
            "rep-file-handoff",
            "Bookkeeping handoff",
            format!("EA / Jahresabschluss {year} · XLSX"),
            export_button(
                app,
                tk,
                "rep-fmt-year_handoff",
                &[Fmt::YearHandoff],
                "Export Bookkeeping handoff as XLSX".into(),
                false,
                year,
                None,
            ),
            None,
            false,
        ))
        .child(file_row(
            tk,
            "rep-file-summary",
            "Summary",
            format!("Portfolio {year}, no tax tables · PDF"),
            export_button(
                app,
                tk,
                "rep-fmt-summary_pdf",
                &[Fmt::SummaryPdf],
                "Export Summary as PDF".into(),
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
            "Audit package",
            "Book evidence for an auditor".into(),
            export_button(
                app,
                tk,
                "rep-fmt-audit_package",
                &[Fmt::AuditPackage],
                "Export Audit package".into(),
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
