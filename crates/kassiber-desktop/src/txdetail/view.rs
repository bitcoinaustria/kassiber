//! The panel frame (TransactionDetailSheetFrame.tsx, RightPanel geometry, TransactionDetailSheet
//! body layout): the docked right panel with its resize edge, the sheet mode below 1023 pt, the
//! tab bar, the two-column body at ≥ 72 rem, and the footer.

use rune::{Color, Cx, Drag, El, KeyEvent, Phase, Tx, col, el, row, text};
use serde_json::Value;

use super::draft::{opt_s, s};
use super::widgets::{dirty_dot, kbd, spinner};
use super::{DEFAULT_WIDTH, MAX_WIDTH, MIN_WIDTH, Open, SHEET_BELOW, Tab};
use crate::app::App;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;
use crate::ui::popup::TriggerExt;

pub fn sheet_mode(app: &App) -> bool {
    sheet_at(app.shell.window_w)
}

fn sheet_at(w: f32) -> bool {
    w > 0.0 && w < SHEET_BELOW
}

/// `hostedDetailLabel`: hidden when sensitive, else counterparty (`counter`), else a short id
/// (`txnId || id`, `first8…last4`).
pub(crate) fn tab_label(app: &App, o: &Open) -> String {
    if app.prefs.hide_sensitive() {
        return t("common:field.details");
    }
    let Some(r) = &o.record else {
        return t("common:field.details");
    };
    if let Some(c) = opt_s(r, "counter")
        .map(|c| c.trim().to_owned())
        .filter(|c| !c.is_empty())
    {
        return super::graph::route::arrows(&c);
    }
    let id = opt_s(r, "externalId").unwrap_or_else(|| s(r, "id").to_owned());
    if id.chars().count() > 14 {
        let head: String = id.chars().take(8).collect();
        let tail: String = id
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{head}…{tail}")
    } else {
        id
    }
}

fn clamp(w: f32, window: f32) -> f32 {
    // The page keeps at least 480 pt beside the docked panel.
    let max = if window > 0.0 {
        (window - 480.0).max(MIN_WIDTH)
    } else {
        MAX_WIDTH
    };
    w.clamp(MIN_WIDTH, MAX_WIDTH.min(max)).round()
}

fn resize_handle(tk: &Tk) -> El {
    el().key("txd-resize")
        .w(12.0)
        .h_full()
        .absolute(-6.0, 0.0)
        .role("separator")
        .label(t("assistant:panel.resize"))
        .focusable(true)
        .cursor(rune::Cursor::ResizeColumn)
        .on_drag(|a: &mut App, d: &mut Drag, _tx: &mut Tx| match d.phase {
            Phase::Down => a.detail.drag_from = Some((a.detail.width, 0.0)),
            Phase::Move | Phase::Up => {
                if let Some((start, _)) = a.detail.drag_from {
                    a.detail.width = clamp(start - d.dx, a.shell.window_w);
                }
                if d.phase == Phase::Up {
                    a.detail.drag_from = None;
                }
            }
        })
        .on_double_press_tx(|a: &mut App, _: &mut Tx| a.detail.width = DEFAULT_WIDTH)
        .on_key(|a: &mut App, ev: &KeyEvent, _: &mut Tx| {
            use rune::Key;
            let w = a.detail.width;
            let next = match ev.key {
                Key::Left => w + 16.0,
                Key::Right => w - 16.0,
                Key::Home => MIN_WIDTH,
                Key::End => MAX_WIDTH,
                Key::Enter => DEFAULT_WIDTH,
                _ => return false,
            };
            a.detail.width = clamp(next, a.shell.window_w);
            true
        })
        .child(
            el().w(1.0)
                .h_full()
                .ml(6.0)
                .fill(Color::TRANSPARENT)
                .hover(tk.ring),
        )
}

/// One panel tab chip (`PanelTab`): `h-6 max-w-44 rounded-md pr-2 pl-1 text-xs`, filled while
/// in front. Its icon turns into the close mark while the pointer is over the chip; a page's
/// preview reads in italics and a double click keeps it open.
fn panel_tab(app: &App, tk: &Tk, i: usize, label: String, active: bool, preview: bool) -> El {
    let hover = app.detail.tab_hover == Some(i);
    let close_label = tr("assistant:panel.closeTab", &[("label", &label)]);
    let glyph: &'static rune::Icon = if hover { &I::X } else { &I::RECEIPT };
    let close = row()
        .key(format!("txd-tab-close-{i}"))
        .size(tk.r(1.0), tk.r(1.0))
        .center()
        .radius(tk.r(0.25))
        .hover(tk.muted)
        .role("button")
        .label(close_label.clone())
        .help(close_label)
        .on_press(move |a: &mut App| super::close_tab(a, i))
        .child(ic(
            glyph,
            tk.r(0.75),
            if active { tk.fg } else { tk.muted_fg },
        ));
    let mut style = tk.txs(400.0);
    if preview {
        style = style.italic();
    }
    let title = if preview {
        Some(format!("{label} · {}", t("assistant:panel.previewHint")))
    } else {
        None
    };
    let mut name = row()
        .key(format!("txd-tab-label-{i}"))
        .h_full()
        .min_w(0.0)
        .center_y()
        .radius(tk.r(0.25))
        .role("tab")
        .checked(active)
        .label(label.clone())
        .focusable(active)
        .on_press(move |a: &mut App| super::activate(a, i))
        .on_double_press_tx(move |a: &mut App, _: &mut Tx| {
            if preview {
                super::activate(a, i);
                super::pin(a);
            }
        })
        .child(
            text(label)
                .font(style)
                .color(if active { tk.fg } else { tk.muted_fg })
                .truncate(),
        );
    if let Some(h) = title {
        name = name.help(h);
    }
    row()
        .key(format!("txd-tab-{i}"))
        .h(tk.r(1.5))
        .max_w(tk.r(11.0))
        .pl(tk.r(0.25))
        .pr(tk.r(0.5))
        .gap(tk.r(0.125))
        .center_y()
        .radius(tk.r(0.375))
        .fill(if active {
            tk.accent
        } else {
            Color::TRANSPARENT
        })
        .hover(if active {
            tk.accent
        } else {
            tk.accent.alpha(0.6)
        })
        .on_hover_change(move |a: &mut App, on: bool, _: &mut Tx| {
            if on {
                a.detail.tab_hover = Some(i);
            } else if a.detail.tab_hover == Some(i) {
                a.detail.tab_hover = None;
            }
        })
        // A middle click closes the tab as its × does (`onAuxClick`), focus untouched.
        .on_middle_press(move |a: &mut App, _: &mut Tx| super::close_tab(a, i))
        .child(close)
        .child(name)
}

/// The side panel's tab bar (`RightPanelBody`): `h-10 border-b border-border/60 px-2 gap-1`, the
/// detail tabs, `+` (Open in side panel), then Keep open (previews) and Maximize/Restore.
fn tab_bar(app: &App, tk: &Tk, o: &Open, sheet: bool, layer: &mut Vec<El>) -> El {
    let max = app.detail.maximized;
    let mut tabs = row()
        .gap(tk.r(0.25))
        .center_y()
        .min_w(0.0)
        .role("tablist")
        .label(t("assistant:panel.tabs"))
        .children(crate::assistant::panel::detail_bar_tab(app, tk));
    let active = app.detail.active_tab;
    let n = app.detail.tabs.len().max(1);
    for i in 0..n {
        let slot = app.detail.tabs.get(i);
        let is_active = i == active;
        let label = if is_active {
            tab_label(app, o)
        } else {
            slot.map(|s| s.label.clone()).unwrap_or_default()
        };
        let preview = !slot.is_some_and(|s| s.pinned);
        tabs = tabs.child(panel_tab(app, tk, i, label, is_active, preview));
    }
    let add = super::widgets::icon_button(
        tk,
        "txd-add-surface",
        super::widgets::Variant::Ghost,
        tk.r(1.75),
        &I::PLUS,
        tk.r(0.875),
        t("assistant:panel.add"),
    )
    .menu_trigger(app.detail.add_open, |a: &mut App| {
        a.detail.add_open = !a.detail.add_open
    });
    if app.detail.add_open {
        add_menu(app, tk, layer);
    }
    let left = row()
        .grow(1.0)
        .min_w(0.0)
        .gap(tk.r(0.25))
        .center_y()
        .child(tabs)
        .child(add);
    let mut right = row().gap(tk.r(0.125)).center_y();
    if let Some(r) = &o.record {
        right = right.children(super::header::record_actions(app, tk, r, true));
    }
    let preview = !app.detail.tabs.get(active).is_some_and(|s| s.pinned);
    if preview && o.id().is_some() {
        right = right.child(
            super::widgets::icon_button(
                tk,
                "txd-pin",
                super::widgets::Variant::Ghost,
                tk.r(1.75),
                &I::PIN,
                tk.r(0.875),
                t("assistant:panel.pin"),
            )
            .help(t("assistant:panel.pinHint"))
            .on_press(super::pin),
        );
    }
    if sheet {
        right = right.child(
            super::widgets::icon_button(
                tk,
                "txd-hide",
                super::widgets::Variant::Ghost,
                tk.r(1.75),
                &I::X,
                tk.r(0.875),
                t("assistant:panel.hide"),
            )
            .on_press(super::close),
        );
    } else {
        let label = if max {
            t("assistant:panel.restore")
        } else {
            t("assistant:panel.maximize")
        };
        right = right.child(
            super::widgets::icon_button(
                tk,
                "txd-maximize",
                super::widgets::Variant::Ghost,
                tk.r(1.75),
                if max { &I::MINIMIZE_2 } else { &I::MAXIMIZE_2 },
                tk.r(0.875),
                label,
            )
            .checked(max)
            .on_press_tx(|a: &mut App, tx: &mut Tx| {
                // In graph focus, Restore leaves it too (back to the docked body).
                if super::graph::focus_shown(a) {
                    super::graph::collapse(a, tx, true);
                } else {
                    a.detail.maximized = !a.detail.maximized;
                }
            }),
        );
    }
    row()
        .key("txd-tabbar")
        .w_full()
        .h(tk.r(2.5))
        .px(tk.r(0.5))
        .gap(tk.r(0.25))
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border.alpha(tk.border.a * 0.6))
        .child(left)
        .child(right)
}

/// `AddSurfaceMenu`: the panel's surfaces with their single-key shortcuts.
fn add_menu(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let dev = app.prefs.developer_tools;
    /// (glyph, label, key, available, action).
    type Item = (
        &'static rune::Icon,
        String,
        &'static str,
        bool,
        fn(&mut App),
    );
    let items: [Item; 5] = [
        (
            &I::SPARKLES,
            t("assistant:panel.title"),
            "A",
            app.prefs.ai_features,
            |a| {
                a.detail.add_open = false;
                crate::assistant::open_about_detail(a);
            },
        ),
        (
            &I::LIST_CHECKS,
            t("assistant:panel.surfaceReview"),
            "Q",
            true,
            |a| {
                a.detail.add_open = false;
                a.navigate("/review");
            },
        ),
        (
            &I::GIT_COMPARE_ARROWS,
            t("assistant:panel.surfaceChanges"),
            "H",
            dev,
            |a| crate::assistant::panel::open_surface(a, 2),
        ),
        (
            &I::SCROLL_TEXT,
            t("assistant:panel.surfaceLogs"),
            "L",
            dev,
            |a| crate::assistant::panel::open_surface(a, 3),
        ),
        (
            &I::RECEIPT,
            t("assistant:panel.surfaceDetails"),
            "D",
            true,
            |a| {
                a.detail.add_open = false;
                let last = a.detail.tabs.len().saturating_sub(1);
                super::activate(a, last);
            },
        ),
    ];
    let mut p =
        crate::ui::popup::Popup::menu("txd-add-surface", |a: &mut App| a.detail.add_open = false)
            .list_key("txd-add-menu");
    for (i, (glyph, label, key, ok, act)) in items.into_iter().enumerate() {
        let mut r = row()
            .w_full()
            .px(tk.r(0.5))
            .py(tk.r(0.375))
            .gap(tk.r(0.5))
            .center_y()
            .radius(tk.r(0.25))
            .role("menuitem")
            .label(label.clone())
            .disabled(!ok)
            .opacity(if ok { 1.0 } else { 0.5 })
            .child(ic(glyph, tk.r(1.0), tk.muted_fg))
            .child(
                text(label.clone())
                    .font(tk.tsm(400.0))
                    .color(tk.fg)
                    .grow(1.0),
            )
            .child(
                text(key.to_owned())
                    .font(tk.txs(400.0).tracking(1.0))
                    .color(tk.muted_fg),
            );
        if ok {
            r = r.hover(tk.accent).focus_highlight(tk.accent).on_press(act);
        } else {
            r = r.help(t("assistant:panel.developerOnly"));
        }
        p.item(r, format!("txd-add-{i}"), label, ok, false);
    }
    // The rows' single-key shortcuts, ahead of typeahead.
    let list = col()
        .min_w(tk.r(12.0))
        .padding(tk.r(0.25))
        .label(t("assistant:panel.add"))
        .on_key(|a: &mut App, ev: &KeyEvent, _: &mut Tx| {
            let rune::Key::Text(s) = &ev.key else {
                return false;
            };
            let target = match s.to_ascii_lowercase().as_str() {
                "a" => Some(0),
                "q" => Some(1),
                "h" if a.prefs.developer_tools => Some(2),
                "l" if a.prefs.developer_tools => Some(3),
                "d" => Some(4),
                _ => None,
            };
            let Some(t) = target else { return false };
            a.detail.add_open = false;
            match t {
                0 => crate::assistant::open_about_detail(a),
                1 => a.navigate("/review"),
                2 => crate::assistant::panel::open_surface(a, 2),
                3 => crate::assistant::panel::open_surface(a, 3),
                _ => {
                    let last = a.detail.tabs.len().saturating_sub(1);
                    super::activate(a, last)
                }
            }
            true
        });
    p.mount(
        tk,
        layer,
        list,
        |l| crate::shell::glass(tk, l),
        rune_kit::Placement::BottomStart,
        6.0,
    );
}

/// The record's tabs (`TabsList`: `h-9 p-[3px] rounded-lg bg-muted`; triggers `flex-auto px-2
/// rounded-md border text-sm font-medium`, the active one `bg-background` / dark
/// `bg-input/30 border-input` with `shadow-sm`).
fn tab_strip(cx: &mut Cx<App>, tk: &Tk, o: &Open, linked_count: usize) -> El {
    let d = o.dirty();
    // `flex-auto`: each trigger is its content plus an equal share of the free width.
    let content: Vec<f32> = Tab::ALL
        .iter()
        .map(|tab| {
            let label_w = rune::text::measure(&tab.label(), &[], tk.tsm(500.0), None).0;
            let extra = if *tab == Tab::Linked && linked_count > 0 {
                tk.r(0.375)
                    + rune::text::measure(&linked_count.to_string(), &[], tk.t2xs(400.0), None).0
                    + tk.r(0.75)
            } else {
                0.0
            };
            label_w + extra + tk.r(1.0) + 2.0
        })
        .collect();
    let inner = cx.layout_size("txd-tabs").map(|[w, _]| w - 6.0);
    let share = inner.map(|w| ((w - content.iter().sum::<f32>()) / content.len() as f32).max(0.0));
    let mut strip = row()
        .key("txd-tabs")
        .w_full()
        .h(tk.r(2.25))
        .padding(3.0)
        .center_y()
        .radius(tk.r(0.5))
        .fill(tk.muted)
        .role("tablist")
        .label(
            Tab::ALL
                .iter()
                .map(|t| t.label())
                .collect::<Vec<_>>()
                .join(" "),
        );
    for tab in Tab::ALL {
        let active = o.tab == tab;
        let idx = Tab::ALL.iter().position(|t| *t == tab).unwrap_or(0);
        let (fill, border) = if !active {
            (Color::TRANSPARENT, Color::TRANSPARENT)
        } else if tk.dark {
            (tk.input.alpha(tk.input.a * 0.3), tk.input)
        } else {
            (tk.background, Color::TRANSPARENT)
        };
        let mut b = row()
            .key(format!("txd-tab-{}", tab.code()))
            .h_full()
            .px(tk.r(0.5))
            .center()
            .gap(tk.r(0.375))
            .radius(tk.r(0.375))
            .fill(fill)
            .border(1.0, border)
            .role("tab")
            .checked(active)
            .label(tab.label())
            .focusable(true)
            .on_press(move |a: &mut App| {
                if let Some(o) = a.detail.open.as_mut() {
                    o.tab = tab;
                }
            })
            .child(text(tab.label()).font(tk.tsm(500.0)).color(if active {
                tk.fg
            } else {
                tk.muted_fg
            }));
        b = match share {
            Some(sh) => b.w(content[idx] + sh),
            None => b.grow(1.0),
        };
        if active {
            b = b.box_shadow(rune::Shadow {
                y: 1.0,
                blur: 2.0,
                color: Color::BLACK.alpha(0.05),
                ..Default::default()
            });
        }
        let dirty = match tab {
            Tab::Classify => d.label || d.tags || d.note || d.review_status,
            Tab::Pricing => d.pricing,
            Tab::Tax => d.excluded || d.review_status || d.taxable || d.at,
            _ => false,
        };
        if dirty {
            b = b.child(dirty_dot(tk));
        }
        if tab == Tab::Linked && linked_count > 0 {
            // `rounded-full bg-muted px-1.5 text-2xs tabular-nums text-muted-foreground`.
            b = b.child(
                row()
                    .px(tk.r(0.375))
                    .radius(tk.r(0.5))
                    .fill(tk.muted)
                    .child(
                        text(linked_count.to_string())
                            .font(tk.t2xs(400.0))
                            .color(tk.muted_fg),
                    ),
            );
        }
        strip = strip.child(b);
    }
    strip
}

fn footer(app: &App, tk: &Tk, o: &Open) -> El {
    let dirt = o.dirty();
    // `flex flex-wrap gap-x-3 gap-y-1`: the shortcut hint wraps under the unsaved count.
    let mut left = row()
        .gap(tk.r(0.75))
        .center_y()
        .grow(1.0)
        .min_w(0.0)
        .flex_wrap();
    if dirt.count > 0 {
        left = left.child(
            row()
                .gap(tk.r(0.375))
                .center_y()
                .child(el().size(6.0, 6.0).radius(3.0).fill(tk.warn.solid))
                .child(
                    text(super::unsaved_label(dirt.count))
                        .font(tk.txs(500.0))
                        .color(tk.warn.text),
                ),
        );
    }
    let wide = app.shell.window_w <= 0.0 || app.shell.window_w >= 640.0;
    if wide {
        // `<kbd>⌘S</kbd> save · <kbd>1-5</kbd> tabs · <kbd>e</kbd> exclude` (text-xs, gap-1.5).
        let words = |s: String| text(s).font(tk.txs(400.0)).color(tk.muted_fg);
        left = left.child(
            row()
                .gap(tk.r(0.375))
                .center_y()
                .child(kbd(tk, "⌘S"))
                .child(words(format!(
                    "{} ·",
                    t("transactions:sheet.footer.shortcutSave")
                )))
                .child(kbd(tk, "1-5"))
                .child(words(format!(
                    "{} ·",
                    t("transactions:sheet.footer.shortcutTabs")
                )))
                .child(kbd(tk, "e"))
                .child(words(t("transactions:sheet.footer.shortcutExclude"))),
        );
    }
    if let Some(e) = &o.save_error {
        left = left.child(
            text(e.clone())
                .font(tk.txs(400.0))
                .color(tk.destructive)
                .role("alert")
                .truncate()
                .help_when_truncated(e.clone()),
        );
    }
    let mut buttons = row().gap(tk.r(0.5)).center_y();
    let mut cancel = super::widgets::button(
        tk,
        "txd-cancel",
        super::widgets::Variant::Outline,
        super::widgets::Size::Default,
        None,
        &t("common:actions.cancel"),
        !o.saving,
    );
    if !o.saving {
        cancel = cancel.on_press(super::close);
    }
    buttons = buttons.child(cancel);
    if dirt.count > 0 {
        let mut discard = super::widgets::button(
            tk,
            "txd-discard",
            super::widgets::Variant::Ghost,
            super::widgets::Size::Default,
            Some(&I::ROTATE_CCW),
            &t("transactions:sheet.footer.discard"),
            !o.saving,
        )
        .gap(tk.r(0.375));
        if !o.saving {
            discard = discard.on_press(super::actions::discard);
        }
        buttons = buttons.child(discard);
    }
    let next = super::actions::has_next(app);
    let label = if o.saving {
        t("transactions:sheet.footer.saving")
    } else if next {
        t("transactions:sheet.footer.saveAndNext")
    } else {
        t("transactions:sheet.footer.save")
    };
    let ok = dirt.count > 0 && !o.saving;
    let mut save = super::widgets::button(
        tk,
        "txd-save",
        super::widgets::Variant::Default,
        super::widgets::Size::Default,
        Some(&I::SAVE),
        &label,
        ok,
    );
    if next && !o.saving {
        save = save.child(ic(&I::ARROW_RIGHT, tk.r(1.0), tk.primary_fg));
    }
    if ok {
        save = save.on_press(move |a: &mut App| super::actions::save(a, !next, next));
    }
    buttons = buttons.child(save);
    // `SheetFooter border-t p-3 sm:px-6`.
    row()
        .key("txd-footer")
        .w_full()
        .px(tk.r(1.5))
        .py(tk.r(0.75))
        .gap(tk.r(0.75))
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .fill(tk.background)
        .child(left)
        .child(buttons)
}

fn status_body(tk: &Tk, o: &Open) -> El {
    let mut c = col()
        .w_full()
        .grow(1.0)
        .padding(tk.r(1.25))
        .gap(tk.r(0.5))
        .key("txd-status");
    c = c.child(
        text(t("common:field.details"))
            .font(tk.tsm(600.0))
            .color(tk.fg),
    );
    if o.loading {
        c = c.child(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .role("status")
                .label(t("common:state.loading"))
                .child(spinner(tk, tk.r(1.0)))
                .child(
                    text(t("common:state.loading"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                ),
        );
    } else {
        c = c.child(
            text(t("common:state.error"))
                .font(tk.txs(500.0))
                .color(tk.destructive)
                .role("alert"),
        );
        if let Some(e) = &o.error {
            c = c.child(
                text(e.clone())
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap(),
            );
        }
        c = c.child(
            row().child(
                super::widgets::small_button(
                    tk,
                    "txd-retry",
                    Some(&I::REFRESH_CW),
                    &t("common:actions.retry"),
                    true,
                    true,
                )
                .on_press(super::retry),
            ),
        );
    }
    c
}

thread_local! {
    static TAB_W: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
    static SCROLL_Y: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// The panel body's scroll offset as last drawn (graph focus gives it back on the way out).
pub fn scroll_y() -> f32 {
    SCROLL_Y.with(|c| c.get())
}

/// Width of the active tab's panel (0 before the first layout).
pub fn tab_width() -> f32 {
    TAB_W.with(|c| c.get())
}

fn body(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    o: &Open,
    r: &Value,
    layer: &mut Vec<El>,
    sheet: bool,
) -> El {
    // `(pair ? 1 : 0) + (loanMark ? 1 : 0) + linkedLoanMarks.length`; loan marks come only from the
    // Transactions page's host.
    let loan = super::tabs::loan_of(app, s(r, "id")).filter(|_| super::host_loans(app));
    let linked_loans = loan.as_ref().map(|m| {
        let lid = s(m, "loan_id");
        app.detail
            .loans
            .as_ref()
            .map(|all| {
                all.iter()
                    .filter(|x| {
                        !lid.is_empty()
                            && s(x, "loan_id") == lid
                            && s(x, "transaction_id") != s(r, "id")
                    })
                    .count()
            })
            .unwrap_or(0)
    });
    let linked_count = usize::from(r.get("pair").is_some_and(|p| p.is_object()))
        + usize::from(loan.is_some())
        + linked_loans.unwrap_or(0);
    // The tab panel's width as laid out last frame: the web's `@container` queries inside the tabs
    // (`@2xl` = 42 rem) read it through `tab_width()`.
    let w = cx
        .layout_size(&format!("txd-tabpanel-{}", o.tab.code()))
        .map(|[w, _]| w)
        .unwrap_or(0.0);
    TAB_W.with(|c| c.set(w));
    let tab_body: El = match o.tab {
        Tab::Details => super::tabs::details(app, tk, o, r, layer),
        Tab::Classify => super::tabs::classify(app, tk, o, r, layer),
        Tab::Pricing => super::tabs::pricing(app, tk, o, r),
        Tab::Tax => super::tax::tax(app, tk, o, r, layer),
        Tab::Linked => super::linked::linked(app, tk, o, r),
    };
    // `space-y-4`: the Tabs (list, then `gap-2` + `mt-4` the panel) and the rail (`space-y-3`).
    let tabs = col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(1.5))
        .child(tab_strip(cx, tk, o, linked_count))
        .child(
            col()
                .w_full()
                .key(format!("txd-tabpanel-{}", o.tab.code()))
                .role("tabpanel")
                .label(o.tab.label())
                .child(tab_body),
        );
    let rail = col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.75))
        .child(super::rail::attachments(app, tk, o))
        .child(super::rail::history(app, tk, o));
    let right = col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(1.0))
        .child(tabs)
        .child(rail);
    let mut left = col().w_full().min_w(0.0).gap(tk.r(1.0));
    if let Some(b) = super::header::banner(app, tk, o, r) {
        left = left.child(b);
    }
    if let Some(sp) = super::dialogs::split_card(app, tk, o, r, layer) {
        left = left.child(sp);
    }
    left = left.child(
        col()
            .key("txd-graph-area")
            .w_full()
            .child(super::graph::section(app, cx, tk, r, layer)),
    );
    // Two columns once the detail container is at least 72 rem wide:
    // `grid-cols-[minmax(0,1fr)_minmax(400px,460px)]` (CSS px; the right column takes its 460).
    let wide = cx
        .layout_size("txd-scroll")
        .map(|[w, _]| w >= tk.r(72.0))
        .unwrap_or(false);
    let grid = if wide {
        row()
            .w_full()
            .gap(tk.r(1.0))
            .items_start()
            .child(left.grow(1.0).min_w(0.0))
            .child(right.w(460.0))
    } else {
        col().w_full().gap(tk.r(1.0)).child(left).child(right)
    };
    // `p-4 sm:p-6`: the breakpoint is the window's (640 px), not the panel's.
    let pad = if app.shell.window_w <= 0.0 || app.shell.window_w >= 640.0 {
        tk.r(1.5)
    } else {
        tk.r(1.0)
    };
    let _ = sheet;
    let mut content = col().w_full();
    if !sheet {
        // Hosted: the header scrolls away with the body.
        content = content.child(super::header::header(app, tk, o, r, false));
    }
    content = content.child(col().w_full().padding(pad).child(grid));
    col()
        .key("txd-scroll")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .on_scroll_offset(|at: rune::ScrollPos| SCROLL_Y.with(|c| c.set(at.offset[1])))
        .child(content)
}

/// The docked panel (or None while closed / shown as a sheet).
pub fn dock(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> Option<El> {
    let o = app.detail.open.as_deref()?;
    // The window width as laid out (the shell's stored width lags the first frame).
    let sheet = sheet_at(
        cx.layout_size("sh-root")
            .map(|[w, _]| w)
            .unwrap_or(app.shell.window_w),
    );
    let mut panel = col()
        .key("txd-panel")
        .role("complementary")
        .label(t("assistant:panel.surfaceDetails"))
        .h_full()
        .min_h(0.0)
        .fill(tk.background);
    if !sheet {
        panel = panel.child(tab_bar(app, tk, o, false, layer));
    }
    match &o.record {
        None => panel = panel.child(status_body(tk, o)),
        // Graph focus: the graph takes the whole body (no header, tabs or footer).
        Some(r) if super::graph::focus_shown(app) => {
            panel = panel.child(super::graph::focus_view(
                app,
                cx,
                tk,
                r,
                Some(tab_label(app, o)),
            ))
        }
        Some(r) => {
            if sheet {
                panel = panel.child(super::header::header(app, tk, o, r, true));
            }
            panel = panel
                .child(body(app, cx, tk, o, r, layer, sheet))
                .child(footer(app, tk, o));
            super::dialogs::mount(app, tk, o, layer);
        }
    }
    if sheet {
        layer.push(
            el().key("txd-sheet-scrim")
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .fill(Color::BLACK.alpha(0.5))
                .on_press(super::close)
                .role("none"),
        );
        layer.push(
            row()
                .key("txd-sheet-layer")
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .pl(48.0)
                .justify_end()
                .child(
                    panel
                        .w_full()
                        .max_w(1120.0)
                        .border_sides([0.0, 0.0, 0.0, 1.0], tk.border)
                        .box_shadow(rune::Shadow {
                            x: -8.0,
                            blur: 40.0,
                            color: Color::BLACK.alpha(0.35),
                            ..Default::default()
                        })
                        .modal(true, super::close),
                ),
        );
        return None;
    }
    // `border-l`: a CSS border, so the panel's content starts after it.
    let mut outer = row()
        .key("txd-dock")
        .h_full()
        .min_h(0.0)
        .border_sides([0.0, 0.0, 0.0, 1.0], tk.border)
        .border_layout();
    outer = if app.detail.maximized {
        // Maximized: the panel takes the whole page column (the page squeezes to nothing).
        let w = cx.layout_size("app-main").map(|[w, _]| w).unwrap_or(0.0)
            + cx.layout_size("txd-dock")
                .map(|[w, _]| w)
                .unwrap_or(app.detail.width);
        outer.w(w).min_w(w)
    } else {
        let w = clamp(app.detail.width, app.shell.window_w);
        outer.w(w).min_w(w)
    };
    Some(
        outer
            .child(resize_handle(tk))
            .child(panel.grow(1.0).min_w(0.0)),
    )
}

/// The page column is hidden while the panel is maximized (the shell leaves it out, so its texts
/// and controls leave the accessibility tree as the web's do).
pub fn page_hidden(app: &App) -> bool {
    app.detail.open.is_some() && app.detail.maximized && !sheet_mode(app)
}
