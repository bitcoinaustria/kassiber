//! Holdings by Asset: the sibling of Holdings by Source (`HoldingsBySource.tsx`), in its visual
//! language, grouping what the book holds by asset and unit. An owner-requested addition with
//! no web counterpart yet; the model and its rules live in `assets.rs`.

use rune::{Cx, El, TextStyle, col, row, text};

use super::assets::{self, AssetGroup, AssetRow, Denomination, MSAT_PER_BTC, UnitKind};
use super::grid::frame::{self, Slot};
use super::stats::mask;
use crate::app::{App, Currency};
use crate::format;
use crate::i18n::{Lang, lang};
use crate::icons as I;
use crate::model::{Rail, Snapshot};
use crate::theme::Tk;
use crate::ui::{hairline, ic, sensitive};

/// The card's own copy. The web has no keys for it yet, so English and German live here; the
/// keys to add under `overview:holdingsByAsset.*` are named in the comments.
fn l(en: &'static str, de: &'static str) -> String {
    match lang() {
        Lang::De => de.to_owned(),
        Lang::En => en.to_owned(),
    }
}

fn title() -> String {
    l("Holdings by Asset", "Bestände nach Asset") // title
}
fn aria() -> String {
    l("Holdings by asset", "Bestände nach Asset") // aria
}
fn empty() -> String {
    l(
        "No wallet holds an asset yet.",
        "Noch keine Wallet hält ein Asset.",
    ) // empty
}
fn no_price() -> String {
    l("No price", "Kein Preis") // noPrice
}
fn no_price_hint() -> String {
    // noPriceHint
    l(
        "No market price for this asset, so Kassiber shows no value rather than a guess.",
        "Für dieses Asset gibt es keinen Marktpreis, daher zeigt Kassiber keinen Wert statt einer Schätzung.",
    )
}
fn synthetic_badge() -> String {
    l("Synthetic sample", "Synthetisches Beispiel") // synthetic
}

fn group_label(d: &Denomination) -> String {
    match d {
        Denomination::Btc => l("Bitcoin", "Bitcoin"), // group.bitcoin
        Denomination::Fiat(code) => format!("{} · {code}", l("Fiat", "Fiat")), // group.fiat
        Denomination::Mixed => l("Other assets", "Sonstige Assets"), // group.other
    }
}

/// Known rows by symbol in the UI language; anything else keeps its own name.
fn row_name(r: &AssetRow) -> String {
    match (r.kind, r.symbol.as_str()) {
        (UnitKind::NativeCoin, "BTC") => l("On-chain", "On-Chain"),
        (UnitKind::NativeCoin, "LN-BTC") => l("Lightning", "Lightning"),
        (UnitKind::Pegged, "L-BTC") => l("Liquid", "Liquid"),
        (UnitKind::Fiat, _) => l("Cash", "Bargeld"),
        _ => r.name.clone(),
    }
}

/// Exact amount for the tooltip: BTC to the satoshi (11 decimals below one), fiat to the cent.
fn exact(r: &AssetRow) -> String {
    match r.denomination() {
        Denomination::Btc => {
            let abs = r.units.unsigned_abs();
            if !abs.is_multiple_of(1000) {
                format!(
                    "₿ {}.{:011}",
                    abs / MSAT_PER_BTC as u128,
                    abs % MSAT_PER_BTC as u128
                )
            } else {
                let sat = abs / 1000;
                format!("₿ {}.{:08}", sat / 100_000_000, sat % 100_000_000)
            }
        }
        Denomination::Fiat(code) => format::fiat(r.amount(), &code),
        Denomination::Mixed => format!("{:.*} {}", r.precision as usize, r.amount(), r.symbol),
    }
}

/// What a row or group shows: (text, priced). Fiat cash is money already and shows its face
/// value in both modes; bitcoin shows ₿ or its value; an unpriced value reads "No price".
fn shown(
    app: &App,
    cur: &str,
    d: &Denomination,
    units: i128,
    precision: u8,
    symbol: &str,
    value: Option<f64>,
) -> (String, bool) {
    let amount = units as f64 / 10f64.powi(precision as i32);
    match (d, app.prefs.currency) {
        (Denomination::Fiat(code), _) => (format::fiat_whole(amount, code), true),
        (Denomination::Btc, Currency::Btc) => (format::btc(amount, 3, false), true),
        (Denomination::Mixed, Currency::Btc) => {
            (format!("{} {symbol}", trim_amount(amount, precision)), true)
        }
        (_, Currency::Fiat) => match value {
            Some(v) => (format::fiat_whole(v, cur), true),
            None => (no_price(), false),
        },
    }
}

fn trim_amount(v: f64, precision: u8) -> String {
    let s = format!("{v:.*}", precision.min(8) as usize);
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

/// The asset's mark: the layer logo for bitcoin, a banknote for fiat, a dashed coin otherwise.
fn asset_icon(app: &App, cx: &Cx<App>, tk: &Tk, r: &AssetRow) -> El {
    match (r.layer, r.kind) {
        (Some(rail @ (Rail::Onchain | Rail::Lightning | Rail::Liquid)), _) => {
            super::holdings::rail_icon(app, cx, tk, rail)
        }
        (_, kind) => {
            let glyph = if kind == UnitKind::Fiat {
                &I::BANKNOTE
            } else {
                &I::CIRCLE_DASHED
            };
            // The holdings' non-rail mark (`size-5 rounded-full bg-muted`, a `size-3` glyph) in
            // the rail logo's slot, ringless like the rail logos.
            row()
                .size(tk.r(1.5), tk.r(1.5))
                .center()
                .help(r.symbol.clone())
                .child(
                    row()
                        .size(tk.r(1.25), tk.r(1.25))
                        .center()
                        .radius(tk.r(0.625))
                        .fill(tk.muted)
                        .child(ic(glyph, tk.r(0.75), tk.muted_fg)),
                )
        }
    }
}

/// The amber "Synthetic sample" label on scene-injected rows.
fn synthetic_pill(tk: &Tk) -> El {
    let tone = &tk.warn;
    row().child(
        text(synthetic_badge())
            .font(tk.t2xs(500.0))
            .color(tone.text),
    )
}

/// The four-column grid of a holding row: `1.5rem | 1fr | 6.5rem | 2.5rem`.
fn grid_row(tk: &Tk, key: String, lead: El, name: El, value: El, share: Option<El>) -> El {
    let r = row()
        .key(key)
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.5))
        .py(tk.r(0.125))
        .center_y()
        .child(row().w(tk.r(1.5)).justify_center().child(lead))
        .child(name.grow(1.0).min_w(0.0))
        .child(value.w(tk.r(6.5)).align(rune::Align::End).truncate());
    match share {
        Some(share) => r.child(share.w(tk.r(2.5)).align(rune::Align::End)),
        None => r,
    }
}

fn value_el(
    app: &App,
    tk: &Tk,
    style: TextStyle,
    txt: String,
    priced: bool,
    exact_help: Option<String>,
) -> El {
    let hide = app.prefs.hide_sensitive();
    if !priced {
        // Nothing to hide: the absence of a price is not an amount.
        return text(txt)
            .font(TextStyle {
                weight: 400.0,
                ..style
            })
            .color(tk.muted_fg)
            .help(no_price_hint());
    }
    let el = sensitive(mask(txt, hide), style.features("'tnum' 1"), tk.fg, hide);
    match exact_help {
        Some(h) if !hide => el.help(h),
        _ => el,
    }
}

#[allow(clippy::too_many_arguments)]
fn asset_row(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    cur: &str,
    r: &AssetRow,
    total_units: i128,
    key: String,
    compact: bool,
) -> El {
    let xs = tk.txs(400.0);
    let mut name = row()
        .gap(tk.r(0.375))
        .items_end()
        .child(
            text(row_name(r))
                .font(xs)
                .color(tk.fg)
                .truncate()
                .min_w(0.0)
                .help(r.name.clone()),
        )
        .child(
            text(r.symbol.clone())
                .font(tk.t2xs(400.0))
                .color(tk.muted_fg),
        );
    if r.synthetic && !compact {
        name = name.child(synthetic_pill(tk));
    }
    let (txt, priced) = shown(
        app,
        cur,
        &r.denomination(),
        r.units,
        r.precision,
        &r.symbol,
        r.fiat_value,
    );
    let shared_unit = r.denomination() != Denomination::Mixed;
    let pct = if !shared_unit {
        String::new()
    } else if r.share == 0.0 && r.units > 0 {
        "<1%".to_owned()
    } else if r.share >= 100.0 && r.units < total_units {
        // Rounding must not claim the whole group while another row holds some of it.
        ">99%".to_owned()
    } else {
        format!("{}%", r.share)
    };
    let hide = app.prefs.hide_sensitive();
    grid_row(
        tk,
        key,
        asset_icon(app, cx, tk, r),
        name,
        value_el(app, tk, tk.txs(500.0), txt, priced, Some(exact(r))),
        (!compact).then(|| {
            sensitive(
                mask(pct, app.prefs.hide_shares()),
                xs.features("'tnum' 1"),
                tk.muted_fg,
                hide,
            )
        }),
    )
}

#[allow(clippy::too_many_arguments)]
fn group_block(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    cur: &str,
    g: &AssetGroup,
    gi: usize,
    slot: &Slot,
    max_rows: usize,
) -> El {
    let compact = !slot.wide();
    let label = group_label(&g.denomination);
    let total: El = match g.total_units {
        Some(units) => {
            let (precision, symbol) = g
                .rows
                .first()
                .map(|r| (r.precision, r.symbol.clone()))
                .unwrap_or((11, "BTC".into()));
            let (txt, priced) = shown(
                app,
                cur,
                &g.denomination,
                units,
                precision,
                &symbol,
                g.total_value,
            );
            value_el(app, tk, tk.txs(600.0), txt, priced, None)
        }
        // Different units are never summed.
        None => text("").font(tk.txs(400.0)),
    };
    let head = grid_row(
        tk,
        slot.key(format!("asset-group-{gi}")),
        rune::el().size(tk.r(1.5), 1.0),
        text(label.clone())
            .font(tk.txs(500.0))
            .color(tk.muted_fg)
            .truncate()
            .role("heading"),
        total,
        (!compact).then(|| text("").font(tk.txs(400.0))),
    );
    let mut c = col()
        .key(slot.key(format!("asset-group-block-{gi}")))
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.25))
        .role("group")
        .label(label)
        .child(head)
        .child(hairline(tk.fg.alpha(0.08)));
    for (i, r) in g.rows.iter().enumerate().take(max_rows) {
        c = c.child(asset_row(
            app,
            cx,
            tk,
            cur,
            r,
            g.total_units.unwrap_or(0),
            slot.key(format!("asset-{gi}-{i}")),
            compact,
        ));
    }
    c
}

/// The groups this card shows for the snapshot (plus any scene's synthetic rows).
pub fn groups(app: &App, s: &Snapshot) -> Vec<AssetGroup> {
    let raw = app.read("ui.overview.snapshot");
    let sources = assets::sources(s, raw);
    let mut rows = assets::bitcoin_rows(&sources, assets::btc_rate(s));
    let cur = s.fiat_currency();
    for r in &app.ov.synthetic_assets {
        let mut r = r.clone();
        // Fiat cash is valued at face only in the display currency, never converted.
        if r.kind == UnitKind::Fiat {
            r.fiat_value = r.symbol.eq_ignore_ascii_case(&cur).then_some(r.amount());
        }
        rows.push(r);
    }
    assets::build(rows)
}

/// The widget's title (`overview:holdingsByAsset.title` when the web adds it).
pub fn widget_title() -> String {
    title()
}

/// The widget's content for its slot: the bitcoin total as the figure with the layers under it
/// (Small, no share column), or every group, side by side when there are several (Medium).
pub fn body(app: &App, cx: &mut Cx<App>, tk: &Tk, s: &Snapshot, slot: &Slot) -> El {
    let cur = s.fiat_currency();
    let groups = groups(app, s);
    if groups.is_empty() {
        return frame::empty(tk, slot.key("assets-empty"), empty());
    }
    let compact = !slot.wide();
    // Whole rows only: a group's head and rule, then its rows.
    let line = tk.r(1.75) + tk.r(0.25);
    let fit = ((slot.h / line).floor() as usize).max(2);
    let content = if !compact && groups.len() > 1 {
        let mut r = row().w_full().gap(tk.r(1.5)).items_start();
        for (gi, g) in groups.iter().enumerate().take(2) {
            r = r.child(col().grow(1.0).w(0.0).min_w(0.0).child(group_block(
                app,
                cx,
                tk,
                &cur,
                g,
                gi,
                slot,
                fit - 1,
            )));
        }
        r
    } else {
        let mut c = col().w_full().gap(tk.r(0.5));
        let mut left = fit;
        for (gi, g) in groups.iter().enumerate() {
            if left < 2 {
                break;
            }
            c = c.child(group_block(app, cx, tk, &cur, g, gi, slot, left - 1));
            left = left.saturating_sub(g.rows.len() + 2);
        }
        c
    };
    col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .clip_rounded(0.0)
        .label(aria())
        .child(content)
}
