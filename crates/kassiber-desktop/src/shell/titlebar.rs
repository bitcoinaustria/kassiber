//! The title bar (AppShell.tsx header): nav toggle and history, the book crumb with its
//! switcher, the page title and Alpha chip, then the right cluster — book status, the refresh
//! split button, notifications, currency, theme, privacy, network, lock and the side panel.

use rune::{Color, El, Icon, LoopSpec, col, el, row, text};
use rune_kit::Placement;

use super::sidebar::shortcut;
use super::{Popup, books, check_row, menu_row, menu_sep, network, notifications, page_title};
use crate::app::{App, Currency, Privacy, ThemeMode};
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::popup::{Popup as Popup_, TriggerExt};
use crate::ui::{dot, ic};

/// A title-bar icon button (`size-8 rounded-md`, muted glyph, row-hover fill, open = active).
pub(crate) fn bar_button(
    tk: &Tk,
    key: &str,
    glyph: &'static Icon,
    label: &str,
    active: bool,
) -> El {
    row()
        .key(key.to_owned())
        .size(tk.r(2.0), tk.r(2.0))
        .center()
        .radius(tk.r(0.375))
        .fill(if active {
            tk.row_active
        } else {
            Color::TRANSPARENT
        })
        .border(1.0, Color::TRANSPARENT)
        .border_layout()
        .hover(tk.row_hover)
        .role("button")
        .label(label.to_owned())
        .help(label.to_owned())
        .focusable(true)
        .child(ic(
            glyph,
            tk.r(1.0),
            if active {
                tk.sidebar_fg
            } else {
                tk.sidebar_muted_fg
            },
        ))
}

fn back_label(forward: bool) -> String {
    let shortcut = match (cfg!(target_os = "macos"), forward) {
        (true, false) => "⌘[",
        (true, true) => "⌘]",
        (false, false) => "Alt+Left",
        (false, true) => "Alt+Right",
    };
    let key = if forward {
        "chrome:shell.forwardTitle"
    } else {
        "chrome:shell.backTitle"
    };
    tr(key, &[("shortcut", shortcut)])
}

/// The pre-alpha tag, Omarchy-style: a solid accent block with "ALPHA" cut out of it in the
/// wordmark's pixel letters (the owner's pick of six variants). `cell` points per pixel.
pub(crate) fn alpha_block(tk: &Tk, cell: f32) -> El {
    row()
        .padding(2.0 * cell)
        .radius(cell)
        .fill(tk.kb_accent)
        .child(super::wordmark::alpha_mark(cell, tk.chrome))
}

/// The pre-alpha chip beside the sidebar's wordmark (an owner request; the web puts it in the
/// title bar). `folded`: only a small accent dot on the K, opening the same popover.
pub(crate) fn alpha_chip(app: &App, tk: &Tk, layer: &mut Vec<El>, folded: bool) -> El {
    let chip = if folded {
        row()
            .key("sh-alpha")
            .size(tk.r(0.75), tk.r(0.75))
            .center()
            .radius(tk.r(0.375))
            .role("button")
            .label(t("chrome:preAlpha.label"))
            .help(t("chrome:preAlpha.label"))
            .focusable(true)
            .on_press(|s: &mut App| s.shell.toggle(Popup::Alpha))
            .child(crate::ui::dot(tk.r(0.4), tk.kb_accent))
    } else {
        let tag = alpha_block(tk, 1.0);
        row()
            .key("sh-alpha")
            .padding(tk.r(0.125))
            .radius(tk.r(0.25))
            .hover(tk.kb_accent.alpha(0.10))
            .role("button")
            .label(t("chrome:preAlpha.label"))
            .help(t("chrome:preAlpha.label"))
            .focusable(true)
            .focus_ring(2.0, 0.0, tk.ring)
            .on_press(|s: &mut App| s.shell.toggle(Popup::Alpha))
            .child(tag)
    };
    if app.shell.popup == Some(Popup::Alpha) {
        // PopoverContent `w-72`: the title and body as the glass header, then the hint and an
        // outline `xs` Hide button side by side (`items-end justify-between gap-3`).
        let body = col()
            .w(tk.r(18.0))
            .role("dialog")
            .label(t("chrome:preAlpha.title"))
            .child(super::popover_header(
                tk,
                &t("chrome:preAlpha.title"),
                Some(&t("chrome:preAlpha.body")),
                None,
            ))
            .child(
                row()
                    .w_full()
                    .px(tk.r(1.0))
                    .pt(tk.r(0.75))
                    .pb(tk.r(1.0))
                    .gap(tk.r(0.75))
                    .items_end()
                    .justify_between()
                    .child(
                        text(t("chrome:preAlpha.hideHint"))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .wrap()
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(
                        crate::ui::button_xs_outline(
                            tk,
                            "sh-alpha-hide",
                            &t("chrome:preAlpha.hide"),
                        )
                        .on_press(|s: &mut App| {
                            s.prefs.alpha_visible = false;
                            s.shell.popup = None;
                        }),
                    ),
            );
        super::mount_popup_as(
            tk,
            layer,
            "sh-alpha-pop",
            "sh-alpha",
            Placement::BottomStart,
            body,
            false,
        );
    }
    chip
}

/// The book's freshness (BookFreshnessIndicator's states): what's updating, failed or held back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Freshness {
    Syncing,
    Processing,
    Failed,
    NeedsJournals,
    Current,
}

pub(crate) fn freshness_state(app: &App) -> Freshness {
    let sources_failed = app
        .read("ui.freshness.status")
        .and_then(|f| f["sources"].as_array())
        .is_some_and(|s| {
            s.iter()
                .any(|x| x["status"] == "failed" || x["blocking_reports"] == true)
        });
    let stale = app
        .badges
        .as_ref()
        .is_some_and(|b| b.journals_needs_processing)
        || app.snapshot().is_some_and(|s| s.needs_journals());
    if app.busy.syncing {
        Freshness::Syncing
    } else if app.busy.processing {
        Freshness::Processing
    } else if app.shell.sync_failed || sources_failed {
        Freshness::Failed
    } else if stale {
        Freshness::NeedsJournals
    } else {
        Freshness::Current
    }
}

/// The state's dot colour, label and whether the dot pulses.
fn freshness_look(tk: &Tk, f: Freshness) -> (Color, String, bool) {
    let (color, key, pulse) = match f {
        Freshness::Syncing => (tk.sky.solid, "chrome:shell.freshness.syncing", true),
        Freshness::Processing => (tk.sky.solid, "chrome:shell.freshness.processing", true),
        Freshness::Failed => (tk.alert.solid, "chrome:shell.freshness.failed", false),
        Freshness::NeedsJournals => (tk.warn.solid, "chrome:shell.freshness.needsJournals", false),
        Freshness::Current => (tk.good.solid, "chrome:shell.freshness.current", false),
    };
    (color, t(key), pulse)
}

fn freshness_dot(tk: &Tk, color: Color, pulse: bool) -> El {
    let d = dot(tk.r(0.375), color).a11y_hidden();
    if pulse {
        d.looping(LoopSpec::opacity(1.2, 0.0, 0.35, 1.0))
    } else {
        d
    }
}

/// The title bar's book status, only when it's not OK (owner feedback: no standing "Up to date"):
/// syncing (with the running step), processing, a failed refresh or source, journals waiting.
/// Up to date draws nothing; the refresh button's tooltip and the refresh menu's header still say
/// so, and a zero-size live region keeps announcing it to assistive technology.
fn freshness(app: &App, tk: &Tk) -> El {
    let f = freshness_state(app);
    let (color, label, pulse) = freshness_look(tk, f);
    if f == Freshness::Current {
        return el()
            .key("sh-freshness")
            .size(0.0, 0.0)
            .role("status")
            .label(label);
    }
    let detail = match (f, &app.busy.progress) {
        (Freshness::Syncing, Some(p)) => Some(p.clone()),
        _ => None,
    };
    let spoken = match &detail {
        Some(d) => format!("{label} · {d}"),
        None => label.clone(),
    };
    let mut r = row()
        .key("sh-freshness")
        .min_w(0.0)
        .px(tk.r(0.5))
        .gap(tk.r(0.375))
        .center_y()
        .role("status")
        .label(spoken.clone())
        .help(format!("{} · {spoken}", t("chrome:shell.freshness.label")))
        .child(freshness_dot(tk, color, pulse))
        .child(text(label).font(tk.txs(400.0)).color(tk.muted_fg));
    if let Some(d) = detail {
        r = r.child(
            text(format!("· {d}"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg.alpha(0.8))
                .max_w(tk.r(16.0))
                .truncate(),
        );
    }
    r
}

fn refresh_split(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let open = app.shell.popup == Some(Popup::RefreshMenu);
    let mut spin = ic(&I::REFRESH_CW, tk.r(1.0), tk.sidebar_muted_fg);
    if app.busy.syncing {
        spin = spin.looping(LoopSpec::rotation(1.0));
    }
    let left = row()
        .key("sh-refresh")
        .size(tk.r(2.0), tk.r(2.0))
        .center()
        .corners([tk.r(0.375), 0.0, 0.0, tk.r(0.375)])
        .hover(tk.row_hover)
        .role("button")
        .label(t("chrome:shell.refresh"))
        // The freshness lives here while it's fine: "Up to date · Refresh book set (⌘R)".
        .help(format!(
            "{} · {}",
            freshness_look(tk, freshness_state(app)).1,
            tr(
                "chrome:shell.refreshTitle",
                &[("shortcut", &shortcut("R", false))]
            )
        ))
        .focusable(true)
        .busy(app.busy.syncing)
        .on_press(|s: &mut App| s.refresh_book_from_title_bar(false))
        .child(spin);
    let caret = row()
        .key("sh-refresh-caret")
        .w(tk.r(1.25))
        .h(tk.r(2.0))
        .center()
        .corners([0.0, tk.r(0.375), tk.r(0.375), 0.0])
        .border_sides([0.0, 0.0, 0.0, 1.0], tk.border.alpha(tk.border.a * 0.6))
        .border_layout()
        .fill(if open {
            tk.row_active
        } else {
            Color::TRANSPARENT
        })
        .hover(tk.row_hover)
        .role("button")
        .label(t("chrome:shell.refreshMenu.options"))
        .help(t("chrome:shell.refreshMenu.options"))
        .focusable(true)
        .child(ic(&I::CHEVRON_DOWN, tk.r(0.75), tk.sidebar_muted_fg))
        .menu_trigger(open, |s: &mut App| s.shell.toggle(Popup::RefreshMenu));
    if open {
        let processing = app.busy.processing;
        let mut p = Popup_::menu("sh-refresh-caret", |s: &mut App| s.shell.popup = None)
            .list_key("sh-refresh-menu-surface");
        let all_l = t("chrome:shell.refresh");
        let all = p.prepare(
            menu_row(
                tk,
                "sh-refresh-all",
                Some(&I::REFRESH_CW),
                &all_l,
                Some(&shortcut("R", false)),
            )
            .on_press(|s: &mut App| {
                s.shell.popup = None;
                s.refresh_book_from_title_bar(false);
            }),
            "sh-refresh-all",
            all_l,
            true,
            false,
        );
        let rep_l = t("chrome:shell.refreshMenu.reprocessJournals");
        let mut reprocess = menu_row(
            tk,
            "sh-refresh-journals",
            Some(&I::CLIPBOARD_LIST),
            &rep_l,
            Some(&shortcut("J", true)),
        );
        reprocess = if processing {
            reprocess.disabled(true).opacity(0.5)
        } else {
            reprocess.on_press(|s: &mut App| {
                s.shell.popup = None;
                s.process_journals();
            })
        };
        let reprocess = p.prepare(reprocess, "sh-refresh-journals", rep_l, !processing, false);
        let res_l = t("chrome:shell.refreshMenu.fullRescan");
        let rescan = p.prepare(
            menu_row(
                tk,
                "sh-refresh-rescan",
                Some(&I::ROTATE_CCW),
                &res_l,
                Some(&shortcut("R", true)),
            )
            .on_press(|s: &mut App| {
                s.shell.popup = None;
                s.refresh_book_from_title_bar(true);
            }),
            "sh-refresh-rescan",
            res_l,
            true,
            false,
        );
        let menu = col()
            .w(tk.r(15.0))
            .padding(super::menu_pad(tk))
            .child(refresh_status(app, tk))
            .child(menu_sep(tk))
            .child(all)
            .child(reprocess)
            .child(rescan);
        let (menu, keys) = p.finish_custom(menu);
        super::mount_menu_at(
            tk,
            layer,
            "sh-refresh-menu",
            "sh-refresh-group",
            Placement::BottomEnd,
            menu,
            keys,
            true,
            4.0,
        );
    }
    row()
        .key("sh-refresh-group")
        .center_y()
        .child(left)
        .child(caret)
}

/// The refresh menu's first line: the book status, always (the menu is opened on demand, so
/// saying "Up to date" here isn't standing chrome), with the running step while syncing.
fn refresh_status(app: &App, tk: &Tk) -> El {
    let f = freshness_state(app);
    let (color, label, pulse) = freshness_look(tk, f);
    let detail = if f == Freshness::Syncing {
        app.busy.progress.clone()
    } else {
        None
    };
    let mut texts = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(tk.r(0.125))
        .child(
            text(t("chrome:shell.freshness.label"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        )
        .child(
            text(label.clone())
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .truncate(),
        );
    if let Some(d) = &detail {
        texts = texts.child(
            text(d.clone())
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    row()
        .key("sh-refresh-status")
        .w_full()
        .px(tk.r(0.5))
        .py(tk.r(0.375))
        .gap(tk.r(0.5))
        .role("status")
        .label(match &detail {
            Some(d) => format!("{} {label} · {d}", t("chrome:shell.freshness.label")),
            None => format!("{} {label}", t("chrome:shell.freshness.label")),
        })
        .items_start()
        // The dot sits on the state's line (under the `text-xs` caption and the `gap-0.5`).
        .child(
            col().w(tk.r(1.0)).pt(tk.r(1.125)).child(
                row()
                    .w_full()
                    .h(tk.r(1.25))
                    .center()
                    .child(freshness_dot(tk, color, pulse)),
            ),
        )
        .child(texts)
}

fn currency_toggle(app: &App, tk: &Tk) -> El {
    let fiat_symbol = crate::format::symbol(&crate::format::book_currency(app));
    let btc = app.prefs.currency == Currency::Btc;
    let (bitcoin, euro) = (
        t("chrome:currencyToggle.bitcoin"),
        t("chrome:currencyToggle.euro"),
    );
    let (glyph, current, next) = if btc {
        ("₿".to_owned(), bitcoin, euro)
    } else {
        (fiat_symbol, euro, bitcoin)
    };
    row()
        .key("sh-currency")
        .size(tk.r(2.0), tk.r(2.0))
        .center()
        .radius(tk.r(0.375))
        .hover(tk.row_hover)
        .role("button")
        .checked(btc)
        .label(tr(
            "chrome:currencyToggle.label",
            &[("current", &current), ("next", &next)],
        ))
        .help(tr(
            "chrome:currencyToggle.title",
            &[("current", &current), ("next", &next)],
        ))
        .focusable(true)
        .on_press(|s: &mut App| {
            s.prefs.currency = if s.prefs.currency == Currency::Btc {
                Currency::Fiat
            } else {
                Currency::Btc
            };
        })
        .child(text(glyph).font(tk.tsm(600.0)).color(tk.sidebar_fg))
}

fn theme_menu(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let open = app.shell.popup == Some(Popup::Theme);
    let glyph = match app.prefs.theme {
        ThemeMode::Dark => &I::MOON,
        ThemeMode::Light => &I::SUN,
        ThemeMode::System => &I::SUN_MOON,
    };
    let b = bar_button(tk, "sh-theme", glyph, &t("chrome:theme.label"), false)
        .menu_trigger(open, |s: &mut App| s.shell.toggle(Popup::Theme));
    if open {
        let mut p = Popup_::menu("sh-theme", |s: &mut App| s.shell.popup = None)
            .list_key("sh-theme-menu-surface");
        let mut item = |mode: ThemeMode, key: &str, glyph: &'static Icon| {
            let label = t(key);
            let row = check_row(
                tk,
                format!("sh-theme-{mode:?}"),
                app.prefs.theme == mode,
                true,
                Some(glyph),
                &label,
                None,
            )
            .on_press(move |s: &mut App| {
                s.prefs.theme = mode;
                s.shell.popup = None;
            });
            p.prepare(
                row,
                format!("sh-theme-{mode:?}"),
                label,
                true,
                app.prefs.theme == mode,
            )
        };
        let (i1, i2, i3) = (
            item(ThemeMode::System, "chrome:theme.system", &I::SUN_MOON),
            item(ThemeMode::Light, "chrome:theme.light", &I::SUN),
            item(ThemeMode::Dark, "chrome:theme.dark", &I::MOON),
        );
        let menu = col()
            .w(tk.r(11.0))
            .label(t("chrome:theme.label"))
            .child(super::popover_header(
                tk,
                &t("chrome:theme.label"),
                None,
                None,
            ))
            .child(
                col()
                    .w_full()
                    .padding(super::menu_pad(tk))
                    .child(i1)
                    .child(i2)
                    .child(i3),
            );
        let (menu, keys) = p.finish_custom(menu);
        super::mount_menu_at(
            tk,
            layer,
            "sh-theme-menu",
            "sh-theme",
            Placement::BottomEnd,
            menu,
            keys,
            true,
            4.0,
        );
    }
    b
}

pub(crate) fn privacy_label(p: Privacy) -> String {
    t(match p {
        Privacy::Normal => "chrome:sensitive.modes.normal",
        Privacy::HideAmounts => "chrome:sensitive.modes.hideAmounts",
        Privacy::HideAll => "chrome:sensitive.modes.hideAll",
    })
}

fn privacy_menu(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let open = app.shell.popup == Some(Popup::Privacy);
    let glyph = match app.prefs.privacy {
        Privacy::Normal => &I::EYE,
        Privacy::HideAmounts => &I::EYE_OFF,
        Privacy::HideAll => &I::EYE_CLOSED,
    };
    let label = tr(
        "chrome:sensitive.trigger",
        &[("current", &privacy_label(app.prefs.privacy))],
    );
    let b = bar_button(
        tk,
        "sh-privacy",
        glyph,
        &label,
        app.prefs.privacy != Privacy::Normal,
    );
    // `DropdownMenu modal={false}` (PrivacyModeButton.tsx): the page stays live.
    let b = crate::ui::popup::trigger_nonmodal(
        b,
        crate::ui::popup::Kind::Menu,
        open,
        "sh-privacy-menu-surface",
        "sh-privacy-Normal",
        |s: &mut App| s.shell.toggle(Popup::Privacy),
    );
    if open {
        let mut p = Popup_::menu("sh-privacy", |s: &mut App| s.shell.popup = None)
            .list_key("sh-privacy-menu-surface");
        let mut item = |mode: Privacy, detail_key: &str| {
            let detail = t(detail_key);
            let row = check_row(
                tk,
                format!("sh-privacy-{mode:?}"),
                app.prefs.privacy == mode,
                true,
                None,
                &privacy_label(mode),
                Some(&detail),
            )
            .on_press(move |s: &mut App| {
                s.prefs.privacy = mode;
                s.shell.popup = None;
            });
            p.prepare(
                row,
                format!("sh-privacy-{mode:?}"),
                privacy_label(mode),
                true,
                app.prefs.privacy == mode,
            )
        };
        let (i1, i2, i3) = (
            item(Privacy::Normal, "chrome:sensitive.descriptions.normal"),
            item(
                Privacy::HideAmounts,
                "chrome:sensitive.descriptions.hideAmounts",
            ),
            item(Privacy::HideAll, "chrome:sensitive.descriptions.hideAll"),
        );
        let unlock_l = t("chrome:sensitive.hideOnUnlock");
        let unlock = p.prepare(
            check_row(
                tk,
                "sh-privacy-unlock",
                app.prefs.hide_on_unlock,
                false,
                None,
                &unlock_l,
                None,
            )
            .on_press(|s: &mut App| s.prefs.hide_on_unlock = !s.prefs.hide_on_unlock),
            "sh-privacy-unlock",
            unlock_l,
            true,
            false,
        );
        let menu = col()
            .w(tk.r(18.0))
            .label(t("chrome:sensitive.menuTitle"))
            // The label's shortcut is literally `⌘⇧H` in the web, on every platform.
            .child(super::popover_header(
                tk,
                &t("chrome:sensitive.menuTitle"),
                None,
                Some(super::menu_shortcut(tk, "⌘⇧H")),
            ))
            .child(
                col()
                    .w_full()
                    .padding(super::menu_pad(tk))
                    .child(i1)
                    .child(i2)
                    .child(i3)
                    .child(menu_sep(tk))
                    .child(unlock),
            );
        let (menu, keys) = p.finish_custom(menu);
        super::mount_menu_at(
            tk,
            layer,
            "sh-privacy-menu",
            "sh-privacy",
            Placement::BottomEnd,
            menu,
            keys,
            false,
            4.0,
        );
    }
    b
}

/// Locked: the title bar keeps only the Alpha chip, right-aligned.
fn locked_bar(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    // Empty while locked, but still the window's drag region: with drag-regions-only chrome
    // (`main.rs`), a bar without `.window_drag()` cannot move the window.
    let _ = (app, layer);
    row()
        .key("titlebar")
        .w_full()
        .h(40.0)
        .fill(tk.chrome)
        .window_drag()
}

pub fn view(app: &App, tk: &Tk, folded: bool, narrow: bool, layer: &mut Vec<El>) -> El {
    if app.locked {
        return locked_bar(app, tk, layer);
    }
    // macOS keeps the traffic lights' 80 pt free, except in full screen, where the lights are gone
    // and the bar's controls move to the edge (as T3 Code does).
    let leading_pad = if cfg!(target_os = "macos") && !full_screen() {
        80.0_f32.max(tk.r(0.5))
    } else {
        tk.r(0.5)
    };
    let leading_w = if folded {
        leading_pad + tk.r(6.75)
    } else {
        tk.r(16.0)
    };
    let leading = row()
        .key("sh-leading")
        .w(leading_w)
        .pl(leading_pad)
        .pr(tk.r(0.5))
        .gap(tk.r(0.125))
        .center_y()
        .child(
            bar_button(
                tk,
                "sh-toggle-nav",
                &I::PANEL_LEFT,
                &tr(
                    "chrome:shell.toggleSidebarTitle",
                    &[("shortcut", &shortcut("B", false))],
                ),
                false,
            )
            .label(t("chrome:shell.toggleSidebar"))
            .on_press(move |s: &mut App| super::toggle_nav(s, narrow)),
        )
        // Like `window.history.back()`, the buttons never dim; the name is plain, the title
        // carries the shortcut.
        .child(
            bar_button(tk, "sh-back", &I::ARROW_LEFT, &back_label(false), false)
                .label(t("chrome:shell.back"))
                .on_press(|s: &mut App| s.go_back()),
        )
        .child(
            bar_button(tk, "sh-forward", &I::ARROW_RIGHT, &back_label(true), false)
                .label(t("chrome:shell.forward"))
                .on_press(|s: &mut App| s.go_forward()),
        );

    let crumbs = row()
        .key("sh-crumbs")
        .grow(1.0)
        .min_w(0.0)
        .pl(tk.r(1.25))
        .gap(tk.r(0.75))
        .center_y()
        // BreadcrumbBook: the crumb button and its `/` in a `gap-2` span.
        .child(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .child(books::crumb(app, tk, layer))
                .child(
                    text("/")
                        .font(tk.tbase(400.0))
                        .color(tk.sidebar_muted_fg.alpha(0.4))
                        .a11y_hidden(),
                ),
        )
        .child(
            text(page_title(app.route.path()))
                .font(tk.tsm(500.0))
                .color(tk.sidebar_fg)
                .role("heading")
                .level(1)
                .truncate(),
        );

    let mut right = row()
        .key("sh-right")
        .gap(tk.r(0.125))
        .pl(tk.r(0.5))
        .pr(tk.r(0.5))
        .center_y()
        .child(freshness(app, tk))
        .child(refresh_split(app, tk, layer))
        .child(notifications::button(app, tk, layer))
        .child(currency_toggle(app, tk))
        .child(theme_menu(app, tk, layer))
        .child(privacy_menu(app, tk, layer))
        .child(network::button(app, tk, layer))
        .child(
            bar_button(
                tk,
                "sh-lock",
                &I::LOCK_KEYHOLE,
                &tr(
                    "chrome:shell.lockKassiberTitle",
                    &[("shortcut", &shortcut("L", false))],
                ),
                false,
            )
            .label(t("chrome:shell.lockKassiber"))
            .on_press(|s: &mut App| super::overlays::lock(s)),
        );
    if crate::assistant::toggle_available(app) {
        let open = crate::assistant::panel_shown(app);
        right = right.child(
            bar_button(
                tk,
                "sh-side-panel",
                if open {
                    &I::PANEL_RIGHT_CLOSE
                } else {
                    &I::PANEL_RIGHT_OPEN
                },
                &t("assistant:panel.toggle"),
                open,
            )
            .on_press(crate::assistant::toggle_side_panel),
        );
    }

    row()
        .key("titlebar")
        .w_full()
        .h(40.0)
        .center_y()
        .fill(tk.chrome)
        .window_drag()
        .child(leading)
        .child(crumbs)
        .child(right)
        .child(el().size(0.0, 0.0))
}

/// Whether the main window is in full screen (no traffic lights to leave room for). Read from the
/// live window: entering or leaving full screen resizes it, which redraws the bar.
fn full_screen() -> bool {
    crate::persist::window::handle()
        .and_then(|h| h.placement())
        .is_some_and(|p| p.full_screen)
}
