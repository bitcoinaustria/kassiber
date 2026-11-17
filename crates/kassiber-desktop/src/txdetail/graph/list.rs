//! Inputs and outputs below the drawing (TransactionInputsOutputsPanel): two columns of legs
//! that follow the money inside the book (funding / spending transaction) or open an explorer,
//! light up with their leg in the drawing, fold past eight rows, and end in totals.

use std::rc::Rc;

use rune::{Color, Cx, El, ScrollSpec, Tx, col, el, row, text};

use super::data::{Node, Payload};
use super::model::{self, Side};
use super::{Ctx, View};
use crate::app::{App, Currency};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::ui::ic;

const COLLAPSED_ROWS: usize = 8;

pub fn row_key(v: View, part: &str) -> String {
    format!("graph-row-{}-{part}", v as usize)
}

fn toggle_currency(a: &mut App) {
    a.prefs.currency = if a.prefs.currency == Currency::Btc {
        Currency::Fiat
    } else {
        Currency::Btc
    };
}

/// The amount, which switches the app between bitcoin and fiat where the leg is priced.
fn amount(c: &Ctx, text_value: String, priced: bool, key: String) -> El {
    let tk = c.tk;
    let mut e = crate::ui::sensitive(
        text_value,
        tk.txs(500.0).features("'tnum' 1"),
        tk.fg,
        c.hide,
    );
    if priced && c.priced && !c.hide {
        let title = if c.app.prefs.currency == Currency::Btc {
            t("chrome:currencyToggleText.showFiat")
        } else {
            t("chrome:currencyToggleText.showBitcoin")
        };
        e = row()
            .key(key)
            .role("button")
            .label(title.clone())
            .help(title)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .on_press(toggle_currency)
            .child(e);
    }
    e
}

fn marker(c: &Ctx, side: Side) -> El {
    let tk = c.tk;
    let (border, fill, glyph) = match side {
        Side::Input => (tk.alert.border, tk.alert.bg, tk.alert.solid),
        _ => (tk.good.border, tk.good.bg, tk.good.solid),
    };
    // `mt-0.5 size-5 rounded-full`, the arrow `size-3`: rem, so they follow the UI scale.
    row()
        .size(tk.r(1.25), tk.r(1.25))
        .mt(tk.r(0.125))
        .center()
        .radius(tk.r(0.625))
        .border(1.0, border)
        .fill(fill)
        .role("img")
        .label(if side == Side::Input {
            t("transactions:graph.inputsOutputs.spentInput")
        } else {
            t("transactions:graph.inputsOutputs.createdOutput")
        })
        .child(ic(&I::ARROW_RIGHT, tk.r(0.75), glyph))
}

fn io_row(c: &Ctx, p: &Payload, n: &Node, side: Side, v: View) -> El {
    let tk = c.tk;
    let part = model::part(side, &n.id);
    let active = c.app.graph.active.as_deref() == Some(part.as_str());
    let explorer = if c.hide || n.overflow {
        None
    } else {
        super::sources::explorer_for_node(c.app, p, n)
    };
    let book = if c.hide {
        None
    } else if side == Side::Input {
        n.funded_by_transaction_id.clone()
    } else {
        n.spent_by_transaction_id.clone()
    };
    type Target = (String, Rc<dyn Fn(&mut App, &mut Tx)>);
    let target: Option<Target> = match (book, explorer) {
        (Some(id), _) => {
            let label = if side == Side::Input {
                tr(
                    "transactions:graph.inputsOutputs.openFundingTransaction",
                    &[("reference", &model::short(n.txid.as_deref().unwrap_or("")))],
                )
            } else {
                tr(
                    "transactions:graph.inputsOutputs.openSpendingTransaction",
                    &[(
                        "reference",
                        &model::short(n.spent_by_txid.as_deref().unwrap_or("")),
                    )],
                )
            };
            Some((
                label,
                Rc::new(move |a: &mut App, _tx: &mut Tx| crate::txdetail::follow(a, &id))
                    as Rc<dyn Fn(&mut App, &mut Tx)>,
            ))
        }
        (None, Some(e)) => {
            let label = tr(
                "transactions:graph.inputsOutputs.openExplorer",
                &[
                    ("explorer", &e.label),
                    ("reference", &model::detail_reference(n, false)),
                ],
            );
            let url = e.url.clone();
            Some((
                label,
                Rc::new(move |_a: &mut App, tx: &mut Tx| {
                    let _ = tx.open_url(&url);
                }) as Rc<dyn Fn(&mut App, &mut Tx)>,
            ))
        }
        (None, None) => None,
    };
    let amount_text = {
        let a = model::amount(n, c.hide, c.fiat);
        if a.is_empty() {
            t("transactions:graph.inputsOutputs.unknownAmount")
        } else {
            a
        }
    };
    let meta = model::detail_meta(
        n,
        side,
        c.hide,
        p.transaction.as_ref().and_then(|t| t.block_height),
    );
    let reference = crate::ui::sensitive(
        model::detail_reference(n, c.hide),
        tk.txs(500.0).mono(),
        tk.fg,
        c.hide,
    )
    .truncate();
    let mut meta_row = row().w_full().flex_wrap().gaps(2.0, tk.r(0.5));
    for (i, m) in meta.into_iter().enumerate() {
        let narrow = c.app.shell.window_w > 0.0 && c.app.shell.window_w < 640.0;
        if i > 3 && narrow {
            continue;
        }
        // A CSS flex item shrinks to its longest word and wraps ("spent by" / its txid in a
        // narrow column); a Rune one keeps its one-line width unless it may shrink.
        meta_row = meta_row.child(
            text(m)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap()
                .shrink(),
        );
    }
    let (hover_part, leave_part) = (part.clone(), part.clone());
    let mut r = row()
        .key(row_key(v, &part))
        .w_full()
        .gap(tk.r(0.5))
        .py(tk.r(0.5))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .fill(if active {
            tk.muted.alpha(0.45)
        } else {
            Color::TRANSPARENT
        })
        .on_hover_change(move |s: &mut App, on, _tx: &mut Tx| {
            if on {
                s.graph.active = Some(hover_part.clone());
            } else if s.graph.active.as_deref() == Some(leave_part.as_str()) {
                s.graph.active = None;
            }
        })
        .child(marker(c, side))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(reference)
                .child(meta_row),
        )
        .child(amount(
            c,
            amount_text,
            !c.hide && model::has_known_btc(n),
            format!("graph-amt-{}-{part}", v as usize),
        ));
    if let Some((label, open)) = target {
        r = r
            .role("button")
            .label(label.clone())
            .help(label)
            .focusable(true)
            .hover(tk.muted.alpha(0.35))
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |s: &mut App, tx: &mut Tx| open(s, tx));
    } else {
        r = r.role("listitem").label(model::detail_reference(n, c.hide));
    }
    r
}

fn column(c: &Ctx, cx: &mut Cx<App>, p: &Payload, nodes: &[Node], side: Side, v: View) -> El {
    let tk = c.tk;
    let i = if side == Side::Input { 0 } else { 1 };
    let open = c.app.graph.view(v).open[i];
    let shown = if open {
        nodes.len()
    } else {
        nodes.len().min(COLLAPSED_ROWS)
    };
    let title = if side == Side::Input {
        t("transactions:graph.inputsOutputs.inputs")
    } else {
        t("transactions:graph.inputsOutputs.outputs")
    };
    let rows_key = format!("graph-rows-{}-{i}", v as usize);
    let mut rows = col().key(rows_key.clone()).w_full();
    for n in &nodes[..shown] {
        rows = rows.child(io_row(c, p, n, side, v));
    }
    if nodes.len() > COLLAPSED_ROWS {
        let label = if open {
            t("transactions:graph.inputsOutputs.showFewer")
        } else {
            tc(
                "transactions:graph.inputsOutputs.showAll",
                nodes.len() - shown,
                &[],
            )
        };
        rows = rows.child(
            row()
                .key(format!("graph-more-{}-{i}", v as usize))
                .w_full()
                .gap(tk.r(0.25))
                .py(tk.r(0.5))
                .center_y()
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .role("button")
                .label(label.clone())
                .focusable(true)
                .on_press(move |a: &mut App| {
                    let st = a.graph.view_mut(v);
                    st.open[i] = !st.open[i];
                })
                .child(ic(
                    if open {
                        &I::CHEVRON_UP
                    } else {
                        &I::CHEVRON_DOWN
                    },
                    tk.r(0.875),
                    tk.muted_fg,
                ))
                .child(text(label).font(tk.txs(400.0)).color(tk.muted_fg)),
        );
    }
    let max_h = if v == View::Expanded { 520.0 } else { 360.0 };
    // `overflow-auto pr-1`: the legs keep a 0.25 rem gutter on the right. A Rune scroll box
    // takes the wheel even when its content fits (it does not hand the scroll on to the panel),
    // so the column only scrolls once its rows are taller than the cap.
    let tall = cx
        .layout_size(&rows_key)
        .is_some_and(|[_, h]| h > max_h + 0.5);
    let body = if tall {
        col()
            .w_full()
            .max_h(max_h)
            .pr(tk.r(0.25))
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(rows)
    } else {
        col().w_full().pr(tk.r(0.25)).child(rows)
    };
    col()
        .key(format!("graph-col-{}-{i}", v as usize))
        .grow(1.0)
        .min_w(0.0)
        .role("list")
        .label(title.clone())
        .child(
            row()
                .w_full()
                .pb(tk.r(0.375))
                .justify_between()
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .child(
                    text(title.to_uppercase())
                        .font(tk.t2xs(600.0).tracking(0.4))
                        .color(tk.muted_fg),
                )
                .child(
                    text(crate::format::count(nodes.len()))
                        .font(tk.t2xs(400.0))
                        .color(tk.muted_fg),
                ),
        )
        .child(body)
}

fn totals(c: &Ctx, p: &Payload) -> El {
    let tk = c.tk;
    let mut r = row()
        .key(format!("graph-totals{}", c.sfx))
        .w_full()
        .gap(tk.r(1.0))
        .mt(tk.r(0.5))
        .pt(tk.r(0.5))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
    for (side, nodes) in [("in", &p.inputs), ("out", &p.outputs)] {
        let caption = if model::complete_total(nodes) {
            t("transactions:graph.inputsOutputs.total")
        } else {
            t("transactions:graph.inputsOutputs.knownTotal")
        };
        let value = if c.hide {
            t("transactions:graph.hidden")
        } else {
            model::format_total(nodes, c.fiat)
        };
        let priced = model::summary(nodes).known > 0;
        r = r.child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .justify_between()
                .center_y()
                .child(text(caption).font(tk.txs(400.0)).color(tk.muted_fg))
                .child(amount(
                    c,
                    value,
                    priced,
                    format!("graph-total-{side}{}", c.sfx),
                )),
        );
    }
    r
}

pub fn panel(c: &Ctx, cx: &mut Cx<App>, p: &Payload, v: View) -> Option<El> {
    if p.inputs.is_empty() && p.outputs.is_empty() {
        return None;
    }
    let tk = c.tk;
    let key = format!("graph-io-{}", v as usize);
    // `md:grid-cols-2`: a viewport breakpoint (48rem in a media query is 768 px, the browser's
    // 16 px rem, not the app's root size), so the columns sit side by side in the narrow detail
    // panel too whenever the window is at least that wide.
    let wide = c.app.shell.window_w == 0.0 || c.app.shell.window_w >= 768.0;
    let cols = if wide {
        row()
            .w_full()
            .gap(tk.r(1.0))
            .items_start()
            .child(column(c, cx, p, &p.inputs, Side::Input, v))
            .child(column(c, cx, p, &p.outputs, Side::Output, v))
    } else {
        col()
            .w_full()
            .gap(tk.r(1.0))
            .child(column(c, cx, p, &p.inputs, Side::Input, v))
            .child(column(c, cx, p, &p.outputs, Side::Output, v))
    };
    Some(
        col()
            .key(key)
            .w_full()
            .pt(tk.r(0.75))
            .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
            .role("region")
            .label(t("transactions:graph.inputsOutputs.title"))
            .child(cols)
            .child(totals(c, p))
            .child(el().size(0.0, 0.0)),
    )
}
