//! routes/Imports.tsx: integrations and connection onboarding as a screen — the page header
//! (description, back to Connections) and `SettingsIntegrations4` (category tabs over the
//! catalog's sources as integration cards). Choosing a ready source opens the Add connection
//! dialog on its setup step.

use rune::{Color, Cx, El, KeyEvent, Shadow, Tx, col, row, text};

use super::add::meta::{SetupKind, meta};
use super::add::picker;
use crate::app::{App, Reply};
use crate::i18n::t;
use crate::icons as I;
use crate::overview::add_connection::{Art, Frame, Source};
use crate::overview::catalog::{CATEGORIES, SOURCES};
use crate::theme::Tk;
use crate::txdetail::widgets::{Variant, button_surface};
use crate::ui::{ic, surface};

pub struct State {
    /// `selectedId` (defaults to the wallet export source).
    pub selected: &'static str,
    /// `activeCategoryId` (None: the first category).
    pub category: Option<&'static str>,
}

impl Default for State {
    fn default() -> Self {
        State {
            selected: "descriptor",
            category: None,
        }
    }
}

/// The visible sources (dev-only ones need developer tools).
fn items(app: &App) -> Vec<&'static Source> {
    SOURCES
        .iter()
        .filter(|s| app.prefs.developer_tools || !s.dev_only)
        .collect()
}

/// The categories present, in source order.
fn categories(app: &App) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for s in items(app) {
        if !v.contains(&s.category) {
            v.push(s.category);
        }
    }
    v
}

fn category_label(id: &str) -> String {
    CATEGORIES
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.label())
        .unwrap_or_default()
}

/// The active category: the chosen tab, else the first.
fn active(app: &App) -> &'static str {
    let cats = categories(app);
    app.wallets
        .imports
        .category
        .filter(|c| cats.contains(c))
        .unwrap_or_else(|| cats.first().copied().unwrap_or("wallets"))
}

fn select(a: &mut App, id: &'static str) {
    a.wallets.imports.selected = id;
    if let Some(s) = SOURCES.iter().find(|s| s.id == id) {
        a.wallets.imports.category = Some(s.category);
        if !s.planned {
            super::add::open(a, Some(id));
        }
    }
}

/// The card's logo: a `size-10` box (the source's frame fill when it has one) around the
/// image or lettered badge (every one at the catalog's usual `size-9`), or a bordered box
/// around an icon.
fn artwork(app: &App, cx: &Cx<App>, tk: &Tk, src: &'static Source) -> El {
    let side = tk.r(2.5);
    let content: El = match &src.art {
        Art::Icon(i) => {
            return row()
                .size(side, side)
                .center()
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .border_layout()
                .fill(tk.background)
                .a11y_hidden()
                .child(ic(i, tk.r(1.25), tk.muted_fg));
        }
        _ => picker::art(
            app,
            cx,
            tk,
            src,
            tk.r(crate::logos::role::SOURCE_CARD),
            side,
            side,
        ),
    };
    let mut b = row()
        .size(side, side)
        .center()
        .role("img")
        .label(src.title());
    if let Frame::Fill(c) = src.frame {
        b = b.fill(rune::color::hex(c).expect("hex"));
    }
    b.child(content)
}

fn card(app: &App, cx: &Cx<App>, tk: &Tk, src: &'static Source) -> El {
    let selected = app.wallets.imports.selected == src.id;
    let action = if src.planned {
        t("review:imports.actionPlanned")
    } else if meta(src.id).setup == Some(SetupKind::BackendSettings) {
        t("review:imports.actionConfigure")
    } else {
        t("review:imports.actionSetup")
    };
    // `IntegrationCard` hardcodes "Selected"; the desktop translates it.
    let label = if selected {
        t("review:imports.actionSelected")
    } else {
        action
    };
    let id = src.id;
    // `Button size="sm"`: `secondary` when selected, else `default`.
    let btn = if selected {
        row()
            .key(format!("imports-action-{id}"))
            .h(tk.r(2.0))
            .px(tk.r(0.75))
            .center()
            .radius(tk.r(0.375))
            .fill(tk.muted)
            .hover(tk.muted.alpha(0.8))
            .child(text(label.clone()).font(tk.tsm(500.0)).color(tk.fg))
    } else {
        button_surface(
            tk,
            row()
                .key(format!("imports-action-{id}"))
                .h(tk.r(2.0))
                .px(tk.r(0.75))
                .center(),
            Variant::Default,
        )
        .child(text(label.clone()).font(tk.tsm(500.0)).color(tk.primary_fg))
    }
    .role("button")
    .label(label)
    .focusable(true)
    .on_press(move |a: &mut App| select(a, id));
    row()
        .key(format!("imports-card-{id}"))
        .w_full()
        .padding(tk.r(1.0))
        .gap(tk.r(1.0))
        .items_start()
        .radius(tk.r(0.5))
        .border(1.0, if selected { tk.primary } else { tk.border })
        .border_layout()
        .fill(if selected {
            tk.primary.alpha(0.05)
        } else {
            Color::TRANSPARENT
        })
        .child(artwork(app, cx, tk, src))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(tk.r(0.25))
                .child(
                    text(src.title())
                        .font(tk.tbase(500.0))
                        .color(tk.fg)
                        .truncate(),
                )
                .child(
                    text(src.description())
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                ),
        )
        .child(btn)
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, _layer: &mut Vec<El>) -> El {
    let sm = app.shell.window_w <= 0.0 || app.shell.window_w >= 640.0;
    // `pageHeaderClassName`: description left, the outline `Connections` button right.
    let back_label = t("review:imports.connections");
    let back = crate::ui::button::button(
        tk,
        "imports-back",
        crate::ui::button::Variant::Outline,
        crate::ui::button::Size::Sm,
        Some(&I::ARROW_LEFT),
        &back_label,
        true,
    )
    .on_press(|a: &mut App| a.navigate("/connections"));
    let desc = text(t("review:imports.description"))
        .font(tk.tsm(400.0))
        .color(tk.muted_fg)
        .wrap();
    let header = if sm {
        row()
            .w_full()
            .gap(tk.r(0.75))
            .center_y()
            .justify_between()
            .child(col().grow(1.0).min_w(0.0).max_w(tk.r(48.0)).child(desc))
            .child(back)
    } else {
        col()
            .w_full()
            .gap(tk.r(0.625))
            .child(desc)
            .child(row().child(back))
    };
    let act = active(app);
    // The tab strip: `inline-flex h-9 rounded-lg bg-muted p-[3px]`.
    let mut tabs = row()
        .h(tk.r(2.25))
        .padding(3.0)
        .radius(tk.r(0.5))
        .fill(tk.muted)
        .role("tablist")
        .focus_group();
    for c in categories(app) {
        let on = c == act;
        let name = category_label(c);
        let mut b = row()
            .key(format!("imports-tab-{c}"))
            .h_full()
            .px(tk.r(0.5))
            .py(tk.r(0.25))
            .center()
            .radius(tk.r(0.375))
            .border(
                1.0,
                if on && tk.dark {
                    tk.input
                } else {
                    Color::TRANSPARENT
                },
            )
            .border_layout()
            .role("tab")
            .checked(on)
            .label(name.clone())
            .focusable(true)
            .on_press(move |a: &mut App| a.wallets.imports.category = Some(c))
            .child(
                text(name)
                    .font(tk.tsm(500.0))
                    .color(if on { tk.fg } else { tk.muted_fg }),
            );
        if on {
            b = b
                .fill(if tk.dark {
                    tk.input.alpha(tk.input.a * 0.3)
                } else {
                    tk.background
                })
                .box_shadow(Shadow {
                    y: 1.0,
                    blur: 2.0,
                    color: Color::BLACK.alpha(0.05),
                    ..Shadow::default()
                });
        }
        tabs = tabs.child(b);
    }
    let mut list = col().w_full().padding(tk.r(1.0)).gap(tk.r(0.75));
    for s in items(app).into_iter().filter(|s| s.category == act) {
        list = list.child(card(app, cx, tk, s));
    }
    let pad = tk.card_pad();
    let card = surface(tk)
        .key("imports-card")
        .w_full()
        .min_w(0.0)
        .py(pad)
        .gap(tk.r(1.0))
        .clip_rounded(tk.r(0.75))
        .child(
            col()
                .w_full()
                .px(pad)
                .pb(pad)
                .gap(tk.r(0.375))
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .border_layout()
                .child(
                    text(t("review:imports.sourcesHeading"))
                        .font(tk.tsm(600.0).lh(tk.r(0.875)))
                        .color(tk.fg)
                        .role("heading"),
                )
                .child(
                    text(t("review:imports.sourcesSubHeading"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                ),
        )
        // `CardContent className="p-0"` keeps its `px-(--kb-card-padding)`: tailwind-merge does not
        // read the v4 variable syntax as a padding, so the later `p-0` does not drop it.
        .child(
            col()
                .w_full()
                .px(pad)
                .child(
                    row()
                        .w_full()
                        .px(tk.r(1.0))
                        .py(tk.r(0.75))
                        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                        .border_layout()
                        .child(tabs),
                )
                .child(list),
        );
    col()
        .key("imports-page")
        .w_full()
        .padding(tk.page_pad())
        .gap(tk.section_gap())
        .fill(tk.background)
        .child(header)
        .child(card)
}

pub fn poll(_app: &mut App, _tx: &mut Tx) -> bool {
    false
}

pub fn on_reply(_app: &mut App, _tag: &str, _r: &Reply) {}

pub fn on_key(_app: &mut App, _ev: &KeyEvent, _tx: &mut Tx) -> bool {
    false
}

pub fn prime(_app: &mut App) {}

pub const SCENES: &[crate::app::Scene] = &[
    ("wallets-imports", |a| super::scene_route(a, "/imports")),
    ("wallets-imports-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        super::scene_route(a, "/imports");
    }),
    ("wallets-imports-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        super::scene_route(a, "/imports");
    }),
    ("wallets-imports-files", |a| {
        super::scene_route(a, "/imports");
        a.wallets.imports.category = Some("files");
    }),
    ("wallets-imports-exchanges", |a| {
        super::scene_route(a, "/imports");
        a.wallets.imports.category = Some("exchanges");
    }),
    ("wallets-imports-setup", |a| {
        super::scene_route(a, "/imports");
        select(a, "csv");
        super::add::settle(a);
    }),
];
