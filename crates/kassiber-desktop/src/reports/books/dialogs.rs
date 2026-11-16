//! The Books dialogs (CreateProfileDialog, RenameWorkspaceDialog, CreateWorkspaceDialog in
//! Books.tsx) and their mutations. Book settings (RenameProfileDialog's successor) is
//! `super::settings`; its apply and rename replies land here too.

use rune::{El, TextEdit, Tx, col, row, text};
use rune_kit::TextInput;
use serde_json::{Value, json};

use super::kit::{
    dialog, dialog_header, error_box, identity_box, labelled, note, preview_badge, select_menu,
    select_trigger,
};
use super::model::{self, Profile, Workspace};
use super::{Dialog, error_text};
use crate::app::{App, Reply};
use crate::i18n::{t, tr};
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button, field, form_kit};

const DEFAULT_SOURCE: &str = "__default_settings__";

fn fresh_name(text: &str) -> TextEdit {
    let mut e = TextEdit::new(text);
    e.focused = true;
    e
}

/// `requestCreateProfile`: seeds region and method from the source book, else the set's first.
pub fn request_create_profile(app: &mut App, ws: &Workspace, source: Option<&Profile>) {
    let base = source.or_else(|| ws.profiles.first());
    let country = base
        .map(|p| p.country().to_owned())
        .unwrap_or_else(|| "generic".into());
    let method = base
        .and_then(|p| p.gains_algorithm.clone())
        .unwrap_or_else(|| {
            model::methods_for(&country)
                .first()
                .cloned()
                .unwrap_or_default()
        });
    let b = &mut app.reports.books;
    b.popup = None;
    b.error = None;
    b.name = fresh_name("");
    b.dialog = Some(Dialog::CreateProfile {
        ws: ws.id.clone(),
        source: source.map(|p| p.id.clone()),
        country,
        method,
    });
}

/// Book settings for `p` (`requestRenameProfile`).
pub fn request_book_settings(app: &mut App, ws: &Workspace, p: &Profile) {
    super::settings::open(app, &ws.id, &p.id, &p.name);
}

/// `requestRenameWorkspace`.
pub fn request_rename_workspace(app: &mut App, ws: &Workspace) {
    let b = &mut app.reports.books;
    b.popup = None;
    b.error = None;
    b.name = fresh_name(&ws.name);
    b.dialog = Some(Dialog::RenameWorkspace { ws: ws.id.clone() });
}

pub fn request_create_workspace(app: &mut App) {
    let b = &mut app.reports.books;
    b.popup = None;
    b.error = None;
    b.name = fresh_name("");
    b.dialog = Some(Dialog::CreateWorkspace);
}

/// `onOpenChange(false)`: closes and resets (not while a save runs).
pub fn close(app: &mut App) {
    let b = &mut app.reports.books;
    if b.submitting {
        return;
    }
    b.dialog = None;
    b.popup = None;
    b.error = None;
    b.name = TextEdit::new("");
    b.form = super::settings::Form::default();
}

/// A region change resets the method to the region's default.
pub fn set_country(app: &mut App, c: &str) {
    let b = &mut app.reports.books;
    b.error = None;
    if let Some(Dialog::CreateProfile {
        country, method, ..
    }) = b.dialog.as_mut()
    {
        *country = c.to_owned();
        *method = model::default_method(c);
    }
}

fn set_method(app: &mut App, m: &str) {
    if m.is_empty() {
        return;
    }
    let b = &mut app.reports.books;
    b.error = None;
    if let Some(Dialog::CreateProfile { method, .. }) = b.dialog.as_mut() {
        *method = m.to_owned();
    }
}

fn set_source(app: &mut App, v: &str) {
    let b = &mut app.reports.books;
    b.error = None;
    if let Some(Dialog::CreateProfile { source, .. }) = b.dialog.as_mut() {
        *source = (v != DEFAULT_SOURCE).then(|| v.to_owned());
    }
}

/// The form's submit (Enter in the name field, or the primary button).
pub fn submit(app: &mut App) {
    if app.reports.books.submitting {
        return;
    }
    let label = app.reports.books.name.text.trim().to_owned();
    let Some(d) = app.reports.books.dialog.clone() else {
        return;
    };
    match d {
        Dialog::CreateProfile {
            ws,
            source,
            country,
            method,
        } => {
            if label.is_empty() {
                app.reports.books.error = Some(t("onboarding:books.create.errorEmptyName"));
                return;
            }
            let mut args = json!({"workspace_id": ws, "label": label});
            match source {
                Some(src) => args["source_profile_id"] = json!(src),
                None => {
                    args["tax_country"] = json!(country);
                    args["gains_algorithm"] = json!(method);
                }
            }
            send(
                app,
                "ui.profiles.create",
                args,
                "reports.books.create_profile",
            );
        }
        Dialog::Settings { .. } => super::settings::review(app),
        Dialog::RenameWorkspace { ws } => {
            if label.is_empty() {
                app.reports.books.error =
                    Some(t("onboarding:books.renameWorkspace.errorEmptyName"));
                return;
            }
            send(
                app,
                "ui.workspace.rename",
                json!({"workspace_id": ws, "label": label}),
                "reports.books.rename_ws",
            );
        }
        Dialog::CreateWorkspace => {
            if label.is_empty() {
                app.reports.books.error =
                    Some(t("onboarding:books.createWorkspace.errorEmptyName"));
                return;
            }
            send(
                app,
                "ui.workspace.create",
                json!({"label": label}),
                "reports.books.create_ws",
            );
        }
    }
}

fn send(app: &mut App, kind: &str, args: Value, tag: &'static str) {
    app.reports.books.error = None;
    if app.send(kind, args, tag).is_some() {
        app.reports.books.submitting = true;
    }
}

/// A successful save: close, and re-read what the book list and the shell show.
fn finish(app: &mut App) {
    if matches!(app.reports.books.dialog, Some(Dialog::Settings { .. })) {
        super::settings::applied(app);
    }
    let b = &mut app.reports.books;
    b.submitting = false;
    b.then_rename = None;
    b.dialog = None;
    b.popup = None;
    b.error = None;
    b.name = TextEdit::new("");
    b.form = super::settings::Form::default();
    app.refresh_reads();
    app.send("ui.projects.list", Value::Null, "app.projects");
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    let fallback = match tag {
        "reports.books.create_profile" => "onboarding:books.create.errorGeneric",
        "reports.books.update" | "reports.books.rename_profile" => {
            "onboarding:books.renameProfile.errorGeneric"
        }
        "reports.books.rename_ws" => "onboarding:books.renameWorkspace.errorGeneric",
        "reports.books.create_ws" => "onboarding:books.createWorkspace.errorGeneric",
        _ => return,
    };
    if r.error.is_some() {
        let b = &mut app.reports.books;
        b.submitting = false;
        b.then_rename = None;
        b.error = Some(error_text(r, fallback));
        // A refused change stays on its preview, with the reason in plain words.
        if tag == "reports.books.update" {
            super::settings::apply_failed(app, r);
        }
        // The update may have landed before a failed rename: show what the book holds now.
        if tag == "reports.books.rename_profile" {
            app.send("ui.profiles.snapshot", Value::Null, "app.profiles");
        }
        return;
    }
    if tag == "reports.books.update"
        && let Some((id, label)) = app.reports.books.then_rename.take()
    {
        app.send(
            "ui.profiles.rename",
            json!({"profile_id": id, "label": label}),
            "reports.books.rename_profile",
        );
        return;
    }
    finish(app);
}

// ---- View -----------------------------------------------------------------------------------

fn name_input(app: &App, tk: &Tk, key: &str, label: &str, placeholder: Option<String>) -> El {
    let b = &app.reports.books;
    let mut i = TextInput::new(&b.name)
        .label(label.to_owned())
        .height(tk.r(2.25))
        .text_style(tk.tsm(400.0))
        .invalid(b.error.is_some())
        .disabled(b.submitting);
    if let Some(p) = placeholder {
        i = i.placeholder(p);
    }
    let built = i
        .on_edit(|a: &mut App| &mut a.reports.books.name)
        .on_change(|a: &mut App, _: &mut Tx| a.reports.books.error = None)
        .on_submit(|a: &mut App, _: &mut Tx| submit(a))
        .build(&form_kit(tk))
        .key(key.to_owned())
        .w_full();
    field(tk, built)
}

/// `DialogFooter` (`sm:flex-row sm:justify-end gap-2`): an outline secondary, the submit.
fn footer(
    app: &App,
    tk: &Tk,
    key: &str,
    secondary: &str,
    on_secondary: fn(&mut App),
    primary: &str,
) -> El {
    let busy = app.reports.books.submitting;
    row()
        .w_full()
        .gap(tk.r(0.5))
        .justify_end()
        .child(
            button(
                tk,
                format!("{key}-cancel"),
                Variant::Outline,
                Size::Default,
                None,
                secondary,
                !busy,
            )
            .on_press(move |a: &mut App| {
                if !a.reports.books.submitting {
                    on_secondary(a);
                }
            }),
        )
        .child(
            button(
                tk,
                format!("{key}-submit"),
                Variant::Default,
                Size::Default,
                None,
                primary,
                !busy,
            )
            .on_press(|a: &mut App| submit(a)),
        )
}

fn sel_open(app: &App, which: &str) -> bool {
    app.reports
        .books
        .popup
        .as_deref()
        .and_then(|p| p.strip_prefix("sel:"))
        == Some(which)
}

fn toggle_sel(a: &mut App, which: &str) {
    let k = format!("sel:{which}");
    a.reports.books.popup = if a.reports.books.popup.as_deref() == Some(k.as_str()) {
        None
    } else {
        Some(k)
    };
}

/// A select field: label, trigger, and its menu when open.
#[allow(clippy::too_many_arguments)]
fn select_field(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    which: &'static str,
    label: &str,
    value: &str,
    badge: bool,
    options: Vec<(String, String, bool)>,
    current: &str,
    enabled: bool,
    full: bool,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) -> El {
    let key = format!("books-sel-{which}");
    let trig = select_trigger(
        tk,
        &key,
        label,
        value,
        badge.then(|| preview_badge(tk)),
        enabled,
        full,
    );
    let trig = crate::ui::popup::trigger(
        trig,
        crate::ui::popup::Kind::Listbox,
        sel_open(app, which) && enabled,
        move |a: &mut App| {
            if !a.reports.books.submitting {
                toggle_sel(a, which)
            }
        },
    );
    if sel_open(app, which) && enabled {
        select_menu(
            tk,
            layer,
            &key,
            if full { tk.r(25.5) } else { tk.r(8.0) },
            options,
            current,
            choose,
        );
    }
    labelled(tk, label, trig)
}

fn region_options() -> Vec<(String, String, bool)> {
    model::COUNTRIES
        .iter()
        .map(|c| (c.code.to_owned(), model::region_label(c.code), c.preview))
        .collect()
}

/// `TaxCountryPreviewNotice` (`role="note"`, `rounded-md border border-line px-3 py-2 text-xs`).
fn preview_notice(tk: &Tk, country: &str) -> Option<El> {
    model::is_preview(country).then(|| {
        let badge = t("common:taxCountry.previewBadge");
        let body = t("common:taxCountry.previewNotice");
        col()
            .w_full()
            .px(tk.r(0.75))
            .py(tk.r(0.5))
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .role("note")
            .label(format!("{badge}: {body}"))
            .child(
                text(format!("{badge}: {body}"))
                    .font(tk.txs(400.0).lh(tk.r(1.25)))
                    .color(tk.fg)
                    .wrap()
                    .w_full(),
            )
    })
}

pub fn mount(app: &App, tk: &Tk, profiles: &model::Profiles, layer: &mut Vec<El>) {
    let b = &app.reports.books;
    let Some(d) = &b.dialog else { return };
    if matches!(d, Dialog::Settings { .. }) {
        return super::settings::mount(app, tk, layer);
    }
    let busy = b.submitting;
    let mut inner: Vec<El> = Vec::new();
    let (key, title, desc, body) = match d {
        Dialog::CreateProfile {
            ws,
            source,
            country,
            method,
        } => {
            let w = profiles.workspace(ws);
            let src = source
                .as_ref()
                .and_then(|s| w.and_then(|w| w.profiles.iter().find(|p| &p.id == s)));
            let title = t("onboarding:books.create.title");
            let desc = match (w, src) {
                (Some(w), Some(s)) => tr(
                    "onboarding:books.create.descriptionFromSource",
                    &[("workspace", &w.name), ("source", &s.name)],
                ),
                (Some(w), None) => tr(
                    "onboarding:books.create.descriptionInWorkspace",
                    &[("workspace", &w.name)],
                ),
                _ => t("onboarding:books.create.descriptionDefault"),
            };
            let mut body = col()
                .w_full()
                .gap(tk.r(1.0))
                .child(dialog_header(tk, &title, &desc));
            if let Some(w) = w {
                body = body.child(identity_box(tk, &w.name, &model::workspace_meta(w, false)));
                if !w.profiles.is_empty() {
                    let mut opts = vec![(
                        DEFAULT_SOURCE.to_owned(),
                        t("onboarding:books.create.defaultSettings"),
                        false,
                    )];
                    opts.extend(
                        w.profiles
                            .iter()
                            .map(|p| (p.id.clone(), p.name.clone(), false)),
                    );
                    let cur = source.clone().unwrap_or_else(|| DEFAULT_SOURCE.into());
                    let value = src
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| t("onboarding:books.create.defaultSettings"));
                    let mut block = col()
                        .w_full()
                        .gap(tk.r(0.5))
                        .child(select_field(
                            app,
                            tk,
                            &mut inner,
                            "source",
                            &t("onboarding:books.create.startFrom"),
                            &value,
                            false,
                            opts,
                            &cur,
                            !busy,
                            true,
                            set_source,
                        ))
                        .child(
                            text(t("onboarding:books.create.copyHint"))
                                .font(tk.txs(400.0).lh(tk.r(1.25)))
                                .color(tk.muted_fg)
                                .wrap()
                                .w_full(),
                        );
                    if let Some(s) = src {
                        block = block.child(
                            row()
                                .w_full()
                                .px(tk.r(0.5))
                                .py(tk.r(0.25))
                                .radius(tk.r(0.375))
                                .border(1.0, tk.border)
                                .fill(tk.muted.alpha(0.25))
                                .child(
                                    text(s.tax_policy.clone())
                                        .font(tk.txs(400.0))
                                        .color(tk.fg)
                                        .wrap()
                                        .w_full(),
                                ),
                        );
                    }
                    body = body.child(block);
                }
            }
            body = body.child(labelled(
                tk,
                &t("onboarding:books.create.nameLabel"),
                name_input(
                    app,
                    tk,
                    "books-profile-name",
                    &t("onboarding:books.create.nameLabel"),
                    Some(t("onboarding:books.create.namePlaceholder")),
                ),
            ));
            if src.is_none() {
                body = body.child(select_field(
                    app,
                    tk,
                    &mut inner,
                    "region",
                    &t("onboarding:books.create.regionLabel"),
                    &model::region_label(country),
                    model::is_preview(country),
                    region_options(),
                    country,
                    !busy,
                    true,
                    set_country,
                ));
                if let Some(n) = preview_notice(tk, country) {
                    body = body.child(n);
                }
                let methods = model::methods_for(country);
                let opts = methods
                    .iter()
                    .map(|m| (m.clone(), model::method_label(m), false))
                    .collect();
                let mut m = col().w_full().gap(tk.r(0.5)).child(select_field(
                    app,
                    tk,
                    &mut inner,
                    "method",
                    &t("onboarding:books.create.methodLabel"),
                    &model::method_label(method),
                    false,
                    opts,
                    method,
                    !busy && methods.len() > 1,
                    true,
                    set_method,
                ));
                if country == "at" {
                    m = m.child(note(tk, &t("onboarding:books.create.austrianMethodNote")));
                }
                body = body.child(m);
            }
            (
                "books-create-profile",
                title,
                desc,
                body.child_opt(b.error.as_ref().map(|e| error_box(tk, e)))
                    .child(footer(
                        app,
                        tk,
                        "books-create-profile",
                        &t("common:actions.cancel"),
                        close,
                        &t("onboarding:books.create.submit"),
                    )),
            )
        }
        Dialog::Settings { .. } => return,
        Dialog::RenameWorkspace { ws } => {
            let title = t("onboarding:books.renameWorkspace.title");
            let desc = t("onboarding:books.renameWorkspace.description");
            let mut body = col()
                .w_full()
                .gap(tk.r(1.0))
                .child(dialog_header(tk, &title, &desc));
            if let Some(w) = profiles.workspace(ws) {
                body = body.child(identity_box(tk, &w.name, &model::workspace_meta(w, false)));
            }
            body = body.child(labelled(
                tk,
                &t("onboarding:books.renameWorkspace.nameLabel"),
                name_input(
                    app,
                    tk,
                    "books-rename-ws-name",
                    &t("onboarding:books.renameWorkspace.nameLabel"),
                    None,
                ),
            ));
            (
                "books-rename-ws",
                title,
                desc,
                body.child_opt(b.error.as_ref().map(|e| error_box(tk, e)))
                    .child(footer(
                        app,
                        tk,
                        "books-rename-ws",
                        &t("common:actions.cancel"),
                        close,
                        &t("onboarding:books.renameWorkspace.submit"),
                    )),
            )
        }
        Dialog::CreateWorkspace => {
            let title = t("onboarding:books.createWorkspace.title");
            let desc = t("onboarding:books.createWorkspace.description");
            let body = col()
                .w_full()
                .gap(tk.r(1.0))
                .child(dialog_header(tk, &title, &desc))
                .child(labelled(
                    tk,
                    &t("onboarding:books.createWorkspace.nameLabel"),
                    name_input(
                        app,
                        tk,
                        "books-ws-name",
                        &t("onboarding:books.createWorkspace.nameLabel"),
                        Some(t("onboarding:books.createWorkspace.namePlaceholder")),
                    ),
                ));
            (
                "books-create-ws",
                title,
                desc,
                body.child_opt(b.error.as_ref().map(|e| error_box(tk, e)))
                    .child(footer(
                        app,
                        tk,
                        "books-create-ws",
                        &t("common:actions.cancel"),
                        close,
                        &t("onboarding:books.createWorkspace.submit"),
                    )),
            )
        }
    };
    dialog(app, tk, layer, key, &title, &desc, body, close);
    layer.extend(inner);
}

/// `El::child` for an optional child.
trait ChildOpt {
    fn child_opt(self, c: Option<El>) -> Self;
}

impl ChildOpt for El {
    fn child_opt(self, c: Option<El>) -> Self {
        match c {
            Some(c) => self.child(c),
            None => self,
        }
    }
}
