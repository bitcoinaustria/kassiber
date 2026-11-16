//! The right panel (RightPanel.tsx, AssistantPanel.tsx): docked beside the page inside the page
//! frame, a sheet over the page below 1023 pt. Its tab bar holds the Assistant tab, the panel's
//! own surfaces (Changes, Logs; `reports::diag::surfaces`) and any open transactions' tabs,
//! `+` (Open in side panel), the assistant's actions and Maximize; a panel without tabs shows
//! the launcher.
//!
//! While a transaction's details are in front, `txdetail` draws the panel (its own tab bar, which
//! takes this module's tabs through `detail_bar_tab`); this module draws it while the assistant,
//! a surface or the launcher is in front, with the same width, resize edge and maximize flag (the
//! web keeps one width for every tab).

use rune::{Color, Cx, Drag, El, Key, KeyEvent, Phase, Tx, col, el, row, text};

use super::composer::{self, Opts};
use super::empty::{self, Host};
use super::{Front, Menu, Surface, page};
use crate::app::App;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::{DEFAULT_WIDTH, MAX_WIDTH, MIN_WIDTH, SHEET_BELOW};
use crate::ui::ic;
use crate::ui::popup::TriggerExt;

/// Whether this module draws the panel now (otherwise `txdetail` does, or it is closed).
pub fn in_front(app: &App) -> bool {
    super::panel_shown(app) && super::front(app) != Some(Front::Detail)
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

/// The panel, docked (`Some`) or as a sheet in `layer` (`None`).
pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> Option<El> {
    if !in_front(app) {
        return None;
    }
    let window = cx
        .layout_size("sh-root")
        .map(|[w, _]| w)
        .unwrap_or(app.shell.window_w);
    let sheet = window > 0.0 && window < SHEET_BELOW;
    let front = super::front(app);
    let front_label = match front {
        Some(Front::Assistant) => t("assistant:panel.title"),
        Some(Front::Surface(s)) => s.label(),
        _ => String::new(),
    };
    let aside_label = if front_label.is_empty() {
        t("assistant:panel.sideLabel")
    } else {
        format!("{}: {front_label}", t("assistant:panel.sideLabel"))
    };
    let mut panel = col()
        .key("asst-panel")
        .role("complementary")
        .label(aside_label)
        .h_full()
        .min_h(0.0)
        .fill(if sheet { tk.popover } else { tk.background });
    panel = panel.child(tab_bar(app, tk, sheet, layer));
    panel = match front {
        Some(Front::Assistant) => panel.child(body(app, cx, tk, sheet, layer)),
        Some(Front::Surface(s)) => {
            panel.child(crate::reports::diag::surfaces::view(app, cx, tk, s, layer))
        }
        _ => panel.child(launcher(app, tk)),
    };
    if sheet {
        layer.push(
            crate::shell::window_backdrop("asst-sheet-scrim", tk.background.alpha(0.6))
                .backdrop_blur(4.0)
                .on_press(super::hide_panel)
                .role("none"),
        );
        let w = clamp(app.detail.width, window).min((window - 48.0).max(0.0));
        // Its own layer root on the right edge, so it slides in and out itself.
        layer.push(
            panel
                .pin(None, Some(0.0), Some(0.0), Some(0.0))
                .w(w)
                .min_w(w.min(MIN_WIDTH))
                .border_sides([0.0, 0.0, 0.0, 1.0], tk.border)
                .box_shadow(rune::Shadow {
                    blur: 15.0,
                    y: 10.0,
                    color: Color::BLACK.alpha(0.05),
                    ..Default::default()
                })
                .transition(crate::shell::sheet_enter(tk))
                .modal(true, super::hide_panel),
        );
        return None;
    }
    let mut outer = row().key("asst-dock").h_full().min_h(0.0);
    outer = if app.detail.maximized {
        // Maximized: the panel takes the whole page column (the page squeezes to nothing).
        let w = cx.layout_size("app-main").map(|[w, _]| w).unwrap_or(0.0)
            + cx.layout_size("asst-dock")
                .map(|[w, _]| w)
                .unwrap_or(app.detail.width);
        outer.w(w).min_w(w)
    } else {
        let w = clamp(app.detail.width, window);
        outer
            .w(w)
            .min_w(w)
            .border_sides([0.0, 0.0, 0.0, 1.0], tk.border)
            .child(resize_handle(tk))
    };
    Some(outer.child(panel.grow(1.0).min_w(0.0)))
}

/// The left-edge separator: drag, arrows, Home/End, Enter or a double press resets.
fn resize_handle(tk: &Tk) -> El {
    el().key("asst-resize")
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

/// A ghost `icon-xs` button (`size-6`, a `size-3.5` glyph).
fn icon_xs(tk: &Tk, key: &str, glyph: &'static rune::Icon, label: String, ink: Color) -> El {
    row()
        .key(key.to_owned())
        .size(tk.r(1.5), tk.r(1.5))
        .center()
        .radius(tk.r(0.375))
        .hover(if tk.dark {
            tk.accent.alpha(0.5)
        } else {
            tk.accent
        })
        .role("button")
        .label(label.clone())
        .help(label)
        .focusable(true)
        .child(ic(glyph, tk.r(0.875), ink))
}

/// Arrow keys on a tab: the neighbouring tab of the panel comes forward (with wrap).
fn step_tab(a: &mut App, ev: &KeyEvent) -> bool {
    let step: isize = match ev.key {
        Key::Right => 1,
        Key::Left => -1,
        _ => return false,
    };
    let tabs = super::tabs(a);
    if tabs.is_empty() {
        return false;
    }
    let cur = super::front(a)
        .and_then(|f| tabs.iter().position(|t| *t == f))
        .unwrap_or(0) as isize;
    let next = tabs[((cur + step).rem_euclid(tabs.len() as isize)) as usize];
    super::set_front(a, next);
    true
}

/// `PanelTab`: a compact chip, filled while in front; its icon turns into the close mark on
/// hover; Delete or Backspace on the tab closes it.
#[allow(clippy::too_many_arguments)]
fn chip_tab(
    app: &App,
    tk: &Tk,
    key: &str,
    which: Front,
    glyph: &'static rune::Icon,
    label: String,
    close_label: String,
    close_title: String,
    close: fn(&mut App),
) -> El {
    let active = in_front(app) && super::front(app) == Some(which);
    let hover = app.assistant.tab_hover == Some(which);
    row()
        .key(key.to_owned())
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
                a.assistant.tab_hover = Some(which);
            } else if a.assistant.tab_hover == Some(which) {
                a.assistant.tab_hover = None;
            }
        })
        // A middle click closes the tab as its × does (`onAuxClick`), focus untouched.
        .on_middle_press(move |a: &mut App, _: &mut Tx| {
            close(a);
            a.assistant.tab_hover = None;
        })
        .child(
            row()
                .key(format!("{key}-close"))
                .size(tk.r(1.0), tk.r(1.0))
                .center()
                .radius(tk.r(0.25))
                .hover(tk.muted)
                .role("button")
                .label(close_label)
                .help(close_title)
                .on_press(move |a: &mut App| {
                    close(a);
                    a.assistant.tab_hover = None;
                })
                .child(ic(
                    if hover { &I::X } else { glyph },
                    tk.r(0.75),
                    if active { tk.fg } else { tk.muted_fg },
                )),
        )
        .child(
            row()
                .key(format!("{key}-label"))
                .h_full()
                .min_w(0.0)
                .center_y()
                .radius(tk.r(0.25))
                .role("tab")
                .checked(active)
                .label(label.clone())
                .focusable(active)
                .on_press(move |a: &mut App| super::set_front(a, which))
                .on_key(move |a: &mut App, ev: &KeyEvent, _: &mut Tx| match ev.key {
                    Key::Delete | Key::Backspace => {
                        close(a);
                        true
                    }
                    _ => step_tab(a, ev),
                })
                .child(
                    text(label)
                        .font(tk.txs(400.0))
                        .color(if active { tk.fg } else { tk.muted_fg })
                        .truncate(),
                ),
        )
}

/// The Assistant tab. Its close hides the assistant (`setAssistantPanelOpen(false)`).
pub fn assistant_tab(app: &App, tk: &Tk) -> El {
    let close = format!("{} (⌘I)", t("assistant:panel.close"));
    chip_tab(
        app,
        tk,
        "asst-tab",
        Front::Assistant,
        &I::SPARKLES,
        t("assistant:panel.title"),
        t("assistant:panel.close"),
        close,
        |a| a.assistant.tab_open = false,
    )
}

/// A surface's tab (Changes, Logs).
fn surface_tab(app: &App, tk: &Tk, s: Surface) -> El {
    let label = s.label();
    let close_label = tr("assistant:panel.closeTab", &[("label", &label)]);
    let close: fn(&mut App) = match s {
        Surface::Changes => |a| super::close_surface(a, Surface::Changes),
        Surface::Logs => |a| super::close_surface(a, Surface::Logs),
    };
    chip_tab(
        app,
        tk,
        &format!("asst-stab-{}", s.id()),
        Front::Surface(s),
        s.icon(),
        label,
        close_label.clone(),
        close_label,
        close,
    )
}

/// The panel's tabs before the details' (assistant, then surfaces), as chips.
fn own_tabs(app: &App, tk: &Tk) -> Vec<El> {
    super::tabs(app)
        .into_iter()
        .filter_map(|f| match f {
            Front::Assistant => Some(assistant_tab(app, tk)),
            Front::Surface(s) => Some(surface_tab(app, tk, s)),
            Front::Detail => None,
        })
        .collect()
}

/// A detail's tab while another tab is in front: pressing it brings the details forward.
fn detail_tab(app: &App, tk: &Tk, i: usize) -> El {
    let slot = app.detail.tabs.get(i);
    let label = match (i == app.detail.active_tab, app.detail.open.as_deref()) {
        (true, Some(o)) => crate::txdetail::view::tab_label(app, o),
        _ => slot.map(|s| s.label.clone()).unwrap_or_default(),
    };
    let preview = !slot.is_some_and(|s| s.pinned);
    let mut style = tk.txs(400.0);
    if preview {
        style = style.italic();
    }
    let close_label = tr("assistant:panel.closeTab", &[("label", &label)]);
    row()
        .key(format!("asst-dtab-{i}"))
        .h(tk.r(1.5))
        .max_w(tk.r(11.0))
        .pl(tk.r(0.25))
        .pr(tk.r(0.5))
        .gap(tk.r(0.125))
        .center_y()
        .radius(tk.r(0.375))
        .hover(tk.accent.alpha(0.6))
        .on_middle_press(move |a: &mut App, _: &mut Tx| crate::txdetail::close_tab(a, i))
        .child(
            row()
                .key(format!("asst-dtab-close-{i}"))
                .size(tk.r(1.0), tk.r(1.0))
                .center()
                .radius(tk.r(0.25))
                .hover(tk.muted)
                .role("button")
                .label(close_label.clone())
                .help(close_label)
                .on_press(move |a: &mut App| crate::txdetail::close_tab(a, i))
                .child(ic(&I::RECEIPT, tk.r(0.75), tk.muted_fg)),
        )
        .child(
            row()
                .key(format!("asst-dtab-label-{i}"))
                .h_full()
                .min_w(0.0)
                .center_y()
                .role("tab")
                .checked(false)
                .label(label.clone())
                .on_press(move |a: &mut App| {
                    super::set_front(a, Front::Detail);
                    crate::txdetail::activate(a, i);
                })
                .child(text(label).font(style).color(tk.muted_fg).truncate()),
        )
}

/// The tab bar (`h-10 border-b border-border/60 px-2 gap-1`).
fn tab_bar(app: &App, tk: &Tk, sheet: bool, layer: &mut Vec<El>) -> El {
    let a = &app.assistant;
    let mut tabs = row()
        .gap(tk.r(0.25))
        .center_y()
        .min_w(0.0)
        .role("tablist")
        .label(t("assistant:panel.tabs"));
    let own = own_tabs(app, tk);
    let mut any = !own.is_empty();
    tabs = tabs.children(own);
    if app.detail.open.is_some() {
        for i in 0..app.detail.tabs.len().max(1) {
            tabs = tabs.child(detail_tab(app, tk, i));
            any = true;
        }
    }
    let add_open = a.menu == Some(Menu::PanelAdd);
    let add = icon_xs(
        tk,
        "asst-add-surface",
        &I::PLUS,
        t("assistant:panel.add"),
        tk.muted_fg,
    )
    .menu_trigger(add_open, |s: &mut App| {
        s.assistant.menu = if s.assistant.menu == Some(Menu::PanelAdd) {
            None
        } else {
            Some(Menu::PanelAdd)
        }
    });
    if add_open {
        add_menu(app, tk, layer);
    }
    let mut left = row().grow(1.0).min_w(0.0).gap(tk.r(0.25)).center_y();
    if any {
        left = left.child(tabs);
    }
    left = left.child(add);
    let mut right = row().gap(tk.r(0.125)).center_y();
    if super::front(app) == Some(Front::Assistant) {
        if a.streaming {
            let label = if a.queued.is_empty() {
                t("assistant:dock.working")
            } else {
                crate::i18n::tc("assistant:dock.workingQueued", a.queued.len(), &[])
            };
            right = right.child(
                row()
                    .gap(tk.r(0.25))
                    .center_y()
                    .radius(999.0)
                    .fill(tk.primary.alpha(0.1))
                    .px(tk.r(0.375))
                    .py(tk.r(0.125))
                    .child(crate::ui::dot(tk.r(0.375), tk.primary).a11y_hidden())
                    .child(text(label).font(tk.t2xs(500.0)).color(tk.primary)),
            );
        }
        if !a.messages.is_empty() {
            let open = a.menu == Some(Menu::PanelMore);
            right = right.child(crate::ui::popup::trigger(
                icon_xs(
                    tk,
                    "asst-panel-more",
                    &I::ELLIPSIS,
                    t("assistant:page.moreActions"),
                    tk.fg,
                ),
                crate::ui::popup::Kind::Menu,
                open,
                |s: &mut App| {
                    s.assistant.menu = if s.assistant.menu == Some(Menu::PanelMore) {
                        None
                    } else {
                        Some(Menu::PanelMore)
                    }
                },
            ));
            if open {
                let mut p = crate::ui::popup::Popup::menu("asst-panel-more", |s: &mut App| {
                    s.assistant.menu = None
                })
                .list_key("asst-panel-more-menu");
                let l = t("assistant:page.clearChat");
                p.item(
                    page::destructive_item(tk, "asst-panel-clear", &l).on_press(super::reset),
                    "asst-panel-clear",
                    l,
                    true,
                    false,
                );
                p.mount(
                    tk,
                    layer,
                    col()
                        .min_w(tk.r(11.0))
                        .padding(tk.r(0.25))
                        .label(t("assistant:page.moreActions")),
                    |l| crate::shell::glass(tk, l),
                    rune_kit::Placement::BottomEnd,
                    4.0,
                );
            }
        }
    }
    if sheet {
        right = right.child(
            icon_xs(tk, "asst-hide", &I::X, t("assistant:panel.hide"), tk.fg)
                .help(format!("{} (Esc)", t("assistant:panel.hide")))
                .on_press(super::hide_panel),
        );
    } else {
        let max = app.detail.maximized;
        let label = if max {
            t("assistant:panel.restore")
        } else {
            t("assistant:panel.maximize")
        };
        right = right.child(
            icon_xs(
                tk,
                "asst-maximize",
                if max { &I::MINIMIZE_2 } else { &I::MAXIMIZE_2 },
                label,
                tk.fg,
            )
            .checked(max)
            .on_press(|a: &mut App| a.detail.maximized = !a.detail.maximized),
        );
    }
    row()
        .key("asst-tabbar")
        .w_full()
        .h(tk.r(2.5))
        .px(tk.r(0.5))
        .gap(tk.r(0.25))
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border.alpha(tk.border.a * 0.6))
        .child(left)
        .child(right)
}

/// The surfaces `+` and the launcher offer: (icon, label, key, available, why not).
fn surfaces(app: &App) -> Vec<(&'static rune::Icon, String, &'static str, bool, String)> {
    let dev = app.prefs.developer_tools;
    vec![
        (
            &I::SPARKLES,
            t("assistant:panel.title"),
            "A",
            app.route.path() != "/assistant",
            t("assistant:panel.assistantUnavailable"),
        ),
        (
            &I::LIST_CHECKS,
            t("assistant:panel.surfaceReview"),
            "Q",
            true,
            String::new(),
        ),
        (
            &I::GIT_COMPARE_ARROWS,
            t("assistant:panel.surfaceChanges"),
            "H",
            dev,
            t("assistant:panel.developerOnly"),
        ),
        (
            &I::SCROLL_TEXT,
            t("assistant:panel.surfaceLogs"),
            "L",
            dev,
            t("assistant:panel.developerOnly"),
        ),
        (
            &I::RECEIPT,
            t("assistant:panel.surfaceDetails"),
            "D",
            app.detail.open.is_some(),
            t("assistant:panel.detailsHint"),
        ),
    ]
}

/// Opens surface `i`. Changes and Logs open as tabs of the panel; the review queue still opens
/// its page (its panel surface is not part of this build).
pub fn open_surface(a: &mut App, i: usize) {
    a.assistant.menu = None;
    a.detail.add_open = false;
    match i {
        0 if a.route.path() != "/assistant" => {
            super::open_panel(a);
            a.assistant.draft.focused = true;
        }
        1 => a.navigate("/review"),
        2 => super::open_surface(a, Surface::Changes),
        3 => super::open_surface(a, Surface::Logs),
        4 if a.detail.open.is_some() => {
            super::set_front(a, Front::Detail);
            let last = a.detail.tabs.len().saturating_sub(1);
            crate::txdetail::activate(a, last);
        }
        _ => {}
    }
}

fn surface_for_key(app: &App, ev: &KeyEvent) -> Option<usize> {
    let Key::Text(s) = &ev.key else { return None };
    if ev.mods.cmd || ev.mods.ctrl || ev.mods.alt {
        return None;
    }
    let k = s.to_ascii_uppercase();
    surfaces(app)
        .iter()
        .position(|(_, _, key, ok, _)| *ok && *key == k)
}

/// `AddSurfaceMenu` (`align="start" sideOffset={6} min-w-48`), its rows answering single keys.
fn add_menu(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let mut p =
        crate::ui::popup::Popup::menu("asst-add-surface", |s: &mut App| s.assistant.menu = None)
            .list_key("asst-add-menu");
    for (i, (glyph, label, key, ok, hint)) in surfaces(app).into_iter().enumerate() {
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
            .child(crate::shell::menu_shortcut(tk, key));
        if ok {
            r = r
                .hover(tk.accent)
                .focus_highlight(tk.accent)
                .on_press(move |a: &mut App| open_surface(a, i));
        } else if !hint.is_empty() {
            r = r.help(hint);
        }
        p.item(r, format!("asst-add-{i}"), label, ok, false);
    }
    // The rows answer their single-key shortcuts before typeahead.
    let list = col()
        .min_w(tk.r(12.0))
        .padding(tk.r(0.25))
        .label(t("assistant:panel.add"))
        .on_key(
            |a: &mut App, ev: &KeyEvent, _: &mut Tx| match surface_for_key(a, ev) {
                Some(i) => {
                    open_surface(a, i);
                    true
                }
                None => false,
            },
        );
    p.mount(
        tk,
        layer,
        list,
        |l| crate::shell::glass(tk, l),
        rune_kit::Placement::BottomStart,
        6.0,
    );
}

/// `RightPanelLauncher`: the panel without tabs; each row opens a surface by press, arrows and
/// Return, or its key while no field or menu has the keyboard.
fn launcher(app: &App, tk: &Tk) -> El {
    let mut rows = col().w_full().gap(2.0);
    for (i, (glyph, label, key, ok, hint)) in surfaces(app).into_iter().enumerate() {
        let mut r = row()
            .key(format!("asst-launch-{i}"))
            .w_full()
            .h(tk.r(2.0))
            .px(tk.r(0.625))
            .gap(tk.r(0.625))
            .center_y()
            .radius(tk.r(0.375))
            .child(ic(glyph, tk.r(1.0), tk.fg))
            .child(
                text(label.clone())
                    .font(tk.tsm(400.0))
                    .color(tk.fg)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            )
            .child(crate::txdetail::widgets::kbd(tk, key));
        r = if ok {
            r.role("button")
                .label(label)
                .focusable(true)
                .hover(tk.accent.alpha(0.6))
                .focus_highlight(tk.accent.alpha(0.6))
                .on_press(move |a: &mut App| open_surface(a, i))
        } else {
            r.role("button")
                .label(format!("{label} {hint}"))
                .disabled(true)
                .focusable(true)
                .opacity(0.5)
                .help(hint)
        };
        rows = rows.child(r);
    }
    col()
        .key("asst-launcher")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(tk.r(1.5))
        .center()
        .role("group")
        .label(t("assistant:panel.launcherTitle"))
        .focusable(true)
        .focus_group()
        .on_key(
            |a: &mut App, ev: &KeyEvent, _: &mut Tx| match surface_for_key(a, ev) {
                Some(i) => {
                    open_surface(a, i);
                    true
                }
                None => false,
            },
        )
        .child(
            col()
                .w_full()
                .max_w(tk.r(20.0))
                .py(tk.r(1.5))
                .child(
                    text(t("assistant:panel.launcherTitle"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .align(rune::Align::Center)
                        .w_full()
                        .mb(tk.r(0.75))
                        .role("heading"),
                )
                .child(rows),
        )
}

/// `AssistantPanelBody`: the conversation with the error, the focus chip and the composer docked
/// under it; without messages, the shared empty state (`empty.rs`) centred in the panel, drawn
/// like the page when maximized.
fn body(app: &App, cx: &mut Cx<App>, tk: &Tk, sheet: bool, layer: &mut Vec<El>) -> El {
    let a = &app.assistant;
    if !super::owns(app.route.path()) {
        super::checks::dialog(app, tk, layer);
    }
    let c = col()
        .key("asst-panel-body")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .role("tabpanel")
        .label(t("assistant:panel.title"));
    if a.messages.is_empty() {
        // A sheet ignores Maximize, so it keeps the docked look.
        let host = if app.detail.maximized && !sheet {
            Host::Maximized
        } else {
            Host::Docked
        };
        // Scrolls when the panel is shorter than the group; otherwise centres it.
        let h = cx
            .layout_size("asst-panel-empty")
            .map(|[_, h]| h)
            .unwrap_or(0.0);
        let pad = if host == Host::Docked {
            tk.r(1.0)
        } else {
            tk.r(1.5)
        };
        let inner = col()
            .w_full()
            .min_h(h)
            .px(pad)
            .py(tk.r(1.5))
            .center()
            .child(empty::view(app, cx, tk, host, layer));
        return c.child(
            col()
                .key("asst-panel-empty")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(inner),
        );
    }
    let c = c.child(
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(super::thread::view(app, cx, tk, false, layer)),
    );
    let mut foot = col().w_full().gap(tk.r(0.5)).padding(tk.r(0.5));
    if let Some(e) = &a.error {
        foot = foot.child(
            row()
                .w_full()
                .radius(tk.r(0.375))
                .border(1.0, tk.destructive.alpha(0.4))
                .border_layout()
                .fill(tk.destructive.alpha(0.1))
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .role("alert")
                .child(
                    text(e.clone())
                        .font(tk.txs(400.0))
                        .color(tk.destructive)
                        .wrap(),
                ),
        );
    }
    if let Some(chip) = composer::focus_chip(app, tk, false) {
        foot = foot.child(chip);
    }
    foot = foot.child(composer::view(
        app,
        cx,
        tk,
        Opts {
            host: "panel",
            hero: false,
            suggestions: false,
            compact: false,
        },
        layer,
    ));
    c.child(foot)
}

/// The panel's other tabs for `txdetail`'s tab bar while a transaction is in front (the
/// Assistant chip, then the surfaces'): pressing one brings it forward (hook for
/// `txdetail::view::tab_bar`, which takes any `IntoIterator<Item = El>`).
pub fn detail_bar_tab(app: &App, tk: &Tk) -> Vec<El> {
    if !super::panel_shown(app) {
        return vec![];
    }
    own_tabs(app, tk)
}
