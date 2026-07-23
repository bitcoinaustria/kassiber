use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const TRUSTED_RELEASE_URL_PREFIX: &str = "https://github.com/bitcoinaustria/kassiber/releases/";
const SIDECAR_UPDATE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_SIDECAR_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_PREFERENCE_BYTES: u64 = 1024;
const PREFERENCE_SCHEMA_VERSION: u8 = 1;
const DISABLE_UPDATE_CHECK_ENV: &str = "KASSIBER_DISABLE_UPDATE_CHECK";
const PREFERENCE_FILENAME: &str = "update-checks.json";
const PREFERENCE_LOCK_FILENAME: &str = "update-checks.lock";
const UPDATE_CHECKS_DISABLED_MESSAGE: &str =
    "GitHub update checks are disabled. Enable them in Settings > Privacy.";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct UpdateCheckPreference {
    schema_version: u8,
    enabled: bool,
}

struct UpdateCheckPreferenceLock {
    file: File,
}

impl Drop for UpdateCheckPreferenceLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateCheck {
    current_version: String,
    latest_version: Option<String>,
    release_url: Option<String>,
    update_available: bool,
    prerelease: bool,
    checked_at: u64,
}

/// `kassiber update --format json` success envelope. Extra fields the CLI
/// adds over time (install method, update command) are ignored on purpose.
#[derive(Debug, Deserialize)]
struct CliUpdateEnvelope {
    kind: String,
    data: Option<CliUpdateData>,
    error: Option<CliErrorBody>,
}

#[derive(Debug, Deserialize)]
struct CliUpdateData {
    current_version: String,
    latest_version: String,
    update_available: bool,
    prerelease: bool,
    release_url: String,
}

#[derive(Debug, Deserialize)]
struct CliErrorBody {
    code: String,
    message: String,
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn debug_update_check(current_version: String, prerelease: bool) -> AppUpdateCheck {
    AppUpdateCheck {
        latest_version: Some(current_version.clone()),
        current_version,
        release_url: None,
        update_available: false,
        prerelease,
        checked_at: now_unix_seconds(),
    }
}

fn update_check_from_envelope(body: &[u8]) -> Result<AppUpdateCheck, String> {
    let envelope: CliUpdateEnvelope = serde_json::from_slice(body)
        .map_err(|_| "The Kassiber CLI returned an invalid update response.".to_string())?;
    if let Some(error) = envelope.error {
        return Err(cli_error_message(&error));
    }
    if envelope.kind != "update" {
        return Err("The Kassiber CLI returned an unexpected response kind.".to_string());
    }
    let data = envelope
        .data
        .ok_or_else(|| "The Kassiber CLI returned an empty update response.".to_string())?;
    if data.update_available && !data.release_url.starts_with(TRUSTED_RELEASE_URL_PREFIX) {
        return Err("The update check returned an untrusted release URL.".to_string());
    }
    Ok(AppUpdateCheck {
        current_version: data.current_version,
        latest_version: Some(data.latest_version),
        release_url: data.update_available.then_some(data.release_url),
        update_available: data.update_available,
        prerelease: data.prerelease,
        checked_at: now_unix_seconds(),
    })
}

fn cli_error_message(error: &CliErrorBody) -> String {
    if error.code == "update_checks_disabled" {
        return UPDATE_CHECKS_DISABLED_MESSAGE.to_string();
    }
    if error.message.is_empty() {
        "Could not reach GitHub to check for updates.".to_string()
    } else {
        format!("Could not check for updates: {}.", error.message)
    }
}

fn sidecar_failure_message(stdout: &[u8]) -> String {
    match serde_json::from_slice::<CliUpdateEnvelope>(stdout) {
        Ok(envelope) => match envelope.error {
            Some(error) => cli_error_message(&error),
            None => "Could not reach GitHub to check for updates.".to_string(),
        },
        Err(_) => "Could not reach GitHub to check for updates.".to_string(),
    }
}

fn run_sidecar_update_check(resource_dir: Option<&Path>) -> Result<AppUpdateCheck, String> {
    // `--format` is a global option and must precede the subcommand.
    let (program, args, cwd) = crate::supervisor::cli_invocation(
        resource_dir,
        vec!["--format".into(), "json".into(), "update".into()],
    );
    let mut child = Command::new(&program)
        .args(&args)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Could not start the Kassiber CLI to check for updates.".to_string())?;

    // Drain stdout on a separate thread so a chatty child can never fill the
    // pipe buffer and deadlock against the bounded wait below.
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not read the Kassiber CLI update output.".to_string())?;
    let reader = std::thread::spawn(move || {
        let mut body = Vec::new();
        stdout
            .by_ref()
            .take(MAX_SIDECAR_OUTPUT_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .map(|_| body)
    });

    let deadline = Instant::now() + SIDECAR_UPDATE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return Err("The update check timed out.".to_string());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err("Could not run the Kassiber CLI update check.".to_string());
            }
        }
    };
    let body = reader
        .join()
        .map_err(|_| "Could not read the Kassiber CLI update output.".to_string())?
        .map_err(|_| "Could not read the Kassiber CLI update output.".to_string())?;
    if body.len() > MAX_SIDECAR_OUTPUT_BYTES {
        return Err("The Kassiber CLI returned an unexpectedly large update response.".to_string());
    }
    if !status.success() {
        return Err(sidecar_failure_message(&body));
    }
    update_check_from_envelope(&body)
}

fn environment_disables_update_checks() -> bool {
    env::var(DISABLE_UPDATE_CHECK_ENV)
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
}

fn preference_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    let home = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME"));
    #[cfg(not(target_os = "windows"))]
    let home = env::var_os("HOME");

    home.filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|home| {
            home.join(".kassiber")
                .join("config")
                .join(PREFERENCE_FILENAME)
        })
        .ok_or_else(|| "Could not locate the user update-check preference.".to_string())
}

fn preference_lock_path(path: &Path) -> Result<PathBuf, String> {
    path.parent()
        .map(|parent| parent.join(PREFERENCE_LOCK_FILENAME))
        .ok_or_else(|| "Update-check preference has no parent directory.".to_string())
}

fn acquire_update_check_preference_lock(
    preference: &Path,
) -> Result<UpdateCheckPreferenceLock, String> {
    let lock_path = preference_lock_path(preference)?;
    match fs::symlink_metadata(&lock_path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err("Update-check lock must be a regular file.".to_string());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("Could not inspect the update-check lock.".to_string()),
    }

    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        options
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(&lock_path)
        .map_err(|_| "Could not open the update-check lock.".to_string())?;
    let metadata = file
        .metadata()
        .map_err(|_| "Could not inspect the update-check lock.".to_string())?;
    if !metadata.file_type().is_file() {
        return Err("Update-check lock must be a regular file.".to_string());
    }
    #[cfg(unix)]
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| "Could not protect the update-check lock.".to_string())?;

    // Python's Windows implementation locks byte zero with `msvcrt.locking`.
    // Keep that byte present so fs2's whole-file Windows lock range overlaps
    // it; the same file is harmless for Unix `flock` interoperability.
    if metadata.len() == 0 {
        file.write_all(b"\0")
            .and_then(|_| file.sync_all())
            .map_err(|_| "Could not initialize the update-check lock.".to_string())?;
    }
    file.lock_exclusive()
        .map_err(|_| "Could not acquire the update-check lock.".to_string())?;
    Ok(UpdateCheckPreferenceLock { file })
}

fn update_checks_enabled_at(path: &Path) -> bool {
    if environment_disables_update_checks() {
        return false;
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };
    if !metadata.file_type().is_file() || metadata.len() > MAX_PREFERENCE_BYTES {
        return false;
    }
    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut raw = Vec::new();
    if file
        .take(MAX_PREFERENCE_BYTES + 1)
        .read_to_end(&mut raw)
        .is_err()
        || raw.len() as u64 > MAX_PREFERENCE_BYTES
    {
        return false;
    }
    serde_json::from_slice::<UpdateCheckPreference>(&raw)
        .ok()
        .is_some_and(|preference| {
            preference.schema_version == PREFERENCE_SCHEMA_VERSION && preference.enabled
        })
}

fn update_checks_enabled() -> bool {
    preference_path()
        .ok()
        .is_some_and(|path| update_checks_enabled_at(&path))
}

fn write_update_checks_enabled_at(path: &Path, enabled: bool) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Update-check preference has no parent directory.".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create the update-check settings folder: {error}"))?;
    #[cfg(unix)]
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("Could not protect the update-check settings folder: {error}"))?;

    let _lock = acquire_update_check_preference_lock(path)?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(
        ".{PREFERENCE_FILENAME}.{}.{nonce}.tmp",
        std::process::id()
    ));
    let encoded = serde_json::to_vec(&UpdateCheckPreference {
        schema_version: PREFERENCE_SCHEMA_VERSION,
        enabled,
    })
    .map_err(|error| format!("Could not encode the update-check preference: {error}"))?;
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("Could not create the update-check preference: {error}"))?;
    let write_result = file
        .write_all(&encoded)
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all());
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Could not write the update-check preference: {error}"
        ));
    }
    #[cfg(target_os = "windows")]
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("Could not replace the update-check preference: {error}"))?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Could not replace the update-check preference: {error}"
        ));
    }
    Ok(())
}

#[tauri::command]
pub fn set_app_update_checks_enabled(enabled: bool) -> Result<bool, String> {
    write_update_checks_enabled_at(&preference_path()?, enabled)?;
    Ok(enabled)
}

#[tauri::command]
pub fn get_app_update_checks_enabled() -> bool {
    update_checks_enabled()
}

#[tauri::command]
pub async fn check_app_update(app: AppHandle) -> Result<AppUpdateCheck, String> {
    // Fail-closed native gate before anything is spawned. The bundled CLI
    // re-checks the same consent file under the shared cross-process lock, so
    // revocation racing an in-flight check is closed there, not here.
    let preference = preference_path()?;
    if !update_checks_enabled_at(&preference) {
        return Err(UPDATE_CHECKS_DISABLED_MESSAGE.to_string());
    }
    let current = app.package_info().version.clone();
    if cfg!(debug_assertions) {
        return Ok(debug_update_check(
            current.to_string(),
            !current.pre.is_empty(),
        ));
    }
    let resource_dir = app.path().resource_dir().ok();
    tauri::async_runtime::spawn_blocking(move || run_sidecar_update_check(resource_dir.as_deref()))
        .await
        .map_err(|_| "Could not run the Kassiber CLI update check.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::{
        cli_error_message, preference_lock_path, sidecar_failure_message,
        update_check_from_envelope, update_checks_enabled_at, write_update_checks_enabled_at,
        CliErrorBody, UPDATE_CHECKS_DISABLED_MESSAGE,
    };
    use std::fs::{self, OpenOptions};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn update_check_consent_is_explicit_and_fail_closed() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "kassiber-update-preference-{}-{nonce}",
            std::process::id()
        ));
        let path = root.join("update-checks.json");
        assert!(!update_checks_enabled_at(&path));
        write_update_checks_enabled_at(&path, true).unwrap();
        assert!(update_checks_enabled_at(&path));
        let lock_path = preference_lock_path(&path).unwrap();
        let lock_metadata = fs::metadata(lock_path).unwrap();
        assert!(lock_metadata.is_file());
        assert!(lock_metadata.len() >= 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(lock_metadata.permissions().mode() & 0o077, 0);
        }
        write_update_checks_enabled_at(&path, false).unwrap();
        assert!(!update_checks_enabled_at(&path));
        fs::write(&path, b"not-json\n").unwrap();
        assert!(!update_checks_enabled_at(&path));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preference_writes_wait_for_an_inflight_check_lock() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "kassiber-update-lock-ordering-{}-{nonce}",
            std::process::id()
        ));
        let path = root.join("update-checks.json");
        write_update_checks_enabled_at(&path, true).unwrap();
        let inflight = super::acquire_update_check_preference_lock(&path).unwrap();
        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(preference_lock_path(&path).unwrap())
            .unwrap();
        assert!(fs2::FileExt::try_lock_exclusive(&contender).is_err());
        drop(contender);
        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let writer_path = path.clone();
        let writer = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = write_update_checks_enabled_at(&writer_path, false);
            finished_tx.send(result).unwrap();
        });

        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(finished_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        drop(inflight);
        finished_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        writer.join().unwrap();
        assert!(!update_checks_enabled_at(&path));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn preference_lock_rejects_symlinks() {
        use std::os::unix::fs::symlink;

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "kassiber-update-lock-symlink-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("update-checks.json");
        let lock_path = preference_lock_path(&path).unwrap();
        let target = root.join("lock-target");
        fs::write(&target, b"\0").unwrap();
        symlink(&target, &lock_path).unwrap();

        assert!(write_update_checks_enabled_at(&path, true).is_err());
        assert!(!update_checks_enabled_at(&path));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn forwards_an_available_update_with_its_trusted_release_url() {
        let result = update_check_from_envelope(
            br#"{
                "kind": "update",
                "schema_version": 1,
                "data": {
                    "current_version": "0.22.55",
                    "latest_version": "0.22.56",
                    "update_available": true,
                    "prerelease": false,
                    "release_url": "https://github.com/bitcoinaustria/kassiber/releases/tag/v0.22.56",
                    "checked_at": "2026-07-23T08:30:00Z",
                    "install_method": "manual",
                    "update_command": null
                }
            }"#,
        )
        .expect("valid update envelope");

        assert!(result.update_available);
        assert!(!result.prerelease);
        assert_eq!(result.current_version, "0.22.55");
        assert_eq!(result.latest_version.as_deref(), Some("0.22.56"));
        assert_eq!(
            result.release_url.as_deref(),
            Some("https://github.com/bitcoinaustria/kassiber/releases/tag/v0.22.56")
        );
    }

    #[test]
    fn omits_the_release_url_when_no_update_is_available() {
        let result = update_check_from_envelope(
            br#"{
                "kind": "update",
                "schema_version": 1,
                "data": {
                    "current_version": "0.22.56",
                    "latest_version": "0.22.56",
                    "update_available": false,
                    "prerelease": true,
                    "release_url": "https://github.com/bitcoinaustria/kassiber/releases/tag/v0.22.56"
                }
            }"#,
        )
        .expect("valid update envelope");

        assert!(!result.update_available);
        assert!(result.prerelease);
        assert_eq!(result.release_url, None);
    }

    #[test]
    fn rejects_an_untrusted_release_url() {
        let result = update_check_from_envelope(
            br#"{
                "kind": "update",
                "schema_version": 1,
                "data": {
                    "current_version": "0.22.55",
                    "latest_version": "0.22.56",
                    "update_available": true,
                    "prerelease": false,
                    "release_url": "https://example.com/kassiber"
                }
            }"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn rejects_unexpected_kinds_and_invalid_payloads() {
        assert!(update_check_from_envelope(b"not json").is_err());
        assert!(update_check_from_envelope(
            br#"{"kind": "status", "schema_version": 1, "data": {
                "current_version": "1", "latest_version": "1",
                "update_available": false, "prerelease": false, "release_url": ""
            }}"#
        )
        .is_err());
        assert!(update_check_from_envelope(br#"{"kind": "update", "schema_version": 1}"#).is_err());
    }

    #[test]
    fn maps_cli_error_envelopes_to_user_facing_messages() {
        assert_eq!(
            cli_error_message(&CliErrorBody {
                code: "update_checks_disabled".to_string(),
                message: "GitHub update checks are disabled".to_string(),
            }),
            UPDATE_CHECKS_DISABLED_MESSAGE
        );
        assert_eq!(
            sidecar_failure_message(
                br#"{"kind": "error", "schema_version": 1, "error": {
                    "code": "network", "message": "GitHub is unreachable",
                    "hint": null, "details": null, "retryable": true, "debug": null
                }}"#
            ),
            "Could not check for updates: GitHub is unreachable."
        );
        assert_eq!(
            sidecar_failure_message(b"garbage"),
            "Could not reach GitHub to check for updates."
        );
    }
}
