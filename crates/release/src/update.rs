//! `kassiber update`: the persisted consent and the one explicit GitHub check.
//! Nothing here runs in the background or downloads a release.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_net::{
    Consent, Endpoint, Gate, HttpOptions, HttpRequest, Ledger, Offline, Request, Subsystem,
    TlsTrust,
};
use kassiber_paths::Context;
use kassiber_pyjson::Value;

use crate::version::compare;
pub use crate::version::{is_newer, parse_version, Version};
use crate::{strip, Result};

const RELEASES_URL: &str =
    "https://api.github.com/repos/bitcoinaustria/kassiber/releases?per_page=20";
const LATEST_URL: &str = "https://api.github.com/repos/bitcoinaustria/kassiber/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/bitcoinaustria/kassiber/releases";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_PREFERENCE_BYTES: usize = 1024;
const CASK_COMMAND: &str = "brew upgrade --cask bitcoinaustria/kassiber/kassiber";
const FORMULA_COMMAND: &str = "brew upgrade bitcoinaustria/kassiber/kassiber-cli";

/// `urllib.parse.quote(tag, safe="")`.
fn quote(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
/// `release_url_for_tag(tag)`.
pub fn release_url(tag: &str) -> String {
    format!("{RELEASES_PAGE}/tag/{}", quote(tag))
}

/// The running build, as `packaged_build_info()` reports it.
pub struct Build<'a> {
    pub version: &'a str,
    pub channel: Option<&'a str>,
}
impl Build<'_> {
    /// `current_version()`.
    fn current(&self) -> String {
        let value = strip(self.version);
        value.strip_prefix('v').unwrap_or(value).to_owned()
    }
    /// `current_release_channel()`.
    fn stable(&self) -> bool {
        self.channel.map(strip) == Some("release")
    }
}

fn env<'a>(context: &'a Context, key: &str) -> Option<&'a OsStr> {
    context
        .environ
        .get(OsStr::new(key))
        .map(|v| v.as_os_str())
        .filter(|v| !v.is_empty())
}
fn state_file(context: &Context, override_key: &str, name: &str) -> Result<PathBuf> {
    if let Some(path) = env(context, override_key) {
        return context
            .expand_user(Path::new(path))
            .map_err(|e| AppError::new(e.code, e.message));
    }
    Ok(context
        .default_state_root()
        .map_err(|e| AppError::new(e.code, e.message))?
        .join("config")
        .join(name))
}
/// `preference_path()`.
pub fn preference_path(context: &Context) -> Result<PathBuf> {
    state_file(
        context,
        "KASSIBER_UPDATE_PREFERENCE_FILE",
        "update-checks.json",
    )
}
/// `cache_path()`.
pub fn cache_path(context: &Context) -> Result<PathBuf> {
    state_file(context, "KASSIBER_UPDATE_CACHE_FILE", "update-check.json")
}

/// `update_checks_enabled(path)`: explicit consent, failing closed.
pub fn enabled(context: &Context) -> Result<bool> {
    let disabled = env(context, "KASSIBER_DISABLE_UPDATE_CHECK")
        .map(|v| v.to_string_lossy().trim().to_lowercase())
        .is_some_and(|v| ["1", "true", "yes"].contains(&strip(&v)));
    if disabled {
        return Ok(false);
    }
    let path = preference_path(context)?;
    let Some(raw) = kassiber_paths::read_small_regular_file(&path, MAX_PREFERENCE_BYTES) else {
        return Ok(false);
    };
    let Some(payload) = std::str::from_utf8(&raw)
        .ok()
        .and_then(|text| kassiber_pytext::loads(text).ok())
    else {
        return Ok(false);
    };
    Ok(matches!(payload, Value::Dict(_))
        && get(&payload, "schema_version") == Some(&Value::Int(1.into()))
        && get(&payload, "enabled") == Some(&Value::Bool(true)))
}

/// `set_update_checks_enabled(enabled)`: owner-only, atomically replaced.
pub fn set_enabled(context: &Context, value: bool) -> Result<()> {
    let path = preference_path(context)?;
    let io = |e: std::io::Error| AppError::new("internal_error", e.to_string());
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Best effort for existing directories, as in Python.
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    let document = format!(
        "{{\"enabled\": {}, \"schema_version\": 1}}\n",
        if value { "true" } else { "false" }
    );
    kassiber_store::layout::write_private_preference(&path, document.as_bytes()).map_err(io)
}

/// `update --status` / `--disable-checks`: never contacts GitHub.
pub fn preference(context: &Context) -> Result<Value> {
    Ok(dict([
        ("enabled", Value::Bool(enabled(context)?)),
        ("contacts_github", Value::Bool(false)),
    ]))
}

fn offline_enabled(context: &Context) -> Result<bool> {
    let path = kassiber_setup::offline::preference_path(context)?;
    Ok(kassiber_setup::offline::enabled(&path))
}

/// `require_update_checks_enabled()`: offline mode first, then consent.
fn require_enabled(context: &Context) -> Result<()> {
    if offline_enabled(context)? {
        return Err(AppError::new(
            "network_egress_disabled",
            "GitHub update checks are disabled: Kassiber is in offline mode",
        )
        .with_hint("Turn off offline mode to connect."));
    }
    if enabled(context)? {
        return Ok(());
    }
    Err(AppError::new(
        "update_checks_disabled",
        "GitHub update checks are disabled",
    )
    .with_hint("Enable them in Settings > Privacy or run `kassiber update --enable-checks`."))
}

fn unreachable() -> AppError {
    AppError::new(
        "update_check_failed",
        "Could not check GitHub for a Kassiber update",
    )
    .with_hint(format!("Open {RELEASES_PAGE} to check manually."))
    .retryable(true)
}
fn invalid() -> AppError {
    AppError::new(
        "update_check_failed",
        "GitHub returned an invalid Kassiber update response",
    )
    .with_hint(format!("Open {RELEASES_PAGE} to check manually."))
    .retryable(true)
}

/// `_release_from_response(payload)`: the highest published version.
pub fn select_release(payload: &Value) -> Result<Value> {
    let Value::List(items) = payload else {
        return Err(invalid());
    };
    let mut selected: Option<(Value, Version)> = None;
    for item in items {
        if !matches!(item, Value::Dict(_)) || get(item, "draft").is_some_and(truthy) {
            continue;
        }
        let tag = match get(item, "tag_name").filter(|v| truthy(v)) {
            Some(value) => py_str(value),
            None => String::new(),
        };
        let tag = strip(&tag).to_owned();
        let Some(parsed) = parse_version(&tag) else {
            continue;
        };
        let prerelease =
            get(item, "prerelease").is_some_and(truthy) || !parsed.prerelease.is_empty();
        let candidate = dict([
            ("latest_version", s(tag.strip_prefix('v').unwrap_or(&tag))),
            ("release_tag", s(tag.as_str())),
            ("release_url", s(release_url(&tag))),
            ("prerelease", Value::Bool(prerelease)),
        ]);
        if selected
            .as_ref()
            .is_none_or(|(_, best)| compare(&parsed, best) == std::cmp::Ordering::Greater)
        {
            selected = Some((candidate, parsed));
        }
    }
    selected.map(|(candidate, _)| candidate).ok_or_else(invalid)
}

/// `datetime.now(timezone.utc).isoformat()` with a `Z` suffix.
fn checked_at() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs() as i64;
    let days = seconds.div_euclid(86_400);
    let rem = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let micros = now.subsec_micros();
    let fraction = if micros == 0 {
        String::new()
    } else {
        format!(".{micros:06}")
    };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}{fraction}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// `detect_install_method()`.
fn install_method(context: &Context) -> &'static str {
    let explicit = env(context, "KASSIBER_HOMEBREW_PACKAGE")
        .map(|v| strip(&v.to_string_lossy()).to_lowercase())
        .unwrap_or_default();
    match explicit.as_str() {
        "cask" => return "homebrew_cask",
        "formula" => return "homebrew_formula",
        _ => {}
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        candidates.push(exe);
    }
    if let Some(arg0) = std::env::args_os().next() {
        candidates.push(PathBuf::from(arg0));
    }
    let resolved: Vec<PathBuf> = candidates
        .iter()
        .filter_map(|p| std::fs::canonicalize(p).ok())
        .collect();
    let normalized = candidates
        .iter()
        .chain(&resolved)
        .map(|p| p.to_string_lossy().replace('\\', "/").to_lowercase())
        .collect::<Vec<_>>()
        .join("\n");
    if normalized.contains("/cellar/kassiber-cli/") {
        "homebrew_formula"
    } else {
        "manual"
    }
}

/// `_result_from_release(release, checked_at)`.
fn result(context: &Context, build: &Build<'_>, release: &Value, at: String) -> Value {
    let latest = py_str(get(release, "latest_version").unwrap_or(&Value::Null));
    let current = build.current();
    let method = install_method(context);
    let command = match method {
        "homebrew_cask" => s(CASK_COMMAND),
        "homebrew_formula" => s(FORMULA_COMMAND),
        _ => Value::Null,
    };
    dict([
        ("current_version", s(current.as_str())),
        ("latest_version", s(latest.as_str())),
        ("update_available", Value::Bool(is_newer(&latest, &current))),
        (
            "prerelease",
            Value::Bool(get(release, "prerelease").is_some_and(truthy)),
        ),
        (
            "release_url",
            get(release, "release_url").cloned().unwrap_or(Value::Null),
        ),
        ("checked_at", s(at)),
        ("install_method", s(method)),
        ("update_command", command),
    ])
}

/// The GitHub GET, through the consent-bound gate and the process ledger.
fn fetch(context: &Context, build: &Build<'_>) -> Result<Value> {
    require_enabled(context)?;
    let url = if build.stable() {
        LATEST_URL
    } else {
        RELEASES_URL
    };
    let trust = TlsTrust::python_default().map_err(|_| unreachable())?;
    let request = Request::new(
        Subsystem::Update,
        Endpoint::new("api.github.com", 443).map_err(|_| unreachable())?,
        trust,
    );
    let consent = Consent::explicit_trigger(&request);
    let offline_path = kassiber_setup::offline::preference_path(context)?;
    let gate = Gate::new(
        move || {
            if kassiber_setup::offline::enabled(&offline_path) {
                Offline::Offline
            } else {
                Offline::Online
            }
        },
        Ledger::process(),
    );
    let route = gate
        .authorize(request, Some(consent))
        .map_err(|error| match error {
            kassiber_net::Error::Offline | kassiber_net::Error::OfflineUnavailable => {
                AppError::new(
                    "network_egress_disabled",
                    "GitHub update checks are disabled: Kassiber is in offline mode",
                )
                .with_hint("Turn off offline mode to connect.")
            }
            other => other.app_error(),
        })?;
    let response = HttpRequest::new("GET", url)
        .and_then(|r| r.header("Accept", "application/vnd.github+json"))
        .and_then(|r| r.header("User-Agent", &format!("kassiber/{}", build.current())))
        .and_then(|r| r.header("X-GitHub-Api-Version", "2022-11-28"))
        .map_err(|_| unreachable())?
        .send(
            route,
            HttpOptions {
                timeout: Duration::from_secs(5),
                max_response_bytes: MAX_RESPONSE_BYTES + 1,
                ..Default::default()
            },
        )
        .map_err(|error| match error {
            kassiber_net::Error::ResponseTooLarge => invalid(),
            kassiber_net::Error::NoEgress => error.app_error(),
            _ => unreachable(),
        })?;
    if !(200..300).contains(&response.status) {
        return Err(unreachable());
    }
    if response.body.len() > MAX_RESPONSE_BYTES {
        return Err(invalid());
    }
    let payload = std::str::from_utf8(&response.body)
        .ok()
        .and_then(|text| kassiber_pytext::loads(text).ok())
        .ok_or_else(invalid)?;
    select_release(&if build.stable() {
        Value::List(vec![payload])
    } else {
        payload
    })
}

/// `check_for_update()`: consent, one fetch, then the local cache.
pub fn check(context: &Context, build: &Build<'_>) -> Result<Value> {
    require_enabled(context)?;
    let release = fetch(context, build)?;
    let checked = result(context, build, &release, checked_at());
    let cache = dict([
        ("checked_at", get(&checked, "checked_at").cloned().unwrap()),
        (
            "latest_version",
            get(&checked, "latest_version").cloned().unwrap(),
        ),
        ("prerelease", get(&checked, "prerelease").cloned().unwrap()),
        (
            "release_url",
            get(&checked, "release_url").cloned().unwrap(),
        ),
        ("schema_version", Value::Int(1.into())),
    ]);
    let text = kassiber_pyjson::dumps(&cache, &Default::default())
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    kassiber_store::layout::write_private_preference(
        &cache_path(context)?,
        format!("{text}\n").as_bytes(),
    )
    .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    Ok(checked)
}

/// `render_update_status(result, color)`.
pub fn render(result: &Value, color: bool) -> String {
    let field = |key| py_str(get(result, key).unwrap_or(&Value::Null));
    let (current, latest, url) = (
        field("current_version"),
        field("latest_version"),
        field("release_url"),
    );
    if !get(result, "update_available").is_some_and(truthy) {
        return format!("Kassiber {current} is current (latest: {latest}).\n");
    }
    let mut title = format!("✨ Update available: Kassiber {current} → {latest}");
    if color {
        title = format!("\x1b[1;36m{title}\x1b[0m");
    }
    let instruction = match get(result, "update_command") {
        Some(Value::Str(command)) if !command.is("") => {
            let command = py_str(&Value::Str(command.clone()));
            if color {
                format!("Run \x1b[36m{command}\x1b[0m to update.")
            } else {
                format!("Run {command} to update.")
            }
        }
        _ => "Download and install the release manually.".into(),
    };
    format!("{title}\n  {instruction}\n  Release notes: {url}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_order_like_semver_and_python() {
        assert!(parse_version("v1.2.3-rc.1+build.5").is_some());
        assert!(parse_version("01.2.3").is_none());
        assert!(parse_version("1.2.3-01").is_none());
        assert!(parse_version("1.2.3-").is_none());
        assert!(is_newer("1.2.3", "1.2.3-rc.1"));
        assert!(is_newer("1.2.3-rc.10", "1.2.3-rc.9"));
        assert!(is_newer("1.2.3-rc.a", "1.2.3-rc.1"));
        assert!(is_newer("1.2.3-rc.1.1", "1.2.3-rc.1"));
        assert!(is_newer("10.0.0", "9.99.99"));
        assert!(!is_newer("1.2.3+b", "1.2.3"));
        assert_eq!(quote("v1.0.0/ä"), "v1.0.0%2F%C3%A4");
    }

    #[test]
    fn release_selection_skips_drafts_and_keeps_the_first_highest() {
        let payload = kassiber_pytext::loads(
            r#"[{"tag_name":"v1.0.0"},{"tag_name":"v2.0.0","draft":true},
               {"tag_name":" v1.1.0-rc.1 "},{"tag_name":"junk"},{"tag_name":"v1.1.0-rc.1","prerelease":false}]"#,
        )
        .unwrap();
        let chosen = select_release(&payload).unwrap();
        assert_eq!(get(&chosen, "release_tag"), Some(&s("v1.1.0-rc.1")));
        assert_eq!(get(&chosen, "prerelease"), Some(&Value::Bool(true)));
        assert!(select_release(&kassiber_pytext::loads("[]").unwrap()).is_err());
        assert!(select_release(&kassiber_pytext::loads("{}").unwrap()).is_err());
    }
}
