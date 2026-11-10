//! `read_skill_reference(name)` (`kassiber/ai/tools.py:4049`): only the
//! allowlisted references, embedded at build time from
//! `crates/ai/skill_references/`. Nothing is read from disk, and never
//! from the working directory, which under MCP is an agent's untrusted
//! repository.

use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::{PyStr, Value};

use crate::catalog::skill_reference_index;

/// `SKILL_REFERENCE_NAMES`, in the advertised enum's order.
pub const SKILL_REFERENCE_NAMES: [&str; 11] = [
    "index",
    "command-templates",
    "journal-processing",
    "metadata",
    "onboarding",
    "reports",
    "secrets-and-backup",
    "swap-matching",
    "troubleshooting",
    "verification",
    "wallets-backends",
];

const REFERENCES: [(&str, &str); 10] = [
    (
        "command-templates",
        include_str!("../skill_references/command-templates.md"),
    ),
    (
        "journal-processing",
        include_str!("../skill_references/journal-processing.md"),
    ),
    (
        "metadata",
        include_str!("../skill_references/metadata.md"),
    ),
    (
        "onboarding",
        include_str!("../skill_references/onboarding.md"),
    ),
    (
        "reports",
        include_str!("../skill_references/reports.md"),
    ),
    (
        "secrets-and-backup",
        include_str!("../skill_references/secrets-and-backup.md"),
    ),
    (
        "swap-matching",
        include_str!("../skill_references/swap-matching.md"),
    ),
    (
        "troubleshooting",
        include_str!("../skill_references/troubleshooting.md"),
    ),
    (
        "verification",
        include_str!("../skill_references/verification.md"),
    ),
    (
        "wallets-backends",
        include_str!("../skill_references/wallets-backends.md"),
    ),
];

/// The content of one allowlisted reference.
pub fn skill_reference_content(name: &str) -> Option<&'static str> {
    if name == "index" {
        return Some(skill_reference_index());
    }
    REFERENCES
        .iter()
        .find(|(reference, _)| *reference == name)
        .map(|(_, content)| *content)
}

/// `read_skill_reference(name)`: `{"name", "content"}`, or
/// `tool_not_allowed` for any other name.
pub fn read_skill_reference(name: &PyStr) -> Result<Value, AppError> {
    let content = name
        .as_str()
        .filter(|name| SKILL_REFERENCE_NAMES.contains(name))
        .and_then(skill_reference_content);
    match content {
        Some(content) => Ok(dict([
            ("name", Value::Str(name.clone())),
            ("content", s(content)),
        ])),
        None => Err(
            AppError::new("tool_not_allowed", "skill reference is not allowlisted").with_details(
                dict([
                    ("name", Value::Str(name.clone())),
                    (
                        "allowed",
                        Value::List(SKILL_REFERENCE_NAMES.iter().map(|name| s(*name)).collect()),
                    ),
                ]),
            ),
        ),
    }
}
