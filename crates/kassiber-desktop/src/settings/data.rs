//! Data & storage (DataStorageSettingsPanel.tsx, BackupSettings.tsx): import tools, encrypted
//! backup export / restore, the local database paths and size, and the danger zone.
//!
//! Backups pick their file through the native dialogs, so a real path outside the disposable
//! book: wired, not exercised.

use rune::{El, FileDialogOptions, TextEdit, Tx, col, row, text};
use serde_json::{Value, json};

use super::controls::{
    Btn, PressIf, btn, copy_row, desc, error_line, group, input, para, srow, srow_a,
};
use super::model::format_bytes;
use super::send;
use crate::app::{App, Reply};
use crate::i18n::{t, tc, tr};
use crate::theme::Tk;

#[derive(Default)]
pub struct Backup {
    /// "export" | "restore" while the passphrase step shows.
    pub mode: Option<&'static str>,
    pub pass: TextEdit,
    pub second: TextEdit,
    pub confirm: TextEdit,
    pub preview: Option<Value>,
    pub error: Option<String>,
    pub message: Option<String>,
    /// The path a save or open dialog chose for the request in flight.
    pub path: Option<String>,
    /// `toastopen`: Show in Finder for the backup just exported (beside its message).
    pub saved: Vec<crate::toastopen::ToastAction>,
}

fn error_key(code: &str) -> Option<&'static str> {
    Some(match code {
        "invalid_backup" | "missing_backup" => "settings:backup.errors.invalid",
        "backup_passphrase_required" => "settings:backup.errors.passphrase",
        "invalid_backup_destination" | "backup_export_failed" | "unsafe_restore_target" => {
            "settings:backup.errors.destination"
        }
        "plaintext_database" => "settings:backup.errors.encryption",
        "stale_backup_preview" => "settings:backup.errors.stale",
        "backup_confirmation_required" => "settings:backup.errors.confirmation",
        "project_in_use" => "settings:backup.errors.busy",
        "backup_target_locked" => "settings:backup.errors.locked",
        "restore_install_failed" => "settings:backup.errors.rolledBack",
        "restore_rollback_failed" => "settings:backup.errors.recovery",
        "restore_failed" => "settings:backup.errors.failedLocked",
        _ => return None,
    })
}

/// `reset()`: cancels a previewed restore and clears the form.
fn reset(app: &mut App) {
    if let Some(tok) = app
        .settings
        .backup
        .preview
        .as_ref()
        .and_then(|p| p["token"].as_str())
        .map(str::to_owned)
    {
        app.send(
            "ui.backup.cancel",
            json!({"token": tok}),
            "settings.backup_cancel",
        );
    }
    let b = &mut app.settings.backup;
    b.preview = None;
    b.pass = TextEdit::new("");
    b.second = TextEdit::new("");
    b.confirm = TextEdit::new("");
    b.mode = None;
    b.error = None;
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    let code = r.error_detail["code"]
        .as_str()
        .unwrap_or("backup_failed")
        .to_owned();
    let failure = |app: &mut App| {
        let locked = code == "backup_target_locked"
            || r.error_detail["details"]["locked"].as_bool() == Some(true);
        let text_ = error_key(&code)
            .map(t)
            .unwrap_or_else(|| t("settings:backup.failed"));
        if locked {
            reset(app);
            app.toast(
                crate::app::ToastTone::Error,
                &text_,
                r.error_detail["details"]["recovery_path"]
                    .as_str()
                    .map(|p| tr("settings:backup.recoverySaved", &[("path", p)])),
                None,
            );
            super::close(app);
            crate::shell::overlays::lock(app);
        } else {
            app.settings.backup.error = Some(text_);
        }
    };
    match tag {
        "settings.backup_export" => {
            app.settings.backup.pass = TextEdit::new("");
            app.settings.backup.second = TextEdit::new("");
            if r.error.is_some() {
                failure(app);
            } else {
                let path = app.settings.backup.path.take().unwrap_or_default();
                reset(app);
                app.settings.backup.saved = crate::toastopen::for_path(&path, false);
                app.settings.backup.message =
                    Some(tr("settings:backup.exported", &[("path", &path)]));
            }
        }
        "settings.backup_preview" => {
            app.settings.backup.pass = TextEdit::new("");
            app.settings.backup.second = TextEdit::new("");
            if r.error.is_some() {
                failure(app);
            } else {
                app.settings.backup.preview = Some(r.data.clone());
            }
        }
        "settings.backup_apply" => {
            if r.error.is_some() {
                failure(app);
            } else {
                app.settings.backup.preview = None;
                reset(app);
                if let Some(p) = r.data["pre_restore_backup"].as_str() {
                    app.shell.note(
                        &t("settings:data.backupHeading"),
                        Some(tr("settings:backup.recoverySaved", &[("path", p)])),
                        crate::app::ToastTone::Info,
                        app.clock,
                    );
                }
                super::close(app);
                crate::shell::overlays::lock(app);
            }
        }
        _ => {}
    }
}

fn export(app: &mut App, tx: &mut Tx) {
    app.settings.backup.error = None;
    app.settings.backup.message = None;
    app.settings.backup.saved = vec![];
    let opts = FileDialogOptions {
        title: t("settings:backup.export"),
        file_name: Some("backup.kassiber".into()),
        ..Default::default()
    }
    .filter("Kassiber", &["kassiber"]);
    tx.save_file(
        opts,
        |a: &mut App, r: rune::FileDialogResult, _: &mut Tx| {
            if let Ok(Some(p)) = r {
                let path = p.to_string_lossy().into_owned();
                a.settings.backup.path = Some(path.clone());
                let args =
                    json!({"path": path, "backup_passphrase_secret": a.settings.backup.pass.text});
                send(a, "ui.backup.export", args, "settings.backup_export");
            }
        },
    );
}

fn preview(app: &mut App, tx: &mut Tx) {
    app.settings.backup.error = None;
    app.settings.backup.message = None;
    let opts = FileDialogOptions {
        title: t("settings:backup.choose"),
        ..Default::default()
    }
    .filter("Kassiber", &["kassiber"]);
    tx.open_file(opts, |a: &mut App, r: rune::FileDialogResult, _: &mut Tx| {
        if let Ok(Some(p)) = r {
            let args = json!({"path": p.to_string_lossy(), "backup_passphrase_secret": a.settings.backup.pass.text, "database_passphrase_secret": a.settings.backup.second.text});
            send(a, "ui.backup.preview", args, "settings.backup_preview");
        }
    });
}

fn backup(app: &App, tk: &Tk) -> El {
    let b = &app.settings.backup;
    let busy = [
        "settings.backup_export",
        "settings.backup_preview",
        "settings.backup_apply",
    ]
    .iter()
    .any(|t| app.settings.is_busy(t));
    let mut g = group()
        .anchor("backup")
        .title(t("settings:data.backupHeading"))
        .footer(t("settings:backup.scope"));
    match (b.mode, &b.preview) {
        (None, _) => {
            g = g
                .row(srow(
                    app,
                    tk,
                    t("settings:backup.exportLabel"),
                    desc(tk, t("settings:backup.exportDescription")),
                    vec![
                        btn(
                            tk,
                            "set-backup-export",
                            &t("settings:backup.export"),
                            Btn::Outline,
                            true,
                        )
                        .on_press(|a: &mut App| {
                            a.settings.backup.mode = Some("export");
                            a.settings.backup.message = None;
                        }),
                    ],
                ))
                .row(srow(
                    app,
                    tk,
                    t("settings:backup.restoreLabel"),
                    desc(tk, t("settings:backup.restoreDescription")),
                    vec![
                        btn(
                            tk,
                            "set-backup-restore",
                            &t("settings:backup.restore"),
                            Btn::Outline,
                            true,
                        )
                        .on_press(|a: &mut App| {
                            a.settings.backup.mode = Some("restore");
                            a.settings.backup.message = None;
                        }),
                    ],
                ));
        }
        (Some(mode), None) => {
            let second_label = t(if mode == "export" {
                "settings:backup.confirmPassphrase"
            } else {
                "settings:backup.databasePassphrase"
            });
            g = g
                .row(srow(
                    app,
                    tk,
                    t("settings:backup.passphrase"),
                    None,
                    vec![
                        input(
                            tk,
                            &b.pass,
                            "set-backup-pass",
                            &t("settings:backup.passphrase"),
                            "",
                            true,
                            !busy,
                            tk.r(2.0),
                            false,
                            |a: &mut App| &mut a.settings.backup.pass,
                        )
                        .w(tk.r(12.0)),
                    ],
                ))
                .row(srow(
                    app,
                    tk,
                    second_label.clone(),
                    None,
                    vec![
                        input(
                            tk,
                            &b.second,
                            "set-backup-second",
                            &second_label,
                            "",
                            true,
                            !busy,
                            tk.r(2.0),
                            false,
                            |a: &mut App| &mut a.settings.backup.second,
                        )
                        .w(tk.r(12.0)),
                    ],
                ));
            let ok = !busy
                && !b.pass.text.is_empty()
                && !b.second.text.is_empty()
                && (mode != "export" || b.pass.text == b.second.text);
            let go_label = t(if busy {
                "settings:backup.working"
            } else if mode == "export" {
                "settings:backup.export"
            } else {
                "settings:backup.preview"
            });
            let go = btn(tk, "set-backup-go", &go_label, Btn::Outline, ok);
            let go = if ok {
                go.on_press_tx(move |a: &mut App, tx: &mut Tx| {
                    if mode == "export" {
                        export(a, tx)
                    } else {
                        preview(a, tx)
                    }
                })
            } else {
                go
            };
            g = g.row(
                row()
                    .w_full()
                    .px(tk.r(1.0))
                    .py(tk.r(0.75))
                    .gap(tk.r(0.5))
                    .justify_end()
                    .child(
                        btn(
                            tk,
                            "set-backup-cancel",
                            &t("settings:backup.cancel"),
                            Btn::Outline,
                            !busy,
                        )
                        .press_if(!busy, reset),
                    )
                    .child(go),
            );
        }
        (Some(_), Some(p)) => {
            let list = |v: &Value| {
                v.as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let info = col()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .gap(tk.r(0.25))
                .child(
                    text(tc(
                        "settings:backup.replaceTarget",
                        p["target"]["book_count"].as_u64().unwrap_or(0) as usize,
                        &[],
                    ))
                    .font(tk.tsm(500.0))
                    .color(tk.fg),
                )
                .child(
                    text(p["target_data_root"].as_str().unwrap_or("").to_owned())
                        .font(tk.txs(400.0).mono())
                        .color(tk.muted_fg)
                        .wrap_anywhere(),
                )
                .child(
                    text(list(&p["target"]["books"]))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                )
                .child(
                    text(tc(
                        "settings:backup.incoming",
                        p["incoming"]["book_count"].as_u64().unwrap_or(0) as usize,
                        &[(
                            "attachments",
                            &p["attachments_files"].as_u64().unwrap_or(0).to_string(),
                        )],
                    ))
                    .font(tk.tsm(500.0))
                    .color(tk.fg)
                    .mt(tk.r(0.5)),
                )
                .child(
                    text(list(&p["incoming"]["books"]))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                )
                .child(
                    text(tr(
                        "settings:backup.replaces",
                        &[("paths", &list(&p["replaces"]))],
                    ))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .mt(tk.r(0.5)),
                )
                .child(
                    text(t("settings:backup.recovery"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                );
            g = g.row(info);
            if p["unavailable_secrets"].as_bool() == Some(true) {
                g = g.row(para(
                    tk,
                    t("settings:backup.secretWarning"),
                    super::controls::amber_soft(tk),
                ));
            }
            g = g.row(srow(
                app,
                tk,
                t("settings:backup.confirmRestore"),
                None,
                vec![
                    input(
                        tk,
                        &b.confirm,
                        "set-backup-confirm",
                        &t("settings:backup.confirmRestore"),
                        "",
                        false,
                        !busy,
                        tk.r(2.0),
                        false,
                        |a: &mut App| &mut a.settings.backup.confirm,
                    )
                    .w(tk.r(12.0)),
                ],
            ));
            let ok = !busy && b.confirm.text == "RESTORE";
            g = g.row(
                row()
                    .w_full()
                    .px(tk.r(1.0))
                    .py(tk.r(0.75))
                    .gap(tk.r(0.5))
                    .justify_end()
                    .child(
                        btn(
                            tk,
                            "set-backup-cancel2",
                            &t("settings:backup.cancel"),
                            Btn::Outline,
                            !busy,
                        )
                        .press_if(!busy, reset),
                    )
                    .child(
                        btn(
                            tk,
                            "set-backup-replace",
                            &t(if busy {
                                "settings:backup.working"
                            } else {
                                "settings:backup.replace"
                            }),
                            Btn::Destructive,
                            ok,
                        )
                        .press_if(ok, |a| {
                            let tok = a
                                .settings
                                .backup
                                .preview
                                .as_ref()
                                .and_then(|p| p["token"].as_str())
                                .unwrap_or("")
                                .to_owned();
                            let args =
                                json!({"token": tok, "confirm": a.settings.backup.confirm.text});
                            send(a, "ui.backup.apply", args, "settings.backup_apply");
                        }),
                    ),
            );
        }
    }
    // Time Machine and synced folders for the open book (crate::osleaks::backup).
    g = g.rows(crate::osleaks::ui::data_rows(app, tk));
    if let Some(e) = &b.error {
        let row_ = col()
            .w_full()
            .px(tk.r(1.0))
            .py(tk.r(0.75))
            .child(error_line(tk, e.clone()));
        g = g.row(super::controls::dismissable(
            tk,
            "set-backup-error-dismiss",
            row_,
            tk.destructive,
            |a| a.settings.backup.error = None,
        ));
    }
    if let Some(m) = &b.message {
        let mut content = para(tk, m.clone(), tk.fg).role("status");
        // `toastopen`: the backup's Show in Finder under its message (no Open: a backup is not
        // a document).
        if !b.saved.is_empty() {
            content = col().w_full().child(content).child(
                crate::toastopen::links(
                    tk,
                    &b.saved,
                    |a| format!("set-backup-{}", a.id),
                    tk.muted_fg,
                    tk.fg,
                )
                .px(tk.r(1.0))
                .pb(tk.r(0.75))
                .mt(tk.r(-0.375)),
            );
        }
        g = g.row(super::controls::dismissable(
            tk,
            "set-backup-message-dismiss",
            content,
            tk.muted_fg,
            |a| a.settings.backup.message = None,
        ));
    }
    g.build(app, tk)
}

pub fn view(app: &App, tk: &Tk, _layer: &mut Vec<El>) -> El {
    let status = app.read("status");
    let storage = app.settings.storage.as_ref().filter(|s| !s.is_null());
    let loading = t("common:state.loading");
    let import = group()
        .row(srow_a(
            app,
            tk,
            t("settings:data.importHeading"),
            desc(tk, t("settings:data.importDescription")),
            vec![
                btn(
                    tk,
                    "set-open-imports",
                    &t("settings:data.openImportTools"),
                    Btn::Outline,
                    true,
                )
                .on_press(|a: &mut App| {
                    super::close(a);
                    a.navigate("/imports");
                }),
            ],
            Some("importData"),
        ))
        .build(app, tk);
    let size = match storage {
        Some(s) => match s["database_bytes"].as_f64() {
            Some(b) => format_bytes(b),
            None => t("settings:data.databaseSizeUnknown"),
        },
        None => loading.clone(),
    };
    let wal = storage
        .and_then(|s| s["wal_bytes"].as_f64())
        .filter(|w| *w > 0.0)
        .map(|w| {
            tr(
                "settings:data.databaseSizeWal",
                &[("size", &format_bytes(w))],
            )
        });
    let copy = |label: &str| tr("settings:page.copy", &[("label", label)]);
    let local = group()
        .title(t("settings:data.localDatabaseHeading"))
        .row(copy_row(
            app,
            tk,
            "set-copy-root",
            &t("settings:data.dataRootLabel"),
            status
                .and_then(|s| s["data_root"].as_str())
                .map(str::to_owned),
            copy(&t("settings:data.dataRootLabel")),
            &loading,
            Some("dataRoot"),
        ))
        .row(copy_row(
            app,
            tk,
            "set-copy-db",
            &t("settings:data.databaseLabel"),
            status
                .and_then(|s| s["database"].as_str())
                .map(str::to_owned),
            copy(&t("settings:data.databaseLabel")),
            &loading,
            Some("databasePath"),
        ))
        .row(srow_a(
            app,
            tk,
            t("settings:data.databaseSizeLabel"),
            wal.and_then(|w| desc(tk, w)),
            vec![
                text(size)
                    .font(tk.tsm(400.0).features("'tnum' 1"))
                    .color(tk.muted_fg),
            ],
            Some("databaseSize"),
        ))
        .build(app, tk);
    let book = super::dialogs::book_label(app);
    let reset_enabled = !app.settings.is_busy("settings.reset_book") && book.is_some();
    let delete_enabled = !app.settings.is_busy("settings.delete_books");
    let danger = group()
        .title(t("settings:data.dangerHeading"))
        .row(srow_a(
            app,
            tk,
            t("settings:data.resetWelcomeTitle"),
            desc(tk, t("settings:data.resetWelcomeDescription")),
            vec![
                btn(
                    tk,
                    "set-reset-welcome",
                    &t("settings:data.resetWelcomeButton"),
                    Btn::Outline,
                    true,
                )
                .on_press(|a: &mut App| {
                    super::dialogs::confirm(
                        a,
                        t("settings:resetWelcome.confirm"),
                        super::dialogs::Confirmed::ResetWelcome,
                    )
                }),
            ],
            Some("resetWelcome"),
        ))
        .row(srow_a(
            app,
            tk,
            t("settings:data.resetBookTitle"),
            desc(tk, t("settings:data.resetBookDescription")),
            vec![
                btn(
                    tk,
                    "set-reset-book",
                    &t("settings:data.resetBookButton"),
                    Btn::Outline,
                    reset_enabled,
                )
                .press_if(reset_enabled, super::dialogs::open_reset_book),
            ],
            Some("resetBook"),
        ))
        .row(srow_a(
            app,
            tk,
            t("settings:data.deleteBooksTitle"),
            desc(tk, t("settings:data.deleteBooksDescription")),
            vec![
                btn(
                    tk,
                    "set-delete-books",
                    &t("settings:data.deleteBooksButton"),
                    Btn::Outline,
                    delete_enabled,
                )
                .press_if(delete_enabled, super::dialogs::open_delete_books),
            ],
            Some("deleteBooks"),
        ))
        .build(app, tk);
    // The database paths and size are reference detail: behind Advanced, after the danger zone.
    col()
        .w_full()
        .gap(tk.r(1.5))
        .child(
            col()
                .w_full()
                .gap(tk.r(1.5))
                .child(import)
                .child(backup(app, tk)),
        )
        .child(danger)
        .child(super::controls::advanced(app, tk, ADVANCED, vec![local]))
}

/// The Advanced fold's open-state key (`search::reveal_key`).
pub const ADVANCED: &str = "set-adv-data";
