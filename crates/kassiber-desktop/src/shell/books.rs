//! The book crumb and its switcher (BookSwitcherPopover.tsx): books grouped by book set from
//! `ui.profiles.snapshot`, filtered by the search field, switched with `ui.profiles.switch`.
//! Book-set (project) switching from the user menu uses `ui.projects.select`.

use rune::{Color, El, Key, KeyEvent, Tx, col, el, row, text};
use rune_kit::{Placement, TextInput};
use serde_json::{Value, json};

use super::Popup;
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

#[derive(Clone, Debug)]
pub struct BookRow {
    pub workspace_id: String,
    pub workspace: String,
    pub currency: String,
    pub profile_id: String,
    pub name: String,
    pub wallets: usize,
    pub policy: String,
    pub active: bool,
}

pub fn rows(app: &App) -> Vec<BookRow> {
    let Some(p) = app.read("ui.profiles.snapshot") else {
        return vec![];
    };
    let active = p["activeProfileId"].as_str().unwrap_or_default();
    let q = app.shell.books_filter.text.trim().to_lowercase();
    let mut out = Vec::new();
    for ws in p["workspaces"].as_array().into_iter().flatten() {
        let wname = ws["name"].as_str().unwrap_or_default().to_owned();
        for prof in ws["profiles"].as_array().into_iter().flatten() {
            let name = prof["name"].as_str().unwrap_or_default().to_owned();
            if !q.is_empty()
                && !name.to_lowercase().contains(&q)
                && !wname.to_lowercase().contains(&q)
            {
                continue;
            }
            let id = prof["id"].as_str().unwrap_or_default().to_owned();
            out.push(BookRow {
                workspace_id: ws["id"].as_str().unwrap_or_default().to_owned(),
                workspace: wname.clone(),
                currency: ws["currency"].as_str().unwrap_or_default().to_owned(),
                active: id == active,
                profile_id: id,
                name,
                wallets: prof["wallets"].as_u64().unwrap_or(0) as usize,
                policy: prof["taxPolicy"].as_str().unwrap_or_default().to_owned(),
            });
        }
    }
    out
}

pub fn switch_to(app: &mut App, profile_id: &str, active: bool) {
    if active {
        app.shell.popup = None;
        return;
    }
    app.shell.books_error = None;
    app.shell.switching_profile = Some(profile_id.to_owned());
    app.send(
        "ui.profiles.switch",
        json!({"profile_id": profile_id}),
        "shell.switch_profile",
    );
}

pub fn on_switched(app: &mut App, r: &Reply) {
    app.shell.switching_profile = None;
    if r.error.is_some() {
        app.shell.books_error = Some(t("chrome:bookSwitcher.switchError"));
        return;
    }
    // Another book: `App::deliver` starts a new session once every screen heard the answer.
    app.shell.popup = None;
}

/// Switches the book set (project container). The project list comes from the account's
/// default state root, so this can open a real book: only ever from an explicit pick.
pub fn select_project(app: &mut App, id: &str) {
    app.shell.popup = None;
    // Another book set is never opened by the demo's daemon: the demo ends first.
    if app.demo.active.is_some() {
        crate::demo::leave_then(app, crate::demo::Then::SelectProject(id.to_owned()));
        return;
    }
    app.send(
        "ui.projects.select",
        json!({"project_id": id}),
        "shell.select_project",
    );
}

pub fn on_project_selected(app: &mut App, r: &Reply) {
    match &r.error {
        Some(e) => app.toast(
            ToastTone::Error,
            &t("chrome:projects.switchFailedTitle"),
            Some(format!("{} {e}", t("chrome:projects.switchFailedBody"))),
            Some("project"),
        ),
        None => {
            app.begin_book_session();
            app.navigate("/overview");
        }
    }
}

fn book_label(app: &App) -> String {
    app.snapshot()
        .and_then(|s| s.book_label())
        .unwrap_or_else(|| t("chrome:shell.booksFallback"))
}

fn initials(name: &str) -> String {
    let s: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect();
    s.to_uppercase()
}

fn on_list_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    // Escape closes the popover (the field would otherwise keep it), focus back on the crumb.
    if ev.key == Key::Escape {
        s.shell.popup = None;
        return true;
    }
    let n = rows(s).len();
    if n == 0 {
        return false;
    }
    match ev.key {
        Key::Down => s.shell.books_sel = (s.shell.books_sel + 1) % n,
        Key::Up => s.shell.books_sel = (s.shell.books_sel + n - 1) % n,
        Key::Home => s.shell.books_sel = 0,
        Key::End => s.shell.books_sel = n - 1,
        Key::Enter => {
            let r = rows(s);
            if let Some(b) = r.get(s.shell.books_sel.min(n - 1)) {
                switch_to(s, &b.profile_id.clone(), b.active);
            }
        }
        _ => return false,
    }
    let _ = tx;
    true
}

pub fn crumb(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let open = app.shell.popup == Some(Popup::Books);
    let book = book_label(app);
    // The demo tag is drawn only; its name joins the book's.
    let spoken = if app.demo.active.is_some() {
        format!("{book}, {}", t("chrome:demo.region"))
    } else {
        book.clone()
    };
    let trigger = row()
        .key("sh-book-crumb")
        // `px-1 -mx-1`: the hover fill bleeds past the text, which stays on the crumb's edge.
        .px(tk.r(0.25))
        .mx(-tk.r(0.25))
        .py(tk.r(0.125))
        .gap(tk.r(0.375))
        .center_y()
        .radius(tk.r(0.375))
        .fill(if open {
            tk.row_active
        } else {
            Color::TRANSPARENT
        })
        .hover(tk.row_hover)
        .role("button")
        .label(tr("chrome:shell.switchBooksLabel", &[("book", &spoken)]))
        // In the demo the tag's explanation is the hover help (the name keeps the action).
        .help(if app.demo.active.is_some() {
            t("chrome:demo.body")
        } else {
            tr("chrome:shell.switchBooksLabel", &[("book", &book)])
        })
        .expanded(open)
        .focusable(true)
        .no_window_drag()
        .on_press(|s: &mut App| {
            if s.read("ui.profiles.snapshot").is_none() {
                s.send("ui.profiles.snapshot", Value::Null, "app.profiles");
            }
            s.shell.toggle(Popup::Books);
        })
        .child(ic(&I::FOLDER, tk.r(0.875), tk.muted_fg))
        .child(
            text(book)
                .font(tk.tsm(500.0))
                .color(tk.muted_fg)
                .max_w(tk.r(10.0))
                .truncate(),
        )
        .children(crate::demo::tag(app, tk))
        .child(ic(&I::CHEVRONS_UP_DOWN, tk.r(0.75), tk.muted_fg.alpha(0.5)));
    if open {
        super::mount_popup_at(
            tk,
            layer,
            "sh-books",
            "sh-book-crumb",
            Placement::BottomStart,
            popover(app, tk),
            false,
            6.0,
        );
    }
    trigger
}

fn popover(app: &App, tk: &Tk) -> El {
    let k = tk.kit();
    let input = TextInput::new(&app.shell.books_filter)
        .placeholder(t("chrome:bookSwitcher.searchPlaceholder"))
        .label(t("chrome:bookSwitcher.title"))
        .leading_icon(&I::SEARCH)
        .plain()
        .height(tk.r(2.5))
        .text_style(tk.tsm(400.0))
        .on_edit(|s: &mut App| &mut s.shell.books_filter)
        .on_change(|s: &mut App, _| s.shell.books_sel = 0)
        .on_key(on_list_key)
        .build(&k)
        .key("sh-books-input")
        .w_full()
        .px(tk.r(0.5));
    let list = rows(app);
    let mut body = col()
        .key("sh-books-list")
        .w_full()
        .max_h(tk.r(20.0))
        .padding(super::menu_pad(tk))
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        });
    if app.read("ui.profiles.snapshot").is_none() {
        body = body.child(note(tk, &t("chrome:bookSwitcher.loading")));
    } else if list.is_empty() {
        body = body.child(note(
            tk,
            &t(if app.shell.books_filter.text.is_empty() {
                "chrome:bookSwitcher.empty"
            } else {
                "chrome:bookSwitcher.noMatches"
            }),
        ));
    }
    let mut last_ws = String::new();
    for (i, b) in list.iter().enumerate() {
        if b.workspace_id != last_ws {
            last_ws = b.workspace_id.clone();
            let ws = b.workspace_id.clone();
            body = body.child(
                // `flex h-7 items-center gap-2 pr-1 pl-2 text-xs font-medium text-muted-foreground`
                row()
                    .key(format!("sh-ws-{}", b.workspace_id))
                    .w_full()
                    .h(tk.r(1.75))
                    .pl(tk.r(0.5))
                    .pr(tk.r(0.25))
                    .gap(tk.r(0.5))
                    .center_y()
                    .child(
                        text(b.workspace.clone())
                            .font(tk.txs(500.0))
                            .color(tk.muted_fg)
                            .truncate(),
                    )
                    .child(
                        text(b.currency.clone())
                            .font(tk.txs(500.0))
                            .color(tk.muted_fg.alpha(0.7))
                            .grow(1.0),
                    )
                    .child(
                        row()
                            .key(format!("sh-ws-eye-{}", b.workspace_id))
                            .size(tk.r(1.5), tk.r(1.5))
                            .center()
                            .radius(tk.r(0.25))
                            .hover(tk.accent)
                            .role("link")
                            .label(t("chrome:bookSwitcher.bookSetOverview"))
                            .help(t("chrome:bookSwitcher.bookSetOverview"))
                            .on_press(move |s: &mut App| {
                                s.shell.popup = None;
                                s.navigate(&format!("/books/{ws}/birds-eye"));
                            })
                            .child(ic(&I::EYE, tk.r(0.875), tk.muted_fg)),
                    ),
            );
        }
        let selected = i == app.shell.books_sel.min(list.len().saturating_sub(1));
        let pending = app.shell.switching_profile.as_deref() == Some(b.profile_id.as_str());
        let (id, active) = (b.profile_id.clone(), b.active);
        let wallets = tc("chrome:bookSwitcher.wallets", b.wallets, &[]);
        // `flex items-center gap-2.5 rounded-sm px-2 py-1.5 text-sm`; the option's name is its
        // text content (initials, name, detail), as the web has no aria-label on it.
        let detail = format!("{wallets} · {}", b.policy);
        let mut r = row()
            .key(format!("sh-book-{}", b.profile_id))
            .w_full()
            .px(tk.r(0.5))
            .py(tk.r(0.375))
            .gap(tk.r(0.625))
            .center_y()
            .radius(tk.r(0.25))
            .fill(if selected {
                tk.accent
            } else {
                Color::TRANSPARENT
            })
            .role("option")
            .checked(active)
            .label(format!("{} {} {detail}", initials(&b.name), b.name))
            .on_press(move |s: &mut App| switch_to(s, &id, active))
            .on_hover(move |s: &mut App| s.shell.books_sel = i)
            .child(
                row()
                    .size(tk.r(1.75), tk.r(1.75))
                    .center()
                    .radius(tk.r(0.375))
                    .fill(if active { tk.primary } else { tk.muted })
                    .child(
                        text(initials(&b.name))
                            .font(tk.txs(500.0))
                            .color(if active { tk.primary_fg } else { tk.muted_fg }),
                    ),
            )
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(
                        text(b.name.clone())
                            .font(tk.tsm(500.0))
                            .color(tk.fg)
                            .truncate(),
                    )
                    .child(
                        text(detail)
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .truncate(),
                    ),
            );
        if pending {
            r = r.child(
                ic(&I::LOADER_CIRCLE, tk.r(1.0), tk.muted_fg)
                    .looping(rune::LoopSpec::rotation(1.0)),
            );
        } else if active {
            r = r.child(ic(&I::CHECK, tk.r(1.0), tk.fg));
        }
        body = body.child(r);
    }
    let mut out = col()
        .w(tk.r(20.0))
        .role("dialog")
        .label(t("chrome:bookSwitcher.title"))
        .child(input)
        .child(el().h(1.0).w_full().fill(tk.border))
        .child(body);
    if let Some(e) = &app.shell.books_error {
        out = out.child(
            row()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.375))
                .child(text(e.clone()).font(tk.txs(400.0)).color(tk.alert.text)),
        );
    }
    // `border-t p-1`, then the link row `gap-2 rounded-sm px-2 py-1.5 text-sm`.
    out.child(el().h(1.0).w_full().fill(tk.border)).child(
        col()
            .w_full()
            .padding(super::menu_pad(tk))
            .children(crate::demo::switcher_rows(app, tk))
            .child(
                row()
                    .key("sh-books-manage")
                    .w_full()
                    .px(tk.r(0.5))
                    .py(tk.r(0.375))
                    .gap(tk.r(0.5))
                    .center_y()
                    .radius(tk.r(0.25))
                    .hover(tk.accent)
                    .role("link")
                    .label(t("chrome:bookSwitcher.manageBooks"))
                    .on_press(|s: &mut App| {
                        s.shell.popup = None;
                        s.navigate("/books");
                    })
                    .child(ic(&I::SETTINGS_2, tk.r(1.0), tk.muted_fg))
                    .child(
                        text(t("chrome:bookSwitcher.manageBooks"))
                            .font(tk.tsm(400.0))
                            .color(tk.fg),
                    ),
            ),
    )
}

fn note(tk: &Tk, s: &str) -> El {
    row()
        .w_full()
        .px(tk.r(0.5))
        .py(tk.r(1.0))
        .justify_center()
        .child(text(s.to_owned()).font(tk.tsm(400.0)).color(tk.muted_fg))
}
