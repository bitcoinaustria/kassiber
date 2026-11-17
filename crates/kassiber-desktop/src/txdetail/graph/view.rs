//! The interactive drawings: the glass view (seen from one fixed angle; hover to light a leg and
//! show its card, click an input or output to bring its row into view) and the flat bowtie
//! (hover, click or Enter to copy a leg's reference), plus the legend, the hover card and the
//! 2D/3D switch.

use std::rc::Rc;

use rune::{Color, Cursor, Cx, El, Key, Phase, Tx, canvas, col, el, row, text};

use super::data::Payload;
use super::glass::{self, Cam, Draw, REST};
use super::model::{self, Row};
use super::{Ctx, View, flat, gpu};
use crate::app::App;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::{MASK, Tk};

/// A click on a leg in the drawing: its list row opens (past "show all") and scrolls into view.
pub(crate) fn reveal(app: &mut App, v: View, part: &str, tx: &mut Tx) {
    let side = if part.starts_with("input:") { 0 } else { 1 };
    app.graph.view_mut(v).open[side] = true;
    app.graph.active = Some(part.to_owned());
    tx.scroll_into_view(super::list::row_key(v, part));
}

fn canvas_width(cx: &mut Cx<App>, key: &str, fallback: f32) -> f32 {
    cx.layout_size(key)
        .map(|[w, _]| w)
        .filter(|w| *w > 0.0)
        .unwrap_or(fallback)
}

pub fn glass_view(c: &Ctx, cx: &mut Cx<App>, p: &Rc<Payload>, v: View) -> El {
    let tk = c.tk;
    let app = c.app;
    let key = format!("graph-3d-{}", v as usize);
    let w = canvas_width(cx, &key, if v == View::Expanded { 1120.0 } else { 900.0 });
    let h = match v {
        View::Compact => {
            if app.shell.window_w >= 1280.0 {
                440.0
            } else {
                380.0
            }
        }
        // Graph focus: the body's height less the legend and a peek of the inputs/outputs below.
        View::Expanded => match cx
            .layout_size(super::FOCUS_SCROLL)
            .map(|[_, h]| h)
            .filter(|h| *h > 0.0)
        {
            Some(body) => {
                let pad = if app.shell.window_w <= 0.0 || app.shell.window_w >= 640.0 {
                    tk.r(1.5)
                } else {
                    tk.r(1.0)
                };
                (body - 2.0 * pad - tk.r(1.25) - tk.r(0.5) - tk.r(4.5)).max(320.0)
            }
            None => (0.64 * app.shell.window_h.max(600.0)).min(560.0),
        },
    };
    let scene = glass::layout_cached(p, c.hide);
    let vs = app.graph.view(v);
    let cam = Cam::new(scene.frame(), REST.0, REST.1, w, h);
    // A row lit in the list whose leg is folded lights its side's "+N more" strand.
    let lit = vs
        .hover
        .clone()
        .or_else(|| app.graph.active.as_deref().map(|p| scene.drawn_part(p)));
    // The GPU renders the scene through the compositor at an effect slot; without it, the 2.5D
    // drawing (the web's 2D fallback stands in when WebGL fails).
    let gpu_on = gpu::available();
    // The web paints the scene in its nearest painted ancestor's colour: the card inline, the
    // panel's background in graph focus.
    let surface = if v == View::Expanded {
        tk.background
    } else {
        tk.card
    };
    let mark = gpu::MARK_BASE + v as u32;
    // The scroll viewports the drawing sits in (the 3D view is cut to them; see `Job::clip_keys`).
    let clip_keys: &'static [&'static str] = match v {
        View::Expanded if app.route.path() == "/dev/graph" => &[super::FOCUS_SCROLL, "app-main"],
        View::Expanded => &[super::FOCUS_SCROLL],
        View::Compact if app.route.path() == "/dev/graph" => &["app-main"],
        View::Compact => &["txd-scroll"],
    };
    let (shape, picks) = if gpu_on {
        let half = (w as f64 / (2.0 * cam.ppu), h as f64 / (2.0 * cam.ppu));
        app.graph.gpu_jobs.borrow_mut().insert(
            mark,
            gpu::Job {
                key: key.clone(),
                meshes: gpu::meshes_for(&scene),
                yaw: REST.0,
                pitch: REST.1,
                half,
                highlight: lit.clone(),
                dark: tk.dark,
                bg: surface,
                clip_keys,
            },
        );
        (None, glass::picks(&scene, cam))
    } else {
        app.graph.gpu_jobs.borrow_mut().remove(&mark);
        let (s, p) = Draw {
            scene: &scene,
            cam,
            dark: tk.dark,
            highlight: lit.as_deref(),
        }
        .render();
        (Some(s), p)
    };
    let picks = Rc::new(picks);
    let hovered_row: Option<Row> = vs
        .hover
        .as_ref()
        .and_then(|part| scene.rows.get(part).cloned());

    let (pm, ps) = (picks.clone(), scene.clone());
    let mut drawing = el()
        .key(key)
        .w_full()
        .h(h)
        .radius(tk.r(0.375))
        .fill(surface)
        .clip_rounded(tk.r(0.375))
        .role("img")
        .label(t("transactions:graph.view3dAria"))
        .cursor(Cursor::Default)
        .on_pointer_move_local(move |s: &mut App, e, tx: &mut Tx| {
            let part = pm.pick([e.x, e.y]);
            let st = s.graph.view_mut(v);
            if st.hover == part {
                tx.live_only();
                return;
            }
            st.hover = part.clone();
            s.graph.active = part;
        })
        .on_hover_change(move |s: &mut App, on, _tx: &mut Tx| {
            if !on {
                s.graph.view_mut(v).hover = None;
                s.graph.active = None;
            }
        })
        // A click picks where it lands (the web's `onClick`): only an input or output leg with a
        // row of its own is selected; the fee and a "+N more" strand have none.
        .on_drag(move |s: &mut App, d: &mut rune::Drag, tx: &mut Tx| {
            if d.phase == Phase::Up
                && let Some(part) = picks.pick([d.x, d.y]).filter(|part| ps.selectable(part))
            {
                reveal(s, v, &part, tx);
            }
        })
        .child(match shape {
            Some(shape) => canvas(shape).absolute(0.0, 0.0).size(w, h).inert(),
            None => rune::effect(mark).absolute(0.0, 0.0).size(w, h).inert(),
        });
    if let Some(r) = hovered_row {
        drawing = drawing.child(leg_card(c, &r, w));
    }
    col()
        .w_full()
        .gap(tk.r(0.5))
        .child(drawing)
        .child(legend(c, &scene.legend()))
}

pub fn flat_view(c: &Ctx, cx: &mut Cx<App>, p: &Rc<Payload>, v: View) -> El {
    let app = c.app;
    let key = format!("graph-2d-{}", v as usize);
    let w = canvas_width(cx, &key, if v == View::Expanded { 1120.0 } else { 960.0 });
    let f = Rc::new(flat::layout(
        p,
        c.hide,
        v == View::Expanded,
        w as f64,
        app.shell.window_h.max(600.0) as f64,
    ));
    let vs = app.graph.view(v);
    let lit = vs.hover.clone().or_else(|| app.graph.active.clone());
    let shape = flat::render(&f, lit.as_deref());
    let hovered_row = vs.hover.as_ref().and_then(|part| {
        f.strands
            .iter()
            .find(|s| &s.part == part)
            .map(|s| s.row.clone())
    });
    let (fm, fc, fk) = (f.clone(), f.clone(), f.clone());
    let hide = c.hide;
    let copy = move |s: &mut App, part: &str, tx: &mut Tx| {
        if hide {
            return;
        }
        let Some(strand) = fc.strands.iter().find(|x| x.part == part) else {
            return;
        };
        if let Some(r) = model::copy_reference(&strand.row.node) {
            crate::settings::effects::copy(s, tx, r);
            let _ = s;
        }
    };
    let copy2 = copy.clone();
    let focus_label = vs
        .focus
        .and_then(|i| f.strands.get(i))
        .map(|s| strand_label(&s.row, hide));
    let inner = el()
        .key(format!("graph-2d-inner-{}", v as usize))
        .size(f.width as f32, f.height as f32)
        .label(t("transactions:graph.diagramAria"))
        .on_pointer_move_local(move |s: &mut App, e, tx: &mut Tx| {
            let part = flat::pick(&fm, [e.x, e.y]);
            let st = s.graph.view_mut(v);
            if st.hover == part {
                tx.live_only();
                return;
            }
            st.hover = part.clone();
            s.graph.active = part;
        })
        .on_hover_change(move |s: &mut App, on, _tx: &mut Tx| {
            if !on {
                s.graph.view_mut(v).hover = None;
                s.graph.active = None;
            }
        })
        .on_press_tx(move |s: &mut App, tx: &mut Tx| {
            if let Some(part) = s.graph.view(v).hover.clone() {
                copy(s, &part, tx);
            }
        })
        .child(
            canvas(shape)
                .absolute(0.0, 0.0)
                .size(f.width as f32, f.height as f32)
                .inert(),
        );
    let mut viewport = el()
        .key(key)
        .w_full()
        .h(f.viewport as f32)
        .role("img")
        .label(focus_label.unwrap_or_else(|| t("transactions:graph.diagramAria")))
        .focusable(true)
        .cursor(Cursor::Pointer)
        .on_key(move |s: &mut App, ev, tx: &mut Tx| {
            let n = fk.strands.len();
            if n == 0 {
                return false;
            }
            let cur = s.graph.view(v).focus;
            let next = match ev.key {
                Key::Down | Key::Right => cur.map_or(0, |i| (i + 1) % n),
                Key::Up | Key::Left => cur.map_or(n - 1, |i| (i + n - 1) % n),
                Key::Enter => {
                    if let Some(i) = cur {
                        let part = fk.strands[i].part.clone();
                        copy2(s, &part, tx);
                    }
                    return true;
                }
                Key::Text(ref t) if t == " " => {
                    if let Some(i) = cur {
                        let part = fk.strands[i].part.clone();
                        copy2(s, &part, tx);
                    }
                    return true;
                }
                Key::Escape => {
                    s.graph.view_mut(v).focus = None;
                    s.graph.active = None;
                    return false;
                }
                _ => return false,
            };
            s.graph.view_mut(v).focus = Some(next);
            s.graph.active = Some(fk.strands[next].part.clone());
            true
        })
        .child(
            col()
                .w_full()
                .h_full()
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(inner),
        );
    if let Some(r) = hovered_row {
        viewport = viewport.child(leg_card(c, &r, w));
    }
    viewport
}

/// "Copy input outpoint" etc., or "{{role}} graph leg" without a reference.
fn strand_label(r: &Row, hide: bool) -> String {
    if hide || model::copy_reference(&r.node).is_none() {
        return tr(
            "transactions:graph.legAria",
            &[("role", &model::role_label(r.node.role.as_deref()))],
        );
    }
    match r.side {
        model::Side::Input => t("transactions:graph.copyInput"),
        model::Side::Output => t("transactions:graph.copyOutput"),
        model::Side::Fee => t("transactions:graph.copyFee"),
    }
}

/// `GraphLegCard`: the dark card at the drawing's bottom left (the same in both themes).
fn leg_card(c: &Ctx, r: &Row, w: f32) -> El {
    let tk = c.tk;
    let n = &r.node;
    let hidden_label = t("transactions:graph.hidden");
    let title = if c.hide {
        hidden_label.clone()
    } else {
        model::title(n)
    };
    let mut parts = vec![];
    let amount = model::amount(n, c.hide, c.fiat);
    if !amount.is_empty() {
        parts.push(amount);
    }
    parts.push(model::role_label(n.role.as_deref()));
    parts.push(model::ownership_label(n));
    let reference = model::copy_reference(n).map(|r| {
        if c.hide {
            hidden_label.clone()
        } else {
            model::short(&r)
        }
    });
    let card_bg = rune::color::hex("#101114").expect("colour").alpha(0.95);
    let mut line2 = row().flex_wrap().gap(tk.r(0.25)).child(
        text(parts.join(" · "))
            .font(tk.txs(400.0))
            .color(Color::WHITE.alpha(0.7)),
    );
    if let Some(r) = reference {
        line2 = line2
            .child(text("·").font(tk.txs(400.0)).color(Color::WHITE.alpha(0.7)))
            .child(
                text(r)
                    .font(tk.txs(400.0).mono())
                    .color(Color::WHITE.alpha(0.7))
                    .blur(if c.hide { 3.5 } else { 0.0 }),
            );
    }
    col()
        .key("graph-leg-card")
        .pin(Some(12.0), None, None, Some(8.0))
        .max_w((w - tk.r(1.5)).min(520.0))
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .gap(2.0)
        .radius(tk.r(0.375))
        .fill(card_bg)
        .border(1.0, Color::WHITE.alpha(0.1))
        .inert()
        .paint_order(20)
        .child(
            text(title)
                .font(tk.txs(500.0))
                .color(Color::WHITE)
                .truncate()
                .blur(if c.hide { 3.5 } else { 0.0 })
                .label(if c.hide {
                    MASK.to_owned()
                } else {
                    model::title(n)
                }),
        )
        .child(line2)
}

/// The swatches a drawing uses, then the reading note behind an info hint.
fn legend(c: &Ctx, keys: &[&'static str]) -> El {
    let tk = c.tk;
    let swatch = |k: &str| -> (Color, Option<Color>) {
        let h = |s: &str| rune::color::hex(s).expect("colour");
        match k {
            "known" => (h("#8fb4ff"), Some(Color::WHITE.alpha(0.4))),
            "unknown" => (
                if tk.dark {
                    h("#94a3b8").alpha(0.6)
                } else {
                    h("#cbd5e1").alpha(0.8)
                },
                Some(Color::WHITE.alpha(0.6)),
            ),
            "own" => (h("#2563eb"), None),
            "other" => (if tk.dark { h("#64748b") } else { h("#7b8798") }, None),
            _ => (h("#f5b544"), None),
        }
    };
    let mut r = row()
        .key(format!("graph-legend{}", c.sfx))
        .flex_wrap()
        .gap(tk.r(1.0))
        .center_y();
    for k in keys {
        let (fill, ring) = swatch(k);
        let mut sw = el().size(10.0, 10.0).radius(2.0).fill(fill);
        if let Some(ring) = ring {
            sw = sw.border(1.0, ring);
        }
        r = r.child(
            row().gap(tk.r(0.375)).center_y().child(sw).child(
                text(t(&format!("transactions:graph.legend.{k}")))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            ),
        );
    }
    r.child(
        row()
            .key(format!("graph-legend-hint{}", c.sfx))
            .size(tk.r(1.25), tk.r(1.25))
            .center()
            .radius(tk.r(0.625))
            .role("button")
            .label(t("transactions:graph.legend.readingLabel"))
            .help(t("transactions:graph.view3dLegend"))
            .focusable(true)
            .child(crate::ui::ic(&I::INFO, tk.r(0.875), tk.muted_fg)),
    )
}

pub fn expand_button(tk: &Tk) -> El {
    row()
        .key("graph-expand")
        .size(tk.r(2.0), tk.r(2.0))
        .center()
        .radius(tk.r(0.375))
        .border(1.0, tk.input)
        .fill(if tk.dark {
            tk.input.alpha(tk.input.a * 0.3)
        } else {
            tk.background
        })
        .hover(tk.accent)
        .role("button")
        .label(t("transactions:graph.expand"))
        .help(t("transactions:graph.expand"))
        .focusable(true)
        .on_press_tx(super::expand)
        .child(crate::ui::ic(&I::MAXIMIZE_2, tk.r(1.0), tk.fg))
}
