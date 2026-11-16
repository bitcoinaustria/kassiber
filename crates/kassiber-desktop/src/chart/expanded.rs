//! The full-window chart dialog and its PortfolioInspector header (PortfolioInspector.tsx).

use rune::{Cx, El, col, row, text};

use super::{Geo, View, card};
use crate::app::App;
use crate::format;
use crate::i18n::t;
use crate::model::Snapshot;
use crate::theme::Tk;
use crate::ui::sensitive;

/// The dialog: the same card at full window size with its own brush and hover. It starts under
/// the window's title bar (the traffic lights on macOS), not under the window's top edge.
pub fn dialog(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    s: &Snapshot,
    has_data: bool,
    layer: &mut Vec<El>,
) -> El {
    let body = card(app, cx, tk, s, View::Expanded, has_data, layer);
    col()
        .key("chart-dialog")
        .pin(
            Some(0.0),
            Some(crate::shell::window_top_inset()),
            Some(0.0),
            Some(0.0),
        )
        .fill(tk.card)
        .role("dialog")
        .label(t("overview:treasury.expandedTitle"))
        .modal(false, |a: &mut App| a.chart.expanded = false)
        .child(body)
}

fn cell(tk: &Tk, label: &str, value: El) -> El {
    col()
        .min_w(92.0)
        .px(8.0)
        .py(4.0)
        .gap(1.0)
        .radius(tk.r(0.25))
        .fill(tk.muted.alpha(0.2))
        .child(
            text(label.to_owned())
                .font(tk.t3xs(500.0))
                .color(tk.muted_fg),
        )
        .child(value)
}

/// The position at the hovered (else the last) column of the selection.
pub fn inspector(app: &App, tk: &Tk, s: &Snapshot, g: &Geo) -> El {
    let hide = app.prefs.hide_sensitive();
    let cur = s.fiat_currency();
    let p = app
        .chart
        .inspect_col
        .and_then(|i| g.plot.get(i))
        .or_else(|| {
            g.plot
                .iter()
                .rev()
                .find(|p| !p.is_event())
                .or(g.plot.last())
        });
    let v = tk.txs(600.0).features("'tnum' 1");
    let mut a = row()
        .key("portfolio-inspector")
        .gap(6.0)
        .center_y()
        .px(8.0)
        .py(6.0)
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .fill(tk.background.alpha(0.85))
        .label(t("overview:inspector.position"));
    let date = p
        .map(|p| p.detail.clone())
        .unwrap_or_else(|| t("overview:inspector.noDateSelected"));
    a = a.child(
        col()
            .min_w(96.0)
            .gap(1.0)
            .child(
                text(t("overview:inspector.position").to_uppercase())
                    .font(tk.t3xs(600.0).tracking(0.4))
                    .color(tk.muted_fg),
            )
            .child(text(date).font(tk.txs(500.0)).color(tk.fg)),
    );
    let Some(p) = p else { return a };
    let dash = || "—".to_owned();
    let basis = if p.basis_incomplete {
        dash()
    } else {
        format::fiat(p.cost_basis, &cur)
    };
    let (unreal, tone) = if p.basis_incomplete {
        (dash(), tk.fg)
    } else {
        (
            format::signed_fiat(p.unrealized, &cur),
            if p.unrealized >= 0.0 {
                tk.good.solid
            } else {
                tk.kb_accent
            },
        )
    };
    let hint = t("overview:completeness.hint.incomplete");
    let mut basis_cell = cell(
        tk,
        &t("overview:inspector.costBasis"),
        sensitive(basis, v, tk.fg, hide),
    );
    let mut unreal_cell = cell(
        tk,
        &t("overview:inspector.unrealized"),
        sensitive(unreal, v, tone, hide),
    );
    if p.basis_incomplete {
        basis_cell = basis_cell.help(hint.clone());
        unreal_cell = unreal_cell.help(hint);
    }
    a.child(cell(
        tk,
        &t("overview:inspector.value"),
        sensitive(format::fiat(p.value_eur, &cur), v, tk.fg, hide),
    ))
    .child(cell(
        tk,
        &t("overview:inspector.btc"),
        sensitive(format::btc(p.balance, 8, false), v, tk.fg, hide),
    ))
    .child(basis_cell)
    .child(unreal_cell)
}
