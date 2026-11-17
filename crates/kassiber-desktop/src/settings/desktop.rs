//! Terminal integration (TerminalCommandSettingsPanel.tsx) and Developer tools
//! (DeveloperToolsSettingsPanel.tsx).
//!
//! The terminal command status is the desktop's own file inspection (`terminal_command_status`
//! in src-tauri), ported in `native::terminal`: a user-local launcher in `~/.local/bin` or
//! `~/bin` that runs this app as the CLI, plus the shell profile's PATH block. Reading it touches
//! only local files. Install and Remove write outside the disposable book, so they are wired
//! and exercised only against a temporary home.

use rune::{El, col, text};

use super::controls::{
    Btn, PressIf, amber, btn, copy_row, desc, emerald, group, para, srow_a, switch,
};
use super::model::format_bytes;
use crate::app::{App, ToastTone};
use crate::i18n::{t, tr};
use crate::theme::Tk;

const VERIFY_COMMAND: &str = "kassiber status";

#[derive(Clone, Debug, Default)]
pub struct TerminalStatus {
    pub available: bool,
    pub installed: bool,
    pub managed: bool,
    pub needs_repair: bool,
    pub conflict: bool,
    pub path_on_path: bool,
    pub bin_dir: String,
    pub command_path: String,
    pub target_path: String,
    pub path_hint: String,
    pub message: String,
}

#[derive(Default)]
pub struct Terminal {
    pub status: Option<TerminalStatus>,
    pub error: Option<String>,
    pub pending: bool,
}

/// `terminal_command_status`, from `native::terminal` (local file reads only).
pub fn inspect(_app: &App) -> Result<TerminalStatus, String> {
    let s = crate::native::terminal::status(&crate::native::terminal::Env::current())?;
    Ok(TerminalStatus {
        available: s.available,
        installed: s.installed,
        managed: s.managed,
        needs_repair: s.needs_repair,
        conflict: s.conflict,
        path_on_path: s.path_on_path,
        bin_dir: s.bin_dir,
        command_path: s.command_path,
        target_path: s.target_path,
        path_hint: s.path_hint,
        message: s.message,
    })
}

pub fn load_terminal(app: &mut App) {
    match inspect(app) {
        Ok(s) => {
            app.settings.terminal.status = Some(s);
            app.settings.terminal.error = None;
        }
        Err(e) => app.settings.terminal.error = Some(e),
    }
}

/// Writes the launcher and the shell profile's PATH block (`native::terminal`; outside the
/// disposable book, never run by tests against the real home).
fn install(app: &mut App) {
    let was_repair = app
        .settings
        .terminal
        .status
        .as_ref()
        .is_some_and(|s| s.needs_repair);
    let result = crate::native::terminal::install(&crate::native::terminal::Env::current());
    match result.and_then(|_| inspect(app)) {
        Ok(s) => {
            let (tone, body) = if s.path_on_path {
                (
                    ToastTone::Success,
                    t("settings:terminal.installedBodyOnPath"),
                )
            } else {
                (
                    ToastTone::Warning,
                    tr(
                        "settings:terminal.installedBodyOffPath",
                        &[("binDir", &s.bin_dir)],
                    ),
                )
            };
            app.toast(
                tone,
                &t(if was_repair {
                    "settings:terminal.repairedTitle"
                } else {
                    "settings:terminal.installedTitle"
                }),
                Some(body),
                None,
            );
            app.settings.terminal.status = Some(s);
            app.settings.terminal.error = None;
        }
        // Tauri's command rejects with a string, which the web replaces with its own sentence.
        Err(_) => app.settings.terminal.error = Some(t("settings:terminal.installError")),
    }
}

/// Removes a launcher this app manages and its PATH blocks (`native::terminal`; outside the
/// disposable book, never run by tests against the real home).
fn remove(app: &mut App) {
    let result = crate::native::terminal::remove(&crate::native::terminal::Env::current())
        .and_then(|_| inspect(app));
    match result {
        Ok(s) => {
            let cmd = if s.command_path.is_empty() {
                t("settings:terminal.removedBodyFallback")
            } else {
                s.command_path.clone()
            };
            app.settings.terminal.status = Some(s);
            app.settings.terminal.error = None;
            app.toast(
                ToastTone::Success,
                &t("settings:terminal.removedTitle"),
                Some(tr("settings:terminal.removedBody", &[("command", &cmd)])),
                None,
            );
        }
        Err(_) => app.settings.terminal.error = Some(t("settings:terminal.removeError")),
    }
}

pub fn terminal_view(app: &App, tk: &Tk) -> El {
    let term = &app.settings.terminal;
    let s = term.status.as_ref();
    let action = t(if s.is_some_and(|s| s.needs_repair) {
        "settings:terminal.repairAction"
    } else if s.is_some_and(|s| s.installed) {
        "settings:terminal.reinstallAction"
    } else {
        "settings:terminal.installAction"
    });
    // The intro's `<0>kassiber</0>` is set in mono in the web; the tags are dropped here.
    let intro = t("settings:terminal.intro")
        .replace("<0>", "")
        .replace("</0>", "");
    let mut g = group().footer(intro);
    if let Some(e) = &term.error {
        g = g.row(para(tk, e.clone(), tk.destructive).role("alert"));
    }
    let can_install = !term.pending && !s.is_some_and(|s| s.conflict || !s.available);
    let mut controls = vec![
        btn(tk, "set-term-install", &action, Btn::Outline, can_install)
            .press_if(can_install, install),
        btn(
            tk,
            "set-term-refresh",
            &t("common:actions.refresh"),
            Btn::Outline,
            !term.pending,
        )
        .press_if(!term.pending, load_terminal),
    ];
    if s.is_some_and(|s| s.managed) {
        controls.push(
            btn(
                tk,
                "set-term-remove",
                &t("common:actions.remove"),
                Btn::Outline,
                !term.pending,
            )
            .press_if(!term.pending, remove),
        );
    }
    let d = match s {
        Some(s) => Some(
            text(s.message.clone())
                .font(tk.tsm(400.0))
                .color(if s.installed && s.path_on_path {
                    emerald(tk)
                } else {
                    amber(tk)
                })
                .wrap(),
        ),
        None => desc(tk, t("settings:terminal.inspecting")),
    };
    g = g.row(srow_a(
        app,
        tk,
        t("settings:terminal.statusLabel"),
        d,
        controls,
        Some("terminalCommand"),
    ));
    if let Some(s) = s {
        let copy = |l: &str| tr("settings:page.copy", &[("label", l)]);
        let opt = |v: &str| (!v.is_empty()).then(|| v.to_owned());
        g = g
            .row(copy_row(
                app,
                tk,
                "set-term-cmd",
                &t("settings:terminal.commandLabel"),
                opt(&s.command_path),
                copy(&t("settings:terminal.commandLabel")),
                "—",
                None,
            ))
            .row(copy_row(
                app,
                tk,
                "set-term-target",
                &t("settings:terminal.targetLabel"),
                opt(&s.target_path),
                copy(&t("settings:terminal.targetLabel")),
                "—",
                None,
            ))
            .row(copy_row(
                app,
                tk,
                "set-term-verify",
                &t("settings:terminal.verifyLabel"),
                Some(VERIFY_COMMAND.into()),
                copy(VERIFY_COMMAND),
                "—",
                None,
            ));
        if !s.path_on_path {
            g = g.row(copy_row(
                app,
                tk,
                "set-term-path",
                &t("settings:terminal.pathLabel"),
                opt(&s.path_hint),
                copy(&t("settings:terminal.pathLabel")),
                "—",
                None,
            ));
        }
    }
    g.build(app, tk)
}

/// Bytes the in-app log buffer holds.
/// `useAppLogBufferSize`: the in-memory log ring's approximate size (2 bytes when empty, `[]`).
fn log_bytes(app: &App) -> f64 {
    app.reports.diag.logs.store.bytes as f64
}

pub fn developer_view(app: &App, tk: &Tk) -> El {
    let on = app.prefs.developer_tools;
    let mut logs_controls = vec![];
    if on {
        logs_controls.push(
            btn(
                tk,
                "set-open-logs",
                &t("settings:developer.openLogs"),
                Btn::Outline,
                true,
            )
            .on_press(|a: &mut App| {
                super::close(a);
                crate::shell::go(a, "/logs");
            }),
        );
    }
    let records = crate::format::count(10_000);
    group()
        .footer(t("settings:developer.intro"))
        .row(srow_a(
            app,
            tk,
            t("settings:developer.enableLabel"),
            desc(
                tk,
                t(if on {
                    "settings:developer.enableOn"
                } else {
                    "settings:developer.enableOff"
                }),
            ),
            vec![switch(
                tk,
                "set-dev-tools",
                on,
                &t("settings:developer.enableLabel"),
                true,
                |a, v| a.prefs.developer_tools = v,
            )],
            Some("earlyFeatures"),
        ))
        .row(srow_a(
            app,
            tk,
            t("settings:developer.alphaBannerLabel"),
            desc(tk, t("settings:developer.alphaBannerDescription")),
            vec![switch(
                tk,
                "set-alpha",
                app.prefs.alpha_visible,
                &t("settings:developer.alphaBannerLabel"),
                true,
                |a, v| a.prefs.alpha_visible = v,
            )],
            Some("alphaWarning"),
        ))
        .row(srow_a(
            app,
            tk,
            t("settings:developer.bufferHeading"),
            desc(
                tk,
                tr(
                    "settings:developer.bufferDescription",
                    &[
                        ("retained", &format_bytes(log_bytes(app))),
                        ("records", &records),
                        ("bytes", &format_bytes(4.0 * 1024.0 * 1024.0)),
                    ],
                ),
            ),
            logs_controls,
            Some("logBuffer"),
        ))
        .build(app, tk)
        .pipe(|e| col().w_full().child(e))
}

trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}
impl Pipe for El {}
