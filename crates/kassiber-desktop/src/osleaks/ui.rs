//! The Settings rows for these protections: Privacy → "Screen & clipboard" (the screen-share
//! switch and the clipboard row's note) and Data → "Local database" (Time Machine and synced
//! folders). Rows only appear where they apply; the synced-folder warning and the failure line
//! only while there is something to act on.

use rune::El;

use super::{backup, screen};
use crate::app::App;
use crate::i18n::{t, tr};
use crate::settings::controls::{amber, desc, para, srow_a, switch};
use crate::theme::Tk;

/// The clipboard row's description: whether copies clear, and that they stay on this device
/// and out of clipboard histories.
pub fn clipboard_note(clears: bool) -> String {
    format!(
        "{} {}",
        t(if clears {
            "settings:privacy.clipboardOn"
        } else {
            "settings:privacy.clipboardOff"
        }),
        t("settings:privacy.clipboardContained")
    )
}

/// Privacy: "Hide amounts while the screen is shared", and a status line while it holds them.
pub fn privacy_rows(app: &App, tk: &Tk) -> Vec<El> {
    let available = screen::available();
    let on = app.settings.prefs.hide_on_screen_share;
    let label = t("settings:privacy.screenShareLabel");
    let note = if !available {
        "settings:privacy.screenShareUnavailable"
    } else if on {
        "settings:privacy.screenShareOn"
    } else {
        "settings:privacy.screenShareOff"
    };
    let toggle = switch(
        tk,
        "set-hide-on-share",
        on && available,
        &label,
        available,
        |a, v| a.settings.prefs.hide_on_screen_share = v,
    );
    let mut rows = vec![srow_a(
        app,
        tk,
        label,
        desc(tk, t(note)),
        vec![toggle],
        Some("hideOnScreenShare"),
    )];
    if screen::forced() {
        rows.push(para(tk, t("settings:privacy.screenShareActive"), amber(tk)).role("status"));
    }
    rows
}

/// Data: Time Machine for the open book, and the synced-folder warning.
pub fn data_rows(app: &App, tk: &Tk) -> Vec<El> {
    let v = backup::view();
    let Some(book) = &v.book else { return vec![] };
    let mut rows = vec![];
    if cfg!(target_os = "macos") {
        let on = app.settings.prefs.exclude_unencrypted_backups;
        let label = t("settings:data.osBackupLabel");
        let note = if !on {
            "settings:data.osBackupOff"
        } else if book.encrypted {
            "settings:data.osBackupEncrypted"
        } else {
            "settings:data.osBackupExcluded"
        };
        let toggle = switch(tk, "set-os-backup", on, &label, true, |a, v| {
            a.settings.prefs.exclude_unencrypted_backups = v
        });
        rows.push(srow_a(
            app,
            tk,
            label,
            desc(tk, t(note)),
            vec![toggle],
            Some("osBackup"),
        ));
        if on && !book.encrypted && v.outcome.is_some_and(|o| o.failed > 0) {
            rows.push(para(tk, t("settings:data.osBackupFailed"), tk.destructive).role("alert"));
        }
    } else if !book.encrypted {
        rows.push(para(tk, t("settings:data.osBackupOther"), tk.muted_fg));
    }
    if !book.encrypted
        && let Some(service) = v.synced
    {
        let name = if service.is_empty() {
            t("settings:data.syncedFolderService")
        } else {
            service.to_owned()
        };
        rows.push(
            para(
                tk,
                tr("settings:data.syncedFolderWarning", &[("service", &name)]),
                amber(tk),
            )
            .role("alert"),
        );
    }
    rows
}
