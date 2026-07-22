use reqwest::header::{ACCEPT, USER_AGENT};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::AppHandle;

const RELEASES_API_URL: &str =
    "https://api.github.com/repos/bitcoinaustria/kassiber/releases?per_page=10";
const RELEASE_PAGE_URL: &str = "https://github.com/bitcoinaustria/kassiber/releases/tag";
const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RESPONSE_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
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

fn parse_release_version(tag_name: &str) -> Option<Version> {
    let normalized = tag_name.trim().strip_prefix('v').unwrap_or(tag_name.trim());
    Version::parse(normalized).ok()
}

fn newest_release(
    releases: &[GitHubRelease],
    include_prereleases: bool,
) -> Option<(&GitHubRelease, Version)> {
    releases
        .iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            parse_release_version(&release.tag_name).and_then(|version| {
                let prerelease = release.prerelease || !version.pre.is_empty();
                (include_prereleases || !prerelease).then_some((release, version))
            })
        })
        .max_by(|(_, left), (_, right)| left.cmp_precedence(right))
}

fn release_url(tag_name: &str) -> String {
    // Only semver-valid tag names reach this helper, so the tag cannot smuggle
    // a host, query, or path separator into the trusted GitHub release URL.
    format!("{RELEASE_PAGE_URL}/{}", tag_name.trim())
}

fn append_response_chunk(body: &mut Vec<u8>, chunk: &[u8]) -> Result<(), String> {
    if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(body.len()) {
        return Err("GitHub returned an unexpectedly large release response.".to_string());
    }
    body.extend_from_slice(chunk);
    Ok(())
}

fn build_update_check(
    current: &Version,
    releases: &[GitHubRelease],
    include_prereleases: bool,
) -> Result<AppUpdateCheck, String> {
    let newest = newest_release(releases, include_prereleases)
        .ok_or_else(|| "GitHub did not return a valid Kassiber release.".to_string())?;
    let update_available = newest.1.cmp_precedence(current).is_gt();
    Ok(AppUpdateCheck {
        current_version: current.to_string(),
        latest_version: Some(newest.1.to_string()),
        release_url: update_available.then(|| release_url(&newest.0.tag_name)),
        update_available,
        prerelease: newest.0.prerelease || !newest.1.pre.is_empty(),
        checked_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    })
}

fn debug_update_check(current: &Version) -> AppUpdateCheck {
    AppUpdateCheck {
        current_version: current.to_string(),
        latest_version: Some(current.to_string()),
        release_url: None,
        update_available: false,
        prerelease: !current.pre.is_empty(),
        checked_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    }
}

fn include_prereleases() -> bool {
    env!("KASSIBER_BUILD_CHANNEL") != "release"
}

#[tauri::command]
pub async fn check_app_update(app: AppHandle) -> Result<AppUpdateCheck, String> {
    let current = app.package_info().version.clone();
    if cfg!(debug_assertions) {
        return Ok(debug_update_check(&current));
    }
    let client = reqwest::Client::builder()
        .timeout(UPDATE_CHECK_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not prepare the GitHub update check.".to_string())?;
    let mut response = client
        .get(RELEASES_API_URL)
        .header(ACCEPT, "application/vnd.github+json")
        .header(USER_AGENT, format!("kassiber/{current}"))
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|_| "Could not reach GitHub to check for updates.".to_string())?
        .error_for_status()
        .map_err(|_| "GitHub did not accept the update check.".to_string())?;
    if response.content_length().unwrap_or(0) > MAX_RESPONSE_BYTES as u64 {
        return Err("GitHub returned an unexpectedly large release response.".to_string());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read GitHub's release response.".to_string())?
    {
        append_response_chunk(&mut body, &chunk)?;
    }
    let releases = serde_json::from_slice::<Vec<GitHubRelease>>(&body)
        .map_err(|_| "GitHub returned an invalid release response.".to_string())?;

    build_update_check(&current, &releases, include_prereleases())
}

#[cfg(test)]
mod tests {
    use super::{
        append_response_chunk, build_update_check, newest_release, GitHubRelease,
        MAX_RESPONSE_BYTES,
    };
    use semver::Version;

    #[derive(serde::Deserialize)]
    struct SemverComparison {
        latest: String,
        current: String,
        newer: bool,
    }

    #[derive(serde::Deserialize)]
    struct SemverCases {
        comparisons: Vec<SemverComparison>,
        invalid: Vec<String>,
    }

    fn release(tag_name: &str, draft: bool, prerelease: bool) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag_name.to_string(),
            draft,
            prerelease,
        }
    }

    #[test]
    fn selects_highest_semver_and_ignores_drafts_or_invalid_tags() {
        let releases = vec![
            release("nightly", false, true),
            release("v0.22.57", true, false),
            release("v0.22.56", false, true),
            release("0.22.55", false, false),
        ];

        let (selected, version) = newest_release(&releases, true).expect("release");
        assert_eq!(selected.tag_name, "v0.22.56");
        assert_eq!(version, Version::new(0, 22, 56));
    }

    #[test]
    fn reports_new_prerelease_without_selecting_an_asset() {
        let result = build_update_check(
            &Version::new(0, 22, 55),
            &[release("v0.22.56-rc.1", false, false)],
            true,
        )
        .expect("valid release");

        assert!(result.update_available);
        assert!(result.prerelease);
        assert_eq!(result.latest_version.as_deref(), Some("0.22.56-rc.1"));
        assert_eq!(
            result.release_url.as_deref(),
            Some("https://github.com/bitcoinaustria/kassiber/releases/tag/v0.22.56-rc.1")
        );
    }

    #[test]
    fn omits_download_link_when_installed_version_is_current_or_newer() {
        let result = build_update_check(
            &Version::new(0, 22, 56),
            &[release("v0.22.56", false, true)],
            true,
        )
        .expect("valid release");

        assert!(!result.update_available);
        assert_eq!(result.release_url, None);
    }

    #[test]
    fn bounds_release_response_even_without_a_content_length() {
        let mut body = vec![0; MAX_RESPONSE_BYTES - 2];
        append_response_chunk(&mut body, &[1, 2]).expect("within limit");
        assert!(append_response_chunk(&mut body, &[3]).is_err());
    }

    #[test]
    fn semver_precedence_ignores_build_metadata() {
        let result = build_update_check(
            &Version::parse("0.22.56+installed").unwrap(),
            &[release("v0.22.56+published", false, false)],
            true,
        )
        .expect("valid release");

        assert!(!result.update_available);
        assert_eq!(result.release_url, None);
    }

    #[test]
    fn matches_the_shared_python_rust_semver_contract() {
        let cases: SemverCases = serde_json::from_str(include_str!(
            "../../../tests/fixtures/update_semver_cases.json"
        ))
        .expect("shared semver cases");

        for case in cases.comparisons {
            let latest = super::parse_release_version(&case.latest).expect("latest version");
            let current = super::parse_release_version(&case.current).expect("current version");
            assert_eq!(
                latest.cmp_precedence(&current).is_gt(),
                case.newer,
                "{} compared with {}",
                case.latest,
                case.current
            );
        }
        for value in cases.invalid {
            assert!(
                super::parse_release_version(&value).is_none(),
                "unexpectedly accepted {value}"
            );
        }
    }

    #[test]
    fn rejects_an_empty_or_invalid_release_list() {
        let current = Version::new(0, 22, 56);
        assert!(build_update_check(&current, &[], true).is_err());
        assert!(build_update_check(&current, &[release("nightly", false, true)], true).is_err());
    }

    #[test]
    fn stable_channel_ignores_prereleases() {
        let releases = vec![
            release("v1.1.0-rc.1", false, true),
            release("v1.0.1", false, false),
        ];

        let result =
            build_update_check(&Version::new(1, 0, 0), &releases, false).expect("stable release");

        assert_eq!(result.latest_version.as_deref(), Some("1.0.1"));
        assert!(!result.prerelease);
    }
}
