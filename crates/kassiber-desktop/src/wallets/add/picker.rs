//! The dialog's first step, the source picker, as the web draws it: the command palette's three
//! columns (categories, a searchable source list, the selected source's summary). Moved here
//! from overview/add_connection.rs; the catalog is the web's own (overview/catalog.rs).

use rune::{Color, Cx, El, Shadow, TextStyle, Tx, col, el, row, text, text_input};

use crate::app::App;
use crate::i18n::t;
use crate::icons as I;
use crate::logos::{self, Logo};
use crate::overview::add_connection::{Art, Category, Frame, Source};
use crate::overview::catalog::{CATEGORIES, SOURCES};
use crate::theme::Tk;
use crate::ui::ic;

fn h(s: &str) -> Color {
    rune::color::hex(s).expect("hex")
}

pub fn selected(app: &App) -> &'static Source {
    let id = app.ov.add_selected.unwrap_or(SOURCES[0].id);
    SOURCES.iter().find(|s| s.id == id).unwrap_or(&SOURCES[0])
}

/// `visibleSources`: the active category, or the whole catalog matched by a search.
fn visible(app: &App) -> Vec<&'static Source> {
    let query = app.ov.add_search.text.trim().to_lowercase();
    let category = app.ov.add_category.unwrap_or("wallets");
    SOURCES
        .iter()
        .filter(|s| app.prefs.developer_tools || !s.dev_only)
        .filter(|s| !super::history_import(app) || super::is_history_source(s))
        .filter(|s| {
            if query.is_empty() {
                s.category == category
            } else {
                format!("{} {} {}", s.title(), s.description(), s.id)
                    .to_lowercase()
                    .contains(&query)
            }
        })
        .collect()
}

/// The source's logo file, flattened to a black (dark: white) silhouette where the web's
/// `brightness-0 dark:invert` asks for one.
pub fn logo_file(tk: &Tk, src: &Source) -> Option<Logo> {
    let Art::File(bytes, ext, mono) = src.art else {
        return None;
    };
    let svg = ext == "svg";
    Some(Logo {
        bytes,
        svg,
        ink: (svg && mono).then_some(if tk.dark { "#ffffff" } else { "#000000" }),
    })
}

/// A source's artwork content at one size for every source: a logo file or a lettered badge
/// whose painted art is `art` points on its longer side, centred in a `frame` square, or a
/// lucide icon of `icon_pt`. The web sizes each logo by its own class (`size-7` … `size-10`,
/// `h-9 w-auto`), so the same Bitcoin coin is 28 px for one source and 36 px for the next and
/// the summary jumps while clicking through the list; here every logo shares its role's size.
pub fn art(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    src: &'static Source,
    art: f32,
    frame: f32,
    icon_pt: f32,
) -> El {
    match &src.art {
        Art::File(..) => logo_file(tk, src).map_or_else(
            || el().size(frame, frame),
            |l| logos::fitted(app, cx, l, art, frame),
        ),
        // `sourceIcon`: a 40 × 40 rounded square (rx 10) with a 10 px bold label, drawn at `art`.
        Art::Badge { fill, ink, label } => row().size(frame, frame).center().a11y_hidden().child(
            row()
                .size(art, art)
                .center()
                .radius(art * 0.25)
                .fill(h(fill))
                .child(
                    text(*label)
                        .font(TextStyle::new(art * 0.25, 700.0))
                        .color(h(ink)),
                ),
        ),
        Art::Icon(i) => {
            row()
                .size(frame, frame)
                .center()
                .a11y_hidden()
                .child(ic(i, icon_pt, tk.muted_fg))
        }
        Art::None => el().size(frame, frame),
    }
}

/// `SourceArtwork`: a bordered, muted tile around the logo (or icon / lettered badge).
#[allow(clippy::too_many_arguments)]
fn artwork(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    src: &'static Source,
    frame_pt: f32,
    pad: f32,
    art_pt: f32,
    icon_pt: f32,
    radius: f32,
) -> El {
    let fill = match src.frame {
        Frame::Fill(c) => h(c),
        Frame::Default => tk.muted.alpha(if tk.dark { 0.55 } else { 0.6 }),
    };
    let border = tk
        .border
        .alpha(tk.border.a * if tk.dark { 0.8 } else { 0.7 });
    let inner = frame_pt - 2.0 * pad - 2.0;
    row()
        .size(frame_pt, frame_pt)
        .center()
        .radius(radius)
        .fill(fill)
        .border(1.0, border)
        .border_layout()
        .box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: Color::BLACK.alpha(if tk.dark { 0.2 } else { 0.05 }),
            ..Shadow::default()
        })
        .a11y_hidden()
        .child(art(app, cx, tk, src, art_pt, inner, icon_pt))
}

fn category_button(app: &App, tk: &Tk, c: &'static Category) -> El {
    let active = app.ov.add_category.unwrap_or("wallets") == c.id
        && app.ov.add_search.text.trim().is_empty();
    let ink = if active { tk.fg } else { tk.muted_fg };
    let id = c.id;
    // `flex min-h-8 items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-sm`.
    row()
        .key(format!("add-category-{id}"))
        .w_full()
        .min_h(tk.r(2.0))
        .px(tk.r(0.625))
        .py(tk.r(0.375))
        .gap(tk.r(0.625))
        .center_y()
        .radius(tk.r(0.5))
        .fill(if active {
            tk.accent
        } else {
            Color::TRANSPARENT
        })
        .hover(tk.accent.alpha(0.6))
        .role("button")
        .label(c.label())
        .focusable(true)
        .on_press(move |a: &mut App| {
            // `selectCategory`: the first source of the category becomes the selection (a
            // history import only offers its file formats).
            super::mark_history_chosen(a);
            let Some(first) = super::source_for_category(id, super::history_import(a)) else {
                return;
            };
            a.ov.add_search = rune::TextEdit::new("");
            a.ov.add_category = Some(id);
            a.ov.add_selected = Some(first.id);
        })
        .child(ic(c.icon, tk.r(1.0), ink))
        .child(text(c.label()).font(tk.tsm(400.0)).color(ink))
}

fn source_row(app: &App, cx: &Cx<App>, tk: &Tk, src: &'static Source) -> El {
    let is_selected = selected(app).id == src.id;
    let id = src.id;
    let title = src.title();
    let detail = src
        .format_label()
        .or_else(|| src.path_label())
        .unwrap_or_default();
    let mut r = row()
        .key(format!("add-source-{id}"))
        .w_full()
        .px(tk.r(0.625))
        .py(tk.r(0.375))
        .gap(tk.r(0.625))
        .center_y()
        .radius(tk.r(0.5))
        .fill(if is_selected {
            tk.accent
        } else {
            Color::TRANSPARENT
        })
        .hover(tk.accent.alpha(0.6))
        .role("button")
        .checked(is_selected)
        .label(format!("{title} {detail}"))
        .help(src.description())
        .focusable(true)
        .on_press(move |a: &mut App| {
            super::mark_history_chosen(a);
            a.ov.add_selected = Some(id);
        })
        // `size-7 rounded-md p-0.5 [&_svg]:size-4`: every logo fills the 22 px inner box.
        .child(artwork(
            app,
            cx,
            tk,
            src,
            tk.r(1.75),
            tk.r(0.125),
            tk.r(logos::role::SOURCE_ROW),
            tk.r(1.0),
            tk.r(0.375),
        ))
        .child(
            text(title)
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .truncate()
                .min_w(0.0),
        );
    if src.planned {
        r = r.child(planned_badge(tk));
    }
    // The palette's right-aligned detail column (`ml-auto max-w-[45%] truncate text-xs`).
    r.child(el().grow(1.0)).child(
        text(detail)
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .truncate()
            .max_w(tk.r(14.0)),
    )
}

/// Badge `variant="outline"`: text-xs medium, a hairline, rounded-md, px-2 py-0.5.
fn planned_badge(tk: &Tk) -> El {
    row()
        .px(tk.r(0.5))
        .py(tk.r(0.125))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .child(
            text(t("connections:add.plannedLabel"))
                .font(tk.txs(500.0))
                .color(tk.fg),
        )
}

/// `renderSourceSummary` for `src`.
pub fn summary_of(app: &App, cx: &Cx<App>, tk: &Tk, src: &'static Source) -> El {
    let sub = match (src.path_label(), src.format_label()) {
        (Some(p), Some(f)) => format!("{p} · {f}"),
        (Some(p), None) => p,
        (None, Some(f)) => format!(" · {f}"),
        (None, None) => String::new(),
    };
    let mut head = col().w_full().gap(tk.r(0.5)).child(
        row()
            .w_full()
            .gap(tk.r(0.75))
            .center_y()
            // Default `size-12` frame with `p-1`; every logo at the catalog's usual `size-9`.
            .child(artwork(
                app,
                cx,
                tk,
                src,
                tk.r(3.0),
                tk.r(0.25),
                tk.r(logos::role::SOURCE_SUMMARY),
                tk.r(1.5),
                tk.r(0.5),
            ))
            .child(
                col()
                    .min_w(0.0)
                    .grow(1.0)
                    .child(
                        text(src.title())
                            .font(tk.tsm(500.0))
                            .color(tk.fg)
                            .truncate(),
                    )
                    .child(text(sub).font(tk.txs(400.0)).color(tk.muted_fg).wrap()),
            ),
    );
    let mut badges = row().gap(tk.r(0.5)).flex_wrap();
    if src.planned {
        badges = badges.child(planned_badge(tk));
    }
    if let Some(href) = src.docs_href {
        badges = badges.child(
            row()
                .px(tk.r(0.5))
                .py(tk.r(0.125))
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .role("link")
                .label(t("connections:add.docs"))
                .focusable(true)
                .on_press_tx(move |_a: &mut App, tx: &mut Tx| {
                    let _ = tx.open_url(href);
                })
                .child(
                    text(t("connections:add.docs"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                ),
        );
    }
    head = head.child(badges);
    let mut details = col().w_full().gap(tk.r(0.5));
    for d in src.details() {
        details = details.child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .items_start()
                .child(
                    el().mt(tk.r(0.5))
                        .size(tk.r(0.25), tk.r(0.25))
                        .radius(tk.r(0.125))
                        .fill(tk.muted_fg.alpha(0.6)),
                )
                .child(
                    text(d)
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }
    col()
        .w_full()
        .gap(tk.r(1.0))
        .child(head)
        .child(
            text(src.description())
                .font(tk.tsm(400.0))
                .color(tk.fg)
                .wrap(),
        )
        .child(details)
        .child(
            col()
                .w_full()
                .pt(tk.r(0.75))
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .border_layout()
                .child(
                    text(t("connections:add.trademarkDisclaimer"))
                        .font(tk.txs(400.0).lh(tk.r(1.0)))
                        .color(tk.muted_fg)
                        .wrap(),
                ),
        )
}

/// shadcn `Button` (`h-9 px-4 text-sm font-medium rounded-md`), outline or primary.
pub fn button(tk: &Tk, key: &str, label: &str, primary: bool, enabled: bool) -> El {
    use crate::ui::button::{Size, Variant};
    let variant = if primary {
        Variant::Default
    } else {
        Variant::Outline
    };
    crate::ui::button::button(tk, key, variant, Size::Default, None, label, enabled)
}

/// The source step's body (`renderSourceStep`): categories, the searchable source list, and the
/// selected source's summary.
pub fn body(app: &App, cx: &mut Cx<App>, tk: &Tk) -> El {
    let shown = visible(app);
    let mut cats = col().w_full().padding(tk.r(0.5)).gap(tk.r(0.125));
    let history = super::history_import(app);
    for c in CATEGORIES
        .iter()
        .filter(|c| !history || super::source_for_category(c.id, true).is_some())
    {
        cats = cats.child(category_button(app, tk, c));
    }
    let search = row()
        .w_full()
        .h(tk.r(3.0))
        .px(tk.r(1.0))
        .gap(tk.r(0.625))
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .child(ic(&I::SEARCH, tk.r(1.0), tk.muted_fg))
        .child(
            text_input(&app.ov.add_search, tk.tbase(400.0))
                .key("add-search")
                .single_line()
                .placeholder(
                    t("connections:add.searchPlaceholder"),
                    tk.muted_fg.alpha(0.8),
                )
                .caret(tk.fg, tk.primary.alpha(0.3))
                .label(t("connections:add.searchPlaceholder"))
                .on_edit(|a: &mut App| &mut a.ov.add_search)
                .grow(1.0)
                .min_w(0.0),
        );
    let mut list = col().w_full().padding(tk.r(0.5)).gap(tk.r(0.125));
    if shown.is_empty() {
        let msg = if app.ov.add_search.text.trim().is_empty() {
            "connections:add.noFilterMatch"
        } else {
            "connections:add.noSearchMatch"
        };
        list = list.child(
            text(t(msg))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .px(tk.r(0.625))
                .py(tk.r(0.5)),
        );
    }
    for s in &shown {
        list = list.child(source_row(app, cx, tk, s));
    }
    let middle = col()
        .h_full()
        .grow(1.0)
        .min_w(0.0)
        .border_sides([0.0, 1.0, 0.0, 0.0], tk.border)
        .border_layout()
        .child(search)
        .child(
            col()
                .key("add-sources")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        );
    let right = col()
        .w(340.0)
        .h_full()
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .padding(tk.r(1.0))
                .child(summary_of(app, cx, tk, selected(app))),
        );
    row()
        .w_full()
        .h_full()
        .child(
            col()
                .w(190.0)
                .h_full()
                .border_sides([0.0, 1.0, 0.0, 0.0], tk.border)
                .border_layout()
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(cats),
        )
        .child(middle)
        .child(right)
}
