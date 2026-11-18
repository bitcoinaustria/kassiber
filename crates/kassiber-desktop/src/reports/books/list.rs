//! The Books page (BooksView, WorkspaceSection and BookRow in Books.tsx): the intro and the
//! New book set action, then one card per book set with its books as rows.

use rune::{Color, El, col, el, row, text};
use rune_kit::{Placement, TextInput};
use serde_json::json;

use super::kit::{self, badge_secondary, preview_badge};
use super::model::{self, Profile, Workspace};
use super::{dialogs, error_text};
use crate::app::{App, Reply};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button, button_surface, form_kit, spinner};
use crate::ui::{ic, surface};

/// `openBook`: switch at once and land on its Overview (nothing for the open book, or while a
/// switch runs).
pub fn open_book(app: &mut App, profile_id: &str) {
    let active = app
        .read("ui.profiles.snapshot")
        .and_then(|p| p["activeProfileId"].as_str())
        .unwrap_or_default()
        .to_owned();
    if profile_id == active || app.reports.books.pending_open.is_some() {
        return;
    }
    app.reports.books.switch_error = None;
    if app
        .send(
            "ui.profiles.switch",
            json!({"profile_id": profile_id}),
            "reports.books.switch",
        )
        .is_some()
    {
        app.reports.books.pending_open = Some(profile_id.to_owned());
    }
}

pub fn on_switched(app: &mut App, r: &Reply) {
    let opened = app.reports.books.pending_open.take();
    if r.error.is_none() {
        // Its journals are the shell's to report now (the overview's own signal).
        if let Some(id) = opened {
            app.reports.books.stale.remove(&id);
        }
    }
    if r.error.is_some() {
        app.reports.books.switch_error =
            Some(error_text(r, "onboarding:books.switch.errorGeneric"));
        return;
    }
    // Another book: its Overview (`App::deliver` then starts a new session).
    app.navigate("/overview");
}

/// The page frame (`screenShellClassName`) on the app's spacing scale: the page inset, the
/// section gap between blocks.
pub fn page(tk: &Tk) -> El {
    col()
        .key("books-page")
        .w_full()
        .padding(tk.page_pad())
        .gap(tk.section_gap())
        .fill(tk.background)
}

/// `ScreenSkeleton`: header bars, a metric strip and a list card, in muted blocks.
pub fn skeleton(app: &App, tk: &Tk, metrics: usize, title_w: f32) -> El {
    let bar = |w: f32, h: f32| el().w(w).h(h).radius(tk.r(0.375)).fill(tk.muted);
    let wide = app.shell.window_w >= 1280.0 || app.shell.window_w == 0.0;
    let mut strip = row().w_full();
    for i in 0..metrics {
        strip = strip.child(
            col()
                .grow(1.0)
                .padding(tk.r(1.0))
                .gap(tk.r(0.5))
                .border_sides(
                    [
                        0.0,
                        if i + 1 < metrics && wide { 1.0 } else { 0.0 },
                        0.0,
                        0.0,
                    ],
                    tk.border,
                )
                .child(bar(tk.r(6.0), tk.r(0.75)))
                .child(bar(tk.r(5.0), tk.r(1.5)))
                .child(bar(tk.r(8.0), tk.r(0.75))),
        );
    }
    let mut list = col().w_full();
    for _ in 0..8 {
        list = list.child(
            row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .gap(tk.r(0.75))
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .child(
                    col()
                        .grow(1.0)
                        .gap(tk.r(0.5))
                        .child(bar(tk.r(11.0), tk.r(1.0)))
                        .child(bar(tk.r(16.0), tk.r(0.75))),
                )
                .child(
                    col()
                        .items_end()
                        .gap(tk.r(0.5))
                        .child(bar(tk.r(5.0), tk.r(1.0)))
                        .child(bar(tk.r(3.5), tk.r(0.75))),
                ),
        );
    }
    page(tk)
        .busy(true)
        .label(t("common:state.loading"))
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(
                    col()
                        .gap(tk.r(0.5))
                        .child(bar(tk.r(7.0), tk.r(0.75)))
                        .child(bar(title_w, tk.r(1.25))),
                )
                .child(
                    row()
                        .gap(tk.r(0.75))
                        .child(bar(tk.r(6.0), tk.r(2.0)))
                        .child(bar(tk.r(7.0), tk.r(2.0))),
                ),
        )
        .child(surface(tk).w_full().clip_rounded(tk.r(0.75)).child(strip))
        .child(
            surface(tk)
                .w_full()
                .clip_rounded(tk.r(0.75))
                .child(
                    col()
                        .w_full()
                        .px(tk.r(1.0))
                        .py(tk.r(0.75))
                        .gap(tk.r(0.5))
                        .child(bar(tk.r(9.0), tk.r(1.0)))
                        .child(bar(tk.r(14.0), tk.r(0.75))),
                )
                .child(list),
        )
}

pub fn view(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let Some(raw) = app.read("ui.profiles.snapshot") else {
        return skeleton(app, tk, 3, tk.r(7.0));
    };
    let p = model::parse(raw);
    let b = &app.reports.books;
    let mut actions = row().gap(tk.r(0.75)).center_y();
    if p.profile_count() > model::BOOK_FILTER_THRESHOLD {
        let input = TextInput::new(&b.query)
            .placeholder(t("onboarding:books.filter.placeholder"))
            .label(t("onboarding:books.filter.placeholder"))
            .leading_icon(&I::SEARCH)
            .height(tk.r(2.0))
            .text_style(tk.tsm(400.0))
            .on_edit(|a: &mut App| &mut a.reports.books.query)
            .build(&form_kit(tk))
            .key("books-filter")
            .w(tk.r(14.0));
        actions = actions.child(crate::txdetail::widgets::field(tk, input));
    }
    actions = actions.child(
        button(
            tk,
            "books-new-set",
            Variant::Outline,
            Size::Sm,
            Some(&I::FOLDER_PLUS),
            &t("onboarding:books.newBookSet"),
            true,
        )
        .px(tk.r(0.75))
        .on_press(|a: &mut App| dialogs::request_create_workspace(a)),
    );
    let header = row()
        .w_full()
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .justify_end()
        .child(actions.help(t("onboarding:books.intro")));
    let mut out = page(tk).child(header);
    if let Some(e) = &b.switch_error {
        out = out.child(
            row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .radius(tk.r(0.5))
                .border(1.0, tk.destructive.alpha(0.3))
                .fill(tk.destructive.alpha(0.1))
                .role("alert")
                .child(
                    text(e.clone())
                        .font(tk.tsm(400.0))
                        .color(tk.destructive)
                        .wrap()
                        .w_full(),
                ),
        );
    }
    let visible = model::filter_workspaces(&p.workspaces, &b.query.text);
    let mut sections = col().w_full().gap(tk.r(0.75));
    for w in &visible {
        sections = sections.child(workspace_section(app, tk, &p.active_profile, w, layer));
    }
    if visible.is_empty() {
        sections = sections.child(
            row().w_full().py(tk.r(1.5)).justify_center().child(
                text(t("onboarding:books.filter.noMatches"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg),
            ),
        );
    }
    out = out.child(sections);
    dialogs::mount(app, tk, &p, layer);
    out
}

/// One book set: identity and set actions, its books, then "New book in …".
fn workspace_section(app: &App, tk: &Tk, active: &str, w: &Workspace, layer: &mut Vec<El>) -> El {
    let menu_key = format!("books-ws-actions-{}", w.id);
    let ws_id = w.id.clone();
    let actions_label = tr("onboarding:books.workspace.actions", &[("name", &w.name)]);
    let more = button_surface(
        tk,
        row()
            .key(menu_key.clone())
            .size(tk.r(2.0), tk.r(2.0))
            .center(),
        Variant::Ghost,
    )
    .role("button")
    .label(actions_label.clone())
    .focusable(true)
    .child(ic(&I::ELLIPSIS, tk.r(1.0), tk.fg));
    let ws_open = app.reports.books.popup.as_deref() == Some(&format!("ws:{}", w.id));
    let more = crate::ui::popup::trigger(
        more,
        crate::ui::popup::Kind::Menu,
        ws_open,
        move |a: &mut App| {
            let k = format!("ws:{ws_id}");
            a.reports.books.popup = if a.reports.books.popup.as_deref() == Some(k.as_str()) {
                None
            } else {
                Some(k)
            };
        },
    );
    if app.reports.books.popup.as_deref() == Some(&format!("ws:{}", w.id)) {
        let (a, bw, oid) = (w.clone(), w.clone(), w.id.clone());
        let items = vec![
            (
                kit::menu_item(
                    tk,
                    "books-ws-overview",
                    &I::EYE,
                    &t("onboarding:books.workspace.overview"),
                )
                .on_press(move |s: &mut App| s.navigate(&format!("/books/{oid}/birds-eye"))),
                "books-ws-overview".to_owned(),
                t("onboarding:books.workspace.overview"),
            ),
            (
                kit::menu_item(
                    tk,
                    "books-ws-new-book",
                    &I::PLUS,
                    &t("onboarding:books.workspace.newBook"),
                )
                .on_press(move |s: &mut App| dialogs::request_create_profile(s, &a, None)),
                "books-ws-new-book".to_owned(),
                t("onboarding:books.workspace.newBook"),
            ),
            (
                kit::menu_item(
                    tk,
                    "books-ws-rename",
                    &I::PENCIL,
                    &t("onboarding:books.workspace.rename"),
                )
                .on_press(move |s: &mut App| dialogs::request_rename_workspace(s, &bw)),
                "books-ws-rename".to_owned(),
                t("onboarding:books.workspace.rename"),
            ),
        ];
        kit::dropdown(
            tk,
            layer,
            &menu_key,
            tk.r(12.0),
            Placement::BottomEnd,
            items,
        );
    }
    let header = row()
        .w_full()
        .px(tk.r(1.25))
        .py(tk.r(0.75))
        .gap(tk.r(0.75))
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .child(
            row()
                .size(tk.r(2.0), tk.r(2.0))
                .center()
                .radius(tk.r(0.5))
                .fill(tk.muted)
                .a11y_hidden()
                .child(ic(&I::LANDMARK, tk.r(1.0), tk.muted_fg)),
        )
        .child({
            // The set's name opens its overview.
            let id = w.id.clone();
            col()
                .key(format!("birds-eye-{}", w.id))
                .grow(1.0)
                .min_w(0.0)
                .role("link")
                .label(format!(
                    "{} · {}",
                    w.name,
                    t("onboarding:books.workspace.overview")
                ))
                .focusable(true)
                .hover_group()
                .on_press(move |a: &mut App| a.navigate(&format!("/books/{id}/birds-eye")))
                .child(
                    text(w.name.clone())
                        .font(tk.tsm(600.0))
                        .color(tk.fg)
                        .group_opacity(1.0, 0.75)
                        .truncate()
                        .role("heading"),
                )
                .child(
                    text(model::workspace_meta(w, true))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .truncate(),
                )
        })
        .child(more);
    let mut card = surface(tk)
        .w_full()
        .clip_rounded(tk.r(0.75))
        .role("group")
        .label(w.name.clone())
        .child(header);
    if w.profiles.is_empty() {
        card = card.child(
            row().w_full().px(tk.r(1.25)).py(tk.r(1.0)).child(
                text(t("onboarding:books.workspace.empty"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg),
            ),
        );
    } else {
        let any_status = w
            .profiles
            .iter()
            .any(|p| book_status(app, p, p.id == active).is_some());
        let table_key = format!("books-table-{}", w.id);
        let c = cols(app, tk, &table_key, any_status);
        let mut table = col()
            .key(table_key)
            .w_full()
            .role("table")
            .label(tr("onboarding:books.table.label", &[("name", &w.name)]))
            .child(header_row(tk, c));
        let pending = app.reports.books.pending_open.clone();
        let n = w.profiles.len();
        for (i, p) in w.profiles.iter().enumerate() {
            table = table.child(book_row(
                app,
                tk,
                layer,
                p,
                p.id == active,
                pending.as_deref() == Some(p.id.as_str()),
                pending.is_some(),
                w,
                c,
                i + 1 == n,
            ));
        }
        card = card.child(table);
    }
    let a = w.clone();
    card.child(
        row()
            .key(format!("books-new-in-{}", w.id))
            .w_full()
            .px(tk.r(1.25))
            .py(tk.r(0.625))
            .gap(tk.r(0.5))
            .center_y()
            .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
            .hover(tk.muted.alpha(0.4))
            .role("button")
            .label(tr(
                "onboarding:books.workspace.newBookIn",
                &[("name", &w.name)],
            ))
            .focusable(true)
            .on_press(move |s: &mut App| dialogs::request_create_profile(s, &a, None))
            .child(ic(&I::PLUS, tk.r(1.0), tk.muted_fg))
            .child(
                text(tr(
                    "onboarding:books.workspace.newBookIn",
                    &[("name", &w.name)],
                ))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg),
            ),
    )
}

/// The columns a book table shows at the window's width: Book, Region (with the tax year when
/// it isn't the country's), Currency, Cost basis (method over pool scope), Wallets, then the
/// status (only where a book needs action) and the row actions.
#[derive(Clone, Copy)]
struct Cols {
    region: bool,
    currency: bool,
    basis: bool,
    wallets: bool,
    status: bool,
}

/// Columns by the table's own width at the last layout (the card, not the window: the sidebar
/// and a side panel take their share), else the window's less the sidebar. The Book column
/// keeps at least 14 rem; status takes room before the facts do.
fn cols(app: &App, tk: &Tk, key: &str, status: bool) -> Cols {
    let win = if app.shell.window_w > 0.0 {
        app.shell.window_w - 260.0
    } else {
        1180.0
    };
    let w = rune::memo::layout_size(key)
        .map(|[w, _]| w)
        .filter(|w| *w > 0.0)
        .unwrap_or(win);
    let mut room = w
        - tk.r(2.0 * crate::ui::table::EDGE)
        - tk.r(14.0)
        - tk.r(W_ACTIONS)
        - if status { tk.r(W_STATUS) } else { 0.0 };
    let mut take = |need: f32| {
        let fits = room >= tk.r(need);
        if fits {
            room -= tk.r(need);
        }
        fits
    };
    let region = take(W_REGION);
    let basis = region && take(W_BASIS);
    let currency = basis && take(W_CURRENCY);
    let wallets = currency && take(W_WALLETS);
    Cols {
        region,
        currency,
        basis,
        wallets,
        status,
    }
}

/// What a book needs, in a few words: only when there is something to do (signal, not
/// reassurance). The open book from the shell's overview; another book when a change applied
/// here left its journals waiting.
fn book_status(app: &App, p: &Profile, active: bool) -> Option<(crate::ui::table::Tone, String)> {
    use crate::ui::table::Tone;
    if active {
        let s = app.snapshot()?;
        if s.needs_journals() {
            return Some((Tone::Warning, t("onboarding:books.status.reprocess")));
        }
        let q = s.quarantines().max(0.0) as usize;
        if q > 0 {
            return Some((Tone::Warning, tc("onboarding:books.status.review", q, &[])));
        }
        return None;
    }
    app.reports
        .books
        .stale
        .contains(&p.id)
        .then(|| (Tone::Warning, t("onboarding:books.status.reprocessOnOpen")))
}

/// "Tax year from 1 July" (and its zone when that isn't the country's), only for a book whose
/// tax year isn't its country's default.
fn calendar_note(app: &App, p: &Profile) -> Option<String> {
    let s = app.reports.books.cache.get(&p.id)?;
    if s.calendar_is_default {
        return None;
    }
    let mut out = tr(
        "onboarding:books.table.taxYearFrom",
        &[("start", &model::start_label(&s.tax_year_start))],
    );
    if s.timezone != s.timezone_default {
        out = format!("{out} · {}", s.timezone);
    }
    Some(out)
}

const W_REGION: f32 = 12.0;
const W_CURRENCY: f32 = 5.5;
const W_BASIS: f32 = 13.0;
const W_WALLETS: f32 = 5.0;
const W_STATUS: f32 = 12.5;
const W_ACTIONS: f32 = 7.25;

fn header_row(tk: &Tk, c: Cols) -> El {
    use crate::ui::table::{EDGE, HEAD_H, head, th, th_text};
    let mut h = head(tk)
        .h(tk.r(HEAD_H))
        .px(tk.r(EDGE - 0.5))
        .center_y()
        .child(th(
            tk,
            None,
            false,
            th_text(tk, t("onboarding:books.table.book")),
        ));
    if c.region {
        h = h.child(th(
            tk,
            Some(tk.r(W_REGION)),
            false,
            th_text(tk, t("onboarding:books.renameProfile.regionLabel")),
        ));
    }
    if c.currency {
        h = h.child(th(
            tk,
            Some(tk.r(W_CURRENCY)),
            false,
            th_text(tk, t("onboarding:books.table.currency")),
        ));
    }
    if c.basis {
        h = h.child(th(
            tk,
            Some(tk.r(W_BASIS)),
            false,
            th_text(tk, t("onboarding:books.settings.sections.costBasis")),
        ));
    }
    if c.wallets {
        h = h.child(th(
            tk,
            Some(tk.r(W_WALLETS)),
            true,
            th_text(tk, t("onboarding:books.profileCard.wallets")),
        ));
    }
    if c.status {
        h = h.child(th(
            tk,
            Some(tk.r(W_STATUS)),
            false,
            th_text(tk, t("onboarding:books.table.status")),
        ));
    }
    // The actions column has no visible header; it is named for assistive technology.
    h.child(th(
        tk,
        Some(tk.r(W_ACTIONS)),
        true,
        text(t("onboarding:books.table.actions"))
            .font(tk.tsm(500.0))
            .color(Color::TRANSPARENT)
            .a11y_hidden(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn book_row(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    p: &Profile,
    active: bool,
    pending: bool,
    locked: bool,
    w: &Workspace,
    c: Cols,
    last: bool,
) -> El {
    use crate::ui::table::{EDGE, PY, cell_text, chip, td, tr as table_row};
    let (tile_bg, tile_fg) = if active {
        (tk.primary, tk.primary_fg)
    } else {
        (tk.muted, tk.muted_fg)
    };
    let region = model::region_label(p.country());
    let cal = calendar_note(app, p);
    let mut name_line = row()
        .min_w(0.0)
        .gap(tk.r(0.5))
        .center_y()
        .child(cell_text(p.name.clone(), tk.tsm(500.0), tk.fg).shrink());
    if active {
        name_line = name_line.child(badge_secondary(
            tk,
            &t("onboarding:books.profileCard.currentLabel"),
            None,
        ));
    }
    let mut book = col().min_w(0.0).gap(tk.r(0.125)).child(name_line);
    if !c.region {
        // Narrow: the region, currency and tax year under the name.
        let mut sub = vec![region.clone()];
        if !p.fiat_currency.is_empty() {
            sub.push(p.fiat_currency.clone());
        }
        if let Some(n) = &cal {
            sub.push(n.clone());
        }
        book = book.child(cell_text(sub.join(" · "), tk.txs(400.0), tk.muted_fg));
    }
    let lead = row()
        .size(tk.r(2.25), tk.r(2.25))
        .center()
        .radius(tk.r(0.5))
        .fill(tile_bg)
        .a11y_hidden()
        .child(
            text(model::initials(&p.name))
                .font(tk.txs(600.0))
                .color(tile_fg),
        );
    let mut r = table_row(tk, last)
        .key(format!("books-row-{}", p.id))
        .px(tk.r(EDGE - 0.5))
        .role("row")
        .label(p.name.clone())
        .child(td(
            tk,
            None,
            false,
            PY,
            row()
                .w_full()
                .min_w(0.0)
                .gap(tk.r(0.75))
                .center_y()
                .child(lead)
                .child(book.grow(1.0)),
        ));
    if active {
        r = r.fill(tk.muted.alpha(0.4));
    }
    if c.region {
        let mut line = row()
            .min_w(0.0)
            .center_y()
            .child(cell_text(region.clone(), tk.tsm(400.0), tk.fg).shrink());
        if p.preview {
            line = line.child(preview_badge(tk));
        }
        let mut cell = col().w_full().min_w(0.0).gap(tk.r(0.125)).child(line);
        if c.currency {
            if let Some(n) = &cal {
                cell = cell.child(cell_text(n.clone(), tk.txs(400.0), tk.muted_fg));
            }
        } else {
            let mut sub = Vec::new();
            if !p.fiat_currency.is_empty() {
                sub.push(p.fiat_currency.clone());
            }
            if let Some(n) = &cal {
                sub.push(n.clone());
            }
            if !sub.is_empty() {
                cell = cell.child(cell_text(sub.join(" · "), tk.txs(400.0), tk.muted_fg));
            }
        }
        r = r.child(td(tk, Some(tk.r(W_REGION)), false, PY, cell));
    }
    if c.currency {
        r = r.child(td(
            tk,
            Some(tk.r(W_CURRENCY)),
            false,
            PY,
            cell_text(
                if p.fiat_currency.is_empty() {
                    "—".to_owned()
                } else {
                    p.fiat_currency.clone()
                },
                tk.tsm(400.0),
                tk.fg,
            ),
        ));
    }
    if c.basis {
        let method = p
            .gains_algorithm
            .as_deref()
            .map(model::method_label)
            .unwrap_or_else(|| "—".into());
        let pool = model::pool_scope_label(p.pool_scope.as_deref().unwrap_or("global"));
        r = r.child(td(
            tk,
            Some(tk.r(W_BASIS)),
            false,
            PY,
            col()
                .w_full()
                .min_w(0.0)
                .gap(tk.r(0.125))
                .child(cell_text(method, tk.tsm(400.0), tk.fg))
                .child(cell_text(pool, tk.txs(400.0), tk.muted_fg)),
        ));
    }
    if c.wallets {
        r = r.child(td(
            tk,
            Some(tk.r(W_WALLETS)),
            true,
            PY,
            text(p.wallets.clone())
                .font(tk.tsm(400.0).features("'tnum' 1"))
                .color(tk.fg),
        ));
    }
    if c.status {
        let cell = match book_status(app, p, active) {
            Some((tone, label)) => chip(tk, tone, None, label).role("status"),
            None => el(),
        };
        r = r.child(td(tk, Some(tk.r(W_STATUS)), false, PY, cell));
    }
    r.child(td(
        tk,
        Some(tk.r(W_ACTIONS)),
        true,
        PY,
        row_actions(app, tk, layer, p, active, pending, locked, w),
    ))
}

/// The row's one visible action (Open; the open book has none) and its "⋯" menu: Book
/// settings, New book from these settings.
#[allow(clippy::too_many_arguments)]
fn row_actions(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    p: &Profile,
    active: bool,
    pending: bool,
    locked: bool,
    w: &Workspace,
) -> El {
    let mut actions = row().gap(tk.r(0.25)).center_y().justify_end();
    if !active {
        let id = p.id.clone();
        let mut open = button_surface(
            tk,
            row()
                .key(format!("books-open-{}", p.id))
                .h(tk.r(2.0))
                .px(tk.r(0.75))
                .gap(tk.r(0.375))
                .center(),
            Variant::Outline,
        )
        .role("button")
        .label(tr(
            "onboarding:books.profileCard.switchTo",
            &[("name", &p.name)],
        ))
        .focusable(!locked)
        .disabled(locked)
        .on_press(move |a: &mut App| open_book(a, &id));
        if locked {
            open = open.opacity(0.5);
        }
        if pending {
            open = open.child(spinner(tk, tk.r(0.875)));
        }
        actions = actions.child(
            open.child(
                text(t("onboarding:books.row.open"))
                    .font(tk.tsm(500.0))
                    .color(tk.fg),
            ),
        );
    }
    let menu_key = format!("books-row-actions-{}", p.id);
    let pop = format!("book:{}", p.id);
    let is_open = app.reports.books.popup.as_deref() == Some(pop.as_str());
    let more = button_surface(
        tk,
        row()
            .key(menu_key.clone())
            .size(tk.r(1.75), tk.r(1.75))
            .center(),
        Variant::Ghost,
    )
    .role("button")
    .label(tr("onboarding:books.row.actions", &[("name", &p.name)]))
    .focusable(true)
    .child(ic(&I::ELLIPSIS, tk.r(1.0), tk.fg));
    let toggle_key = pop.clone();
    let more = crate::ui::popup::trigger(
        more,
        crate::ui::popup::Kind::Menu,
        is_open,
        move |a: &mut App| {
            a.reports.books.popup = if a.reports.books.popup.as_deref() == Some(toggle_key.as_str())
            {
                None
            } else {
                Some(toggle_key.clone())
            };
        },
    );
    if is_open {
        let (ws1, p1, ws2, p2) = (w.clone(), p.clone(), w.clone(), p.clone());
        let settings_label = t("onboarding:books.row.settings");
        let copy_label = t("onboarding:books.row.copy");
        let items = vec![
            (
                kit::menu_item(
                    tk,
                    &format!("books-row-settings-{}", p.id),
                    &I::SETTINGS_2,
                    &settings_label,
                )
                .on_press(move |s: &mut App| dialogs::request_book_settings(s, &ws1, &p1)),
                format!("books-row-settings-{}", p.id),
                settings_label,
            ),
            (
                kit::menu_item(
                    tk,
                    &format!("books-row-copy-{}", p.id),
                    &I::COPY,
                    &copy_label,
                )
                .on_press(move |s: &mut App| dialogs::request_create_profile(s, &ws2, Some(&p2))),
                format!("books-row-copy-{}", p.id),
                copy_label,
            ),
        ];
        kit::dropdown(
            tk,
            layer,
            &menu_key,
            tk.r(15.0),
            Placement::BottomEnd,
            items,
        );
    }
    actions.child(more)
}
