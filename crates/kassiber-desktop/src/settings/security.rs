//! Lock & encryption (SecuritySettingsPanel.tsx): database encryption status and Lock now, the
//! passphrase change, app-lock policy, Touch ID.
//!
//! Touch ID (`crate::secrets`): its status is read when Settings opens on encrypted books on a
//! Mac (a local Keychain lookup, no prompt); Refresh reads it again, Set up opens the
//! enrollment page, Forget removes the saved passphrase. Off a Mac the row says where Touch ID
//! is available and its buttons stay disabled, as the web's browser build.

use rune::{El, col, text};

use super::controls::{
    Btn, PressIf, amber, btn, desc, emerald, group, opt, select, srow, srow_a, switch,
};
use crate::app::App;
use crate::i18n::{t, tc};
use crate::theme::Tk;

const IDLE: [u32; 5] = [1, 5, 15, 30, 60];

/// `status.database_encrypted` (the identity's encryption flag in the web).
pub fn encrypted(app: &App) -> bool {
    app.read("status")
        .is_some_and(|s| s["database_encrypted"].as_bool() == Some(true))
}

/// Touch ID needs the desktop's Keychain commands (`canUseTouchIdPassphraseUnlock`): a Mac.
pub fn touch_id_supported() -> bool {
    crate::secrets::touch_id_supported()
}

pub fn view(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let p = &app.settings.prefs;
    let enc = encrypted(app);
    let lock_now = btn(
        tk,
        "set-lock-now",
        &t("settings:security.lockNow"),
        Btn::Outline,
        true,
    )
    .on_press(|a: &mut App| {
        super::close(a);
        crate::shell::overlays::lock(a);
    });
    let encryption = group()
        .anchor("databaseEncryption")
        .title(t("settings:security.encryptionHeading"))
        .action(lock_now)
        .row(srow(
            app,
            tk,
            t("settings:security.encryptionStatus"),
            desc(
                tk,
                t(if enc {
                    "settings:security.encryptedDetail"
                } else {
                    "settings:security.notEncryptedDetail"
                }),
            ),
            vec![
                text(t(if enc {
                    "settings:security.encrypted"
                } else {
                    "settings:security.notEncrypted"
                }))
                .font(tk.tsm(400.0))
                .color(if enc { emerald(tk) } else { amber(tk) }),
            ],
        ))
        .row(srow_a(
            app,
            tk,
            t("settings:security.passphraseLabel"),
            desc(
                tk,
                t(if enc {
                    "settings:security.passphraseDescription"
                } else {
                    "settings:security.passphraseNeedsEncryption"
                }),
            ),
            vec![
                btn(
                    tk,
                    "set-change-pass",
                    &t("settings:security.changePassphrase"),
                    Btn::Outline,
                    enc,
                )
                .press_if(enc, super::dialogs::open_change_passphrase),
            ],
            Some("passphrase"),
        ));
    let mut lock = group()
        .title(t("settings:security.appLockHeading"))
        .row(srow_a(
            app,
            tk,
            t("settings:security.autoLockLabel"),
            desc(tk, t("settings:security.autoLockDescription")),
            vec![switch(
                tk,
                "set-auto-lock",
                p.auto_lock,
                &t("settings:security.autoLockLabel"),
                true,
                |a, v| a.settings.prefs.auto_lock = v,
            )],
            Some("autoLock"),
        ));
    if p.auto_lock {
        let mut mins: Vec<u32> = IDLE.to_vec();
        if !mins.contains(&p.idle_minutes) {
            mins.push(p.idle_minutes);
            mins.sort();
        }
        let sel = select(
            app,
            tk,
            layer,
            "set-lock-after",
            &t("settings:security.lockAfter"),
            &p.idle_minutes.to_string(),
            mins.iter()
                .map(|m| {
                    opt(
                        m.to_string(),
                        tc("settings:security.minutes", *m as usize, &[]),
                    )
                })
                .collect(),
            true,
            None,
            |a, v| {
                if let Ok(m) = v.parse() {
                    a.settings.prefs.idle_minutes = m;
                }
            },
        );
        lock = lock.row(srow(
            app,
            tk,
            t("settings:security.lockAfter"),
            None,
            vec![sel],
        ));
    }
    lock = lock
        .row(srow_a(
            app,
            tk,
            t("settings:security.requireOnLaunchLabel"),
            desc(
                tk,
                t(if enc {
                    "settings:security.requireOnLaunchEncrypted"
                } else {
                    "settings:security.requireOnLaunchPlaintext"
                }),
            ),
            vec![switch(
                tk,
                "set-require-launch",
                enc || p.require_on_launch,
                &t("settings:security.requireOnLaunchLabel"),
                !enc,
                |a, v| a.settings.prefs.require_on_launch = v,
            )],
            Some("lockOnLaunch"),
        ))
        .row(srow_a(
            app,
            tk,
            t("settings:security.lockOnCloseLabel"),
            desc(tk, t("settings:security.lockOnCloseDescription")),
            vec![switch(
                tk,
                "set-lock-close",
                p.lock_on_close,
                &t("settings:security.lockOnCloseLabel"),
                true,
                |a, v| a.settings.prefs.lock_on_close = v,
            )],
            Some("lockOnClose"),
        ));
    let supported = touch_id_supported();
    let actionable = enc && supported;
    let st = app.secrets.status.as_ref();
    let configured = st.is_some_and(|s| s.configured);
    let pending = app.secrets.status_pending;
    use crate::secrets::touch::TouchIdProtection as P;
    let detail = if !enc {
        t("settings:security.biometricNeedsEncryption")
    } else if !supported {
        t("settings:security.biometricAvailableMacos")
    } else if configured {
        t(match st.and_then(|s| s.protection) {
            Some(P::BiometryCurrentSet) => "settings:security.biometricSavedProtected",
            Some(P::LegacyShared) => "settings:security.biometricSavedLegacy",
            _ => "settings:security.biometricSavedAppGated",
        })
    } else if st.is_some_and(|s| s.stale) {
        t("settings:security.biometricStale")
    } else if let Some(reason) = st.and_then(|s| s.reason.as_deref()) {
        crate::i18n::tr(
            "settings:security.biometricNotSetUpReason",
            &[("reason", reason)],
        )
    } else {
        t("settings:security.biometricVerifyHint")
    };
    let status_text = t(if pending {
        "settings:security.biometricChecking"
    } else if configured {
        "settings:security.biometricEnrolled"
    } else {
        "settings:security.biometricNotEnrolled"
    });
    let status_color = if configured && !pending {
        emerald(tk)
    } else {
        tk.muted_fg
    };
    let action = if configured {
        btn(
            tk,
            "set-touchid-forget",
            &t("settings:security.forget"),
            Btn::Outline,
            actionable,
        )
        .press_if(actionable, crate::secrets::settings_forget)
    } else {
        btn(
            tk,
            "set-touchid-setup",
            &t("settings:security.setUp"),
            Btn::Outline,
            actionable,
        )
        .press_if(actionable, super::dialogs::open_touch_id)
    };
    let forget_busy = app.settings.is_busy("settings.forget_cli");
    // One action per row: the status shows only while it says something ("On", "Checking…");
    // re-reading the Keychain, the lock-screen offer and the cleanup of every saved unlock
    // method sit behind Advanced.
    let mut controls = vec![];
    if pending || configured {
        controls.push(text(status_text).font(tk.tsm(400.0)).color(status_color));
    }
    controls.push(action);
    let bio = group()
        .anchor("biometricUnlock")
        .title(t("settings:security.biometricHeading"))
        .row(srow(
            app,
            tk,
            t("settings:security.biometricLabel"),
            desc(tk, detail),
            controls,
        ));
    let more = group()
        .row(srow(
            app,
            tk,
            t("settings:security.biometricRecheckLabel"),
            desc(tk, t("settings:security.biometricRecheckDescription")),
            vec![
                btn(
                    tk,
                    "set-touchid-refresh",
                    &t("common:actions.refresh"),
                    Btn::Outline,
                    actionable,
                )
                .press_if(actionable, crate::secrets::settings_refresh),
            ],
        ))
        .row(srow(
            app,
            tk,
            t("settings:security.offerBiometricLabel"),
            desc(tk, t("settings:security.offerBiometricDescription")),
            vec![switch(
                tk,
                "set-offer-touchid",
                p.touch_id_unlock && configured,
                &t("settings:security.offerBiometricLabel"),
                enc && configured,
                |a, v| a.settings.prefs.touch_id_unlock = v,
            )],
        ))
        .row(srow(
            app,
            tk,
            t("settings:security.savedUnlockLabel"),
            desc(tk, t("settings:security.forgetAllDescription")),
            vec![
                btn(
                    tk,
                    "set-forget-all",
                    &t("settings:security.forgetAll"),
                    Btn::Outline,
                    !forget_busy,
                )
                .press_if(!forget_busy, |a| {
                    super::dialogs::confirm(
                        a,
                        t("settings:touchId.forgetAllConfirm"),
                        super::dialogs::Confirmed::ForgetAllUnlock,
                    )
                }),
            ],
        ));
    col()
        .w_full()
        .gap(tk.r(1.5))
        .child(encryption.build(app, tk))
        .child(lock.build(app, tk))
        .child(bio.build(app, tk))
        .child(super::controls::advanced(
            app,
            tk,
            "set-adv-security",
            vec![more.build(app, tk)],
        ))
}
