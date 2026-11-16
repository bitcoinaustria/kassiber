//! The widget gallery: a glass sheet listing every widget, with a live preview of the chosen
//! one at the chosen size (the real widget on the book's data, scaled, inert) and the sizes it
//! comes in. "Add widget" places it at the end; for a placed widget it changes the size.

use rune::{Anim, Cx, El, LiveRegion, Tx, col, el, row, text};

use super::model::{Size, WidgetId};
use super::{Geo, copy, icon, tile, title};
use crate::app::App;
use crate::icons as I;
use crate::model::Snapshot;
use crate::settings::controls;
use crate::theme::Tk;
use crate::ui::ic;

pub struct Gallery {
    pub sel: WidgetId,
    pub size: Size,
}

/// Opens the gallery on the first widget that is not placed (or the first).
pub fn open(a: &mut App) {
    let sel =
        a.ov.layout
            .unplaced()
            .first()
            .copied()
            .unwrap_or(WidgetId::Stats);
    select(a, sel);
}

fn select(a: &mut App, sel: WidgetId) {
    let size =
        a.ov.layout
            .size_of(sel)
            .unwrap_or_else(|| sel.default_size());
    a.ov.edit.gallery = Some(Gallery { sel, size });
}

fn close(a: &mut App) {
    a.ov.edit.gallery = None;
}

fn add(a: &mut App, tx: &mut Tx) {
    let Some(g) = a.ov.edit.gallery.take() else {
        return;
    };
    let name = title(a, g.sel);
    let placed = a.ov.layout.contains(g.sel);
    if a.ov.layout.add(g.sel, g.size) {
        tx.animated();
        tx.announce(
            if placed {
                copy::sized(&name, g.size)
            } else {
                copy::added(&name, g.size)
            },
            LiveRegion::Polite,
        );
    }
}

pub fn mount(app: &App, cx: &mut Cx<App>, tk: &Tk, s: Option<&Snapshot>, layer: &mut Vec<El>) {
    let Some(g) = &app.ov.edit.gallery else {
        return;
    };
    let win_w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    let width = tk.r(60.0).min(win_w - tk.r(4.0));
    let pad = tk.r(1.25);
    let list_w = tk.r(17.0);
    let box_w = width - 2.0 * pad - list_w - tk.r(1.0);
    let box_h = (win_h - tk.r(27.0)).clamp(tk.r(13.0), tk.r(24.0));

    // The widgets.
    let mut list = col()
        .key("ov-gal-list")
        .w(list_w)
        .gap(tk.r(0.125))
        .role("radiogroup")
        .label(copy::gallery_title());
    for id in WidgetId::ALL {
        let on = g.sel == id;
        let name = title(app, id);
        let mut head = row().w_full().min_w(0.0).gap(tk.r(0.375)).center_y().child(
            text(name.clone())
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .truncate()
                .min_w(0.0),
        );
        if app.ov.layout.contains(id) {
            head = head.child(
                row()
                    .h(tk.r(1.0))
                    .px(tk.r(0.375))
                    .center_y()
                    .radius(tk.r(0.5))
                    .fill(tk.fg.alpha(0.08))
                    .child(
                        text(copy::on_home())
                            .font(tk.t3xs(600.0))
                            .color(tk.muted_fg),
                    ),
            );
        }
        list = list.child(
            row()
                .key(format!("ov-gal-{}", id.id()))
                .w_full()
                .px(tk.r(0.5))
                .py(tk.r(0.5))
                .gap(tk.r(0.625))
                .center_y()
                .radius(tk.r(0.75))
                .continuous_corners()
                .fill(if on {
                    tk.fg.alpha(0.08)
                } else {
                    rune::Color::TRANSPARENT
                })
                .hover(tk.fg.alpha(if on { 0.08 } else { 0.04 }))
                .role("radio")
                .checked(on)
                .label(name)
                .description(copy::description(id))
                .focusable(true)
                .focus_ring(2.0, 0.0, tk.ring)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |a: &mut App| select(a, id))
                .child(
                    row()
                        .size(tk.r(2.0), tk.r(2.0))
                        .center()
                        .radius(tk.r(0.5))
                        .continuous_corners()
                        .fill(tk.fg.alpha(0.07))
                        .child(ic(icon(id), tk.r(1.0), tk.fg)),
                )
                .child(
                    col().grow(1.0).min_w(0.0).gap(1.0).child(head).child(
                        text(copy::description(id))
                            .font(tk.t2xs(400.0))
                            .color(tk.muted_fg)
                            .wrap(),
                    ),
                ),
        );
    }

    // The preview: the widget as it would sit on the wide grid, scaled into the box.
    let geo = Geo::new(tk, 1144.0);
    let (sw, sh) = g.size.span();
    let (w, h) = geo.span(sw, sh);
    let k = ((box_w - tk.r(2.0)) / w)
        .min((box_h - tk.r(2.0)) / h)
        .min(1.0);
    let mut inner = Vec::new();
    let live = tile(app, cx, tk, s, g.sel, g.size, w, h, true, &mut inner)
        .absolute((w * k - w) / 2.0, (h * k - h) / 2.0)
        .scale(k)
        .scale_motion(Anim::INSTANT)
        .inert()
        .a11y_hidden();
    let preview = col()
        .key("ov-gal-preview")
        .w(box_w)
        .h(box_h)
        .center()
        .radius(tk.r(1.0))
        .continuous_corners()
        .fill(if tk.dark {
            rune::Color::BLACK.alpha(0.28)
        } else {
            tk.fg.alpha(0.06)
        })
        .role("img")
        .label(copy::preview_of(&title(app, g.sel), g.size))
        .child(el().size(w * k, h * k).child(live));

    let mut sizes = row()
        .key("ov-gal-sizes")
        .gap(tk.r(0.375))
        .center_y()
        .role("radiogroup")
        .label(copy::size_label());
    for z in g.sel.sizes() {
        let z = *z;
        let on = g.size == z;
        sizes = sizes.child(
            row()
                .key(format!("ov-gal-size-{}", z.id()))
                .h(tk.r(1.75))
                .px(tk.r(0.75))
                .center()
                .radius(tk.r(0.875))
                .fill(if on { tk.fg } else { tk.fg.alpha(0.07) })
                .hover(if on { tk.fg } else { tk.fg.alpha(0.12) })
                .role("radio")
                .checked(on)
                .label(copy::size(z))
                .focusable(true)
                .focus_ring(2.0, 1.0, tk.ring)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |a: &mut App| {
                    if let Some(g) = a.ov.edit.gallery.as_mut() {
                        g.size = z;
                    }
                })
                .child(text(copy::size(z)).font(tk.txs(600.0)).color(if on {
                    tk.background
                } else {
                    tk.fg
                })),
        );
    }
    let placed = app.ov.layout.size_of(g.sel);
    let action = match placed {
        Some(z) if z == g.size => {
            crate::ui::primary_button(tk, "ov-gal-add", Some(&I::CHECK), &copy::on_home())
                .disabled(true)
                .opacity(0.5)
        }
        Some(_) => {
            crate::ui::primary_button(tk, "ov-gal-add", None, &copy::change_size()).on_press_tx(add)
        }
        None => crate::ui::primary_button(tk, "ov-gal-add", Some(&I::PLUS), &copy::add_widget())
            .on_press_tx(add),
    };
    let right = col().w(box_w).gap(tk.r(0.75)).child(preview).child(
        row()
            .w_full()
            .justify_between()
            .center_y()
            .gap(tk.r(0.75))
            .child(sizes)
            .child(action),
    );
    let content = col()
        .w_full()
        .gap(tk.r(1.0))
        .child(
            col()
                .gap(tk.r(0.375))
                .child(controls::dialog_title(tk, copy::gallery_title()))
                .child(
                    text(copy::gallery_hint())
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(tk.r(1.0))
                .items_start()
                .child(list)
                .child(right),
        );
    controls::dialog(app, tk, layer, "ov-gallery", width, content, close, true);
    layer.extend(inner);
}
