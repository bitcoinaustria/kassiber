//! SettingsScreen's dialogs: Reset book data, Delete books set, Change passphrase, Touch ID
//! enrollment, and the confirmations the web asks through `window.confirm`. All overlay the
//! modal; Escape closes the topmost (`close_top`).

use rune::{El, TextEdit, col, row, text};
use serde_json::{Value, json};

use super::controls::{Btn, PressIf, btn_lg, checkbox, dialog, error_line, input, labelled, muted};
use super::model::{PLAINTEXT_DELETE_ACK, WIPE_EVERYTHING_ACK, passphrase_issue};
use super::send;
use super::subpage::{Parts, cancel_button};
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tr};
use crate::theme::Tk;

/// What a confirmation runs once accepted.
#[derive(Clone, Debug, PartialEq)]
pub enum Confirmed {
    ResetWelcome,
    DeleteBackend(String),
    ForgetAllUnlock,
    DeleteProvider(String),
    /// Check these providers once their off-device use is acknowledged.
    CheckProviders(Vec<String>),
}

#[derive(Default)]
pub struct Dialogs {
    pub reset_open: bool,
    pub reset_pass: TextEdit,
    pub reset_confirm: TextEdit,
    pub reset_ack: TextEdit,
    pub reset_rates: bool,
    pub reset_wipe: bool,
    pub reset_wipe_ack: TextEdit,
    pub reset_error: Option<String>,
    pub delete_open: bool,
    pub delete_pass: TextEdit,
    pub delete_confirm: TextEdit,
    pub delete_ack: TextEdit,
    pub delete_error: Option<String>,
    pub pass_open: bool,
    pub pass_current: TextEdit,
    pub pass_new: TextEdit,
    pub pass_confirm: TextEdit,
    pub pass_error: Option<String>,
    pub touch_open: bool,
    pub touch_pass: TextEdit,
    pub touch_error: Option<String>,
    pub confirm: Option<(String, Confirmed)>,
}

pub fn confirm(app: &mut App, message: String, action: Confirmed) {
    app.settings.dialog.confirm = Some((message, action));
}

/// The books set and book names (`workspaceLabel`, `bookLabel`).
fn workspace_label(app: &App) -> String {
    app.read("status")
        .and_then(|s| s["current_workspace"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| t("settings:workspace.fallback"))
}
pub fn book_label(app: &App) -> Option<String> {
    app.read("status")
        .and_then(|s| s["current_profile"].as_str())
        .map(str::to_owned)
}

pub fn open_reset_book(app: &mut App) {
    let d = &mut app.settings.dialog;
    d.reset_pass = TextEdit::new("");
    d.reset_confirm = TextEdit::new("");
    d.reset_ack = TextEdit::new("");
    d.reset_wipe_ack = TextEdit::new("");
    d.reset_rates = false;
    d.reset_wipe = false;
    d.reset_error = None;
    d.reset_open = true;
    super::subpage::opened(
        app,
        super::subpage::Kind::ResetBook,
        Some("set-reset-book".into()),
    );
}

pub fn open_delete_books(app: &mut App) {
    let d = &mut app.settings.dialog;
    d.delete_pass = TextEdit::new("");
    d.delete_confirm = TextEdit::new("");
    d.delete_ack = TextEdit::new("");
    d.delete_error = None;
    d.delete_open = true;
    super::subpage::opened(
        app,
        super::subpage::Kind::DeleteBooks,
        Some("set-delete-books".into()),
    );
}

pub fn open_change_passphrase(app: &mut App) {
    let d = &mut app.settings.dialog;
    d.pass_current = TextEdit::new("");
    d.pass_new = TextEdit::new("");
    d.pass_confirm = TextEdit::new("");
    d.pass_error = None;
    d.pass_open = true;
    super::subpage::opened(
        app,
        super::subpage::Kind::ChangePassphrase,
        Some("set-change-pass".into()),
    );
}

pub fn open_touch_id(app: &mut App) {
    let d = &mut app.settings.dialog;
    d.touch_pass = TextEdit::new("");
    d.touch_error = None;
    d.touch_open = true;
    super::subpage::opened(
        app,
        super::subpage::Kind::TouchId,
        Some("set-touchid-setup".into()),
    );
}

/// Escape: the topmost overlay over the modal closes (the small confirmations; the rebuild
/// confirmation). Sub-pages are left through `subpage::leave`.
pub fn close_top(app: &mut App) -> bool {
    // The certificate trust dialog over the backend page (stays while a request runs).
    if app.settings.backend.open && app.settings.backend.cert.open {
        super::certtrust::close(app, &super::backend::CERT_HOST);
        return true;
    }
    if app.settings.dialog.confirm.is_some() {
        cancel_confirm(app);
        return true;
    }
    if app.settings.discard.take().is_some() {
        return true;
    }
    if app.settings.market.rebuild_open {
        app.settings.market.rebuild_open = false;
        return true;
    }
    false
}

/// The small dialogs that still overlay the modal: the rate-rebuild and yes/no confirmations
/// and the discard prompt of a sub-page.
pub fn mount(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    if app.settings.market.rebuild_open {
        super::market::rebuild_dialog(app, tk, layer);
    }
    if app.settings.discard.is_some() {
        super::subpage::discard_dialog(app, tk, layer);
    }
    if let Some((msg, _)) = &app.settings.dialog.confirm {
        confirm_dialog(app, tk, layer, msg);
    }
}

fn footer(tk: &Tk, cancel: El, submit: El) -> El {
    row()
        .w_full()
        .gap(tk.r(0.5))
        .justify_end()
        .child(cancel)
        .child(submit)
}

/// Reset book data as a Settings sub-page (was a dialog over the modal).
pub fn reset_page(app: &App, tk: &Tk) -> Parts {
    let d = &app.settings.dialog;
    let enc = super::security::encrypted(app);
    let book = book_label(app).unwrap_or_else(|| t("settings:workspace.bookFallback"));
    let pending = app.settings.is_busy("settings.reset_book");
    let description = format!(
        "{}{}",
        tr("settings:resetBook.description", &[("book", &book)]),
        t(if enc {
            "settings:resetBook.passphraseHint"
        } else {
            "settings:resetBook.plaintextHint"
        })
    );
    let mut c = col().w_full().gap(tk.r(1.0));
    if enc {
        c = c.child(labelled(
            tk,
            &t("settings:resetBook.passphraseLabel"),
            input(
                tk,
                &d.reset_pass,
                "set-reset-pass",
                &t("settings:resetBook.passphraseLabel"),
                "",
                true,
                true,
                tk.r(2.25),
                false,
                |a: &mut App| &mut a.settings.dialog.reset_pass,
            ),
        ));
    } else {
        c = c.child(labelled(
            tk,
            &t("settings:resetBook.plaintextLabel"),
            input(
                tk,
                &d.reset_ack,
                "set-reset-ack",
                &t("settings:resetBook.plaintextLabel"),
                PLAINTEXT_DELETE_ACK,
                false,
                true,
                tk.r(2.25),
                false,
                |a: &mut App| &mut a.settings.dialog.reset_ack,
            ),
        ));
    }
    c = c.child(labelled(
        tk,
        &t("settings:resetBook.bookNameLabel"),
        input(
            tk,
            &d.reset_confirm,
            "set-reset-confirm",
            &t("settings:resetBook.bookNameLabel"),
            &book,
            false,
            true,
            tk.r(2.25),
            false,
            |a: &mut App| &mut a.settings.dialog.reset_confirm,
        ),
    ));
    let amber = rune::color::hex("#f59e0b").unwrap();
    c = c.child(
        row()
            .w_full()
            .gap(tk.r(0.75))
            .items_start()
            .padding(tk.r(0.75))
            .radius(tk.r(0.5))
            .border(1.0, amber.alpha(0.3))
            .fill(amber.alpha(0.05))
            .child(checkbox(
                tk,
                "set-reset-rates",
                d.reset_rates,
                &t("settings:resetBook.clearRatesLabel"),
                |a| a.settings.dialog.reset_rates = !a.settings.dialog.reset_rates,
            ))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(tk.r(0.25))
                    .child(
                        text(t("settings:resetBook.clearRatesLabel"))
                            .font(tk.tsm(500.0))
                            .color(tk.fg)
                            .wrap(),
                    )
                    .child(muted(tk, t("settings:resetBook.clearRatesHint"))),
            ),
    );
    let mut wipe = col()
        .w_full()
        .gap(tk.r(0.75))
        .padding(tk.r(0.75))
        .radius(tk.r(0.5))
        .border(1.0, tk.destructive.alpha(0.4))
        .fill(tk.destructive.alpha(0.05))
        .child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .items_start()
                .child(checkbox(
                    tk,
                    "set-reset-wipe",
                    d.reset_wipe,
                    &t("settings:resetBook.wipeLabel"),
                    |a| {
                        a.settings.dialog.reset_wipe = !a.settings.dialog.reset_wipe;
                        a.settings.dialog.reset_wipe_ack = TextEdit::new("");
                    },
                ))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(tk.r(0.25))
                        .child(
                            text(t("settings:resetBook.wipeLabel"))
                                .font(tk.tsm(500.0))
                                .color(tk.fg)
                                .wrap(),
                        )
                        .child(muted(tk, t("settings:resetBook.wipeHint"))),
                ),
        );
    if d.reset_wipe {
        let label = tr(
            "settings:resetBook.wipeConfirmLabel",
            &[("challenge", WIPE_EVERYTHING_ACK)],
        );
        wipe = wipe.child(labelled(
            tk,
            &label,
            input(
                tk,
                &d.reset_wipe_ack,
                "set-reset-wipe-ack",
                &label,
                WIPE_EVERYTHING_ACK,
                false,
                true,
                tk.r(2.25),
                false,
                |a: &mut App| &mut a.settings.dialog.reset_wipe_ack,
            ),
        ));
    }
    c = c.child(wipe);
    if let Some(e) = &d.reset_error {
        c = c.child(error_line(tk, e.clone()));
    }
    let submit_label = t(if pending {
        "settings:resetBook.submitPending"
    } else if d.reset_wipe {
        "settings:resetBook.submitWipe"
    } else {
        "settings:resetBook.submit"
    });
    Parts {
        title: t("settings:resetBook.title"),
        description: None,
        aside: Some(description),
        side: vec![],
        head_extra: None,
        body: c,
        footer_left: None,
        actions: vec![
            cancel_button(tk, "set-reset-cancel", true),
            btn_lg(
                tk,
                "set-reset-submit",
                &submit_label,
                Btn::Destructive,
                !pending,
            )
            .press_if(!pending, submit_reset),
        ],
        scroll_key: "set-reset-scroll".into(),
    }
}

fn submit_reset(app: &mut App) {
    let enc = super::security::encrypted(app);
    let book = book_label(app).unwrap_or_else(|| t("settings:workspace.bookFallback"));
    let d = &mut app.settings.dialog;
    d.reset_error = None;
    if enc && d.reset_pass.text.is_empty() {
        d.reset_error = Some(t("settings:resetBook.errorPassphrase"));
        return;
    }
    if !enc && d.reset_ack.text.trim() != PLAINTEXT_DELETE_ACK {
        d.reset_error = Some(tr(
            "settings:resetBook.errorPlaintext",
            &[("challenge", PLAINTEXT_DELETE_ACK)],
        ));
        return;
    }
    if d.reset_confirm.text.trim() != book {
        d.reset_error = Some(tr("settings:resetBook.errorConfirm", &[("book", &book)]));
        return;
    }
    if d.reset_wipe && d.reset_wipe_ack.text.trim() != WIPE_EVERYTHING_ACK {
        d.reset_error = Some(tr(
            "settings:resetBook.errorWipe",
            &[("challenge", WIPE_EVERYTHING_ACK)],
        ));
        return;
    }
    let auth = if enc {
        json!({"passphrase_secret": d.reset_pass.text})
    } else {
        json!({"plaintext_delete_ack": PLAINTEXT_DELETE_ACK})
    };
    let args = json!({"confirm": "RESET", "confirm_profile": book, "clear_shared_rates": d.reset_rates, "wipe_everything": d.reset_wipe, "auth_response": auth});
    send(app, "ui.profiles.reset_data", args, "settings.reset_book");
}

/// Delete books set as a Settings sub-page.
pub fn delete_page(app: &App, tk: &Tk) -> Parts {
    let d = &app.settings.dialog;
    let enc = super::security::encrypted(app);
    let ws = workspace_label(app);
    let pending = app.settings.is_busy("settings.delete_books");
    let description = format!(
        "{}{}",
        tr("settings:deleteBooks.description", &[("workspace", &ws)]),
        t(if enc {
            "settings:deleteBooks.passphraseHint"
        } else {
            "settings:deleteBooks.plaintextHint"
        })
    );
    let mut c = col().w_full().gap(tk.r(1.0));
    if enc {
        c = c.child(labelled(
            tk,
            &t("settings:deleteBooks.passphraseLabel"),
            input(
                tk,
                &d.delete_pass,
                "set-delete-pass",
                &t("settings:deleteBooks.passphraseLabel"),
                "",
                true,
                true,
                tk.r(2.25),
                false,
                |a: &mut App| &mut a.settings.dialog.delete_pass,
            ),
        ));
    } else {
        c = c.child(labelled(
            tk,
            &t("settings:deleteBooks.plaintextLabel"),
            input(
                tk,
                &d.delete_ack,
                "set-delete-ack",
                &t("settings:deleteBooks.plaintextLabel"),
                PLAINTEXT_DELETE_ACK,
                false,
                true,
                tk.r(2.25),
                false,
                |a: &mut App| &mut a.settings.dialog.delete_ack,
            ),
        ));
    }
    c = c.child(labelled(
        tk,
        &t("settings:deleteBooks.nameLabel"),
        input(
            tk,
            &d.delete_confirm,
            "set-delete-confirm",
            &t("settings:deleteBooks.nameLabel"),
            &ws,
            false,
            true,
            tk.r(2.25),
            false,
            |a: &mut App| &mut a.settings.dialog.delete_confirm,
        ),
    ));
    if let Some(e) = &d.delete_error {
        c = c.child(error_line(tk, e.clone()));
    }
    Parts {
        title: t("settings:deleteBooks.title"),
        description: None,
        aside: Some(description),
        side: vec![],
        head_extra: None,
        body: c,
        footer_left: None,
        actions: vec![
            cancel_button(tk, "set-delete-cancel", true),
            btn_lg(
                tk,
                "set-delete-submit",
                &t(if pending {
                    "settings:deleteBooks.submitPending"
                } else {
                    "settings:deleteBooks.submit"
                }),
                Btn::Destructive,
                !pending,
            )
            .press_if(!pending, submit_delete),
        ],
        scroll_key: "set-delete-scroll".into(),
    }
}

fn submit_delete(app: &mut App) {
    let enc = super::security::encrypted(app);
    let ws = workspace_label(app);
    let d = &mut app.settings.dialog;
    d.delete_error = None;
    if enc && d.delete_pass.text.is_empty() {
        d.delete_error = Some(t("settings:deleteBooks.errorPassphrase"));
        return;
    }
    if !enc && d.delete_ack.text.trim() != PLAINTEXT_DELETE_ACK {
        d.delete_error = Some(tr(
            "settings:deleteBooks.errorPlaintext",
            &[("challenge", PLAINTEXT_DELETE_ACK)],
        ));
        return;
    }
    if d.delete_confirm.text.trim() != ws {
        d.delete_error = Some(tr(
            "settings:deleteBooks.errorConfirm",
            &[("workspace", &ws)],
        ));
        return;
    }
    let auth = if enc {
        json!({"passphrase_secret": d.delete_pass.text})
    } else {
        json!({"plaintext_delete_ack": PLAINTEXT_DELETE_ACK})
    };
    send(
        app,
        "ui.workspace.delete",
        json!({"confirm": "DELETE", "confirm_workspace": ws, "auth_response": auth}),
        "settings.delete_books",
    );
}

/// Change database passphrase as a Settings sub-page.
pub fn pass_page(app: &App, tk: &Tk) -> Parts {
    let d = &app.settings.dialog;
    let pending = app.settings.is_busy("settings.change_pass");
    let mut c = col()
        .w_full()
        .gap(tk.r(1.0))
        .child(labelled(
            tk,
            &t("settings:changePassphrase.currentLabel"),
            input(
                tk,
                &d.pass_current,
                "set-pass-current",
                &t("settings:changePassphrase.currentLabel"),
                "",
                true,
                true,
                tk.r(2.25),
                false,
                |a: &mut App| &mut a.settings.dialog.pass_current,
            ),
        ))
        .child(labelled(
            tk,
            &t("settings:changePassphrase.newLabel"),
            input(
                tk,
                &d.pass_new,
                "set-pass-new",
                &t("settings:changePassphrase.newLabel"),
                "",
                true,
                true,
                tk.r(2.25),
                false,
                |a: &mut App| &mut a.settings.dialog.pass_new,
            ),
        ))
        .child(labelled(
            tk,
            &t("settings:changePassphrase.confirmLabel"),
            input(
                tk,
                &d.pass_confirm,
                "set-pass-confirm",
                &t("settings:changePassphrase.confirmLabel"),
                "",
                true,
                true,
                tk.r(2.25),
                false,
                |a: &mut App| &mut a.settings.dialog.pass_confirm,
            ),
        ));
    if let Some(e) = &d.pass_error {
        c = c.child(error_line(tk, e.clone()));
    }
    let submit = btn_lg(tk, "set-pass-submit", &t(if pending { "settings:changePassphrase.submitPending" } else { "settings:changePassphrase.submit" }), Btn::Primary, !pending).press_if(!pending, |a| {
            let d = &mut a.settings.dialog;
            d.pass_error = None;
            if d.pass_current.text.is_empty() {
                d.pass_error = Some(t("settings:changePassphrase.errorCurrent"));
                return;
            }
            if let Some(issue) = passphrase_issue(&d.pass_new.text, &d.pass_confirm.text) {
                d.pass_error = Some(tr(&format!("onboarding:security.hint.{issue}"), &[("min", "12")]));
                return;
            }
            let args = json!({"auth_response": {"passphrase_secret": d.pass_current.text}, "new_passphrase_secret": d.pass_new.text});
            let submitted = zeroize::Zeroizing::new(d.pass_new.text.clone());
            if send(a, "ui.secrets.change_passphrase", args, "settings.change_pass") {
                a.secrets.changing_to.push_back(submitted);
            }
        });
    Parts {
        title: t("settings:changePassphrase.title"),
        description: None,
        aside: Some(t("settings:changePassphrase.description")),
        side: vec![],
        head_extra: None,
        body: c,
        footer_left: None,
        actions: vec![cancel_button(tk, "set-pass-cancel", true), submit],
        scroll_key: "set-pass-scroll".into(),
    }
}

/// Touch ID enrollment as a Settings sub-page.
pub fn touch_page(app: &App, tk: &Tk) -> Parts {
    let d = &app.settings.dialog;
    let pending = app.settings.is_busy("settings.touch_unlock") || app.secrets.enroll_pending;
    let mut c = col().w_full().gap(tk.r(1.0)).child(labelled(
        tk,
        &t("settings:touchIdEnroll.passphraseLabel"),
        input(
            tk,
            &d.touch_pass,
            "set-touch-pass",
            &t("settings:touchIdEnroll.passphraseLabel"),
            "",
            true,
            !pending,
            tk.r(2.25),
            false,
            |a: &mut App| &mut a.settings.dialog.touch_pass,
        ),
    ));
    if let Some(e) = &d.touch_error {
        c = c.child(error_line(tk, e.clone()));
    }
    let submit = btn_lg(
        tk,
        "set-touch-submit",
        &t(if pending {
            "settings:touchIdEnroll.submitPending"
        } else {
            "settings:touchIdEnroll.submit"
        }),
        Btn::Primary,
        !pending,
    )
    .press_if(!pending, |a| {
        if a.settings.dialog.touch_pass.text.is_empty() {
            a.settings.dialog.touch_error = Some(t("settings:touchIdEnroll.errorPassphrase"));
            return;
        }
        // Verified by the daemon, then saved for Touch ID (`crate::secrets`).
        crate::secrets::enroll_submit(a);
    });
    Parts {
        title: t("settings:touchIdEnroll.title"),
        description: None,
        aside: Some(t("settings:touchIdEnroll.description")),
        side: vec![],
        head_extra: None,
        body: c,
        footer_left: None,
        actions: vec![cancel_button(tk, "set-touch-cancel", !pending), submit],
        scroll_key: "set-touch-scroll".into(),
    }
}

/// `window.confirm` as an in-app dialog: the message, Cancel and OK.
fn confirm_dialog(app: &App, tk: &Tk, layer: &mut Vec<El>, msg: &str) {
    let c = col()
        .w_full()
        .gap(tk.r(1.0))
        .role("alertdialog")
        .label(msg.lines().next().unwrap_or("").to_owned())
        .child(text(msg.to_owned()).font(tk.tsm(400.0)).color(tk.fg).wrap())
        .child(footer(
            tk,
            btn_lg(
                tk,
                "set-confirm-cancel",
                &t("common:actions.cancel"),
                Btn::Outline,
                true,
            )
            .on_press(cancel_confirm),
            btn_lg(tk, "set-confirm-ok", "OK", Btn::Primary, true).on_press(|a: &mut App| {
                if let Some((_, action)) = a.settings.dialog.confirm.take() {
                    run_confirmed(a, action);
                }
            }),
        ));
    dialog(
        app,
        tk,
        layer,
        "set-confirm-dialog",
        tk.r(26.0),
        c,
        cancel_confirm,
        true,
    );
}

/// A declined confirmation; declined provider checks say so on their cards.
fn cancel_confirm(app: &mut App) {
    if let Some((_, Confirmed::CheckProviders(names))) = app.settings.dialog.confirm.take() {
        app.settings.ai.pending_confirm.clear();
        for n in names {
            app.settings.ai.refusals.insert(n, "declined");
        }
    }
}

fn run_confirmed(app: &mut App, action: Confirmed) {
    match action {
        // Clears the local identity (and an opened folder) and returns to Welcome.
        Confirmed::ResetWelcome => crate::onboarding::reset_welcome(app),
        Confirmed::DeleteBackend(id) => {
            app.settings.backend.deleting = Some(id.clone());
            send(
                app,
                "ui.backends.delete",
                json!({"name": id}),
                "settings.backend_delete",
            );
        }
        // Removes the desktop Touch ID entry, then the CLI remembered unlock, from the OS
        // credential store: outside the book.
        Confirmed::ForgetAllUnlock => crate::secrets::forget_all(app),
        Confirmed::DeleteProvider(name) => {
            send(
                app,
                "ai.providers.delete",
                json!({"name": name}),
                "settings.ai_delete",
            );
        }
        Confirmed::CheckProviders(names) => super::ai::acknowledge_and_check(app, names),
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "settings.reset_book" => match &r.error {
            Some(e) => app.settings.dialog.reset_error = Some(e.clone()),
            None => {
                let removed = &r.data["removed"];
                let n = |k: &str| removed[k].as_f64().unwrap_or(0.0);
                let c = |v: f64| crate::format::count(v as usize);
                let body = if let Some(kept) = r.data["decisions"]["kept"].as_f64() {
                    tr(
                        "settings:resetBook.notifyKeptBody",
                        &[
                            ("transactions", &c(n("transactions"))),
                            ("journalRows", &c(n("journal_entries"))),
                            ("decisions", &c(kept)),
                        ],
                    )
                } else {
                    let mut parts = vec![
                        tr(
                            "settings:resetBook.summary.transactions",
                            &[("value", &c(n("transactions")))],
                        ),
                        tr(
                            "settings:resetBook.summary.journalRows",
                            &[("value", &c(n("journal_entries")))],
                        ),
                        tr(
                            "settings:resetBook.summary.swapPairs",
                            &[("value", &c(n("transaction_pairs")))],
                        ),
                        tr(
                            "settings:resetBook.summary.labels",
                            &[("value", &c(n("bip329_labels")))],
                        ),
                    ];
                    let att = n("attachments") + n("attachment_files");
                    if att > 0.0 {
                        parts.push(tr(
                            "settings:resetBook.summary.attachments",
                            &[("value", &c(att))],
                        ));
                    }
                    let sof = n("source_funds_sources")
                        + n("source_funds_links")
                        + n("source_funds_cases")
                        + n("source_funds_snapshots");
                    if sof > 0.0 {
                        parts.push(tr(
                            "settings:resetBook.summary.sourceFunds",
                            &[("value", &c(sof))],
                        ));
                    }
                    if r.data["shared_rates_cleared"].as_bool() == Some(true) {
                        parts.push(tr(
                            "settings:resetBook.summary.rates",
                            &[("value", &c(n("rates_cache")))],
                        ));
                    }
                    tr(
                        "settings:resetBook.notifyBody",
                        &[("summary", &parts.join(", "))],
                    )
                };
                app.toast(
                    ToastTone::Success,
                    &t("settings:resetBook.notifyTitle"),
                    Some(body),
                    None,
                );
                app.settings.dialog.reset_open = false;
                app.refresh_reads();
            }
        },
        "settings.delete_books" => match &r.error {
            Some(e) => app.settings.dialog.delete_error = Some(e.clone()),
            None => {
                app.settings.dialog.delete_open = false;
                crate::onboarding::reset_welcome(app);
            }
        },
        "settings.change_pass" => match &r.error {
            Some(e) => {
                app.secrets.changing_to.pop_front();
                app.settings.dialog.pass_error = Some(e.clone())
            }
            None => {
                // An enrolled Touch ID follows the new passphrase.
                crate::secrets::passphrase_changed(app, r);
                app.settings.dialog.pass_open = false;
            }
        },
        // The daemon verified (or refused) the passphrase; Touch ID saves it next.
        "settings.touch_unlock" => crate::secrets::enroll_unlocked(app, r),
        "settings.forget_cli" => crate::secrets::forget_all_done(app, r),
        _ => {}
    }
    let _ = Value::Null;
}
