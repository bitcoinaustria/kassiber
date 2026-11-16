//! The export dialog's look: the shared glass dialog (`shell::glass_dialog`, the
//! `kb-glass-backdrop` wash, the `max(1rem, 8vh)` top anchor) laid out by flex constraints, so
//! it fits any window from its first frame; three short choices on the left, a small human
//! preview on the right, the summary and the two buttons in the footer.

use std::borrow::Cow;

use rune::{Color, El, GrowthSpec, ScrollSpec, Shadow, Tx, col, el, row, text};
use rune_kit::Placement;
use serde_json::Value;

use super::summary::{self, Dir, Line};
use super::{ExportUi, Format, PRESETS, strings as s};
use crate::app::App;
use crate::format;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::transactions::Popup;
use crate::txdetail::widgets::{self as w, Size, Variant};
use crate::ui::ic;
use crate::ui::popup::TriggerExt;
use crate::ui::table as tb;

/// The dialog's largest size: `max-w-5xl`-ish and 40 rem tall; smaller windows shrink it.
const MAX_W: f32 = 64.0;
const MAX_H: f32 = 40.0;
/// The choices column.
const LEFT_W: f32 = 18.0;
/// Rows in the summary preview.
const SUMMARY_ROWS: usize = 6;

fn hexc(s: &str) -> Color {
    rune::color::hex(s).unwrap_or(Color::WHITE)
}

/// A field label (`Label`: `text-sm font-medium leading-none`).
fn label(tk: &Tk, s: String) -> El {
    text(s).font(tk.tsm(500.0).lh(tk.r(0.875))).color(tk.fg)
}

/// A `SelectTrigger` surface (`rounded-md border border-input`, `dark:bg-input/30`).
fn trigger_surface(tk: &Tk, el: El) -> El {
    let (fill, hover) = if tk.dark {
        (
            tk.input.alpha(tk.input.a * 0.3),
            tk.input.alpha(tk.input.a * 0.5),
        )
    } else {
        (Color::TRANSPARENT, tk.accent)
    };
    el.radius(tk.r(0.375))
        .border(1.0, tk.input)
        .fill(fill)
        .hover(hover)
        .cursor(rune::Cursor::Pointer)
}

/// `ExportFormatMark`: a tool's logo on a white bordered tile, or a Kassiber format's icon on
/// its solid tone. The dialog has no `Cx`, so logos rasterise at 2× (their mips keep 1× crisp).
pub fn mark(app: &App, tk: &Tk, f: Format, size: f32) -> El {
    let tile = row()
        .size(size, size)
        .center()
        .radius(tk.r(0.375))
        .a11y_hidden();
    let logo = match f {
        Format::CoinTracking => crate::logos::Logo::svg(crate::logos::COINTRACKING),
        Format::Blockpit => crate::logos::Logo::raster(crate::logos::BLOCKPIT),
        _ => {
            let (tone, glyph): (&str, &'static rune::Icon) = match f {
                Format::Xlsx => ("#059669", &I::FILE_SPREADSHEET),
                Format::Labels => ("#7c3aed", &I::TAGS),
                _ => ("#475569", &I::FILE_TEXT),
            };
            return tile
                .fill(hexc(tone))
                .child(ic(glyph, size * 0.57, Color::WHITE));
        }
    };
    let inner = size - 2.0 - 2.0 * tk.r(0.125);
    tile.fill(Color::WHITE)
        .border(1.0, tk.border)
        .border_layout()
        .clip_rounded(tk.r(0.375))
        .child(crate::logos::fitted_at(app, 2.0, logo, inner, inner))
}

/// A select option row: content, then the check of the chosen one (`SelectItem`).
fn option_row(tk: &Tk, label: String, checked: bool, content: El) -> El {
    row()
        .w_full()
        .min_h(tk.r(2.0))
        .px(tk.r(0.5))
        .py(tk.r(0.375))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.25))
        .hover(tk.accent)
        .focus_highlight(tk.accent)
        .cursor(rune::Cursor::Pointer)
        .role("option")
        .checked(checked)
        .label(label)
        .child(content.grow(1.0).min_w(0.0))
        .child(
            row()
                .size(tk.r(1.0), tk.r(1.0))
                .center()
                .a11y_hidden()
                .child(if checked {
                    ic(&I::CHECK, tk.r(0.875), tk.fg)
                } else {
                    el()
                }),
        )
}

/// Mounts a listbox under `anchor` on the popover glass.
fn mount_list(tk: &Tk, layer: &mut Vec<El>, p: crate::ui::popup::Popup, width: f32) {
    let list = col().w(width).padding(tk.r(0.25)).gap(1.0);
    p.max_h(tk.r(24.0)).mount(
        tk,
        layer,
        list,
        |l| crate::shell::glass(tk, col().min_h(0.0).child(l)),
        Placement::BottomStart,
        4.0,
    );
}

// ---- Format ---------------------------------------------------------------------------------

fn format_section(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let e = &app.txs.export;
    let open = app.txs.popup == Some(Popup::ExportFormat);
    let name = e.format.name();
    let trigger = trigger_surface(tk, row())
        .key("txs-export-format")
        // Radix moves focus to the dialog's first control on open.
        .autofocus()
        .w_full()
        .px(tk.r(0.625))
        .py(tk.r(0.5))
        .gap(tk.r(0.625))
        .center_y()
        .role("combobox")
        .label(t("transactions:dashboard.export.format"))
        .a11y_value(name.clone())
        .focusable(true)
        .select_trigger(open, |a: &mut App| {
            a.txs.popup = if a.txs.popup == Some(Popup::ExportFormat) {
                None
            } else {
                Some(Popup::ExportFormat)
            }
        })
        .child(mark(app, tk, e.format, tk.r(1.75)))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(
                    text(name.clone())
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .single_line()
                        .truncate()
                        .min_w(0.0)
                        .help_when_truncated(name),
                )
                .child(
                    text(e.format.hint())
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .single_line()
                        .truncate()
                        .min_w(0.0)
                        .help_when_truncated(e.format.hint()),
                ),
        )
        .child(ic(&I::CHEVRONS_UP_DOWN, tk.r(1.0), tk.muted_fg.alpha(0.5)));
    if open {
        format_menu(app, tk, layer);
    }
    col()
        .w_full()
        .gap(tk.r(0.5))
        .child(label(tk, t("transactions:dashboard.export.format")))
        .child(trigger)
}

fn format_menu(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let e = &app.txs.export;
    let mut p =
        crate::ui::popup::Popup::listbox("txs-export-format", |a: &mut App| a.txs.popup = None);
    let mut last_tool = None;
    for f in Format::ALL {
        if last_tool != Some(f.tool()) {
            if last_tool.is_some() {
                p.add(
                    el().h(1.0)
                        .w_full()
                        .my(tk.r(0.25))
                        .fill(tk.border)
                        .a11y_hidden(),
                );
            }
            let g = t(if f.tool() {
                "transactions:dashboard.export.formatGroups.tool"
            } else {
                "transactions:dashboard.export.formatGroups.kassiber"
            });
            p.add(
                text(g)
                    .font(tk.txs(500.0))
                    .color(tk.muted_fg)
                    .px(tk.r(0.5))
                    .py(tk.r(0.375))
                    .a11y_hidden(),
            );
            last_tool = Some(f.tool());
        }
        let checked = e.format == f;
        let content = row()
            .gap(tk.r(0.625))
            .center_y()
            .a11y_hidden()
            .child(mark(app, tk, f, tk.r(1.5)))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(
                        text(f.name())
                            .font(tk.tsm(500.0))
                            .color(tk.fg)
                            .single_line()
                            .truncate()
                            .min_w(0.0),
                    )
                    .child(
                        text(f.hint())
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .single_line()
                            .truncate()
                            .min_w(0.0)
                            .help_when_truncated(f.hint()),
                    ),
            )
            // The file type (`rounded-sm border px-1.5 font-mono text-[10px]`).
            .child(
                row()
                    .px(tk.r(0.375))
                    .py(1.0)
                    .radius(tk.r(0.25))
                    .border(1.0, tk.border)
                    .border_layout()
                    .child(
                        text(f.badge())
                            .font(tk.px(10.0, 400.0).mono())
                            .color(tk.muted_fg),
                    ),
            );
        let r = option_row(tk, f.name(), checked, content)
            .description(format!("{}, {}", f.hint(), f.badge()))
            .on_press(move |a: &mut App| {
                a.txs.export.format = f;
                a.txs.export.all_columns = false;
                a.txs.popup = None;
            });
        p.item(
            r,
            format!("txs-export-format-opt-{}", f.id()),
            f.name(),
            true,
            checked,
        );
    }
    mount_list(tk, layer, p, tk.r(20.0));
}

// ---- Period ---------------------------------------------------------------------------------

/// The period as the footer and trigger say it (a complete custom range as its days).
pub fn period_label(e: &ExportUi) -> String {
    if super::is_custom(e)
        && let Some((f, to)) = super::custom_range(e)
    {
        return tr(
            "transactions:dashboard.export.period.span",
            &[("from", &f), ("to", &to)],
        );
    }
    t(&format!(
        "transactions:dashboard.export.period.presets.{}",
        PRESETS[e.preset]
    ))
}

fn tz_label(app: &App, zone: &str) -> String {
    if zone == super::book_tz(app) {
        tr(
            "transactions:dashboard.export.period.timezoneDefault",
            &[("zone", zone)],
        )
    } else {
        zone.to_owned()
    }
}

fn period_section(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let e = &app.txs.export;
    let head = label(tk, t("transactions:dashboard.export.period.label"));
    if !e.format.dated() {
        // Labels have no period: only the note.
        return col().w_full().gap(tk.r(0.5)).child(head).child(
            text(t("transactions:dashboard.export.period.labelsAllTime"))
                .font(tk.txs(400.0).lh(tk.r(1.125)))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        );
    }
    let open = app.txs.popup == Some(Popup::ExportPeriod);
    let preset = t(&format!(
        "transactions:dashboard.export.period.presets.{}",
        PRESETS[e.preset]
    ));
    let enabled = app.txs.dash.is_some();
    let trigger = trigger_surface(tk, row())
        .key("txs-export-period")
        .w_full()
        .h(tk.r(2.25))
        .px(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .role("combobox")
        .label(t("transactions:dashboard.export.period.label"))
        .a11y_value(preset.clone())
        .focusable(enabled)
        .disabled(!enabled)
        .select_trigger(open, |a: &mut App| {
            a.txs.popup = if a.txs.popup == Some(Popup::ExportPeriod) {
                None
            } else {
                Some(Popup::ExportPeriod)
            }
        })
        .child(
            text(preset)
                .font(tk.tsm(400.0))
                .color(tk.fg)
                .grow(1.0)
                .single_line()
                .truncate()
                .min_w(0.0),
        )
        .child(ic(&I::CHEVRON_DOWN, tk.r(1.0), tk.muted_fg.alpha(0.5)));
    if open {
        period_menu(app, tk, layer);
    }
    let mut c = col().w_full().gap(tk.r(0.5)).child(head).child(trigger);
    if super::is_custom(e) {
        c = c.child(custom_range(tk, e));
    }
    if PRESETS[e.preset] == "all" {
        return c.child(
            text(t("transactions:dashboard.export.period.everything"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        );
    }
    // The days the preview resolved, then the timezone they are read in (a small select).
    let zone = e.timezone.clone().unwrap_or_else(|| super::book_tz(app));
    let range = e
        .preview
        .as_ref()
        .and_then(|p| p.get("range"))
        .filter(|r| r.is_object())
        .map(|r| {
            format!(
                "{} – {}",
                r["first_day"].as_str().unwrap_or(""),
                r["last_day"].as_str().unwrap_or("")
            )
        });
    let tz_open = e.tz_open;
    let tz_text = tz_label(app, &zone);
    let tz = row()
        .key("txs-export-tz")
        .h(tk.r(1.5))
        .px(tk.r(0.375))
        .ml(-tk.r(0.375))
        .gap(tk.r(0.25))
        .center_y()
        .min_w(0.0)
        .shrink()
        .radius(tk.r(0.25))
        .hover(tk.muted)
        .cursor(rune::Cursor::Pointer)
        .role("combobox")
        .label(t("transactions:dashboard.export.period.timezone"))
        .a11y_value(tz_text.clone())
        .help(s::timezone_of(&zone))
        .focusable(true)
        .select_trigger(tz_open, |a: &mut App| {
            a.txs.export.tz_open = !a.txs.export.tz_open
        })
        .child(ic(&I::GLOBE, tk.r(0.75), tk.muted_fg))
        .child(
            text(tz_text)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .single_line()
                .truncate()
                .min_w(0.0),
        )
        .child(ic(&I::CHEVRON_DOWN, tk.r(0.75), tk.muted_fg.alpha(0.6)));
    if tz_open {
        tz_menu(app, tk, layer);
    }
    let mut line = col().w_full().gap(tk.r(0.125));
    if let Some(r) = range {
        line = line.child(
            text(r)
                .font(tk.txs(400.0).features("'tnum' 1"))
                .color(tk.muted_fg)
                .single_line()
                .truncate()
                .min_w(0.0),
        );
    }
    c.child(line.child(tz))
}

fn period_menu(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let e = &app.txs.export;
    let mut p =
        crate::ui::popup::Popup::listbox("txs-export-period", |a: &mut App| a.txs.popup = None);
    for (i, pr) in PRESETS.iter().enumerate() {
        let name = t(&format!(
            "transactions:dashboard.export.period.presets.{pr}"
        ));
        let checked = e.preset == i;
        if *pr == "custom"
            || *pr == "today"
            || *pr == "this_week"
            || *pr == "this_month"
            || *pr == "this_quarter"
            || *pr == "this_year"
        {
            p.add(
                el().h(1.0)
                    .w_full()
                    .my(tk.r(0.25))
                    .fill(tk.border)
                    .a11y_hidden(),
            );
        }
        let content = text(name.clone())
            .font(tk.tsm(400.0))
            .color(tk.fg)
            .a11y_hidden();
        let r = option_row(tk, name.clone(), checked, content).on_press_tx(
            move |a: &mut App, tx: &mut Tx| {
                a.txs.export.preset = i;
                a.txs.popup = None;
                if PRESETS[i] == "custom" {
                    prefill_custom(a);
                    tx.focus("txs-export-from".to_owned(), tx.by_keys());
                }
            },
        );
        p.item(
            r,
            format!("txs-export-period-opt-{pr}"),
            name,
            true,
            checked,
        );
    }
    mount_list(tk, layer, p, tk.r(15.5));
}

/// A first custom range: the book's span, so the fields start from real days.
fn prefill_custom(a: &mut App) {
    if !a.txs.export.from.text.is_empty() || !a.txs.export.to.text.is_empty() {
        return;
    }
    let (first, last) = a
        .txs
        .dash
        .as_ref()
        .map(|d| {
            (
                d.earliest.chars().take(10).collect::<String>(),
                d.latest.chars().take(10).collect::<String>(),
            )
        })
        .unwrap_or_default();
    if first.len() == 10 && last.len() == 10 {
        a.txs.export.from = rune::TextEdit::new(first);
        a.txs.export.to = rune::TextEdit::new(last);
    }
}

fn custom_range(tk: &Tk, e: &ExportUi) -> El {
    let field = |key: &str,
                 name: String,
                 edit: &rune::TextEdit,
                 on: fn(&mut App) -> &mut rune::TextEdit| {
        col()
            .grow(1.0)
            .min_w(0.0)
            .w(0.0)
            .gap(tk.r(0.25))
            .child(text(name.clone()).font(tk.txs(500.0)).color(tk.muted_fg))
            .child(crate::settings::controls::input(
                tk,
                edit,
                key,
                &name,
                "YYYY-MM-DD",
                false,
                true,
                tk.r(2.0),
                false,
                on,
            ))
    };
    let mut c = col().w_full().gap(tk.r(0.375)).child(
        row()
            .w_full()
            .gap(tk.r(0.5))
            .child(field(
                "txs-export-from",
                t("transactions:dashboard.export.period.from"),
                &e.from,
                |a: &mut App| &mut a.txs.export.from,
            ))
            .child(field(
                "txs-export-to",
                t("transactions:dashboard.export.period.to"),
                &e.to,
                |a: &mut App| &mut a.txs.export.to,
            )),
    );
    let typed = !e.from.text.trim().is_empty() && !e.to.text.trim().is_empty();
    if typed && super::custom_range(e).is_none() {
        c = c.child(
            text(s::invalid_range())
                .font(tk.txs(400.0))
                .color(tk.destructive)
                .wrap()
                .w_full()
                .role("alert"),
        );
    }
    c
}

fn tz_menu(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let e = &app.txs.export;
    let book = super::book_tz(app);
    let mut zones: Vec<String> = vec![book.clone()];
    for z in ["UTC".to_owned()].into_iter().chain(e.viewer_tz.clone()) {
        if !zones.contains(&z) {
            zones.push(z);
        }
    }
    let current = e.timezone.clone().unwrap_or_else(|| book.clone());
    let mut p = crate::ui::popup::Popup::listbox("txs-export-tz", |a: &mut App| {
        a.txs.export.tz_open = false
    });
    for (i, z) in zones.into_iter().enumerate() {
        let name = tz_label(app, &z);
        let checked = current == z;
        let is_book = z == book;
        let content = text(name.clone())
            .font(tk.tsm(400.0))
            .color(tk.fg)
            .single_line()
            .truncate()
            .min_w(0.0)
            .a11y_hidden();
        let r = option_row(tk, name.clone(), checked, content).on_press(move |a: &mut App| {
            a.txs.export.timezone = if is_book { None } else { Some(z.clone()) };
            a.txs.export.tz_open = false;
        });
        p.item(r, format!("txs-export-tz-opt-{i}"), name, true, checked);
    }
    mount_list(tk, layer, p, tk.r(15.5));
}

// ---- Wallets --------------------------------------------------------------------------------

/// `ConnectionAssetBadge` at the list's 1.5 rem: the rail logo on a bordered muted tile,
/// Lightning kinds on their brand tint (wallets/common.rs, without a `Cx`).
fn wallet_badge(app: &App, tk: &Tk, kind: &str, chain: &str, label: &str) -> El {
    use crate::logos::{
        BITCOIN, CORE_LIGHTNING, LIGHTNING, LIGHTNING_LABS_SVG, LIQUID, Logo, fitted_at,
    };
    let core_ln = kind == "core-ln";
    let lnd = kind == "lnd";
    let lightning = core_ln || lnd || kind == "nwc" || kind == "phoenix";
    let l = format!("{chain} {label}").to_lowercase();
    let liquid = !lightning && (l.contains("liquid") || l.contains("lbtc"));
    let (fill, border) = if core_ln {
        (
            hexc("#09090b"),
            if tk.dark {
                hexc("#3f3f46")
            } else {
                hexc("#09090b").alpha(0.8)
            },
        )
    } else if lnd {
        if tk.dark {
            (hexc("#6f4cff").alpha(0.2), hexc("#8b72ff").alpha(0.4))
        } else {
            (hexc("#6f4cff").alpha(0.1), hexc("#6f4cff").alpha(0.3))
        }
    } else if lightning {
        if tk.dark {
            (hexc("#792EE5").alpha(0.2), hexc("#9f6af0").alpha(0.35))
        } else {
            (hexc("#792EE5").alpha(0.1), hexc("#792EE5").alpha(0.25))
        }
    } else if tk.dark {
        (tk.muted.alpha(0.55), tk.border.alpha(tk.border.a * 0.8))
    } else {
        (tk.muted.alpha(0.6), tk.border.alpha(tk.border.a * 0.7))
    };
    let bytes = if core_ln {
        CORE_LIGHTNING
    } else if lnd {
        LIGHTNING_LABS_SVG
    } else if lightning {
        LIGHTNING
    } else if liquid {
        LIQUID
    } else {
        BITCOIN
    };
    let side = tk.r(1.5);
    row()
        .size(side, side)
        .center()
        .radius(tk.r(0.375))
        .fill(fill)
        .border(1.0, border)
        .border_layout()
        .box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: if tk.dark {
                Color::BLACK.alpha(0.2)
            } else {
                hexc("#09090b").alpha(0.05)
            },
            ..Shadow::default()
        })
        .a11y_hidden()
        .child(fitted_at(
            app,
            2.0,
            Logo::svg(bytes),
            side * 0.94 - 2.0,
            side - 2.0,
        ))
}

fn wallets_summary(e: &ExportUi) -> (String, usize) {
    let ws = e.wallets.as_deref().unwrap_or(&[]);
    let sel = super::selected_count(e);
    let txs: usize = ws
        .iter()
        .filter(|w| !e.excluded.contains(&w.id))
        .map(|w| w.count)
        .sum();
    let s = if sel == ws.len() {
        format!(
            "{} ({})",
            t("transactions:dashboard.export.allWallets"),
            format::count(ws.len())
        )
    } else {
        tr(
            "transactions:dashboard.export.someWallets",
            &[
                ("selected", &format::count(sel)),
                ("total", &format::count(ws.len())),
            ],
        )
    };
    (s, txs)
}

fn tx_count(tk: &Tk, n: usize) -> El {
    text(tc(
        "transactions:dashboard.export.selectionTransactions",
        n,
        &[("formatted", &format::count(n))],
    ))
    .font(tk.txs(400.0).features("'tnum' 1"))
    .color(tk.muted_fg)
    .single_line()
}

/// The collapsed section: "All wallets (13)" with the selection's transaction count, and Edit.
fn wallets_section(tk: &Tk, e: &ExportUi) -> El {
    let head = row()
        .w_full()
        .justify_between()
        .center_y()
        .gap(tk.r(0.5))
        .child(label(tk, t("transactions:dashboard.export.wallets")));
    let Some(_) = &e.wallets else {
        return col().w_full().gap(tk.r(0.5)).child(head).child(
            row()
                .w_full()
                .h(tk.r(2.25))
                .radius(tk.r(0.375))
                .fill(tk.muted)
                .label(t("common:state.loading")),
        );
    };
    let edit = w::button(
        tk,
        "txs-export-wallets-edit",
        Variant::Ghost,
        Size::Xs,
        Some(&I::PENCIL),
        &t("common:actions.edit"),
        true,
    )
    .label(s::edit_wallets())
    .expanded(false)
    .on_press_tx(|a: &mut App, tx: &mut Tx| {
        a.txs.export.editing_wallets = true;
        // The first wallet row (the kit's filter field keys its own inner input).
        if let Some(id) = a
            .txs
            .export
            .wallets
            .as_ref()
            .and_then(|w| w.first())
            .map(|w| w.id.clone())
        {
            tx.focus(format!("txs-export-wallet-{id}"), tx.by_keys());
        }
    });
    let (summary, txs) = wallets_summary(e);
    let none = super::selected_count(e) == 0;
    let body = row()
        .w_full()
        .min_h(tk.r(2.25))
        .px(tk.r(0.75))
        .py(tk.r(0.375))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .fill(tk.muted.alpha(0.25))
        .child(
            text(summary.clone())
                .font(tk.tsm(400.0))
                .color(if none { tk.muted_fg } else { tk.fg })
                .grow(1.0)
                .single_line()
                .truncate()
                .min_w(0.0)
                .help_when_truncated(summary),
        )
        .child(tx_count(tk, txs));
    col()
        .w_full()
        .gap(tk.r(0.5))
        .child(head.child(edit))
        .child(body)
}

/// shadcn `Checkbox` (visual; the row is the control).
fn checkbox(tk: &Tk, on: bool) -> El {
    let b = row()
        .size(tk.r(1.0), tk.r(1.0))
        .center()
        .radius(4.0)
        .a11y_hidden();
    if on {
        b.fill(tk.primary)
            .child(ic(&I::CHECK, tk.r(0.875), tk.primary_fg))
    } else {
        b.border(1.0, tk.input).fill(if tk.dark {
            tk.input.alpha(tk.input.a * 0.3)
        } else {
            Color::TRANSPARENT
        })
    }
}

/// The open wallet list: it takes the whole column (a filter past six wallets, All · None, one
/// row per wallet with its logo, plain name and count), Done closes it.
fn wallet_editor(app: &App, tk: &Tk) -> El {
    let e = &app.txs.export;
    let ws = e.wallets.as_deref().unwrap_or(&[]);
    let sel = super::selected_count(e);
    let txs: usize = ws
        .iter()
        .filter(|w| !e.excluded.contains(&w.id))
        .map(|w| w.count)
        .sum();
    let done = w::button(
        tk,
        "txs-export-wallets-done",
        Variant::Outline,
        Size::Xs,
        None,
        &t("common:actions.done"),
        true,
    )
    .on_press_tx(|a: &mut App, tx: &mut Tx| {
        a.txs.export.editing_wallets = false;
        tx.focus("txs-export-wallets-edit".to_owned(), tx.by_keys());
    });
    let head = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(tk.r(0.25))
                .child(label(tk, t("transactions:dashboard.export.wallets")))
                .child(
                    text(format!(
                        "{} · {}",
                        tr(
                            "transactions:dashboard.export.walletsCount",
                            &[
                                ("selected", &format::count(sel)),
                                ("total", &format::count(ws.len()))
                            ]
                        ),
                        tc(
                            "transactions:dashboard.export.selectionTransactions",
                            txs,
                            &[("formatted", &format::count(txs))]
                        )
                    ))
                    .font(tk.txs(400.0).features("'tnum' 1"))
                    .color(tk.muted_fg)
                    .single_line()
                    .truncate()
                    .min_w(0.0),
                ),
        )
        .child(done);
    let mut c = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(tk.r(0.5))
        .child(head);
    let filter = e.wallet_filter.text.trim().to_lowercase();
    let visible: Vec<&super::WalletOpt> = ws
        .iter()
        .filter(|w| filter.is_empty() || w.label.to_lowercase().contains(&filter))
        .collect();
    if ws.len() > 6 {
        let k = tk.kit();
        c = c.child(
            rune_kit::TextInput::new(&e.wallet_filter)
                .label(t("transactions:dashboard.export.filterWallets"))
                .placeholder(t("transactions:dashboard.export.filterWallets"))
                .leading_icon(&I::SEARCH)
                .leading_icon_size(tk.r(0.875))
                .height(tk.r(2.0))
                .text_style(tk.tsm(400.0))
                .on_edit(|a: &mut App| &mut a.txs.export.wallet_filter)
                .build(&k)
                .w_full(),
        );
    }
    let ids: Vec<String> = visible.iter().map(|w| w.id.clone()).collect();
    let ids2 = ids.clone();
    let small = |key: &str, text_: String, on: Box<dyn Fn(&mut App)>| {
        w::button(
            tk,
            key.to_owned(),
            Variant::Ghost,
            Size::Xs,
            None,
            &text_,
            true,
        )
        .on_press(move |a: &mut App| on(a))
    };
    c = c.child(
        row()
            .gap(tk.r(0.25))
            .center_y()
            .ml(-tk.r(0.5))
            .child(small(
                "txs-export-wallets-all",
                t("transactions:dashboard.export.selectAll"),
                Box::new(move |a: &mut App| {
                    ids.iter().for_each(|i| {
                        a.txs.export.excluded.remove(i);
                    })
                }),
            ))
            .child(small(
                "txs-export-wallets-none",
                t("transactions:dashboard.export.selectNone"),
                Box::new(move |a: &mut App| {
                    ids2.iter().for_each(|i| {
                        a.txs.export.excluded.insert(i.clone());
                    })
                }),
            )),
    );
    let mut list = col()
        .key("txs-export-wallet-list")
        .w_full()
        .role("group")
        .label(t("transactions:dashboard.export.wallets"));
    if visible.is_empty() {
        list = list.child(
            text(t("transactions:dashboard.export.noWalletMatch"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .padding(tk.r(0.75)),
        );
    }
    let n = visible.len();
    for (i, wl) in visible.into_iter().enumerate() {
        let id = wl.id.clone();
        let on = !e.excluded.contains(&wl.id);
        let mut r = row()
            .key(format!("txs-export-wallet-{}", wl.id))
            .w_full()
            .px(tk.r(0.625))
            .py(tk.r(0.375))
            .gap(tk.r(0.5))
            .center_y()
            .hover(tk.muted.alpha(0.5))
            .focus_highlight(tk.muted.alpha(0.5))
            .role("checkbox")
            .checked(on)
            .label(wl.label.clone())
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |a: &mut App| {
                if !a.txs.export.excluded.remove(&id) {
                    a.txs.export.excluded.insert(id.clone());
                }
            })
            .child(checkbox(tk, on))
            .child(wallet_badge(app, tk, &wl.kind, &wl.chain, &wl.label))
            .child(
                text(wl.label.clone())
                    .font(tk.tsm(400.0))
                    .color(if on { tk.fg } else { tk.muted_fg })
                    .grow(1.0)
                    .single_line()
                    .truncate()
                    .min_w(0.0)
                    .help_when_truncated(wl.label.clone()),
            )
            .child(
                text(format::count(wl.count))
                    .font(tk.txs(400.0).features("'tnum' 1"))
                    .color(tk.muted_fg)
                    .single_line()
                    .a11y_hidden(),
            );
        if i + 1 < n {
            r = r
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .border_layout();
        }
        list = list.child(r);
    }
    c.child(
        col()
            .key("txs-export-wallet-scroll")
            .w_full()
            .grow(1.0)
            .min_h(tk.r(6.0))
            .radius(tk.r(0.5))
            .border(1.0, tk.border)
            .border_layout()
            .clip_rounded(tk.r(0.5))
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(list),
    )
}

// ---- Preview --------------------------------------------------------------------------------

fn book_currency(app: &App) -> String {
    app.txs
        .rows
        .iter()
        .map(|r| r.fiat_currency.clone())
        .find(|c| !c.is_empty())
        .unwrap_or_else(|| "EUR".into())
}

fn preview_pane(app: &App, tk: &Tk) -> El {
    let e = &app.txs.export;
    let pane = col()
        .key("txs-export-preview-pane")
        .grow(1.0)
        .w(0.0)
        .min_w(0.0)
        .h_full()
        .padding(tk.r(1.25))
        .gap(tk.r(0.75));
    if e.wallets.is_some() && super::selected_count(e) == 0 {
        return pane.child(
            col().w_full().grow(1.0).center().child(
                text(t("transactions:dashboard.export.selectWallet"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .align(rune::Align::Center),
            ),
        );
    }
    if let Some(err) = &e.preview_error {
        if super::blocked(err) {
            return pane.child(quarantine_notice(tk, err));
        }
        return pane.child(
            col()
                .w_full()
                .padding(tk.r(1.0))
                .gap(tk.r(0.25))
                .radius(tk.r(0.5))
                .border(1.0, tk.border)
                .border_dash(4.0, 3.0)
                .role("alert")
                .child(
                    text(t("transactions:dashboard.export.preview.failed"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(
                    text(err["message"].as_str().unwrap_or("").to_owned())
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .w_full(),
                ),
        );
    }
    let Some(p) = &e.preview else {
        let bar = |w: f32| row().w(w).h(tk.r(1.0)).radius(tk.r(0.25)).fill(tk.muted);
        return pane
            .label(t("common:state.loading"))
            .child(bar(tk.r(16.0)))
            .children((0..SUMMARY_ROWS).map(|_| {
                row()
                    .w_full()
                    .h(tk.r(1.75))
                    .radius(tk.r(0.25))
                    .fill(tk.muted.alpha(0.6))
            }));
    };
    let rows_n = p["row_count"].as_u64().unwrap_or(0) as usize;
    let sheet = p["sheets"].get(0).cloned().unwrap_or(Value::Null);
    let n_cols = sheet["columns"].as_array().map(Vec::len).unwrap_or(0);
    let shown = sheet["rows"].as_array().map(Vec::len).unwrap_or(0);
    let total = sheet["total"].as_u64().map(|x| x as usize).unwrap_or(shown);
    // "992 rows · 23 columns · Excel workbook", the file's name under it.
    let headline = format!(
        "{} · {} · {}",
        tc(
            "transactions:dashboard.export.fileRows",
            rows_n,
            &[("formatted", &format::count(rows_n))]
        ),
        s::columns(n_cols),
        e.format.name()
    );
    let file = p["file_name"].as_str().unwrap_or("").to_owned();
    let mut head = row()
        .w_full()
        .gap(tk.r(0.75))
        .center_y()
        .child(mark(app, tk, e.format, tk.r(2.0)))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(tk.r(0.125))
                .child(
                    text(headline.clone())
                        .font(tk.tsm(500.0).features("'tnum' 1"))
                        .color(tk.fg)
                        .single_line()
                        .truncate()
                        .min_w(0.0)
                        .help_when_truncated(headline)
                        .role("status"),
                )
                .child(
                    text(file.clone())
                        .font(tk.txs(400.0).mono())
                        .color(tk.muted_fg)
                        .single_line()
                        .truncate()
                        .min_w(0.0)
                        .help_when_truncated(file),
                ),
        );
    if super::updating(e) {
        head = head.child(
            text(t("transactions:dashboard.export.preview.updating"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .single_line(),
        );
    }
    let pane = pane.child(head);
    // Below the headline: the summary scrolls when the notes run long; the raw columns
    // shrink to the room left instead.
    let mut rest = col()
        .key("txs-export-preview-body")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(tk.r(0.75));
    if shown == 0 {
        rest = rest.child(tb::empty_text(
            tk,
            t("transactions:dashboard.export.preview.empty"),
        ));
    } else if e.all_columns {
        rest = rest.child(raw_table(tk, &sheet));
        if total > shown {
            rest = rest.child(
                text(tc(
                    "transactions:dashboard.export.preview.showing",
                    total,
                    &[
                        ("shown", &format::count(shown)),
                        ("formatted", &format::count(total)),
                    ],
                ))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
            );
        }
    } else {
        rest = rest.child(summary_table(app, tk, e.format, &sheet));
    }
    // The disclosure between the summary and the raw columns.
    if shown > 0 {
        let all = e.all_columns;
        let toggle = w::button(
            tk,
            "txs-export-columns",
            Variant::Ghost,
            Size::Xs,
            Some(if all {
                &I::CHEVRON_UP
            } else {
                &I::CHEVRON_DOWN
            }),
            &if all {
                s::show_summary()
            } else {
                format!("{} ({})", s::show_all_columns(), format::count(n_cols))
            },
            true,
        )
        .expanded(all)
        .on_press(|a: &mut App| a.txs.export.all_columns = !a.txs.export.all_columns);
        let mut bar = row()
            .w_full()
            .gap(tk.r(0.75))
            .center_y()
            .child(toggle.ml(-tk.r(0.5)));
        if !all && rows_n > SUMMARY_ROWS {
            bar = bar.child(el().grow(1.0)).child(
                text(s::first_rows(SUMMARY_ROWS.min(shown)))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .single_line(),
            );
        }
        rest = rest.child(bar);
    }
    if e.format.tool() {
        rest = rest.child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .items_start()
                .child(ic(&I::INFO, tk.r(0.875), tk.muted_fg).mt(1.0))
                .child(
                    text(t(&format!(
                        "transactions:dashboard.export.importHint.{}",
                        e.format.id()
                    )))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0),
                ),
        );
    }
    let warnings: Vec<String> = p["warnings"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|w| w.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if !warnings.is_empty() {
        let amber = hexc(if tk.dark { "#fbbf24" } else { "#d97706" });
        let mut b = col()
            .w_full()
            .padding(tk.r(0.75))
            .gap(tk.r(0.375))
            .radius(tk.r(0.5))
            .border(1.0, hexc("#f59e0b").alpha(0.4))
            .fill(hexc("#f59e0b").alpha(0.1))
            .role("status");
        for w in warnings {
            b = b.child(
                row()
                    .w_full()
                    .gap(tk.r(0.5))
                    .items_start()
                    .child(ic(&I::TRIANGLE_ALERT, tk.r(0.875), amber).mt(1.0))
                    .child(
                        text(w)
                            .font(tk.txs(400.0))
                            .color(tk.fg)
                            .wrap()
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            );
        }
        rest = rest.child(b);
    }
    if !e.all_columns {
        rest = rest.scroll(ScrollSpec {
            stick_bottom: false,
        });
    }
    pane.child(rest)
}

/// The summary: the first rows' date, wallet, direction and amount, value and transaction
/// (BIP 329: type, reference and label), in the table kit's compact look.
fn summary_table(app: &App, tk: &Tk, format: Format, sheet: &Value) -> El {
    let lines = summary::lines(format, sheet, SUMMARY_ROWS);
    let hide = app.prefs.hide_sensitive();
    let xs = tk.txs(400.0);
    let head_cell = |w: Option<f32>, right: bool, l: String| {
        tb::th(tk, w, right, tb::th_text_compact(tk, l)).h(tk.r(tb::HEAD_H_COMPACT))
    };
    let mut table = col()
        .key("txs-export-summary")
        .w_full()
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout()
        .clip_rounded(tk.r(0.5))
        .role("table")
        .label(s::summary_table());
    let n = lines.len();
    if format == Format::Labels {
        let (wt, wr) = (tk.r(4.0), tk.r(8.0));
        table = table.child(
            tb::head(tk)
                .child(head_cell(Some(wt), false, t("common:field.type")))
                .child(head_cell(Some(wr), false, s::reference()))
                .child(head_cell(None, false, t("transactions:details.label"))),
        );
        for (i, l) in lines.iter().enumerate() {
            let r = tb::tr(tk, i + 1 == n)
                .child(tb::td(
                    tk,
                    Some(wt),
                    false,
                    tb::PY_COMPACT,
                    tb::cell_text(l.kind.clone(), xs, tk.muted_fg),
                ))
                .child(tb::td(
                    tk,
                    Some(wr),
                    false,
                    tb::PY_COMPACT,
                    tb::cell_text(summary::short_ref(&l.txid), xs.mono(), tk.fg)
                        .help(l.txid.clone()),
                ))
                .child(tb::td(
                    tk,
                    None,
                    false,
                    tb::PY_COMPACT,
                    tb::cell_text(l.label.clone(), tk.tsm(400.0), tk.fg),
                ));
            table = table.child(r);
        }
        return table;
    }
    let value = summary::has_value(format);
    let cur = book_currency(app);
    let (wd, wa, wv, wt) = (tk.r(8.0), tk.r(8.75), tk.r(6.0), tk.r(6.0));
    let mut h = tb::head(tk)
        .child(head_cell(Some(wd), false, t("common:field.date")))
        .child(head_cell(None, false, s::wallet()))
        .child(head_cell(
            Some(wa),
            true,
            t("transactions:table.column.amount"),
        ));
    if value {
        h = h.child(head_cell(Some(wv), true, s::value()));
    }
    h = h.child(head_cell(
        Some(wt),
        false,
        t("transactions:table.column.transaction"),
    ));
    table = table.child(h);
    for (i, l) in lines.iter().enumerate() {
        let mut r = tb::tr(tk, i + 1 == n)
            .child(tb::td(
                tk,
                Some(wd),
                false,
                tb::PY_COMPACT,
                tb::cell_text(l.date.clone(), xs.features("'tnum' 1"), tk.muted_fg),
            ))
            .child(tb::td(
                tk,
                None,
                false,
                tb::PY_COMPACT,
                tb::cell_text(l.wallet.clone(), tk.tsm(400.0), tk.fg),
            ))
            .child(tb::td(
                tk,
                Some(wa),
                true,
                tb::PY_COMPACT,
                amount_cell(tk, l, hide),
            ));
        if value {
            let v = match l.value {
                Some(v) => crate::ui::sensitive(
                    format::fiat(
                        v,
                        if l.currency.is_empty() {
                            &cur
                        } else {
                            &l.currency
                        },
                    ),
                    xs.features("'tnum' 1"),
                    tk.fg,
                    hide,
                )
                .single_line(),
                None => tb::dash(tk),
            };
            r = r.child(tb::td(tk, Some(wv), true, tb::PY_COMPACT, v));
        }
        let txid = if l.txid.is_empty() {
            tb::dash(tk)
        } else {
            tb::cell_text(summary::short_ref(&l.txid), xs.mono(), tk.muted_fg).help(l.txid.clone())
        };
        r = r.child(tb::td(tk, Some(wt), false, tb::PY_COMPACT, txid));
        table = table.child(r);
    }
    table
}

/// The direction glyph on its tinted tile, then the amount (`+ ₿ 0.08000000`).
fn amount_cell(tk: &Tk, l: &Line, hide: bool) -> El {
    use super::super::model::Flow;
    let flow = match l.dir {
        Some(Dir::In) => Flow::Incoming,
        Some(Dir::Out) => Flow::Outgoing,
        _ => Flow::Transfer,
    };
    let chip = super::super::style::flow_chip(tk, flow);
    let glyph: &'static rune::Icon = match flow {
        Flow::Incoming => &I::ARROW_DOWN_RIGHT,
        Flow::Outgoing => &I::ARROW_UP_RIGHT,
        _ => &I::ARROW_LEFT_RIGHT,
    };
    let dir_name = match flow {
        Flow::Incoming => t("transactions:flow.incoming"),
        Flow::Outgoing => t("transactions:flow.outgoing"),
        _ => {
            if l.kind.is_empty() {
                s::direction()
            } else {
                l.kind.clone()
            }
        }
    };
    let tile = row()
        .size(tk.r(1.25), tk.r(1.25))
        .center()
        .radius(tk.r(0.25))
        .fill(chip.bg)
        .border(1.0, chip.ring)
        .border_layout()
        .role("img")
        .label(dir_name)
        .help(if l.kind.is_empty() {
            s::direction()
        } else {
            l.kind.clone()
        })
        .child(ic(glyph, tk.r(0.75), chip.fg).a11y_hidden());
    let amount = summary::amount_text(l);
    let mut r = row().gap(tk.r(0.375)).center_y().min_w(0.0).child(tile);
    if amount.is_empty() {
        r = r.child(tb::dash(tk));
    } else {
        r = r.child(
            crate::ui::sensitive(
                amount,
                tk.txs(500.0).features("'tnum' 1"),
                super::super::style::amount_color(tk, flow),
                hide,
            )
            .single_line(),
        );
    }
    r
}

/// `previewColumnWidths`: each column's longest cell in characters (at most 40), drawn as
/// `calc(Nch + 1.5rem)`; 1 ch of DM Sans at `text-xs` is 0.555 em.
fn col_width(tk: &Tk, columns: &[String], rows: &[Value], c: usize) -> f32 {
    let mut longest = columns.get(c).map(|s| s.chars().count()).unwrap_or(0);
    for r in rows {
        if let Some(s) = r.get(c).and_then(Value::as_str) {
            longest = longest.max(s.chars().count());
        }
    }
    longest.min(40) as f32 * 0.555 * tk.r(0.75) + tk.r(1.5)
}

fn is_numeric(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_digit() || ".,' -".contains(c))
        && s.chars().any(|c| c.is_ascii_digit())
}

/// Every column of the preview sheet, as the file holds them (the "Show all columns" view).
fn raw_table(tk: &Tk, sheet: &Value) -> El {
    let columns: Vec<String> = sheet["columns"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| c.as_str().unwrap_or("").to_owned())
                .collect()
        })
        .unwrap_or_default();
    let rows = sheet["rows"].as_array().cloned().unwrap_or_default();
    let widths: Vec<f32> = (0..columns.len())
        .map(|c| col_width(tk, &columns, &rows, c))
        .collect();
    let num_w = tk.r(2.25);
    let total_w: f32 = widths.iter().sum::<f32>() + num_w;
    let font = tk.txs(400.0);
    let head_h = tk.r(tb::HEAD_H_COMPACT);
    let mut header = row()
        .w(total_w)
        .h(head_h)
        .fill(tk.muted)
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .sticky_top(0.0)
        .role("row");
    header = header.child(
        row()
            .w(num_w)
            .h_full()
            .border_sides([0.0, 1.0, 0.0, 0.0], tk.border),
    );
    for (c, name) in columns.iter().enumerate() {
        header = header.child(
            row().w(widths[c]).h_full().px(tk.r(0.75)).center_y().child(
                text(name.clone())
                    .font(tk.txs(500.0))
                    .color(tk.muted_fg)
                    .single_line()
                    .truncate()
                    .min_w(0.0),
            ),
        );
    }
    let n = rows.len();
    let tk2 = tk.clone();
    let widths2 = widths.clone();
    let body = col()
        .key("txs-export-rows")
        .w(total_w)
        .growth(GrowthSpec::default())
        .virtual_rows(
            n,
            28.0,
            |_s: &App, i| Cow::Owned(format!("r{i}")),
            move |s: &App, i| {
                let Some(r) = s
                    .txs
                    .export
                    .preview
                    .as_ref()
                    .and_then(|p| p["sheets"][0]["rows"].get(i))
                else {
                    return el().h(28.0);
                };
                let mut rr = row()
                    .w(total_w)
                    .h(28.0)
                    .fill(if i % 2 == 1 {
                        tk2.muted.alpha(0.3)
                    } else {
                        Color::TRANSPARENT
                    })
                    .role("row");
                rr = rr.child(
                    row()
                        .w(num_w)
                        .h(28.0)
                        .px(tk2.r(0.5))
                        .center_y()
                        .justify_end()
                        .fill(tk2.muted.alpha(0.6))
                        .border_sides([0.0, 1.0, 0.0, 0.0], tk2.border)
                        .child(
                            text((i + 1).to_string())
                                .font(tk2.txs(400.0).features("'tnum' 1"))
                                .color(tk2.muted_fg),
                        ),
                );
                for (c, w) in widths2.iter().enumerate() {
                    let v = r.get(c).and_then(Value::as_str).unwrap_or("");
                    let mut cell = row().w(*w).h(28.0).px(tk2.r(0.75)).center_y();
                    if is_numeric(v) {
                        cell = cell.justify_end();
                    }
                    rr = rr.child(
                        cell.child(
                            text(v.to_owned())
                                .font(font.features("'tnum' 1"))
                                .color(tk2.fg)
                                .single_line()
                                .truncate()
                                .min_w(0.0)
                                .help_when_truncated(v.to_owned()),
                        ),
                    );
                }
                rr
            },
        );
    let viewport = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout()
        .clip_rounded(tk.r(0.5))
        .scroll_xy()
        .child(col().w(total_w).child(header).child(body));
    tb::region(
        viewport,
        "txs-export-preview",
        t("transactions:dashboard.export.title"),
        tb::overflows("txs-export-preview", total_w),
        true,
    )
}

/// `QuarantineExportBlockNotice`, as Reports draws it: the count, why, the first five rows and
/// the way to Quarantine.
fn quarantine_notice(tk: &Tk, err: &Value) -> El {
    let d = &err["details"];
    let list = d["transactions"].as_array().cloned().unwrap_or_default();
    let count = d["count"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(list.len());
    let amber = hexc(if tk.dark { "#fbbf24" } else { "#d97706" });
    let mut c = col()
        .key("txs-export-blocked")
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
                                count,
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
                ),
        );
    let mut rows = col().w_full().gap(tk.r(0.25));
    for tx in list.iter().take(5) {
        let id = tx["transaction_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| tx["transaction_ref"].as_str())
            .unwrap_or("");
        let date = summary::app_date(tx["occurred_at"].as_str().unwrap_or(""));
        let wallet = tx["wallet"].as_str().unwrap_or("").to_owned();
        rows = rows.child(
            row()
                .w_full()
                .px(tk.r(0.5))
                .py(tk.r(0.375))
                .gap(tk.r(0.75))
                .center_y()
                .radius(tk.r(0.375))
                .fill(tk.background.alpha(0.6))
                .child(
                    text(date)
                        .font(tk.txs(400.0).features("'tnum' 1"))
                        .color(tk.muted_fg)
                        .single_line(),
                )
                .child(
                    row()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(tk.r(0.5))
                        .child(
                            text(wallet.clone())
                                .font(tk.txs(500.0))
                                .color(tk.fg)
                                .single_line()
                                .truncate()
                                .min_w(0.0)
                                .help_when_truncated(wallet),
                        )
                        .child(
                            text(summary::short_ref(id))
                                .font(tk.txs(400.0).mono())
                                .color(tk.muted_fg)
                                .single_line()
                                .help(id.to_owned()),
                        ),
                )
                .child(
                    text(tx["reason"].as_str().unwrap_or("").to_owned())
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .single_line(),
                ),
        );
    }
    c = c.child(rows);
    if count > 5 {
        c = c.child(
            text(tc(
                "transactions:dashboard.export.blocked.more",
                count - 5,
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
                "txs-export-review",
                Variant::Outline,
                Size::Sm,
                Some(&I::ARROW_UP_RIGHT),
                &t("transactions:dashboard.export.blocked.review"),
                true,
            )
            .on_press(|a: &mut App| {
                a.txs.export.open = false;
                a.navigate("/review");
            }),
        ),
    )
}

// ---- Dialog ---------------------------------------------------------------------------------

/// The footer's summary: wallets · format · period.
fn summary_line(e: &ExportUi) -> String {
    let ws = e.wallets.as_deref().unwrap_or(&[]);
    let sel = super::selected_count(e);
    let wallet = if sel == ws.len() {
        t("transactions:dashboard.export.allWallets")
    } else {
        tr(
            "transactions:dashboard.export.someWallets",
            &[
                ("selected", &format::count(sel)),
                ("total", &format::count(ws.len())),
            ],
        )
    };
    let mut s = if e.format == Format::Labels {
        tr(
            "transactions:dashboard.export.summary.labels",
            &[("wallet", &wallet)],
        )
    } else {
        tr(
            "transactions:dashboard.export.summary.transactions",
            &[("wallet", &wallet), ("format", &e.format.name())],
        )
    };
    if e.format.dated() {
        s = format!("{s} · {}", period_label(e));
    }
    s
}

pub fn dialog(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let e = &app.txs.export;
    let close = super::close;
    let win_h = rune::memo::layout_size("sh-root")
        .map(|[_, h]| h)
        .filter(|h| *h > 0.0)
        .unwrap_or(if app.shell.window_h > 0.0 {
            app.shell.window_h
        } else {
            900.0
        });
    let inset = crate::shell::window_top_inset();
    let top = crate::shell::dialog_top(tk, win_h) - inset;
    let mut popups: Vec<El> = Vec::new();
    // The header (`DialogHeader`, `text-lg` title, `text-sm` description) and the ghost ×.
    let header = col()
        .w_full()
        .px(tk.r(1.25))
        .pt(tk.r(1.25))
        .pb(tk.r(1.0))
        .pr(tk.r(3.5))
        .gap(tk.r(0.5))
        .child(
            text(t("transactions:dashboard.export.title"))
                .font(tk.tlg(600.0).lh(tk.r(1.125)))
                .color(tk.fg)
                .role("heading")
                .level(2),
        )
        .child(
            text(t("transactions:dashboard.export.description"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        );
    let close_btn = w::icon_button(
        tk,
        "txs-export-close",
        Variant::Ghost,
        tk.r(1.75),
        &I::X,
        tk.r(1.0),
        t("common:actions.close"),
    )
    .pin(None, Some(tk.r(0.75)), Some(tk.r(0.75)), None)
    .on_press(close);
    let mut left = col()
        .key("txs-export-choices")
        .w(tk.r(LEFT_W))
        .h_full()
        .min_h(0.0)
        .padding(tk.r(1.25))
        .gap(tk.r(1.25))
        .border_sides([0.0, 1.0, 0.0, 0.0], tk.border)
        .border_layout();
    if e.editing_wallets && e.wallets.is_some() {
        left = left.child(wallet_editor(app, tk));
    } else {
        left = left
            .child(format_section(app, tk, &mut popups))
            .child(period_section(app, tk, &mut popups))
            .child(wallets_section(tk, e));
    }
    let body = row()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .border_sides([1.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .child(left)
        .child(preview_pane(app, tk));
    // The footer: the summary, Cancel and the export ("Export 992 rows").
    let blocked = e.preview_error.is_some();
    let sel = super::selected_count(e);
    let n = e
        .preview
        .as_ref()
        .and_then(|p| p["row_count"].as_u64())
        .unwrap_or(0) as usize;
    let can = !e.busy
        && !super::updating(e)
        && sel > 0
        && !blocked
        && e.preview.is_some()
        && !(e.format.dated() && super::is_custom(e) && super::custom_range(e).is_none());
    let submit_label = if e.busy {
        t("transactions:dashboard.export.exporting")
    } else if e.preview.is_none() {
        t("transactions:dashboard.export.submit")
    } else if e.format == Format::Labels {
        tc(
            "transactions:dashboard.export.submitLabels",
            n,
            &[("formatted", &format::count(n))],
        )
    } else {
        tc(
            "transactions:dashboard.export.submitRows",
            n,
            &[("formatted", &format::count(n))],
        )
    };
    let cancel = w::button(
        tk,
        "txs-export-cancel",
        Variant::Outline,
        Size::Default,
        None,
        &t("common:actions.cancel"),
        !e.busy,
    )
    .on_press(close);
    let submit = w::button(
        tk,
        "txs-export-submit",
        Variant::Default,
        Size::Default,
        Some(&I::DOWNLOAD),
        &submit_label,
        can,
    )
    .on_press(move |a: &mut App| {
        if can {
            super::submit(a);
        }
    });
    let summary = summary_line(e);
    let footer = row()
        .w_full()
        .px(tk.r(1.25))
        .py(tk.r(0.75))
        .gap(tk.r(0.75))
        .center_y()
        .child(
            text(summary.clone())
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .grow(1.0)
                .single_line()
                .truncate()
                .min_w(0.0)
                .help_when_truncated(summary),
        )
        .child(row().gap(tk.r(0.5)).child(cancel).child(submit));
    let surface = crate::shell::glass_dialog(
        tk,
        col()
            .key("txs-export-dialog")
            .role("dialog")
            .label(t("transactions:dashboard.export.title"))
            .description(t("transactions:dashboard.export.description"))
            .w_full()
            .max_w(tk.r(MAX_W))
            .h_full()
            .max_h(tk.r(MAX_H))
            .modal(true, close)
            .on_press(|_: &mut App| {})
            .child(header)
            .child(body)
            .child(footer)
            .child(close_btn),
    );
    // The `kb-glass-backdrop` wash every dialog uses, then the surface placed by flex inside a
    // window-sized wrapper (`centered`'s placement without a measured window width).
    layer.push(
        crate::shell::window_backdrop("txs-export-scrim", tk.glass_backdrop)
            .on_press(close)
            .role("none"),
    );
    layer.push(crate::shell::layer_root(
        col()
            .key("txs-export-wrap")
            .pin(Some(0.0), Some(inset), Some(0.0), Some(0.0))
            .pt(top)
            .pb(tk.r(1.0))
            .px(tk.r(1.0))
            .items_center()
            .on_press(close)
            .role("none")
            .child(surface),
    ));
    layer.extend(popups);
}
