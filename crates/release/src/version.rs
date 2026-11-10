//! Release version parsing and ordering, shared by the update check and the
//! release verification (and so by the release tool, which builds without the
//! update check's network and store crates).
use crate::strip;

/// A parsed semantic version; numeric parts keep their decimal text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub(crate) core: [String; 3],
    pub(crate) prerelease: Vec<String>,
}

/// `0|[1-9][0-9]*`.
fn numeric(part: &str) -> bool {
    !part.is_empty()
        && part.bytes().all(|b| b.is_ascii_digit())
        && (part == "0" || !part.starts_with('0'))
}
fn identifiers(text: &str) -> Option<Vec<String>> {
    let parts: Vec<String> = text.split('.').map(str::to_owned).collect();
    parts
        .iter()
        .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        .then_some(parts)
}

/// `parse_version(value)`.
pub fn parse_version(value: &str) -> Option<Version> {
    let text = strip(value);
    let text = text.strip_prefix('v').unwrap_or(text);
    let (text, build) = match text.split_once('+') {
        Some((head, build)) => (head, Some(build)),
        None => (text, None),
    };
    let (core, prerelease) = match text.split_once('-') {
        Some((head, pre)) => (head, Some(pre)),
        None => (text, None),
    };
    let numbers: Vec<&str> = core.split('.').collect();
    if numbers.len() != 3 || !numbers.iter().all(|n| numeric(n)) {
        return None;
    }
    if let Some(build) = build {
        identifiers(build)?;
    }
    let prerelease = match prerelease {
        Some(pre) => identifiers(pre)?,
        None => Vec::new(),
    };
    if prerelease
        .iter()
        .any(|p| p.bytes().all(|b| b.is_ascii_digit()) && p.len() > 1 && p.starts_with('0'))
    {
        return None;
    }
    Some(Version {
        core: [numbers[0].into(), numbers[1].into(), numbers[2].into()],
        prerelease,
    })
}

/// Compare decimal integers without leading zeros.
fn number_order(left: &str, right: &str) -> std::cmp::Ordering {
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

/// `_compare_versions(left, right)`.
pub(crate) fn compare(left: &Version, right: &Version) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    for (a, b) in left.core.iter().zip(&right.core) {
        match number_order(a, b) {
            Equal => continue,
            other => return other,
        }
    }
    match (left.prerelease.is_empty(), right.prerelease.is_empty()) {
        (true, true) => return Equal,
        (true, false) => return Greater,
        (false, true) => return Less,
        _ => {}
    }
    for (a, b) in left.prerelease.iter().zip(&right.prerelease) {
        if a == b {
            continue;
        }
        let (an, bn) = (
            a.bytes().all(|c| c.is_ascii_digit()),
            b.bytes().all(|c| c.is_ascii_digit()),
        );
        return match (an, bn) {
            (true, true) => number_order(a, b),
            (true, false) => Less,
            (false, true) => Greater,
            _ => a.cmp(b),
        };
    }
    left.prerelease.len().cmp(&right.prerelease.len())
}

/// `is_newer_version(latest, current)`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => compare(&l, &c) == std::cmp::Ordering::Greater,
        _ => false,
    }
}
