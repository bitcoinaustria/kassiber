//! `RecentTransactionsTable.tsx`: the newest transactions, two lines each, as many whole rows as
//! the widget's size holds (three at Medium, up to eight at Large).

use rune::{Color, El, Icon, TextStyle, col, row, text};

use crate::app::{App, Currency};
use crate::format;
use crate::i18n::t;
use crate::icons as I;
use crate::model::{self, Flow, RecentRow, Snapshot, TxStatus};
use crate::theme::Tk;
use crate::ui::{hairline, ic, sensitive};

use super::grid::frame::{self, Slot};
use super::grid::model::{Size, WidgetId};

fn h(s: &str) -> Color {
    rune::color::hex(s).expect("hex")
}

/// (fill, ink, ring) of a Tailwind tint chip: `bg-x-50 text-x-700 ring-x-600/20`, dark
/// `bg-x-900/25 text-x-300 ring-x-400/20`.
#[derive(Clone, Copy)]
pub(super) struct Chip {
    pub bg: Color,
    pub fg: Color,
    pub ring: Color,
}

pub(super) fn flow_chip(tk: &Tk, f: Flow) -> Chip {
    let (l50, l700, ring_l, d900, d_alpha, d300, ring_d) = match f {
        Flow::Incoming => (
            "#ecfdf5",
            "#047857",
            h("#059669").alpha(0.20),
            "#064e3b",
            0.25,
            "#6ee7b7",
            h("#34d399").alpha(0.2),
        ),
        Flow::Outgoing => (
            "#fef2f2",
            "#b91c1c",
            h("#dc2626").alpha(0.15),
            "#7f1d1d",
            0.25,
            "#fca5a5",
            h("#f87171").alpha(0.2),
        ),
        Flow::Transfer => (
            "#fafafa",
            "#3f3f46",
            h("#71717a").alpha(0.20),
            "#27272a",
            0.70,
            "#d4d4d8",
            h("#a1a1aa").alpha(0.2),
        ),
        Flow::Swap => (
            "#f0f9ff",
            "#0369a1",
            h("#0284c7").alpha(0.20),
            "#0c4a6e",
            0.25,
            "#7dd3fc",
            h("#38bdf8").alpha(0.2),
        ),
    };
    if tk.dark {
        Chip {
            bg: h(d900).alpha(d_alpha),
            fg: h(d300),
            ring: ring_d,
        }
    } else {
        Chip {
            bg: h(l50),
            fg: h(l700),
            ring: ring_l,
        }
    }
}

/// `statusStyles`.
pub(super) fn status_chip(tk: &Tk, s: TxStatus) -> Chip {
    let (l50, l700, ring_l, d900, d400) = match s {
        TxStatus::Confirmed => (
            "#ecfdf5",
            "#047857",
            h("#059669").alpha(0.2),
            "#064e3b",
            "#34d399",
        ),
        TxStatus::Pending => (
            "#fffbeb",
            "#b45309",
            h("#d97706").alpha(0.2),
            "#78350f",
            "#fbbf24",
        ),
        TxStatus::Review => (
            "#eff6ff",
            "#1d4ed8",
            h("#1d4ed8").alpha(0.1),
            "#1e3a8a",
            "#60a5fa",
        ),
        TxStatus::Failed => (
            "#fef2f2",
            "#b91c1c",
            h("#dc2626").alpha(0.1),
            "#7f1d1d",
            "#f87171",
        ),
    };
    if tk.dark {
        Chip {
            bg: h(d900).alpha(0.3),
            fg: h(d400),
            ring: h(d400).alpha(0.2),
        }
    } else {
        Chip {
            bg: h(l50),
            fg: h(l700),
            ring: ring_l,
        }
    }
}

pub(super) fn flow_icon(f: Flow) -> &'static Icon {
    match f {
        Flow::Incoming => &I::ARROW_DOWN_RIGHT,
        Flow::Outgoing => &I::ARROW_UP_RIGHT,
        Flow::Transfer | Flow::Swap => &I::ARROW_LEFT_RIGHT,
    }
}

/// The amount ink: emerald in, red out, muted otherwise.
pub(super) fn amount_ink(tk: &Tk, f: Flow) -> Color {
    match (f, tk.dark) {
        (Flow::Incoming, false) => h("#047857"),
        (Flow::Incoming, true) => h("#6ee7b7"),
        (Flow::Outgoing, false) => h("#b91c1c"),
        (Flow::Outgoing, true) => h("#fca5a5"),
        _ => tk.muted_fg,
    }
}

/// The primary and secondary amount of a row in the current display currency.
pub(super) fn amounts(app: &App, s: &Snapshot, r: &RecentRow) -> (String, String) {
    let cur = r.fiat_currency.clone().unwrap_or_else(|| s.fiat_currency());
    match app.prefs.currency {
        Currency::Btc => (
            format::btc(r.amount_btc, 8, true),
            r.amount_fiat
                .map(|v| format::fiat(v.abs(), &cur))
                .unwrap_or_else(|| "Unpriced".into()),
        ),
        Currency::Fiat => (
            r.amount_fiat
                .map(|v| format::signed_fiat(v, &cur))
                .unwrap_or_else(|| "Unpriced".into()),
            format::btc(r.amount_btc, 8, false),
        ),
    }
}

fn row_el(app: &App, tk: &Tk, s: &Snapshot, r: &RecentRow, i: usize, slot: &Slot) -> El {
    let hide = app.prefs.hide_sensitive();
    let chip = flow_chip(tk, r.flow);
    let counterparty = if r.counterparty == "Unassigned" {
        t("overview:recentTx.unassigned")
    } else {
        r.counterparty.clone()
    };
    let type_label = super::l10n::type_label(&r.type_label, r.flow);
    // `leading-tight` (1.25) on both columns: 13 px lines are 16.25 pt, `text-2xs` 12.49 pt.
    let small = TextStyle::new(tk.r(0.694), 400.0).lh(tk.r(0.694) * 1.25);
    let line1 = TextStyle::new(13.0, 500.0).lh(13.0 * 1.25);
    let mut meta = row()
        .min_w(0.0)
        .mt(tk.r(0.125))
        .gap(tk.r(0.375))
        .center_y()
        .child(
            text(r.date.clone())
                .font(small.features("'tnum' 1"))
                .color(tk.muted_fg),
        )
        .child(text("·").font(small).color(tk.muted_fg))
        .child(
            text(if r.extra_tags > 0 {
                format!("{type_label} +{}", r.extra_tags)
            } else {
                type_label.clone()
            })
            .font(small)
            .color(tk.muted_fg)
            .truncate()
            .min_w(0.0),
        );
    if r.status != TxStatus::Confirmed {
        let sc = status_chip(tk, r.status);
        meta = meta.child(
            row()
                .px(tk.r(0.25))
                .radius(tk.r(0.375))
                .fill(sc.bg)
                .border(1.0, sc.ring)
                .child(
                    text(super::l10n::status(r.status))
                        .font(TextStyle {
                            weight: 500.0,
                            ..small
                        })
                        .color(sc.fg),
                ),
        );
    }
    let (primary, secondary) = amounts(app, s, r);
    let toggle_label = t(if app.prefs.currency == Currency::Btc {
        "chrome:currencyToggleText.showFiat"
    } else {
        "chrome:currencyToggleText.showBitcoin"
    });
    // CurrencyToggleText: the amount names the control; the switch is its description/title.
    let amount = sensitive(
        primary.clone(),
        TextStyle::new(13.0, 600.0)
            .lh(13.0 * 1.25)
            .features("'tnum' 1"),
        amount_ink(tk, r.flow),
        hide,
    )
    .key(slot.key(format!("recent-amount-{}", r.id)))
    .role("button")
    // Blurred, not masked: the name stays the amount, as the web's span text does.
    .label(primary)
    .description(toggle_label.clone())
    .help(toggle_label)
    .focusable(true);
    let amount = super::grid::press(amount, WidgetId::Recent, slot.pv, |a| {
        a.prefs.currency = if a.prefs.currency == Currency::Btc {
            Currency::Fiat
        } else {
            Currency::Btc
        };
    });
    let href = format!("/transactions?tx={}", r.id);
    // The rows share the slot's height; the tile's padding is their inset.
    let line = row()
        .key(slot.key(format!("recent-row-{i}")))
        .w_full()
        .min_w(0.0)
        .grow(1.0)
        .h(0.0)
        .gap(tk.r(0.625))
        .center_y()
        .role("link")
        .label(format!("{counterparty} {}", r.date))
        .focusable(true)
        .focus_ring(2.0, 2.0, tk.ring);
    super::grid::press(line, WidgetId::Recent, slot.pv, move |a| a.navigate(&href))
        .child(
            // `size-6` (1.5 rem) with a `size-3.5` glyph.
            row()
                .size(tk.r(1.5), tk.r(1.5))
                .center()
                .radius(tk.r(0.375))
                .fill(chip.bg)
                .border(1.0, chip.ring)
                .a11y_hidden()
                .child(ic(flow_icon(r.flow), tk.r(0.875), chip.fg)),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(
                    sensitive(
                        if r.counterparty.is_empty() {
                            r.txid.clone()
                        } else {
                            counterparty.clone()
                        },
                        line1,
                        tk.fg,
                        hide,
                    )
                    .truncate(),
                )
                .child(meta),
        )
        .child(col().items_end().pl(tk.r(0.5)).child(amount).child(
            sensitive(secondary, small.features("'tnum' 1"), tk.muted_fg, hide).mt(tk.r(0.125)),
        ))
}

/// A row's height: the taller of the 21.6 pt glyph tile and the two text lines, with breathing room.
fn row_h(tk: &Tk) -> f32 {
    2.0 * tk.r(0.375) + (13.0 * 1.25 + tk.r(0.125) + tk.r(0.694) * 1.25).max(tk.r(1.5))
}

/// The list for the slot: only whole rows, spread over its height.
pub fn body(app: &App, tk: &Tk, s: &Snapshot, slot: &Slot) -> El {
    let rows = model::recent_rows(s);
    if rows.is_empty() {
        return frame::empty(tk, slot.key("recent-empty"), t("overview:recentTx.empty"));
    }
    let cap = if slot.size == Size::Medium { 3 } else { 8 };
    let fits = (((slot.h + 1.0) / (row_h(tk) + 1.0)) + 1e-6)
        .floor()
        .max(1.0) as usize;
    let shown = rows.len().min(cap).min(fits);
    let mut list = col()
        .key(slot.key("recent-list"))
        .w_full()
        .h_full()
        .min_h(0.0);
    for (i, r) in rows.iter().take(shown).enumerate() {
        if i > 0 {
            list = list.child(hairline(tk.border.alpha(tk.border.a * 0.6)));
        }
        list = list.child(row_el(app, tk, s, r, i, slot));
    }
    // Fewer transactions than rows: they keep a row's height at the top.
    if shown < cap.min(fits) {
        list = list.child(rune::el().grow((cap.min(fits) - shown) as f32).h(0.0));
    }
    list
}
