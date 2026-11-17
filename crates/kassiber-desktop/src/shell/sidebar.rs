//! The sidebar (AppShell.tsx `AppSidebar`, `SidebarActions`, `NavUser`): brand, search row,
//! the Main group, footer actions, the book-set menu and the version line. Folded it is a
//! 3 rem icon rail whose rows show their names as tooltips.

use rune::{Color, El, Icon, ScrollSpec, col, el, row, text};
use rune_kit::Placement;

use super::{DEV_HIDDEN, DEV_LOCKED, MAIN_NAV, Popup, go, menu_sep, wordmark};
use crate::app::App;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::popup::TriggerExt;
use crate::ui::{dot, ic};

/// Support rows: (i18n key of the label, icon, URL).
pub const SUPPORT_LINKS: &[(&str, &Icon, &str)] = &[
    (
        "chrome:shell.support.bugReport",
        &I::BUG,
        "https://github.com/bitcoinaustria/kassiber/issues/new?template=bug_report.yml",
    ),
    (
        "chrome:shell.support.discussions",
        &I::HEART,
        "https://github.com/bitcoinaustria/kassiber/discussions",
    ),
];

/// Extras rows: (i18n key of the label, icon, route); `None` is the separator.
pub const EXTRAS: &[Option<(&str, &Icon, &str)>] = &[
    Some(("nav:book.ledger", &I::BOOK_OPEN, "/journals")),
    Some(("nav:book.reconcile", &I::FINGERPRINT, "/reconcile")),
    Some(("nav:book.sourceFunds", &I::BADGE_CHECK, "/source-of-funds")),
    None,
    Some((
        "chrome:shell.extras.exitCalculator",
        &I::LOG_OUT,
        "/exit-tax",
    )),
    Some((
        "chrome:shell.extras.privacyMirror",
        &I::EYE,
        "/privacy-mirror",
    )),
    Some((
        "chrome:shell.extras.chainAnalysis",
        &I::NETWORK,
        "/chain-analysis",
    )),
    Some(("chrome:shell.extras.egress", &I::PLANE, "/egress")),
];

/// `extrasActive`: arriving on a page that lives under Extras (a pill, a link on Reports, first
/// load) opens the fold, so the current page shows in the nav. Called on every route change.
pub fn reveal_route(app: &mut App) {
    let path = app.route.path().to_owned();
    let under_extras = EXTRAS
        .iter()
        .flatten()
        .any(|(_, _, href)| path == *href || path.starts_with(&format!("{href}/")));
    if under_extras {
        app.shell.extras_open = true;
    }
}

/// `{label} — {nav:devLocked}` (AppShell `lockRow`).
fn dev_locked_title(label: &str) -> String {
    format!("{label} — {}", t("nav:devLocked"))
}

/// `⌘K` on macOS, `Ctrl+K` elsewhere (with Shift: `⌘⇧A` / `Ctrl+Shift+A`).
pub fn shortcut(key: &str, shift: bool) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{}{key}", if shift { "⇧" } else { "" })
    } else {
        format!("Ctrl+{}{key}", if shift { "Shift+" } else { "" })
    }
}

fn is_active(path: &str, href: &str) -> bool {
    path == href || path.starts_with(&format!("{href}/"))
}

/// A main nav row (`h-8 gap-2 rounded-md p-2 text-sm font-medium`), or a 2 rem square folded.
fn nav_button(
    tk: &Tk,
    key: String,
    glyph: &'static Icon,
    label: &str,
    active: bool,
    folded: bool,
    locked: bool,
) -> El {
    let ink = if active {
        tk.sidebar_fg
    } else {
        tk.sidebar_muted_fg
    };
    let mut r = row()
        .key(key)
        .h(tk.r(2.0))
        .center_y()
        .radius(tk.r(0.375))
        .fill(if active {
            tk.row_active
        } else {
            Color::TRANSPARENT
        })
        .role("link")
        .label(label.to_owned())
        .focusable(!locked);
    r = if folded {
        r.w(tk.r(2.0)).justify_center()
    } else {
        r.w_full().px(tk.r(0.5)).gap(tk.r(0.5))
    };
    if !locked {
        r = r.hover(tk.row_hover);
    } else {
        r = r.opacity(0.5).disabled(true);
    }
    r = r.child(ic(glyph, tk.r(1.0), ink));
    if !folded {
        r = r.child(
            text(label.to_owned())
                .font(tk.tsm(500.0))
                .color(ink)
                .grow(1.0)
                .truncate(),
        );
    } else {
        r = r.help(label.to_owned()).help_side(rune::Side::Right);
    }
    r
}

fn main_row(app: &App, tk: &Tk, item: &super::NavItem, folded: bool) -> El {
    let active = is_active(app.route.path(), item.href);
    let href = item.href;
    let mut r = nav_button(
        tk,
        format!("sh-nav{}", item.href),
        item.icon,
        &item.text(),
        active,
        folded,
        false,
    )
    .on_press(move |s: &mut App| go(s, href));
    if item.href == "/review"
        && let Some(b) = &app.badges
    {
        let total = b.total();
        let blocker = b.quarantine.unwrap_or(0.0) > 0.0;
        let tone = if blocker { &tk.alert } else { &tk.warn };
        if total > 0.0 {
            let label = if total > 99.0 {
                "99+".to_owned()
            } else {
                format!("{}", total as usize)
            };
            let aria = tc("nav:badge.review", total as usize, &[]);
            r = if folded {
                r.child(
                    dot(tk.r(0.5), tone.text)
                        .border(2.0, tk.chrome)
                        .absolute(tk.r(1.25), tk.r(0.25))
                        .label(aria),
                )
            } else {
                r.child(
                    row()
                        .h(tk.r(1.25))
                        .min_w(tk.r(1.25))
                        .px(tk.r(0.25))
                        .center()
                        .radius(tk.r(0.375))
                        .fill(if blocker {
                            tk.alert.solid.alpha(0.15)
                        } else {
                            tk.warn.solid.alpha(0.15)
                        })
                        .label(aria)
                        .child(
                            text(label)
                                .font(tk.txs(500.0).features("'tnum' 1"))
                                .color(tone.text),
                        ),
                )
            };
        } else if b.journals_needs_processing {
            let d = dot(tk.r(0.5), tk.warn.text).label(t("nav:badge.journals"));
            r = if folded {
                r.child(d.border(2.0, tk.chrome).absolute(tk.r(1.25), tk.r(0.25)))
            } else {
                r.child(d.mr(tk.r(0.25)))
            };
        }
    }
    r
}

/// A footer action row; dev-locked rows stay visible but inert with an explanatory title.
fn action_row(
    app: &App,
    tk: &Tk,
    label: &str,
    glyph: &'static Icon,
    href: &'static str,
    folded: bool,
) -> El {
    let locked = DEV_LOCKED.contains(&href) && !app.prefs.developer_tools;
    let active = is_active(app.route.path(), href);
    let r = nav_button(
        tk,
        format!("sh-act{href}"),
        glyph,
        label,
        active,
        folded,
        locked,
    );
    if locked {
        r.help(dev_locked_title(label))
    } else {
        r.on_press(move |s: &mut App| go(s, href))
    }
}

/// A collapsible footer group header (Support, Extras) with its rotating chevron. Folded, a
/// press unfolds the nav and opens the group.
fn group_header(
    tk: &Tk,
    key: &str,
    glyph: &'static Icon,
    label: &str,
    open: bool,
    folded: bool,
    support: bool,
) -> El {
    let mut r = nav_button(tk, key.to_owned(), glyph, label, false, folded, false)
        .role("button")
        .expanded(open)
        .on_press(move |s: &mut App| {
            let flag = if support {
                &mut s.shell.support_open
            } else {
                &mut s.shell.extras_open
            };
            if folded {
                *flag = true;
                if s.prefs.nav_collapsed {
                    s.prefs.nav_collapsed = false;
                } else {
                    s.shell.nav_peek = true;
                }
            } else {
                *flag = !*flag;
            }
        });
    if !folded {
        r = r.child(
            ic(&I::CHEVRON_RIGHT, tk.r(0.875), tk.sidebar_muted_fg).icon_rotation(if open {
                std::f32::consts::FRAC_PI_2
            } else {
                0.0
            }),
        );
    }
    r
}

/// The indented sub-list (`mx-3.5 border-l px-2.5 py-0.5 gap-1`).
fn sub_list(tk: &Tk, rows: Vec<El>) -> El {
    col()
        .mx(tk.r(0.875))
        .px(tk.r(0.625))
        .py(tk.r(0.125))
        .gap(tk.r(0.25))
        .border_sides([0.0, 0.0, 0.0, 1.0], tk.sidebar_border)
        .border_layout()
        .children(rows)
}

/// A sub row (`h-7 px-2 rounded-md text-sm`, 0.875 rem icons).
fn sub_row(
    app: &App,
    tk: &Tk,
    key: String,
    glyph: &'static Icon,
    label: &str,
    href: Option<&'static str>,
) -> El {
    let locked = href.is_some_and(|h| DEV_LOCKED.contains(&h)) && !app.prefs.developer_tools;
    let active = href.is_some_and(|h| is_active(app.route.path(), h));
    let ink = if active {
        tk.sidebar_fg
    } else {
        tk.sidebar_muted_fg
    };
    let mut r = row()
        .key(key)
        .w_full()
        .h(tk.r(1.75))
        .px(tk.r(0.5))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.375))
        .fill(if active {
            tk.row_active
        } else {
            Color::TRANSPARENT
        })
        .role("link")
        .label(label.to_owned())
        .child(ic(glyph, tk.r(0.875), ink))
        .child(
            text(label.to_owned())
                .font(tk.tsm(400.0))
                .color(ink)
                .truncate(),
        );
    if locked {
        r = r.opacity(0.5).disabled(true).help(dev_locked_title(label));
    } else {
        r = r.hover(tk.row_hover).focusable(true);
        if let Some(h) = href {
            r = r.on_press(move |s: &mut App| go(s, h));
        }
    }
    r
}

fn initials(name: &str) -> String {
    let mut out: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect();
    if out.chars().count() < 2 {
        out = name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .collect();
    }
    out.to_uppercase()
}

fn nav_user(app: &App, tk: &Tk, folded: bool, layer: &mut Vec<El>) -> El {
    let status = app.snapshot().and_then(|s| s.status.clone());
    let name = status
        .as_ref()
        .and_then(|s| s.workspace.clone())
        .unwrap_or_else(|| t("chrome:shell.user.fallbackWorkspace"));
    let detail = status
        .as_ref()
        .and_then(|s| s.profile.clone())
        .unwrap_or_else(|| t("chrome:shell.user.fallbackProfile"));
    let avatar = |size: f32| {
        row()
            .size(size, size)
            .center()
            .radius(tk.r(0.5))
            .fill(tk.row_active)
            // AvatarFallback `rounded-lg bg-sidebar-row-active text-sm font-medium`.
            .child(
                text(initials(&name))
                    .font(tk.tsm(500.0))
                    .color(tk.sidebar_fg),
            )
    };
    let open = app.shell.popup == Some(Popup::UserMenu);
    let mut trigger = row()
        .key("sh-user")
        .center_y()
        .radius(tk.r(0.375))
        .fill(if open {
            tk.row_active
        } else {
            Color::TRANSPARENT
        })
        .hover(tk.row_hover)
        .role("button")
        .label(tr(
            "chrome:shell.user.menuLabel",
            &[("name", &name), ("detail", &detail)],
        ))
        .help(tr(
            "chrome:shell.user.menuLabel",
            &[("name", &name), ("detail", &detail)],
        ))
        .focusable(true)
        .menu_trigger(open, |s: &mut App| s.shell.toggle(Popup::UserMenu));
    trigger = if folded {
        trigger
            .size(tk.r(2.25), tk.r(2.25))
            .justify_center()
            .child(avatar(tk.r(2.25)))
    } else {
        trigger
            .w_full()
            .h(tk.r(3.0))
            .px(tk.r(0.5))
            .gap(tk.r(0.5))
            .child(avatar(tk.r(2.0)))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    // `grid text-sm leading-tight`: the name takes the 1.25 line height; the
                    // `text-xs` detail sets its own (1 rem).
                    .child(
                        text(name.clone())
                            .font(tk.tsm(500.0).lh(tk.r(0.875) * 1.25))
                            .color(tk.sidebar_fg)
                            .truncate(),
                    )
                    .child(
                        text(detail.clone())
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .truncate(),
                    ),
            )
            .child(ic(&I::CHEVRONS_UP_DOWN, tk.r(1.0), tk.sidebar_fg))
    };
    if open {
        // `w-(--radix-dropdown-menu-trigger-width) min-w-56 rounded-lg`; the header label is
        // `p-0 font-normal` around `flex items-center gap-2 px-1 py-1.5 text-sm`, the avatar the
        // default (`bg-muted`) fallback in `rounded-lg`.
        let trigger_w = if folded {
            tk.r(14.0)
        } else {
            (tk.r(16.0) - 2.0 * tk.r(0.5)).max(tk.r(14.0))
        };
        let mut pp = crate::ui::popup::Popup::menu("sh-user", |s: &mut App| s.shell.popup = None)
            .list_key("sh-user-menu-surface");
        let mut menu = col()
            .w(trigger_w)
            .padding(super::menu_pad(tk))
            .child(
                row()
                    .w_full()
                    .px(tk.r(0.25))
                    .py(tk.r(0.375))
                    .gap(tk.r(0.5))
                    .center_y()
                    .child(
                        row()
                            .size(tk.r(2.0), tk.r(2.0))
                            .center()
                            .radius(tk.r(0.5))
                            .fill(tk.muted)
                            .child(text(initials(&name)).font(tk.tsm(400.0)).color(tk.fg)),
                    )
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .child(
                                text(name.clone())
                                    .font(tk.tsm(500.0).lh(tk.r(0.875) * 1.25))
                                    .color(tk.fg)
                                    .truncate(),
                            )
                            .child(
                                text(detail.clone())
                                    .font(tk.txs(400.0))
                                    .color(tk.muted_fg)
                                    .truncate(),
                            ),
                    ),
            )
            .child(menu_sep(tk));
        let projects = app
            .read("ui.projects.list")
            .and_then(|p| p["projects"].as_array())
            .cloned()
            .unwrap_or_default();
        if !projects.is_empty() {
            // `px-2 py-1.5 text-xs font-medium text-muted-foreground`
            menu = menu.child(
                row().w_full().px(tk.r(0.5)).py(tk.r(0.375)).child(
                    text(t("chrome:projects.menuLabel"))
                        .font(tk.txs(500.0))
                        .color(tk.muted_fg),
                ),
            );
            for p in projects {
                let id = p["id"].as_str().unwrap_or_default().to_owned();
                let pname = p["name"].as_str().unwrap_or(&id).to_owned();
                let current = p["selected"].as_bool().unwrap_or(false);
                let key = format!("sh-project-{id}");
                let mut r = super::menu_row_spaced(tk, key.clone(), &I::DATABASE, &pname, None);
                if current {
                    r = r
                        .child(ic(&I::BADGE_CHECK, tk.r(1.0), tk.primary).ml(tk.r(0.5)))
                        .opacity(0.5);
                } else {
                    r = r.on_press(move |s: &mut App| super::books::select_project(s, &id));
                }
                menu = menu.child(pp.prepare(r, key, pname, !current, false));
            }
            menu = menu.child(menu_sep(tk));
        }
        let (books_l, lock_l) = (t("chrome:shell.user.books"), t("chrome:shell.lockKassiber"));
        let books = pp.prepare(
            super::menu_row_spaced(tk, "sh-user-books", &I::USER, &books_l, None)
                .on_press(|s: &mut App| go(s, "/books")),
            "sh-user-books",
            books_l,
            true,
            false,
        );
        let lock = pp.prepare(
            super::menu_row_spaced(
                tk,
                "sh-user-lock",
                &I::LOG_OUT,
                &lock_l,
                Some(&super::sidebar::shortcut("L", false)),
            )
            .on_press(|s: &mut App| super::overlays::lock(s)),
            "sh-user-lock",
            lock_l,
            true,
            false,
        );
        menu = menu.child(books).child(menu_sep(tk)).child(lock);
        let (menu, keys) = pp.finish_custom(menu);
        super::mount_menu_at(
            tk,
            layer,
            "sh-user-menu",
            "sh-user",
            if folded {
                Placement::RightStart
            } else {
                Placement::TopEnd
            },
            menu,
            keys,
            true,
            4.0,
        );
    }
    trigger
}

fn version_line(app: &App, tk: &Tk) -> El {
    let release = app
        .daemon_ready
        .as_ref()
        .and_then(|r| r["version"].as_str())
        .map(str::to_owned);
    let commit = env!("KASSIBER_COMMIT");
    let short: String = commit.chars().take(7).collect();
    let (version, title) = match &release {
        Some(v) => (
            tr("chrome:shell.version.releaseLabel", &[("version", v)]),
            tr(
                "chrome:shell.version.releaseTitle",
                &[("version", v), ("commit", &short)],
            ),
        ),
        None => (
            t("chrome:shell.version.devLabel"),
            tr("chrome:shell.version.devTitle", &[("commit", &short)]),
        ),
    };
    // With update-check consent the line asks GitHub (`runManualAppUpdateCheck`); without it it
    // stays a plain link, so the click never reaches GitHub's API.
    let title = if crate::native::update::can_check(app) {
        tr("chrome:shell.version.checkTitle", &[("build", &title)])
    } else {
        title
    };
    // `inline-flex w-full items-center justify-center gap-1 px-2 pb-1 text-xs leading-none`; the
    // link's name is its text without the aria-hidden `·`, its title the build line.
    let leading_none = |s: rune::TextStyle| s.lh(tk.r(0.75));
    row()
        .key("sh-version")
        .w_full()
        .px(tk.r(0.5))
        .pb(tk.r(0.25))
        .center_y()
        .justify_center()
        .gap(tk.r(0.25))
        .role("link")
        .label(format!("{version} · {short}"))
        .help(title)
        .on_press_tx(|a: &mut App, tx| {
            if crate::native::update::can_check(a) {
                crate::native::update::run_manual(a);
            } else {
                let _ = tx.open_url("https://github.com/bitcoinaustria/kassiber");
            }
        })
        .child(
            text(version.clone())
                .font(leading_none(tk.txs(400.0)))
                .color(tk.muted_fg),
        )
        .child(
            text("·")
                .font(leading_none(tk.txs(400.0)))
                .color(tk.muted_fg)
                .a11y_hidden(),
        )
        .child(
            text(short.clone())
                .font(leading_none(tk.txs(400.0).mono()))
                .color(tk.muted_fg),
        )
}

pub fn view(app: &App, tk: &Tk, folded: bool, layer: &mut Vec<El>) -> El {
    let width = if folded { tk.r(3.0) } else { tk.r(16.0) };
    let pad = tk.r(0.5);

    // Header: brand and the search row.
    // SidebarBrand: an `h-8 px-2` row (folded `pl-[calc(1rem-4.5px)]`, inside the header's own
    // `px-2`) around the `h-7 rounded-md` link that holds the wordmark.
    let brand = row()
        .h(tk.r(2.0))
        .center_y()
        .pl(if folded { tk.r(1.0) - 4.5 } else { pad })
        .pr(pad)
        .child(
            row()
                .key("sh-brand")
                .h(tk.r(1.75))
                .center_y()
                .radius(tk.r(0.375))
                .role("link")
                .label(t("chrome:shell.overviewLink"))
                .focusable(true)
                .on_press(|s: &mut App| go(s, "/overview"))
                .child(wordmark::animated(
                    app,
                    wordmark::Site::Shell,
                    1.0,
                    tk.sidebar_fg,
                    folded,
                )),
        );
    let brand = if app.prefs.alpha_visible && !app.locked {
        let chip = super::titlebar::alpha_chip(app, tk, layer, folded);
        // Centred in the brand row, the 13 pt block's bottom lands on the wordmark's baseline
        // (the R's tail drops below it), as on the lock screen.
        brand
            .gap(if folded { 0.0 } else { tk.r(0.375) })
            .child(chip)
    } else {
        brand
    };
    // The trigger is named by the route's search label ("Search overview"), as `aria-label`.
    let search_name = match super::search::route_meta(app.route.path()) {
        Some(id) => t(&format!("chrome:routeMeta.{id}.label")),
        None => t("chrome:shell.fallback.label"),
    };
    let mut search = nav_button(
        tk,
        "sh-search-row".into(),
        &I::SEARCH,
        &t("chrome:shell.searchLabel"),
        app.shell.popup == Some(Popup::Search),
        folded,
        false,
    )
    .label(search_name.clone())
    .help(search_name)
    .on_press(|s: &mut App| s.shell.toggle(Popup::Search));
    if !folded {
        search = search.child(
            row()
                .h(tk.r(1.25))
                .min_w(tk.r(1.25))
                .px(tk.r(0.25))
                .center()
                .radius(tk.r(0.25))
                .fill(tk.sidebar_control)
                .child(
                    text(shortcut("K", false))
                        .font(tk.tkbd())
                        .color(tk.sidebar_muted_fg),
                ),
        );
    }
    let header = col()
        .w_full()
        .px(pad)
        .pt(tk.r(0.25))
        .pb(tk.r(0.25))
        .gap(tk.r(0.25))
        .child(brand)
        .child(search);

    // The Main group.
    let mut nav = col().w_full().gap(tk.r(0.25));
    if folded {
        nav = nav.items_center();
    }
    for item in MAIN_NAV {
        if item.href == "/assistant" && !app.prefs.ai_features {
            continue;
        }
        nav = nav.child(main_row(app, tk, item, folded));
    }
    // SidebarGroup `gap-1.5 px-2 py-1.5`; the label `mb-1 text-xs font-medium`. Folded, the label
    // stays in layout at `-mt-8` and opacity 0: with its line, margin and the group's gap that nets
    // the items up to the group's top edge, so the folded group has no top padding.
    let mut group = col().w_full().px(pad).py(tk.r(0.375)).gap(tk.r(0.375));
    if folded {
        group = group.pt(0.0);
    } else {
        group = group.child(
            text(t("nav:section.main"))
                .font(tk.txs(500.0))
                .color(tk.sidebar_muted_fg.alpha(0.8))
                .px(tk.r(0.5))
                .mb(tk.r(0.25)),
        );
    }
    let content = col()
        .key("sh-sidebar-content")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(group.child(nav));

    // Footer actions.
    let mut actions = col().w_full().gap(tk.r(0.25));
    if folded {
        actions = actions.items_center();
    }
    actions = actions.child(action_row(
        app,
        tk,
        &t("nav:book.activity"),
        &I::HISTORY,
        "/activity",
        folded,
    ));
    if app.prefs.developer_tools {
        actions = actions.child(action_row(
            app,
            tk,
            &t("nav:book.logs"),
            &I::SQUARE_TERMINAL,
            "/logs",
            folded,
        ));
    }
    actions = actions.child(group_header(
        tk,
        "sh-support",
        &I::LIFE_BUOY,
        &t("chrome:shell.support.title"),
        app.shell.support_open && !folded,
        folded,
        true,
    ));
    if app.shell.support_open && !folded {
        let rows = SUPPORT_LINKS
            .iter()
            .enumerate()
            .map(|(i, (label, glyph, url))| {
                let url = *url;
                sub_row(app, tk, format!("sh-support-{i}"), glyph, &t(label), None).on_press_tx(
                    move |s: &mut App, tx| {
                        if tx.open_url(url).is_err() {
                            s.toast(
                                crate::app::ToastTone::Error,
                                &t("settings:marketData.openLinkErrorBody"),
                                Some(url.to_owned()),
                                None,
                            );
                        }
                    },
                )
            })
            .collect();
        actions = actions.child(sub_list(tk, rows));
    }
    actions = actions.child(group_header(
        tk,
        "sh-extras",
        &I::PLUS,
        &t("chrome:shell.extras.title"),
        app.shell.extras_open && !folded,
        folded,
        false,
    ));
    if app.shell.extras_open && !folded {
        let mut rows = Vec::new();
        for (i, e) in EXTRAS.iter().enumerate() {
            match e {
                None => rows.push(
                    el().key(format!("sh-extras-sep{i}"))
                        .h(1.0)
                        .w_full()
                        .mx(tk.r(0.5))
                        .my(tk.r(0.25))
                        .fill(tk.sidebar_border),
                ),
                Some((label, glyph, href)) => {
                    if DEV_HIDDEN.contains(href) && !app.prefs.developer_tools {
                        continue;
                    }
                    rows.push(sub_row(
                        app,
                        tk,
                        format!("sh-extra{href}"),
                        glyph,
                        &t(label),
                        Some(href),
                    ));
                }
            }
        }
        actions = actions.child(sub_list(tk, rows));
    }
    let settings_active = app.shell.settings_open;
    // The web's Settings row has no shortcut hint (⌘, lives in the native menu only).
    let settings = nav_button(
        tk,
        "sh-settings".into(),
        &I::SETTINGS,
        &t("nav:book.settings"),
        settings_active,
        folded,
        false,
    )
    .on_press(|s: &mut App| {
        s.shell.settings_open = true;
        s.shell.popup = None;
    });
    actions = actions.child(
        // `mt-1 -mx-2 border-t border-sidebar-border/60 px-2 pt-1`: the rule spans the footer.
        col()
            .w(width)
            // Folded, the rail's centring already spans the rule across it.
            .ml(if folded { 0.0 } else { -pad })
            .mt(tk.r(0.25))
            .px(pad)
            .pt(tk.r(0.25))
            .border_sides([1.0, 0.0, 0.0, 0.0], tk.sidebar_border.alpha(0.6))
            .border_layout()
            .items_center()
            .child(if folded { settings } else { settings.w_full() }),
    );

    // SidebarFooter, overridden to `gap-1 p-2` in AppShell.
    let mut footer = col().w_full().padding(pad).gap(tk.r(0.25)).child(actions);
    // SidebarUpdatePill (`group-data-[collapsible=icon]:hidden`).
    if let Some(pill) = crate::native::update::pill(app, tk).filter(|_| !folded) {
        footer = footer.child(pill);
    }
    footer = footer.child(nav_user(app, tk, folded, layer));
    if folded {
        footer = footer.items_center();
    } else {
        footer = footer.child(version_line(app, tk));
    }

    // SidebarRail (`after:hidden`): an invisible 1 rem strip centred on the sidebar's right
    // edge that toggles the nav, never a Tab stop, resize cursors per state.
    let rail = el()
        .key("sh-rail")
        .w(tk.r(1.0))
        .h_full()
        .absolute(width - tk.r(0.5), 0.0)
        .paint_order(20)
        .role("button")
        .label(crate::i18n::t("chrome:shell.toggleSidebar"))
        .help(crate::i18n::t("chrome:shell.toggleSidebar"))
        .focusable(false)
        .cursor(rune::Cursor::ResizeColumn)
        .on_press(|s: &mut App| {
            let narrow = super::narrow_now(s);
            super::toggle_nav(s, narrow);
        });

    col()
        .key("sh-sidebar")
        .w(width)
        .h_full()
        .fill(tk.chrome)
        .role("navigation")
        .label(t("nav:section.main"))
        .child(header)
        .child(content)
        .child(footer)
        .child(rail)
}
