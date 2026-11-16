//! `kassiber://` links: the public URL contract of `menu_action_for_deep_link` (src-tauri lib.rs).
//!
//! Supported forms (the host is matched as the URL parser hands it over; path segments are
//! lowercased before matching):
//!   kassiber://<route>                        navigates to /<route>
//!   kassiber://settings                       opens Settings (no section)
//!   kassiber://settings/<section>             opens Settings, focuses panel
//!   kassiber://workflow/add-wallet            opens wallet-source setup
//!   kassiber://workflow/process-journals      rebuilds journal state
//!   kassiber://lock                           locks the workspace
//!
//! Hosts and sections are fixed allowlists, so a link only decides *where the app goes*. Nothing
//! here may reach the network: there is deliberately no `sync` workflow.
//!
//! Tauri parses with the `url` crate (WHATWG). `kassiber` is a non-special scheme, so its host is
//! an *opaque* host: kept as written (not lowercased, not decoded), and a malformed port makes the
//! whole URL invalid. [`parse`] reproduces exactly the parts the matcher reads (scheme, host,
//! path segments after dot-segment removal) and fails where `Url::parse` fails for those parts.

use super::intent::Intent;

pub const SCHEME: &str = "kassiber";

/// `DEEP_LINK_ROUTE_HOSTS`.
pub const ROUTE_HOSTS: &[(&str, &str)] = &[
    ("overview", "/overview"),
    ("transactions", "/transactions"),
    ("connections", "/connections"),
    ("books", "/books"),
    ("reports", "/reports"),
    ("source-of-funds", "/source-of-funds"),
    ("journals", "/journals"),
    ("tax-events", "/journals"),
    ("review", "/review"),
    // The Review screen replaced Quarantine; old links still land there.
    ("quarantine", "/review"),
    ("assistant", "/assistant"),
    ("logs", "/logs"),
    ("diagnostics", "/logs"),
];

/// `DEEP_LINK_SETTINGS_SECTIONS` (the React `settingsSectionForHash` aliases).
pub const SETTINGS_SECTIONS: &[&str] = &[
    "appearance",
    "privacy",
    "developer",
    "logs",
    "display",
    "explorer",
    "explorers",
    "bitcoin",
    "lightning",
    "liquid",
    "market",
    "desktop",
    "terminal",
    "security",
    "lock",
    "backends",
    "rates",
    "ai",
    "assistant",
    "data",
    "storage",
];

/// The parts of a URL the matcher reads.
#[derive(Debug, PartialEq, Eq)]
pub struct Parsed {
    /// ASCII-lowercased.
    pub scheme: String,
    /// `host_str()`: None without an authority (`kassiber:lock`, `kassiber:/lock`).
    pub host: Option<String>,
    /// `path_segments()` after dot-segment removal, empty ones dropped, ASCII-lowercased.
    pub segments: Vec<String>,
}

fn is_single_dot(s: &str) -> bool {
    s == "." || s.eq_ignore_ascii_case("%2e")
}

fn is_double_dot(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        ".." | ".%2e" | "%2e." | "%2e%2e"
    )
}

/// What `Url::parse` keeps of `input`; None where it fails (or has no scheme).
pub fn parse(input: &str) -> Option<Parsed> {
    // Leading/trailing C0 controls and spaces go; tabs and newlines anywhere go.
    let trimmed = input.trim_matches(|c: char| c <= ' ');
    let s: String = trimmed
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let colon = s.find(':')?;
    let scheme = &s[..colon];
    let mut chars = scheme.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return None;
    }
    let scheme = scheme.to_ascii_lowercase();
    let rest = &s[colon + 1..];
    let Some(after) = rest.strip_prefix("//") else {
        // No authority: an opaque or host-less path, `host_str()` is None.
        return Some(Parsed {
            scheme,
            host: None,
            segments: vec![],
        });
    };
    let end = after.find(['/', '?', '#']).unwrap_or(after.len());
    let (authority, tail) = after.split_at(end);
    // Userinfo ends at the last `@`; a host may not be empty after it.
    let host_port = match authority.rfind('@') {
        Some(i) => {
            let hp = &authority[i + 1..];
            if hp.is_empty() {
                return None;
            }
            hp
        }
        None => authority,
    };
    let (host, port) = if host_port.starts_with('[') {
        let close = host_port.find(']')?;
        let (h, p) = host_port.split_at(close + 1);
        (
            h,
            if p.is_empty() {
                None
            } else {
                Some(p.strip_prefix(':')?)
            },
        )
    } else {
        match host_port.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (host_port, None),
        }
    };
    if let Some(p) = port {
        if !p.chars().all(|c| c.is_ascii_digit())
            || (!p.is_empty() && p.parse::<u32>().map_or(true, |n| n > 65535))
        {
            return None;
        }
        if host.is_empty() {
            return None;
        }
    }
    // Opaque-host forbidden code points fail the parse (`%` is allowed, kept encoded).
    const FORBIDDEN: &[char] = &[
        '\0', '\t', '\n', '\r', ' ', '#', '/', ':', '<', '>', '?', '@', '[', '\\', ']', '^', '|',
    ];
    if !host.starts_with('[') && host.contains(FORBIDDEN) {
        return None;
    }
    let path = tail.split(['?', '#']).next().unwrap_or("");
    let mut stack: Vec<&str> = Vec::new();
    if let Some(p) = path.strip_prefix('/') {
        for seg in p.split('/') {
            if is_double_dot(seg) {
                stack.pop();
            } else if !is_single_dot(seg) {
                stack.push(seg);
            }
        }
    }
    let segments = stack
        .into_iter()
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    Some(Parsed {
        scheme,
        host: Some(host.to_owned()),
        segments,
    })
}

fn route(host: &str) -> Option<&'static str> {
    ROUTE_HOSTS
        .iter()
        .find(|(slug, _)| *slug == host)
        .map(|(_, r)| *r)
}

fn settings_section(section: &str) -> Option<&'static str> {
    SETTINGS_SECTIONS
        .iter()
        .copied()
        .find(|known| *known == section)
}

/// `menu_action_for_deep_link`.
pub fn intent_for(url: &str) -> Option<Intent> {
    let p = parse(url)?;
    if p.scheme != SCHEME {
        return None;
    }
    let host = p.host?;
    let first = p.segments.first().map(String::as_str);
    match host.as_str() {
        "lock" if p.segments.is_empty() => Some(Intent::Action("lock-app")),
        "settings" => Some(Intent::OpenSettings(first.and_then(settings_section))),
        // No `sync-all` / `sync` arm: a wallet sync is the largest egress event in the app, and a
        // deep link is a link. `add-wallet` opens a dialog and `process-journals` is local.
        "workflow" => match first {
            Some("add-wallet") => Some(Intent::Action("add-wallet")),
            Some("process-journals") => Some(Intent::Action("process-journals")),
            _ => None,
        },
        h if p.segments.is_empty() => route(h).map(Intent::Navigate),
        _ => None,
    }
}

/// What may be said about a link that was refused: never the link itself (it comes from outside
/// and may carry anything), only its size. Tauri prints the whole URL to stderr; logs here stay
/// redacted (docs/reference/logging.md).
pub fn redacted(url: &str) -> String {
    format!("<{} chars>", url.chars().count())
}

/// Launch arguments that are links on this scheme (`kassiber://…`, case-insensitive scheme).
pub fn links_in(args: &[String]) -> Vec<String> {
    args.iter()
        .filter(|a| {
            a.get(..SCHEME.len() + 3)
                .is_some_and(|p| p.eq_ignore_ascii_case("kassiber://"))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nav(r: &'static str) -> Option<Intent> {
        Some(Intent::Navigate(r))
    }

    #[test]
    fn deep_links_route_to_known_actions() {
        assert_eq!(intent_for("kassiber://transactions"), nav("/transactions"));
        assert_eq!(
            intent_for("kassiber://source-of-funds"),
            nav("/source-of-funds")
        );
        assert_eq!(intent_for("kassiber://review"), nav("/review"));
        assert_eq!(intent_for("kassiber://quarantine"), nav("/review"));
        assert_eq!(intent_for("kassiber://tax-events"), nav("/journals"));
        assert_eq!(intent_for("kassiber://diagnostics"), nav("/logs"));
        assert_eq!(
            intent_for("kassiber://settings"),
            Some(Intent::OpenSettings(None))
        );
        for section in SETTINGS_SECTIONS {
            assert_eq!(
                intent_for(&format!("kassiber://settings/{section}")),
                Some(Intent::OpenSettings(Some(section))),
                "{section}"
            );
        }
        assert_eq!(
            intent_for("kassiber://settings/wallet-of-satoshi"),
            Some(Intent::OpenSettings(None))
        );
        assert_eq!(
            intent_for("kassiber://workflow/add-wallet"),
            Some(Intent::Action("add-wallet"))
        );
        assert_eq!(
            intent_for("kassiber://workflow/process-journals"),
            Some(Intent::Action("process-journals"))
        );
        assert_eq!(
            intent_for("kassiber://lock"),
            Some(Intent::Action("lock-app"))
        );
    }

    #[test]
    fn deep_links_never_trigger_a_wallet_sync() {
        assert_eq!(intent_for("kassiber://workflow/sync"), None);
        assert_eq!(intent_for("kassiber://workflow/sync-all"), None);
        assert_eq!(intent_for("kassiber://workflow/Sync-All"), None);
    }

    #[test]
    fn deep_links_normalize_segment_case() {
        assert_eq!(
            intent_for("kassiber://settings/Privacy"),
            Some(Intent::OpenSettings(Some("privacy")))
        );
        assert_eq!(
            intent_for("kassiber://workflow/PROCESS-JOURNALS"),
            Some(Intent::Action("process-journals"))
        );
        assert_eq!(
            intent_for("kassiber://workflow/Add-Wallet"),
            Some(Intent::Action("add-wallet"))
        );
    }

    #[test]
    fn deep_links_reject_unknown_routes() {
        assert_eq!(intent_for("kassiber://wallet-of-satoshi"), None);
        assert_eq!(intent_for("kassiber://workflow/drain-everything"), None);
        assert_eq!(intent_for("kassiber://lock/maybe"), None);
        assert_eq!(intent_for("https://transactions/abc"), None);
    }

    /// What `url::Url` (WHATWG, non-special scheme) does that the matcher can observe.
    #[test]
    fn parsing_follows_the_url_crate() {
        // The scheme is case-insensitive; an opaque host keeps its case, so it doesn't match.
        assert_eq!(
            intent_for("KASSIBER://lock"),
            Some(Intent::Action("lock-app"))
        );
        assert_eq!(intent_for("kassiber://Lock"), None);
        assert_eq!(intent_for("kassiber://%6Cock"), None);
        // Userinfo, port, query and fragment are not part of the match.
        assert_eq!(
            intent_for("kassiber://user:pw@lock"),
            Some(Intent::Action("lock-app"))
        );
        assert_eq!(
            intent_for("kassiber://lock:80"),
            Some(Intent::Action("lock-app"))
        );
        assert_eq!(
            intent_for("kassiber://lock:/"),
            Some(Intent::Action("lock-app"))
        );
        assert_eq!(
            intent_for("kassiber://settings/privacy?x=1#y"),
            Some(Intent::OpenSettings(Some("privacy")))
        );
        assert_eq!(
            intent_for("kassiber://lock?now"),
            Some(Intent::Action("lock-app"))
        );
        // An invalid port or host fails the whole URL.
        assert_eq!(intent_for("kassiber://lock:abc"), None);
        assert_eq!(intent_for("kassiber://lock:99999"), None);
        assert_eq!(intent_for("kassiber://user@"), None);
        assert_eq!(intent_for("kassiber://lo<k"), None);
        // No authority: no host.
        assert_eq!(intent_for("kassiber:lock"), None);
        assert_eq!(intent_for("kassiber:/lock"), None);
        assert_eq!(intent_for("kassiber:///lock"), None);
        // Dot segments resolve before the match; empty segments drop.
        assert_eq!(
            intent_for("kassiber://settings/x/../privacy"),
            Some(Intent::OpenSettings(Some("privacy")))
        );
        assert_eq!(
            intent_for("kassiber://settings//privacy/"),
            Some(Intent::OpenSettings(Some("privacy")))
        );
        assert_eq!(intent_for("kassiber://overview/."), nav("/overview"));
        assert_eq!(
            intent_for("kassiber://lock/%2E%2E"),
            Some(Intent::Action("lock-app"))
        );
        assert_eq!(intent_for("kassiber://overview/"), nav("/overview"));
        // Surrounding whitespace and embedded tabs/newlines are stripped.
        assert_eq!(
            intent_for("  kassiber://lo\nck \t"),
            Some(Intent::Action("lock-app"))
        );
        // Encoded segments are not decoded.
        assert_eq!(
            intent_for("kassiber://settings/%70rivacy"),
            Some(Intent::OpenSettings(None))
        );
    }

    #[test]
    fn refused_links_are_redacted_and_launch_links_are_found() {
        assert_eq!(redacted("kassiber://xpub6CUGRU…"), "<22 chars>");
        let args = [
            "--data-root".into(),
            "/x".into(),
            "KASSIBER://lock".into(),
            "https://x".into(),
        ];
        assert_eq!(links_in(&args), vec!["KASSIBER://lock".to_owned()]);
    }
}
