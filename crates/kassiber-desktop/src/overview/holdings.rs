//! `HoldingsBySource.tsx`: where the bitcoin sits, in the layout and order the settings menu
//! picks (`HoldingsSettingsMenu`); emptied sources fold into one line unless the menu lists or
//! hides them. The four layouts live in `holdings_layouts.rs`, the rules in `holdings_model.rs`.

use std::rc::Rc;

use rune::{Cx, El, col, el, row, text};

use super::grid::frame::{self, Slot};
use super::grid::model::{Size, WidgetId};
use super::holdings_layouts as layouts;
use super::holdings_model::{self as hm, Empty, Group, HoldingsView, Layout, Sort};
use super::menu::Entry;
use super::stats::mask;
use crate::app::{App, Currency};
use crate::format;
use crate::i18n::{t, tc};
use crate::model::{self, Rail, Snapshot};
use crate::theme::Tk;
use crate::ui::sensitive;

/// `RailIcon size="compact"`: the rail's logo on its own (no white disc or ring), see
/// `crate::logos::rail`.
pub(super) fn rail_icon(app: &App, cx: &Cx<App>, tk: &Tk, rail: Rail) -> El {
    use crate::logos::RailLogo;
    let (logo, label) = match rail {
        Rail::Onchain => (RailLogo::Onchain, "Bitcoin"),
        Rail::Lightning => (RailLogo::Lightning, "Lightning"),
        Rail::Liquid => (RailLogo::Liquid, "Liquid"),
        Rail::None => return el().size(tk.r(1.5), tk.r(1.5)),
    };
    crate::logos::rail(app, cx, tk, logo, label)
}

/// The clock the trend's and the widgets' months end with (a scene may pin it).
pub(super) fn now_secs(app: &App) -> f64 {
    app.ov.now_override.unwrap_or_else(format::now_secs)
}

/// Everything a layout needs to draw its rows.
pub struct Ctx<'a> {
    pub app: &'a App,
    pub tk: &'a Tk,
    pub hide: bool,
    /// "Hide all": shares reveal how the holdings split, so they go too.
    pub hide_shares: bool,
    pub btc_mode: bool,
    pub rate_missing: bool,
    pub cur: String,
    /// The widget's content width (its container queries).
    pub inner: f32,
    /// A list row's height in this slot.
    pub row_h: f32,
    /// A gallery preview: prefixed keys, no handlers.
    pub pv: bool,
}

impl Ctx<'_> {
    pub fn key(&self, k: impl AsRef<str>) -> String {
        if self.pv {
            format!("pv-{}", k.as_ref())
        } else {
            k.as_ref().to_owned()
        }
    }

    /// `valueLabel`: the figure in the display currency, or "—" without a rate.
    pub fn value_label(&self, btc: f64, value: f64) -> String {
        let text = if self.btc_mode {
            format::btc(btc, 3, false)
        } else if self.rate_missing {
            "—".into()
        } else {
            format::fiat_whole(value, &self.cur)
        };
        mask(text, self.hide)
    }

    /// `fiatLabel`: fiat regardless of the display currency, or `None` without a rate.
    pub fn fiat_label(&self, value: f64) -> Option<String> {
        (!self.rate_missing).then(|| mask(format::fiat_whole(value, &self.cur), self.hide))
    }
}

/// The layout the card draws: the stored one, or the list while the daemon sends no
/// month-end balances (a trend would have nothing to draw).
pub fn effective_layout(view: &HoldingsView, trend_ok: bool) -> Layout {
    if view.layout == Layout::Trend && !trend_ok {
        Layout::List
    } else {
        view.layout
    }
}

/// A muted `text-xs` text button (`hover:text-foreground`, focus ring).
fn text_button(
    tk: &Tk,
    key: String,
    label: String,
    help: Option<String>,
    expanded: Option<bool>,
    pv: bool,
    on: fn(&mut App),
) -> El {
    let mut b = row()
        .key(key)
        .role("button")
        .label(label.clone())
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .hover_group()
        .focus_ring(2.0, 0.0, tk.ring)
        .child(
            text(label)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .group_color(tk.fg),
        );
    b = super::grid::press(b, WidgetId::Holdings, pv, on);
    if let Some(h) = help {
        b = b.help(h);
    }
    if let Some(open) = expanded {
        b = b.expanded(open);
    }
    b
}

/// The widget's content for its slot: the total as the figure, the split as one bar, then the
/// sources: the top three as a list (Medium), or the layout the menu picked, as many whole rows
/// as fit, with the folded empty sources underneath (Large). The settings live in the tile's
/// context menu (`menu_entries`).
pub fn body(app: &App, cx: &mut Cx<App>, tk: &Tk, s: &Snapshot, x: &hm::Extras, slot: &Slot) -> El {
    let hide = app.prefs.hide_sensitive();
    let large = slot.size == Size::Large;
    let trend_ok = x.has_month_balances();
    let layout = if large {
        effective_layout(&app.ov.holdings, trend_ok)
    } else {
        Layout::List
    };
    // The table lists every source, so none folds into "Other sources".
    let build_view = if layout == Layout::Table {
        HoldingsView {
            limit: 0,
            ..app.ov.holdings.clone()
        }
    } else {
        app.ov.holdings.clone()
    };
    let built = hm::build(s, x, &build_view, hm::local_month(now_secs(app)));
    let filtered = built.filtered_out > 0;
    let total_value: f64 = built.items.iter().map(|i| i.value).sum();
    let issue = model::completeness_issue(s);
    let btc_mode = app.prefs.currency == Currency::Btc;
    let mut c = Ctx {
        app,
        tk,
        hide,
        hide_shares: app.prefs.hide_shares(),
        btc_mode,
        rate_missing: s.fiat.completeness.market_rate_missing,
        cur: s.fiat_currency(),
        inner: slot.w,
        row_h: tk.r(1.75),
        pv: slot.pv,
    };

    let fig = if large { 2.0 } else { 1.75 };
    let mut head = row().w_full().min_w(0.0).gap(tk.r(0.5)).items_end().child(
        sensitive(
            c.value_label(built.total_btc, total_value),
            frame::figure(tk, fig),
            tk.fg,
            hide,
        )
        .truncate()
        .min_w(0.0),
    );
    // Tile and chart already say when the basis is incomplete, and a book-wide figure does not
    // belong beside a filtered total.
    if !btc_mode && issue.is_none() && !filtered {
        let f = &s.fiat;
        let pct = if f.eur_cost_basis != 0.0 {
            f.eur_unrealized / f.eur_cost_basis * 100.0
        } else {
            0.0
        };
        let txt = format!("{}{:.1}%", if pct >= 0.0 { "+" } else { "" }, pct);
        let pct_color = if pct >= 0.0 {
            tk.good.solid
        } else {
            tk.alert.solid
        };
        let shown = mask(txt, app.prefs.hide_shares());
        let el = if hide {
            row()
                .gap(tk.r(0.25))
                .child(sensitive(shown, tk.txs(500.0), pct_color, true))
                .child(
                    text(t("overview:holdings.vsCostBasis"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                )
        } else {
            let content = format!("{shown} {}", t("overview:holdings.vsCostBasis"));
            row().child(
                rune::rich(
                    content,
                    vec![
                        rune::Span::new(0, shown.len())
                            .weight(500.0)
                            .color(pct_color),
                    ],
                )
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
            )
        };
        head = head.child(el.pb(tk.r(0.25)));
    }
    let head_h = tk.r(fig) * 1.15;
    let shares: Vec<f64> = built
        .items
        .iter()
        .filter(|i| !i.empty)
        .map(|i| i.btc)
        .collect();
    let bar = frame::share_bar(tk, &shares, c.hide_shares);
    let bar = if hide { bar.blur(3.5) } else { bar };
    let gap = tk.r(0.625);

    // The foot (Large): what the filter hides and the folded empty sources.
    let mut foot: Vec<El> = Vec::new();
    if large && !built.folded.is_empty() {
        let open = app.ov.empty_open;
        let label = if open {
            t("overview:holdings.hideEmpty")
        } else {
            tc("overview:holdings.emptySources", built.folded.len(), &[])
        };
        foot.push(text_button(
            tk,
            slot.key("holdings-empty-toggle"),
            label,
            None,
            Some(open),
            slot.pv,
            |a| a.ov.empty_open = !a.ov.empty_open,
        ));
    }
    if filtered {
        let label = tc("overview:holdings.filteredOut", built.filtered_out, &[]);
        foot.push(text_button(
            tk,
            slot.key("holdings-filtered"),
            label,
            Some(t("overview:holdings.showAllRails")),
            None,
            slot.pv,
            |a| a.ov.holdings.rails = Group::ALL.to_vec(),
        ));
    }
    let foot_h = if foot.is_empty() {
        0.0
    } else {
        tk.r(1.125) + gap
    };
    let avail = (slot.h - head_h - tk.r(0.5) - foot_h - 2.0 * gap).max(0.0);

    let mut rows = built.items.clone();
    if large && app.ov.empty_open {
        rows.extend(built.folded.iter().cloned());
    }
    let list = if rows.is_empty() {
        frame::empty(
            tk,
            slot.key("holdings-empty"),
            if filtered {
                t("overview:holdings.noneListed")
            } else {
                t("overview:holdings.empty")
            },
        )
    } else {
        let line_gap = tk.r(0.125);
        // How many whole rows the slot holds in this layout.
        let cap = match layout {
            Layout::List if !large => 3,
            Layout::List => {
                let columns = if slot.w >= tk.r(44.0) { 2 } else { 1 };
                (((avail + line_gap) / (tk.r(1.75) + line_gap))
                    .floor()
                    .max(1.0) as usize)
                    * columns
            }
            Layout::Trend => {
                let columns = if slot.w >= tk.r(72.0) { 2 } else { 1 };
                (((avail + line_gap) / (tk.r(2.0) + line_gap))
                    .floor()
                    .max(1.0) as usize)
                    * columns
            }
            Layout::Tiles => {
                let columns = (((slot.w + tk.r(0.5)) / (tk.r(13.5) + tk.r(0.5)))
                    .floor()
                    .max(1.0)) as usize;
                (((avail + tk.r(0.5)) / (layouts::TILE_H * tk.rem + tk.r(0.5)))
                    .floor()
                    .max(1.0) as usize)
                    * columns
            }
            Layout::Table => ((avail - tk.r(2.0)) / (tk.r(2.25) + 1.0)).floor().max(1.0) as usize,
        };
        if rows.len() > cap {
            // The folded "Other sources" row stays the last one.
            let other = rows
                .iter()
                .rposition(|r| r.other)
                .filter(|_| cap > 1)
                .map(|i| rows.remove(i));
            rows.truncate(cap - usize::from(other.is_some()));
            rows.extend(other);
        }
        if layout == Layout::List {
            // A short list spreads a little; a long one keeps the compact row.
            let lines = if slot.w >= tk.r(44.0) && rows.len() > 1 {
                rows.len().div_ceil(2)
            } else {
                rows.len()
            };
            c.row_h = ((avail + line_gap) / lines as f32 - line_gap).clamp(tk.r(1.75), tk.r(2.5));
        }
        match layout {
            Layout::Tiles => layouts::tiles(&c, cx, &rows),
            Layout::Table => layouts::table(&c, cx, &rows),
            Layout::Trend => layouts::trend(&c, cx, &rows),
            Layout::List => layouts::list(&c, cx, &rows),
        }
    };
    let mut out = col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .gap(gap)
        .label(t("overview:holdings.aria"))
        .child(head)
        .child(bar)
        .child(
            col()
                .w_full()
                .grow(1.0)
                .h(0.0)
                .min_h(0.0)
                .clip_rounded(0.0)
                .child(list),
        );
    if !foot.is_empty() {
        out = out.child(
            row()
                .w_full()
                .h(tk.r(1.125))
                .gap(tk.r(1.0))
                .center_y()
                .children(foot),
        );
    }
    out
}

fn radio<T: Copy + PartialEq + 'static>(
    key: String,
    label: String,
    value: T,
    current: T,
    set: fn(&mut HoldingsView, T),
) -> Entry {
    Entry::Radio {
        key,
        label,
        checked: value == current,
        on: Rc::new(move |a: &mut App| set(&mut a.ov.holdings, value)),
    }
}

/// `HoldingsSettingsMenu`: layout and order on top, the rarer choices one level down, each
/// trigger naming its current setting. The widget's ⋯ menu lists these, then "Hide widget".
pub fn menu_entries(app: &App, tk: &Tk) -> Vec<Entry> {
    let trend_ok = hm::Extras::parse(app.read("ui.overview.snapshot")).has_month_balances();
    let v = &app.ov.holdings;
    let current = effective_layout(v, trend_ok);
    let mut entries = vec![Entry::Label(t("overview:holdings.layoutLabel"))];
    for layout in Layout::ALL {
        // Without the daemon's month-end balances the trend has nothing to draw.
        if layout == Layout::Trend && !trend_ok {
            continue;
        }
        entries.push(radio(
            format!("ov-menu-layout-{}", layout.id()),
            t(&format!("overview:holdings.layout.{}", layout.id())),
            layout,
            current,
            |h, x| h.layout = x,
        ));
    }
    entries.push(Entry::Sep);
    entries.push(Entry::Label(t("overview:holdings.sortLabel")));
    for sort in Sort::ALL {
        entries.push(radio(
            format!("ov-menu-sort-{}", sort.id()),
            t(&format!("overview:holdings.sort.{}", sort.id())),
            sort,
            v.sort,
            |h, x| h.sort = x,
        ));
    }
    entries.push(Entry::Sep);
    let rails_value = if v.all_rails() {
        t("overview:holdings.limitAll")
    } else {
        v.rails
            .iter()
            .map(|g| t(&format!("overview:holdings.rail.{}", g.id())))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let rails = Group::ALL
        .into_iter()
        .map(|g| {
            let checked = v.rails.contains(&g);
            Entry::Check {
                key: String::new(),
                label: t(&format!("overview:holdings.rail.{}", g.id())),
                checked,
                // The last rail stays on: an empty filter would list nothing.
                disabled: checked && v.rails.len() == 1,
                on: Rc::new(move |a: &mut App| a.ov.holdings.toggle_rail(g)),
            }
        })
        .collect();
    entries.push(Entry::Sub {
        name: "rails",
        label: t("overview:holdings.railsLabel"),
        value: rails_value,
        width: tk.r(13.0),
        entries: rails,
    });
    let empties = Empty::ALL
        .into_iter()
        .map(|m| {
            radio(
                String::new(),
                t(&format!("overview:holdings.emptyMode.{}", m.id())),
                m,
                v.empty,
                |h, x| h.empty = x,
            )
        })
        .collect();
    entries.push(Entry::Sub {
        name: "empty",
        label: t("overview:holdings.emptyLabel"),
        value: t(&format!("overview:holdings.emptyMode.{}", v.empty.id())),
        width: tk.r(13.0),
        entries: empties,
    });
    let limit_label = |l: u32| {
        if l == 0 {
            t("overview:holdings.limitAll")
        } else {
            l.to_string()
        }
    };
    let limits = hm::LIMITS
        .into_iter()
        .map(|l| {
            radio(String::new(), limit_label(l), l, v.limit, |h, x| {
                h.limit = x
            })
        })
        .collect();
    entries.push(Entry::Sub {
        name: "limit",
        label: t("overview:holdings.limitLabel"),
        value: limit_label(v.limit),
        width: tk.r(8.0),
        entries: limits,
    });
    entries.push(Entry::Sep);
    entries.push(Entry::Item {
        key: "ov-menu-export-wealth".into(),
        label: t("overview:holdings.exportWealth"),
        on: Rc::new(crate::reports::report::open_wealth),
    });
    entries
}

/// Rows read down the first column, then the next (`TwoColumnList`), once `two` holds.
pub(super) fn two_columns(tk: &Tk, rows: Vec<El>, two: bool) -> El {
    let gap_y = tk.r(0.125);
    if two && rows.len() > 1 {
        let per = rows.len().div_ceil(2);
        let mut first = col().grow(1.0).w(0.0).min_w(0.0).gap(gap_y);
        let mut second = col().grow(1.0).w(0.0).min_w(0.0).gap(gap_y);
        for (i, r) in rows.into_iter().enumerate() {
            if i < per {
                first = first.child(r);
            } else {
                second = second.child(r);
            }
        }
        row()
            .w_full()
            .gap(tk.r(2.0))
            .items_start()
            .role("list")
            .child(first)
            .child(second)
    } else {
        col().w_full().gap(gap_y).role("list").children(rows)
    }
}
