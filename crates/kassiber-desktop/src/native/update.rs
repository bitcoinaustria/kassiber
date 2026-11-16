//! Update checks (src-tauri app_update.rs `check_app_update`, ui-tauri lib/appUpdate.ts): the
//! bundled CLI's `kassiber --format json update`, run only when the user turned automatic checks
//! on (Settings › Privacy, the owner-only consent file the CLI reads too) or pressed "Check for
//! Updates…" / the version line. Consent is re-read before every check and again before a
//! result is shown; nothing is downloaded, a newer release only opens its GitHub page on an
//! explicit press. Headless runs and tests never start a check.
//!
//! - Automatic (`useAppUpdateScheduler`): with consent and an open book, 10 s after it arms,
//!   then once a day. Quiet: failures are dropped.
//! - Manual (`runManualAppUpdateCheck`): always answers in a dialog (disabled, failed with the
//!   CLI's reason, a newer version with Open GitHub / Not now, or up to date).
//! - The sidebar pill (`SidebarUpdatePill`) offers a newer release until dismissed this launch.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

use rune::{Color, El, Tx, col, row, text};
use serde::Deserialize;

use crate::app::App;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::{ic, outline_button, primary_button};

const TRUSTED_RELEASE_URL_PREFIX: &str = "https://github.com/bitcoinaustria/kassiber/releases/";
const SIDECAR_UPDATE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_SIDECAR_OUTPUT_BYTES: usize = 256 * 1024;
pub const UPDATE_CHECKS_DISABLED_MESSAGE: &str =
    "GitHub update checks are disabled. Enable them in Settings > Privacy.";
/// `APP_UPDATE_START_DELAY_MS`, `APP_UPDATE_PERIOD_MS`.
pub const START_DELAY: f64 = 10.0;
pub const PERIOD: f64 = 24.0 * 60.0 * 60.0;

/// `AppUpdateCheck`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppUpdate {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub release_url: Option<String>,
    pub update_available: bool,
    pub prerelease: bool,
    pub checked_at: u64,
}

#[derive(Deserialize)]
struct CliEnvelope {
    kind: String,
    data: Option<CliData>,
    error: Option<CliError>,
}

#[derive(Deserialize)]
struct CliData {
    current_version: String,
    latest_version: String,
    update_available: bool,
    prerelease: bool,
    release_url: String,
}

#[derive(Deserialize)]
struct CliError {
    code: String,
    message: String,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn cli_error_message(e: &CliError) -> String {
    if e.code == "update_checks_disabled" {
        return UPDATE_CHECKS_DISABLED_MESSAGE.into();
    }
    if e.message.is_empty() {
        "Could not reach GitHub to check for updates.".into()
    } else {
        format!("Could not check for updates: {}.", e.message)
    }
}

/// `update_check_from_envelope`: success and failure envelopes alike.
pub fn from_envelope(body: &[u8]) -> Result<AppUpdate, String> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Err("Could not reach GitHub to check for updates.".into());
    }
    let env: CliEnvelope = serde_json::from_slice(body)
        .map_err(|_| "The Kassiber CLI returned an invalid update response.".to_owned())?;
    if let Some(e) = env.error {
        return Err(cli_error_message(&e));
    }
    if env.kind != "update" {
        return Err("The Kassiber CLI returned an unexpected response kind.".into());
    }
    let d = env
        .data
        .ok_or_else(|| "The Kassiber CLI returned an empty update response.".to_owned())?;
    if d.update_available && !d.release_url.starts_with(TRUSTED_RELEASE_URL_PREFIX) {
        return Err("The update check returned an untrusted release URL.".into());
    }
    Ok(AppUpdate {
        current_version: d.current_version,
        latest_version: Some(d.latest_version),
        release_url: d.update_available.then_some(d.release_url),
        update_available: d.update_available,
        prerelease: d.prerelease,
        checked_at: now_secs(),
    })
}

/// `run_sidecar_update_check`: `<cli> --format json update` under a 30 s deadline, stdout read
/// through a hard cap, stderr discarded. `preference` is the consent file the app read: the CLI
/// is told to read the same one (`KASSIBER_UPDATE_PREFERENCE_FILE`).
pub fn run_cli_check(
    program: &std::path::Path,
    cwd: &std::path::Path,
    preference: Option<&std::path::Path>,
) -> Result<AppUpdate, String> {
    let mut cmd = Command::new(program);
    if let Some(p) = preference {
        cmd.env(crate::settings::prefpath::UPDATE_CHECKS.env, p);
    }
    let mut child = cmd
        .args(["--format", "json", "update"])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Could not start the Kassiber CLI to check for updates.".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not read the Kassiber CLI update output.".to_owned())?;
    let (send, recv) = channel();
    std::thread::spawn(move || {
        let mut body = Vec::new();
        let r = stdout
            .take(MAX_SIDECAR_OUTPUT_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .map(|_| body);
        let _ = send.send(r);
    });
    let body = match recv.recv_timeout(SIDECAR_UPDATE_TIMEOUT) {
        Ok(Ok(body)) => body,
        Ok(Err(_)) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Could not read the Kassiber CLI update output.".into());
        }
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("The update check timed out.".into());
        }
    };
    if body.len() > MAX_SIDECAR_OUTPUT_BYTES {
        let _ = child.kill();
        let _ = child.wait();
        return Err("The Kassiber CLI returned an unexpectedly large update response.".into());
    }
    child
        .wait()
        .map_err(|_| "Could not run the Kassiber CLI update check.".to_owned())?;
    from_envelope(&body)
}

/// `check_app_update`: the fail-closed consent gate before anything is spawned (the CLI reads
/// the same file again before it connects).
fn check_blocking() -> Result<AppUpdate, String> {
    if !crate::settings::effects::read_update_checks() {
        return Err(UPDATE_CHECKS_DISABLED_MESSAGE.into());
    }
    let preference = crate::settings::effects::update_preference_path();
    let cmd = super::cli::kassiber_command()
        .map_err(|(m, _)| format!("Could not check for updates: {m}."))?;
    run_cli_check(&cmd.program, &cmd.cwd, preference.as_deref())
}

pub struct Job {
    manual: bool,
    generation: u64,
    rx: Receiver<Result<AppUpdate, String>>,
}

/// A message box the web shows through the native dialog plugin (`showDialog`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dialog {
    pub body: String,
    pub error: bool,
    /// The newer release's page: the dialog asks Open GitHub / Not now.
    pub open_url: Option<String>,
}

#[derive(Default)]
pub struct State {
    /// The latest result (`appUpdate`), kept only while automatic checks are on.
    pub update: Option<AppUpdate>,
    /// The pill's ✕ (until the next launch).
    pub dismissed: bool,
    /// The automatic schedule: when it runs next (app clock), and its arming.
    pub next_due: Option<f64>,
    pub generation: u64,
    pub job: Option<Job>,
    pub dialog: Option<Dialog>,
}

fn consent(app: &App) -> bool {
    app.settings.prefs.update_checks
}

/// `syncAppUpdateChecksEnabled` + `setAutomaticUpdateChecks`: re-read the canonical file and
/// mirror it; turning off forgets the last result.
fn sync_consent(app: &mut App) -> bool {
    let enabled = crate::settings::effects::read_update_checks();
    app.settings.prefs.update_checks = enabled;
    if !enabled {
        app.native.update.update = None;
    }
    enabled
}

/// `setAppUpdate`: only kept while automatic checks are on.
fn set_update(app: &mut App, u: AppUpdate) {
    if consent(app) {
        app.native.update.update = Some(u);
    }
}

fn start(app: &mut App, manual: bool) {
    let (send, rx) = channel();
    let generation = app.native.update.generation;
    if rune::is_headless() || cfg!(test) {
        // Never from agent tooling or tests: no process, no network.
        let _ = send.send(Err("Update checks don't run in headless mode.".into()));
    } else {
        std::thread::Builder::new()
            .name("kassiber-update-check".into())
            .spawn(move || {
                let _ = send.send(check_blocking());
            })
            .ok();
    }
    app.native.update.job = Some(Job {
        manual,
        generation,
        rx,
    });
}

/// `runManualAppUpdateCheck`: the menu's "Check for Updates…" and the version line.
pub fn run_manual(app: &mut App) {
    if app.native.update.job.as_ref().is_some_and(|j| j.manual) {
        return;
    }
    if !sync_consent(app) {
        app.native.update.dialog = Some(Dialog {
            body: t("chrome:shell.version.disabled"),
            error: false,
            open_url: None,
        });
        return;
    }
    start(app, true);
}

/// Whether the version line checks (consent given; the desktop app can check).
pub fn can_check(app: &App) -> bool {
    consent(app)
}

/// Every frame: arm or disarm the schedule, run what's due, take finished checks.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    if !consent(app) && app.native.update.update.is_some() {
        app.native.update.update = None;
        changed = true;
    }
    // `consentLoaded && enabled && hasIdentity && canCheckAppUpdates()`.
    let armed = app.settings.fx.started
        && consent(app)
        && super::intent::has_workspace(app)
        && !rune::is_headless();
    let st = &mut app.native.update;
    match (armed, st.next_due) {
        (true, None) => {
            st.generation += 1;
            st.next_due = Some(app.clock + START_DELAY);
        }
        (false, Some(_)) => {
            // Disposed: a check in flight is dropped when it lands.
            st.generation += 1;
            st.next_due = None;
        }
        _ => {}
    }
    if let Some(due) = app.native.update.next_due {
        if app.clock >= due {
            app.native.update.next_due = Some(due + PERIOD);
            // Mirror a CLI-side revocation before invoking the checker.
            if app.native.update.job.is_none() && sync_consent(app) {
                start(app, false);
            }
        } else {
            tx.wake_after(Duration::from_secs_f64((due - app.clock).max(0.05)));
        }
    }
    let result = app.native.update.job.as_ref().map(|j| j.rx.try_recv());
    match result {
        Some(Ok(r)) => {
            let job = app.native.update.job.take().expect("job");
            changed = true;
            finish(app, &job, r);
        }
        Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => app.native.update.job = None,
        Some(Err(std::sync::mpsc::TryRecvError::Empty)) => {
            tx.wake_after(Duration::from_millis(200))
        }
        None => {}
    }
    changed
}

fn finish(app: &mut App, job: &Job, r: Result<AppUpdate, String>) {
    if !job.manual {
        // Quiet: errors drop; a result counts only while the schedule that asked still runs
        // and consent still holds.
        if let Ok(u) = r
            && job.generation == app.native.update.generation
            && sync_consent(app)
        {
            set_update(app, u);
        }
        return;
    }
    let u = match r {
        Ok(u) => u,
        Err(reason) => {
            let body = if reason.trim().is_empty() {
                t("chrome:shell.version.checkFailed")
            } else {
                reason.trim().to_owned()
            };
            app.native.update.dialog = Some(Dialog {
                body,
                error: true,
                open_url: None,
            });
            return;
        }
    };
    // Consent can be revoked from another CLI process while the check runs.
    if !sync_consent(app) {
        return;
    }
    set_update(app, u.clone());
    app.native.update.dialog = Some(match (&u.latest_version, &u.release_url) {
        (Some(latest), Some(url)) if u.update_available => Dialog {
            body: tr(
                "chrome:shell.version.availablePrompt",
                &[("version", latest)],
            ),
            error: false,
            open_url: Some(url.clone()),
        },
        _ => Dialog {
            body: tr(
                "chrome:shell.version.current",
                &[("version", &u.current_version)],
            ),
            error: false,
            open_url: None,
        },
    });
}

fn close_dialog(app: &mut App) {
    app.native.update.dialog = None;
}

/// The message box, as an in-app alert dialog (Rune has no native message box).
pub fn dialog(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let Some(d) = &app.native.update.dialog else {
        return;
    };
    let mut buttons = row().w_full().justify_end().gap(tk.r(0.5));
    if let Some(url) = d.open_url.clone() {
        buttons = buttons
            .child(
                outline_button(
                    tk,
                    "native-update-not-now",
                    None,
                    &t("chrome:shell.version.notNow"),
                )
                .on_press(close_dialog),
            )
            .child(
                primary_button(
                    tk,
                    "native-update-open",
                    None,
                    &t("chrome:shell.version.openGitHub"),
                )
                .autofocus()
                .on_press_tx(move |a: &mut App, tx| {
                    a.native.update.dialog = None;
                    // The user chose to open the trusted release page.
                    let _ = tx.open_url(&url);
                }),
            );
    } else {
        buttons = buttons.child(
            primary_button(tk, "native-update-ok", None, &t("chrome:shell.version.ok"))
                .autofocus()
                .on_press(close_dialog),
        );
    }
    let card = col()
        .key("native-update-dialog")
        .w(420.0)
        .padding(tk.r(1.25))
        .gap(tk.r(1.0))
        .radius(tk.r(0.75))
        .fill(tk.popover)
        .border(1.0, tk.border)
        .role("alertdialog")
        .label("Kassiber")
        .modal(true, close_dialog)
        .on_press(|_: &mut App| {})
        .child(text("Kassiber").font(tk.tbase(600.0)).color(tk.fg))
        .child(
            text(d.body.clone())
                .font(tk.tsm(400.0))
                .color(if d.error { tk.destructive } else { tk.muted_fg })
                .wrap()
                .w_full(),
        )
        .child(buttons);
    layer.push(
        row()
            .key("native-update-scrim")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .center()
            .fill(Color::BLACK.alpha(if tk.dark { 0.5 } else { 0.25 }))
            .a11y_hidden()
            .child(card),
    );
}

/// `SidebarUpdatePill`: a newer release, until dismissed. None when there is nothing to offer.
pub fn pill(app: &App, tk: &Tk) -> Option<El> {
    let st = &app.native.update;
    let u = st.update.as_ref()?;
    let url = if consent(app) && u.update_available {
        u.release_url.clone()?
    } else {
        return None;
    };
    let latest = u.latest_version.clone()?;
    if st.dismissed {
        return None;
    }
    let tooltip = tr(
        "chrome:shell.version.updateTitle",
        &[("current", &u.current_version), ("latest", &latest)],
    );
    let label = tr("chrome:shell.version.updateLabel", &[("version", &latest)]);
    let small = |c: Color| ic(&I::DOWNLOAD, tk.r(0.875), c);
    let main = row()
        .key("sh-update-main")
        .h_full()
        .grow(1.0)
        .min_w(0.0)
        .px(tk.r(0.5))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.5))
        .hover(tk.primary.alpha(0.07))
        .role("button")
        .label(tooltip.clone())
        .help(tooltip)
        .focusable(true)
        .on_press_tx(move |_: &mut App, tx| {
            let _ = tx.open_url(&url);
        })
        .child(small(tk.primary).a11y_hidden())
        .child(text(label).font(tk.txs(500.0)).color(tk.primary).truncate());
    let dismiss_label = t("chrome:shell.version.updateDismiss");
    let dismiss = row()
        .key("sh-update-dismiss")
        .w(tk.r(1.25))
        .h(tk.r(1.25))
        .mr(tk.r(0.25))
        .center()
        .radius(tk.r(0.375))
        .role("button")
        .label(dismiss_label.clone())
        .help(dismiss_label)
        .focusable(true)
        .on_press(|a: &mut App| a.native.update.dismissed = true)
        .child(ic(&I::X, tk.r(0.875), tk.primary.alpha(0.6)).a11y_hidden());
    Some(
        row()
            .key("sh-update")
            .w_full()
            .h(tk.r(1.75))
            .center_y()
            .radius(tk.r(0.5))
            .fill(tk.primary.alpha(0.15))
            .child(main)
            .child(dismiss),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_an_available_update_with_its_trusted_release_url() {
        let r = from_envelope(br#"{"kind":"update","schema_version":1,"data":{"current_version":"0.22.55","latest_version":"0.22.56","update_available":true,"prerelease":false,"release_url":"https://github.com/bitcoinaustria/kassiber/releases/tag/v0.22.56","checked_at":"2026-07-23T08:30:00Z","install_method":"manual","update_command":null}}"#).unwrap();
        assert!(r.update_available && !r.prerelease);
        assert_eq!(r.latest_version.as_deref(), Some("0.22.56"));
        assert_eq!(
            r.release_url.as_deref(),
            Some("https://github.com/bitcoinaustria/kassiber/releases/tag/v0.22.56")
        );
    }

    #[test]
    fn omits_the_release_url_when_current_and_rejects_untrusted_urls() {
        let r = from_envelope(br#"{"kind":"update","schema_version":1,"data":{"current_version":"0.22.56","latest_version":"0.22.56","update_available":false,"prerelease":true,"release_url":"https://github.com/bitcoinaustria/kassiber/releases/tag/v0.22.56"}}"#).unwrap();
        assert!(!r.update_available && r.prerelease && r.release_url.is_none());
        assert!(from_envelope(br#"{"kind":"update","schema_version":1,"data":{"current_version":"1","latest_version":"2","update_available":true,"prerelease":false,"release_url":"https://example.com/kassiber"}}"#).is_err());
    }

    #[test]
    fn rejects_unexpected_kinds_and_maps_cli_errors() {
        assert!(from_envelope(b"not json").is_err());
        assert!(from_envelope(br#"{"kind":"status","data":{"current_version":"1","latest_version":"1","update_available":false,"prerelease":false,"release_url":""}}"#).is_err());
        assert!(from_envelope(br#"{"kind":"update","schema_version":1}"#).is_err());
        assert_eq!(
            from_envelope(
                br#"{"kind":"error","error":{"code":"update_checks_disabled","message":"x"}}"#
            )
            .unwrap_err(),
            UPDATE_CHECKS_DISABLED_MESSAGE
        );
        assert_eq!(
            from_envelope(br#"{"kind":"error","schema_version":1,"error":{"code":"network","message":"GitHub is unreachable","hint":null,"details":null,"retryable":true,"debug":null}}"#).unwrap_err(),
            "Could not check for updates: GitHub is unreachable."
        );
        assert_eq!(
            from_envelope(b"   \n").unwrap_err(),
            "Could not reach GitHub to check for updates."
        );
    }

    /// The real spawn path against a stand-in CLI that prints a canned envelope: no network.
    #[cfg(unix)]
    #[test]
    fn runs_the_cli_and_caps_its_output() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("kassiber-rune-update-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("kassiber");
        std::fs::write(&fake, "#!/bin/sh\n[ \"$1 $2 $3\" = \"--format json update\" ] || exit 9\nprintf '%s' '{\"kind\":\"update\",\"data\":{\"current_version\":\"0.22.55\",\"latest_version\":\"0.22.56\",\"update_available\":true,\"prerelease\":false,\"release_url\":\"https://github.com/bitcoinaustria/kassiber/releases/tag/v0.22.56\"}}'\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let r = run_cli_check(&fake, &dir, None).unwrap();
        assert_eq!(r.latest_version.as_deref(), Some("0.22.56"));
        let big = dir.join("big");
        std::fs::write(&big, "#!/bin/sh\nhead -c 300000 /dev/zero\n").unwrap();
        std::fs::set_permissions(&big, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            run_cli_check(&big, &dir, None).unwrap_err(),
            "The Kassiber CLI returned an unexpectedly large update response."
        );
        assert_eq!(
            run_cli_check(&dir.join("missing"), &dir, None).unwrap_err(),
            "Could not start the Kassiber CLI to check for updates."
        );
        // The CLI is told which consent file the app read.
        let named = dir.join("named");
        std::fs::write(&named, "#!/bin/sh\n[ \"$KASSIBER_UPDATE_PREFERENCE_FILE\" = /x/config/update-checks.json ] || exit 9\nprintf '%s' '{\"kind\":\"update\",\"data\":{\"current_version\":\"1\",\"latest_version\":\"1\",\"update_available\":false,\"prerelease\":false,\"release_url\":\"\"}}'\n").unwrap();
        std::fs::set_permissions(&named, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            run_cli_check(
                &named,
                &dir,
                Some(std::path::Path::new("/x/config/update-checks.json"))
            )
            .is_ok()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
