//! Sub-pages of the Settings modal (owner request, a deliberate change from the web's stacked
//! dialogs): a multi-field flow opened from a section (add / edit backend, the AI provider
//! wizard, reset book data, delete books set, change passphrase, Touch ID enrollment) replaces
//! the section's content area instead of opening a second modal over Settings.
//!
//! The section stays mounted underneath (inert and hidden from assistive technology), so its
//! scroll position survives the visit and comes back on Back. The breadcrumb grows a third
//! crumb with a back button at its start, the page shows its title once over the form (and a
//! side rail with status and secondary actions on a wide pane), and its actions sit in a footer
//! bar. Leaving a page with changes (Back, Cancel, Escape, another section, closing Settings,
//! a deep link) asks to discard them first.
//!
//! The forms, their validation, egress gating and daemon calls live where they did
//! (`backend`, `aiform`, `dialogs`); this module only frames them and guards leaving.

use std::hash::{Hash, Hasher};

use rune::{El, ScrollSpec, col, row, text};

use super::controls::{Btn, btn_lg};
use crate::app::App;
use crate::i18n::{t, tc};
use crate::theme::Tk;

/// Which sub-page is open. Each one's open flag stays in its form state (the replies that
/// finish a flow clear it), so this is derived, never stored on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Backend,
    AiProvider,
    ResetBook,
    DeleteBooks,
    ChangePassphrase,
    TouchId,
}

impl Kind {
    pub fn slug(self) -> &'static str {
        match self {
            Kind::Backend => "backend",
            Kind::AiProvider => "ai-provider",
            Kind::ResetBook => "reset-book",
            Kind::DeleteBooks => "delete-books",
            Kind::ChangePassphrase => "change-passphrase",
            Kind::TouchId => "touch-id",
        }
    }
}

/// The open page's frame: the section it came from, what its fields held when it opened
/// (hashed per field, so no second copy of a typed secret is kept), and the control focus
/// returns to.
pub struct Sub {
    pub kind: Kind,
    pub parent: usize,
    pub baseline: Vec<u64>,
    pub trigger: Option<String>,
    /// Focus moves into the page on the first poll after it opens.
    pub focus_pending: bool,
}

/// Where a confirmed leave goes.
#[derive(Clone, Debug, PartialEq)]
pub enum After {
    Back,
    Section(usize),
    Close,
    /// A `/settings/<slug>#<anchor>` request (search result, deep link) to run afterwards.
    Link(String),
}

/// The page whose form is open, if any.
pub fn active(app: &App) -> Option<Kind> {
    let s = &app.settings;
    if s.backend.open {
        Some(Kind::Backend)
    } else if s.aiform.open {
        Some(Kind::AiProvider)
    } else if s.dialog.reset_open {
        Some(Kind::ResetBook)
    } else if s.dialog.delete_open {
        Some(Kind::DeleteBooks)
    } else if s.dialog.pass_open {
        Some(Kind::ChangePassphrase)
    } else if s.dialog.touch_open {
        Some(Kind::TouchId)
    } else {
        None
    }
}

fn h<T: Hash + ?Sized>(v: &T) -> u64 {
    let mut s = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut s);
    s.finish()
}

/// One hash per user-editable value of the page's form.
fn fingerprint(app: &App, kind: Kind) -> Vec<u64> {
    let s = &app.settings;
    match kind {
        Kind::Backend => {
            let f = &s.backend;
            vec![
                h(f.type_id),
                h(f.source),
                h(&f.preset),
                h(&f.name.text),
                h(&f.url.text),
                h(&f.auth),
                h(&f.auth_val.text),
                h(&f.auth_val2.text),
                h(&f.e_host.text),
                h(&f.e_port.text),
                h(&f.e_ssl),
                h(&f.trust_ssl),
                h(&f.owner.map(|o| o.wire())),
                h(&f.certificate.text),
                h(&f.sp),
                h(&f.sp_file.text),
                h(&f.sp_path.text),
                h(&f.use_proxy),
                h(&f.proxy_host.text),
                h(&f.proxy_port.text),
                h(&f.ln_cli.text),
                h(&f.ln_dir.text),
                h(&f.rpc_file.text),
            ]
        }
        Kind::AiProvider => {
            let f = &s.aiform;
            vec![
                h(&f.preset),
                h(&f.name.text),
                h(&f.base_url.text),
                h(&f.api_key.text),
                h(&f.model.text),
                h(&f.kind),
                h(&f.notes.text),
                h(&f.remote_ack),
                h(f.api_format),
                h(&f.omit_store),
                h(&f.restrict),
            ]
        }
        Kind::ResetBook => {
            let d = &s.dialog;
            vec![
                h(&d.reset_pass.text),
                h(&d.reset_confirm.text),
                h(&d.reset_ack.text),
                h(&d.reset_rates),
                h(&d.reset_wipe),
                h(&d.reset_wipe_ack.text),
            ]
        }
        Kind::DeleteBooks => {
            let d = &s.dialog;
            vec![
                h(&d.delete_pass.text),
                h(&d.delete_confirm.text),
                h(&d.delete_ack.text),
            ]
        }
        Kind::ChangePassphrase => {
            let d = &s.dialog;
            vec![
                h(&d.pass_current.text),
                h(&d.pass_new.text),
                h(&d.pass_confirm.text),
            ]
        }
        Kind::TouchId => vec![h(&s.dialog.touch_pass.text)],
    }
}

/// How many values differ from when the page opened.
pub fn changes(app: &App) -> usize {
    let (Some(kind), Some(sub)) = (active(app), app.settings.sub.as_ref()) else {
        return 0;
    };
    if sub.kind != kind {
        return 0;
    }
    fingerprint(app, kind)
        .iter()
        .zip(&sub.baseline)
        .filter(|(a, b)| a != b)
        .count()
}

/// Called once a form's open function has filled it: frames it as a page of the current
/// section and records what "unchanged" means.
pub fn opened(app: &mut App, kind: Kind, trigger: Option<String>) {
    let baseline = fingerprint(app, kind);
    app.settings.select = None;
    app.settings.discard = None;
    app.settings.sub = Some(Sub {
        kind,
        parent: super::current(app),
        baseline,
        trigger,
        focus_pending: true,
    });
}

/// Re-takes the baseline (a form whose open fills it in two steps, e.g. a preset applied).
pub fn rebase(app: &mut App) {
    if let Some(kind) = active(app) {
        let baseline = fingerprint(app, kind);
        if let Some(sub) = app.settings.sub.as_mut().filter(|s| s.kind == kind) {
            sub.baseline = baseline;
        }
    }
}

/// The open page's values now (for [`adopt`]).
pub fn snapshot(app: &App) -> Option<Vec<u64>> {
    active(app).map(|k| fingerprint(app, k))
}

/// A value the daemon has just saved for the user (a trusted certificate) is not an unsaved
/// change: fields that were unchanged before (`before`) take their new value as the baseline;
/// fields the user had edited stay edited.
pub fn adopt(app: &mut App, before: Option<Vec<u64>>) {
    let (Some(kind), Some(before)) = (active(app), before) else {
        return;
    };
    let now = fingerprint(app, kind);
    if let Some(sub) =
        app.settings.sub.as_mut().filter(|s| {
            s.kind == kind && s.baseline.len() == now.len() && before.len() == now.len()
        })
    {
        for i in 0..now.len() {
            if before[i] == sub.baseline[i] {
                sub.baseline[i] = now[i];
            }
        }
    }
}

/// Shuts the open page (its form state is dropped with the flag; nothing is sent).
pub fn close_page(app: &mut App) {
    let s = &mut app.settings;
    s.backend.open = false;
    s.aiform.open = false;
    s.dialog.reset_open = false;
    s.dialog.delete_open = false;
    s.dialog.pass_open = false;
    s.dialog.touch_open = false;
    s.sub = None;
    s.discard = None;
    s.select = None;
}

/// Leaves the open page: at once when nothing changed, else after the discard prompt.
pub fn leave(app: &mut App, after: After) {
    if active(app).is_none() {
        finish(app, after);
        return;
    }
    if changes(app) > 0 {
        app.settings.select = None;
        app.settings.discard = Some(after);
        return;
    }
    close_page(app);
    finish(app, after);
}

/// "Discard" in the prompt.
pub fn discard_confirmed(app: &mut App) {
    if let Some(after) = app.settings.discard.take() {
        close_page(app);
        finish(app, after);
    }
}

fn finish(app: &mut App, after: After) {
    match after {
        After::Back => {}
        After::Section(i) => {
            super::open_index(app, i);
        }
        After::Close => super::close(app),
        After::Link(slug) => super::open_section(app, &slug),
    }
}

/// Every frame (with `Tx`): a page closed by its own flow (saved, deleted) drops its frame;
/// a page just opened takes focus.
pub fn poll(app: &mut App, tx: &mut rune::Tx) -> bool {
    let kind = active(app);
    let mut changed = false;
    match (kind, app.settings.sub.as_ref()) {
        (None, Some(_)) => {
            app.settings.sub = None;
            app.settings.discard = None;
            changed = true;
        }
        // Opened without `opened` (a scene, say): frame it now.
        (Some(k), None) => {
            opened(app, k, None);
            changed = true;
        }
        (Some(k), Some(s)) if s.kind != k => {
            opened(app, k, None);
            changed = true;
        }
        _ => {}
    }
    if let (Some(k), Some(sub)) = (kind, app.settings.sub.as_mut())
        && sub.focus_pending
        && app.shell.settings_open
    {
        sub.focus_pending = false;
        tx.focus(first_focus(app, k), false);
    }
    changed
}

/// Headless (`prime`): the frame without focus moves.
pub fn settle(app: &mut App) {
    match (active(app), app.settings.sub.as_ref()) {
        (None, Some(_)) => app.settings.sub = None,
        (Some(k), None) => opened(app, k, None),
        (Some(k), Some(s)) if s.kind != k => opened(app, k, None),
        _ => {}
    }
}

/// Where focus lands when a page opens: the passphrase forms' first field, else the back
/// control (Tab then walks the form in order).
fn first_focus(app: &App, kind: Kind) -> String {
    let enc = super::security::encrypted(app);
    match kind {
        Kind::ResetBook => if enc {
            "set-reset-pass"
        } else {
            "set-reset-ack"
        }
        .into(),
        Kind::DeleteBooks => if enc {
            "set-delete-pass"
        } else {
            "set-delete-ack"
        }
        .into(),
        Kind::ChangePassphrase => "set-pass-current".into(),
        Kind::TouchId => "set-touch-pass".into(),
        _ => "set-page-back".into(),
    }
}

/// The parent section of the open page (the section the rail highlights).
pub fn parent(app: &App) -> Option<usize> {
    active(app).map(|_| {
        app.settings
            .sub
            .as_ref()
            .map(|s| s.parent)
            .unwrap_or_else(|| super::current(app))
    })
}

/// What a page's form hands the frame.
pub struct Parts {
    pub title: String,
    pub description: Option<String>,
    /// Under the heading (the AI wizard's stepper).
    pub head_extra: Option<El>,
    pub body: El,
    /// The footer's left end (Delete, an error line).
    pub footer_left: Option<El>,
    /// The footer's right end: secondary actions, then the primary one.
    pub actions: Vec<El>,
    /// The body's scroll key (a wizard step change starts at the top).
    pub scroll_key: String,
    /// What the page explains rather than asks (a destructive flow's consequences): a card in
    /// the side rail, above the form when the pane is one column.
    pub aside: Option<String>,
    /// The side rail's own cards (status, summary, secondary actions), below the form when the
    /// pane is one column.
    pub side: Vec<El>,
}

/// A side-rail card (`rounded-lg border bg-background p-4`), its title `text-sm font-semibold`.
pub fn side_card(tk: &Tk, key: &str, title: Option<String>, children: Vec<El>) -> El {
    let mut c = col()
        .key(key.to_owned())
        .w_full()
        .padding(tk.r(1.0))
        .gap(tk.r(0.5))
        .radius(tk.r(0.5))
        .fill(tk.background)
        .border(1.0, tk.border)
        .border_layout();
    if let Some(tt) = title {
        c = c.child(
            text(tt)
                .font(tk.tsm(600.0))
                .color(tk.fg)
                .role("heading")
                .level(3),
        );
    }
    c.children(children)
}

/// The width the open page has: the modal's content area (its rail and padding off), as
/// `modal` sizes them.
fn pane_width(app: &App, tk: &Tk) -> f32 {
    let w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    let pad = if w >= 1024.0 {
        tk.r(2.5)
    } else if w >= 640.0 {
        tk.r(1.5)
    } else {
        0.0
    };
    let mw = (w - 2.0 * pad).min(if w >= 640.0 { tk.r(80.0) } else { w });
    let rail = if w < 768.0 { tk.r(3.5) } else { tk.r(15.0) };
    mw - rail - 2.0 * tk.r(1.25)
}

/// Two columns (form | side rail) from this pane width; one column below it.
const TWO_COLUMNS_FROM: f32 = 780.0;

/// The page title (also the breadcrumb's last crumb).
pub fn title(app: &App, kind: Kind) -> String {
    match kind {
        Kind::Backend => t(&format!("{}.title", super::backend::copy_key(app))),
        Kind::AiProvider => t(if app.settings.aiform.initial.is_some() {
            "settings:aiProvider.editTitle"
        } else {
            "settings:aiProvider.addTitle"
        }),
        Kind::ResetBook => t("settings:resetBook.title"),
        Kind::DeleteBooks => t("settings:deleteBooks.title"),
        Kind::ChangePassphrase => t("settings:changePassphrase.title"),
        Kind::TouchId => t("settings:touchIdEnroll.title"),
    }
}

fn section_label(i: usize) -> String {
    t(&format!(
        "settings:nav.section.{}.label",
        super::SECTIONS[i].key
    ))
}

/// The page, laid over the (inert) section in the modal's content area.
pub fn view(app: &App, tk: &Tk, kind: Kind, layer: &mut Vec<El>) -> El {
    let parts = match kind {
        Kind::Backend => super::backend::page(app, tk, layer),
        Kind::AiProvider => super::aiform::page(app, tk),
        Kind::ResetBook => super::dialogs::reset_page(app, tk),
        Kind::DeleteBooks => super::dialogs::delete_page(app, tk),
        Kind::ChangePassphrase => super::dialogs::pass_page(app, tk),
        Kind::TouchId => super::dialogs::touch_page(app, tk),
    };
    // The breadcrumb (with its back button) is the navigation; the page shows its title once,
    // with a one-line subtitle, over the form and, on a wide pane, a side rail.
    let mut heading = col()
        .w_full()
        .gap(tk.r(0.25))
        .pb(tk.r(0.75))
        .mb(tk.r(1.0))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .child(
            text(parts.title.clone())
                .font(tk.tlg(600.0).lh(tk.r(1.125) * 1.25))
                .color(tk.fg)
                .role("heading")
                .level(2),
        );
    if let Some(d) = parts.description {
        heading = heading.child(
            text(d)
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .max_w(tk.r(48.0)),
        );
    }
    let two = pane_width(app, tk) >= TWO_COLUMNS_FROM
        && (parts.aside.is_some() || !parts.side.is_empty());
    let aside = parts.aside.map(|a| {
        side_card(
            tk,
            "set-page-aside",
            None,
            vec![text(a).font(tk.tsm(400.0)).color(tk.muted_fg).wrap()],
        )
    });
    let mut main = col().w_full().min_w(0.0);
    if let Some(e) = parts.head_extra {
        main = main.child(e.mb(tk.r(1.0)));
    }
    let inner = if two {
        let mut rail = col()
            .key("set-page-side")
            .w(tk.r(if aside.is_some() { 20.0 } else { 18.0 }))
            .gap(tk.r(1.0))
            .role("complementary")
            .label(parts.title.clone());
        if let Some(a) = aside {
            rail = rail.child(a);
        }
        rail = rail.children(parts.side);
        col()
            .w_full()
            .max_w(1100.0)
            .min_w(0.0)
            .child(heading)
            .child(
                row()
                    .w_full()
                    .gap(tk.r(1.5))
                    .items_start()
                    .child(main.grow(1.0).max_w(tk.r(48.0)).child(parts.body))
                    .child(rail),
            )
    } else {
        let mut c = col().w_full().max_w(tk.r(48.0)).min_w(0.0).child(heading);
        if let Some(a) = aside {
            c = c.child(a.mb(tk.r(1.0)));
        }
        c = c.child(main.child(parts.body));
        if !parts.side.is_empty() {
            c = c.child(
                col()
                    .key("set-page-side")
                    .w_full()
                    .mt(tk.r(1.25))
                    .gap(tk.r(1.0))
                    .role("complementary")
                    .label(parts.title.clone())
                    .children(parts.side),
            );
        }
        c
    };
    let scroll = col()
        .key(parts.scroll_key)
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .padding(tk.r(1.25))
                .pb(tk.r(2.0))
                .child(inner),
        );
    let footer = row()
        .key("set-page-footer")
        .w_full()
        .px(tk.r(1.25))
        .py(tk.r(0.75))
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .fill(tk.background)
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout()
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .gap(tk.r(0.5))
                .center_y()
                .children(parts.footer_left),
        )
        .child(row().gap(tk.r(0.5)).center_y().children(parts.actions));
    let mut page = col()
        .key(format!("set-page-{}", kind.slug()))
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(tk.background)
        .role("region")
        .label(parts.title)
        .child(scroll)
        .child(footer);
    if let Some(tr) = app.settings.sub.as_ref().and_then(|s| s.trigger.clone()) {
        page = page.focus_return(tr);
    }
    page
}

/// The page's Cancel: the same leave as Back.
pub fn cancel_button(tk: &Tk, key: &str, enabled: bool) -> El {
    let b = btn_lg(tk, key, &t("common:actions.cancel"), Btn::Outline, enabled);
    if enabled {
        b.on_press(|a: &mut App| leave(a, After::Back))
    } else {
        b
    }
}

/// The discard prompt: "N unsaved changes", Cancel keeps editing, Discard leaves.
pub fn discard_dialog(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let n = changes(app).max(1);
    let msg = tc("transactions:sheet.footer.unsavedChanges", n, &[]);
    let c = col()
        .w_full()
        .gap(tk.r(1.0))
        .role("alertdialog")
        .label(msg.clone())
        .child(super::controls::dialog_title(tk, msg))
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .justify_end()
                .child(
                    btn_lg(
                        tk,
                        "set-discard-cancel",
                        &t("common:actions.cancel"),
                        Btn::Outline,
                        true,
                    )
                    .autofocus()
                    .on_press(|a: &mut App| a.settings.discard = None),
                )
                .child(
                    btn_lg(
                        tk,
                        "set-discard-ok",
                        &t("transactions:sheet.footer.discard"),
                        Btn::Destructive,
                        true,
                    )
                    .on_press(discard_confirmed),
                ),
        );
    super::controls::dialog(
        app,
        tk,
        layer,
        "set-discard-dialog",
        tk.r(24.0),
        c,
        |a| a.settings.discard = None,
        true,
    );
}

/// The breadcrumb's middle crumb while a page is open: pressing it is Back.
pub fn crumb(tk: &Tk, parent: usize) -> El {
    let label = section_label(parent);
    row()
        .key("set-crumb-section")
        .radius(tk.r(0.25))
        .px(tk.r(0.125))
        .role("link")
        .label(label.clone())
        .focusable(true)
        .hover(tk.accent)
        .on_press(|a: &mut App| leave(a, After::Back))
        .child(text(label).font(tk.tsm(400.0)).color(tk.muted_fg))
}

/// Page anchors a deep link may name (`/settings/<slug>#<anchor>`): the section they belong to.
pub const ANCHORS: &[(&str, usize)] = &[
    ("addBackend", super::BITCOIN),
    ("addBackend", super::LIQUID),
    ("addBackend", super::LIGHTNING),
    ("addProvider", super::AI),
    ("changePassphrase", super::SECURITY),
];

/// Opens the page a deep link names, once its section shows. Returns false when the anchor is
/// not a page of `sec`. `changePassphrase` waits for `status` (the button needs encryption) and
/// falls back to washing the passphrase row.
pub fn open_anchor(app: &mut App, sec: usize, anchor: &str) -> bool {
    if !ANCHORS.iter().any(|(a, s)| *a == anchor && *s == sec) {
        return false;
    }
    match anchor {
        "addBackend" => {
            let ty = match sec {
                super::LIQUID => "liquid",
                super::LIGHTNING => "lnd",
                _ => "bitcoin",
            };
            let id = match sec {
                super::LIQUID => "liquid",
                super::LIGHTNING => "lightning",
                _ => "bitcoin",
            };
            super::backend::open_add(app, Some(ty));
            if let Some(s) = app.settings.sub.as_mut() {
                s.trigger = Some(format!("set-add-{id}"));
            }
        }
        "addProvider" => {
            super::aiform::open_add(app);
            if let Some(s) = app.settings.sub.as_mut() {
                s.trigger = Some("set-ai-add".into());
            }
        }
        _ => {
            app.settings.pending_page = Some(("changePassphrase", app.clock));
        }
    }
    true
}

/// A page waiting on a read (`pending_page`): resolved once the read is in.
pub fn resolve_pending(app: &mut App) -> bool {
    let Some((name, at)) = app.settings.pending_page else {
        return false;
    };
    if name == "changePassphrase" {
        if app.read("status").is_none() {
            if app.clock - at > 5.0 {
                app.settings.pending_page = None;
                return true;
            }
            return false;
        }
        app.settings.pending_page = None;
        if super::security::encrypted(app) {
            super::dialogs::open_change_passphrase(app);
        } else {
            app.settings.pending_anchor = Some(("passphrase", app.clock));
        }
        return true;
    }
    app.settings.pending_page = None;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_anchors_name_known_sections() {
        for (a, s) in ANCHORS {
            assert!(!a.is_empty());
            assert!(*s < super::super::SECTIONS.len());
        }
    }
}
