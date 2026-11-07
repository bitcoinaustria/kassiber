//! User-controlled access for external agents (MCP), off by default
//! (`kassiber/agent_access.py`).
//!
//! One small global preference beside the offline and update-check files, so
//! the desktop Settings switch and every `kassiber mcp` process enforce the
//! same choice before any book is opened. The desktop also mirrors its AI
//! master switch here: external agents are served only while both are on.
//! Installs that never ran the desktop record no master-switch decision
//! (`None`), which does not block a CLI user's own opt-in. Anything
//! unreadable reads as off.
use super::*;
use kassiber_paths::Context;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

pub const SCHEMA_VERSION: i64 = 1;
pub const FILENAME: &str = "agent-access.json";
pub const PATH_ENV: &str = "KASSIBER_AGENT_ACCESS_FILE";
const MAX_BYTES: usize = 1024;

/// `preference_path()`: the override, or `config/agent-access.json` under
/// the default state root.
pub fn preference_path(context: &Context) -> Result<PathBuf> {
    if let Some(path) = context
        .environ
        .get(OsStr::new(PATH_ENV))
        .filter(|v| !v.is_empty())
    {
        return context.expand_user(Path::new(path)).map_err(storage_error);
    }
    Ok(context
        .default_state_root()
        .map_err(storage_error)?
        .join("config")
        .join(FILENAME))
}

/// `GRANTS`: the operator broker's cumulative non-admin capabilities,
/// lowest first. Admin is never grantable.
pub const GRANTS: [&str; 3] = ["read", "operator", "accounting_decisions"];
/// `DEFAULT_GRANT`.
pub const DEFAULT_GRANT: &str = "read";
/// `AGENT_ACCESS_MODES`: the per-provider meaning (#678).
pub const AGENT_ACCESS_MODES: [&str; 2] = ["full", "restricted"];
/// `DEFAULT_AGENT_ACCESS`: a file from before the field restricts.
pub const DEFAULT_AGENT_ACCESS: &str = "restricted";

/// The recorded choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access {
    pub mcp_enabled: bool,
    /// The desktop's AI master switch; `None` when it never recorded one.
    pub ai_features_enabled: Option<bool>,
    /// What agents may do (#686).
    pub grant: &'static str,
    /// `full` or `restricted`.
    pub agent_access: &'static str,
    /// Whether the user recorded a grant or access choice (the desktop
    /// preselects its recommendation only for a first enable).
    pub access_recorded: bool,
}

impl Default for Access {
    fn default() -> Self {
        Access {
            mcp_enabled: false,
            ai_features_enabled: None,
            grant: DEFAULT_GRANT,
            agent_access: DEFAULT_AGENT_ACCESS,
            access_recorded: false,
        }
    }
}

fn known(value: Option<&Value>, names: &[&'static str]) -> Option<&'static str> {
    match value {
        Some(Value::Str(text)) => names.iter().copied().find(|name| text.is(name)),
        _ => None,
    }
}

/// `_read(path)`: a regular file of at most 1,024 bytes holding a JSON
/// object whose `schema_version` is exactly the integer 1 (not `true`, not
/// `1.0`); `mcp_enabled` counts only as the literal `true`, and an unknown
/// grant or access mode falls back to `read` and `restricted`.
pub fn read(path: &Path) -> Access {
    let Some(raw) = kassiber_paths::read_small_regular_file(path, MAX_BYTES) else {
        return Access::default();
    };
    let Some(payload) = std::str::from_utf8(&raw)
        .ok()
        .and_then(|text| kassiber_pytext::loads(text).ok())
    else {
        return Access::default();
    };
    if !matches!(payload, Value::Dict(_))
        || !matches!(get(&payload, "schema_version"), Some(Value::Int(v)) if *v == SCHEMA_VERSION.into())
    {
        return Access::default();
    }
    Access {
        mcp_enabled: get(&payload, "mcp_enabled") == Some(&Value::Bool(true)),
        ai_features_enabled: match get(&payload, "ai_features_enabled") {
            Some(Value::Bool(enabled)) => Some(*enabled),
            _ => None,
        },
        grant: known(get(&payload, "grant"), &GRANTS).unwrap_or(DEFAULT_GRANT),
        agent_access: known(get(&payload, "agent_access"), &AGENT_ACCESS_MODES)
            .unwrap_or(DEFAULT_AGENT_ACCESS),
        access_recorded: get(&payload, "grant").is_some()
            || get(&payload, "agent_access").is_some(),
    }
}

/// `normalize_grant(value)`.
pub fn normalize_grant(value: &Value) -> Result<&'static str> {
    known(Some(value), &GRANTS).ok_or_else(|| {
        AppError::new(
            "validation",
            "the external agent grant must be read, operator or accounting_decisions",
        )
        .with_hint("Admin work is never granted to agents.")
    })
}

/// `normalize_agent_access(value)`.
pub fn normalize_agent_access(value: &Value) -> Result<&'static str> {
    known(Some(value), &AGENT_ACCESS_MODES).ok_or_else(|| {
        AppError::new(
            "validation",
            "external agent access must be full or restricted",
        )
    })
}

fn rank(grant: &str) -> usize {
    GRANTS
        .iter()
        .position(|name| *name == grant)
        .unwrap_or_else(|| panic!("unknown grant {grant}"))
}

/// `grant_allows(granted, required)`: whether a cumulative grant covers
/// `required` (both from `GRANTS`).
pub fn grant_allows(granted: &str, required: &str) -> bool {
    rank(granted) >= rank(required)
}

/// `narrower_grant(first, second)`.
pub fn narrower_grant<'a>(first: &'a str, second: &'a str) -> &'a str {
    if rank(first) <= rank(second) {
        first
    } else {
        second
    }
}

impl Access {
    /// Why agents are not served: MCP off outranks AI off.
    pub fn reason(&self) -> Option<&'static str> {
        if !self.mcp_enabled {
            Some("mcp_disabled")
        } else if self.ai_features_enabled == Some(false) {
            Some("ai_features_disabled")
        } else {
            None
        }
    }

    /// `agent_access_status(path)`.
    pub fn status(&self) -> Value {
        let reason = self.reason();
        dict([
            ("mcp_enabled", Value::Bool(self.mcp_enabled)),
            (
                "ai_features_enabled",
                self.ai_features_enabled.map_or(Value::Null, Value::Bool),
            ),
            ("mcp_available", Value::Bool(reason.is_none())),
            ("reason", reason.map_or(Value::Null, s)),
            ("grant", s(self.grant)),
            ("agent_access", s(self.agent_access)),
            ("access_recorded", Value::Bool(self.access_recorded)),
        ])
    }
}

/// `agent_access_status(path)`.
pub fn status(path: &Path) -> Value {
    read(path).status()
}

/// The fields `set_agent_access(...)` may change; `None` keeps the
/// recorded value.
#[derive(Clone, Debug, Default)]
pub struct Changes {
    pub mcp_enabled: Option<bool>,
    pub ai_features_enabled: Option<bool>,
    pub grant: Option<Value>,
    pub agent_access: Option<Value>,
}

/// `set_agent_access(...)`: update any field, keeping the others, and write
/// `json.dumps(document, sort_keys=True)` with a final newline. A grant and
/// an access mode are recorded together once either is chosen.
pub fn set(path: &Path, changes: Changes) -> Result<Value> {
    let grant = changes.grant.as_ref().map(normalize_grant).transpose()?;
    let agent_access = changes
        .agent_access
        .as_ref()
        .map(normalize_agent_access)
        .transpose()?;
    let current = read(path);
    let mut document = vec![
        ("schema_version", Value::from(SCHEMA_VERSION)),
        (
            "mcp_enabled",
            Value::Bool(changes.mcp_enabled.unwrap_or(current.mcp_enabled)),
        ),
        (
            "ai_features_enabled",
            changes
                .ai_features_enabled
                .or(current.ai_features_enabled)
                .map_or(Value::Null, Value::Bool),
        ),
    ];
    if grant.is_some() || agent_access.is_some() || current.access_recorded {
        document.push(("grant", s(grant.unwrap_or(current.grant))));
        document.push((
            "agent_access",
            s(agent_access.unwrap_or(current.agent_access)),
        ));
    }
    let text = kassiber_pyjson::dumps(
        &dict(document),
        &kassiber_pyjson::Options::new().sort_keys(true),
    )
    .expect("a preference encodes")
        + "\n";
    kassiber_store::layout::write_private_preference(path, text.as_bytes())
        .map_err(|e| projects::io_error(e, path))?;
    Ok(status(path))
}

/// `set_agent_access(mcp_enabled=..., ai_features_enabled=...)`: the two
/// switches alone.
pub fn set_switches(
    path: &Path,
    mcp_enabled: Option<bool>,
    ai_features_enabled: Option<bool>,
) -> Result<Value> {
    set(
        path,
        Changes {
            mcp_enabled,
            ai_features_enabled,
            ..Changes::default()
        },
    )
}

/// `require_mcp_access(path)`: the typed refusal while agents are off;
/// otherwise the recorded choices, with the user's grant.
pub fn require_mcp_access(path: &Path) -> Result<Access> {
    let access = read(path);
    match access.reason() {
        None => Ok(access),
        Some("ai_features_disabled") => Err(AppError::new(
            "ai_features_disabled",
            "AI features are turned off in Kassiber",
        )
        .with_hint("Ask the user to turn on AI features in Kassiber Settings > AI.")),
        Some(_) => Err(AppError::new(
            "mcp_disabled",
            "External agents are not enabled for Kassiber",
        )
        .with_hint(
            "Ask the user to turn on External agents in Kassiber Settings > AI, or to run `kassiber mcp enable` in their own terminal.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(dir: &tempfile::TempDir, bytes: &[u8]) -> PathBuf {
        let path = dir.path().join(FILENAME);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn changes(mcp: Option<bool>, ai: Option<bool>) -> Changes {
        Changes {
            mcp_enabled: mcp,
            ai_features_enabled: ai,
            ..Changes::default()
        }
    }

    fn on(mcp: bool, ai: Option<bool>) -> Access {
        Access {
            mcp_enabled: mcp,
            ai_features_enabled: ai,
            ..Access::default()
        }
    }

    #[test]
    fn agent_access_parsing_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(&dir.path().join("missing.json")), Access::default());
        for (bytes, expected) in [
            (
                &br#"{"schema_version": 1, "mcp_enabled": true}"#[..],
                on(true, None),
            ),
            (
                br#"{"schema_version": 1, "mcp_enabled": true, "ai_features_enabled": false}"#,
                on(true, Some(false)),
            ),
            (
                br#"{"schema_version": 1, "mcp_enabled": 1, "ai_features_enabled": 0}"#,
                on(false, None),
            ),
            // The schema must be exactly the integer 1.
            (
                br#"{"schema_version": true, "mcp_enabled": true}"#,
                on(false, None),
            ),
            (
                br#"{"schema_version": 1.0, "mcp_enabled": true}"#,
                on(false, None),
            ),
            (
                br#"{"schema_version": 2, "mcp_enabled": true}"#,
                on(false, None),
            ),
            (
                br#"{"schema_version": "1", "mcp_enabled": true}"#,
                on(false, None),
            ),
            (br#"{"mcp_enabled": true}"#, on(false, None)),
            (br#"[1]"#, on(false, None)),
            (
                b"\xef\xbb\xbf{\"schema_version\": 1, \"mcp_enabled\": true}",
                on(false, None),
            ),
            (
                b"{\"schema_version\": 1, \"mcp_enabled\": true, \"x\": \"\xff\"}",
                on(false, None),
            ),
            (b"not json", on(false, None)),
            // Duplicate keys: the last one wins, as in json.loads.
            (
                br#"{"schema_version": 1, "mcp_enabled": true, "mcp_enabled": false}"#,
                on(false, None),
            ),
        ] {
            assert_eq!(read(&file(&dir, bytes)), expected, "{bytes:?}");
        }
        // At most 1,024 bytes.
        let pad = |n: usize| {
            let head = br#"{"schema_version": 1, "mcp_enabled": true, "p": ""#;
            let mut bytes = head.to_vec();
            bytes.resize(n - 2, b'x');
            bytes.extend_from_slice(b"\"}");
            bytes
        };
        assert_eq!(read(&file(&dir, &pad(1024))), on(true, None));
        assert_eq!(read(&file(&dir, &pad(1025))), on(false, None));
        // Symlinks and directories read as off.
        #[cfg(unix)]
        {
            let target = file(&dir, br#"{"schema_version": 1, "mcp_enabled": true}"#);
            let link = dir.path().join("link.json");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert_eq!(read(&link), on(false, None));
        }
        assert_eq!(read(dir.path()), on(false, None));
    }

    #[test]
    fn agent_access_reason_precedence() {
        assert_eq!(on(false, Some(false)).reason(), Some("mcp_disabled"));
        assert_eq!(on(false, None).reason(), Some("mcp_disabled"));
        assert_eq!(on(true, Some(false)).reason(), Some("ai_features_disabled"));
        assert_eq!(on(true, None).reason(), None);
        assert_eq!(on(true, Some(true)).reason(), None);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILENAME);
        assert_eq!(require_mcp_access(&path).unwrap_err().code, "mcp_disabled");
        set(&path, changes(Some(true), Some(false))).unwrap();
        let refused = require_mcp_access(&path).unwrap_err();
        assert_eq!(refused.code, "ai_features_disabled");
        assert_eq!(
            refused.hint.as_deref(),
            Some("Ask the user to turn on AI features in Kassiber Settings > AI.")
        );
        set(&path, changes(None, Some(true))).unwrap();
        require_mcp_access(&path).unwrap();
    }

    #[test]
    fn agent_access_writes_sorted_keys_and_keeps_the_other_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(FILENAME);
        let first = set(&path, changes(Some(true), None)).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{\"ai_features_enabled\": null, \"mcp_enabled\": true, \"schema_version\": 1}\n"
        );
        assert_eq!(
            kassiber_envelope::daemon_line(&first).unwrap(),
            "{\"mcp_enabled\":true,\"ai_features_enabled\":null,\"mcp_available\":true,\"reason\":null,\"grant\":\"read\",\"agent_access\":\"restricted\",\"access_recorded\":false}\n"
        );
        set(&path, changes(None, Some(false))).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{\"ai_features_enabled\": false, \"mcp_enabled\": true, \"schema_version\": 1}\n"
        );
        set(&path, changes(Some(false), None)).unwrap();
        assert_eq!(read(&path), on(false, Some(false)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn grants_and_access_are_recorded_together_and_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILENAME);
        set(&path, changes(Some(true), None)).unwrap();
        // Choosing either records both, the other keeping its value.
        let status = set(
            &path,
            Changes {
                grant: Some(s("operator")),
                ..Changes::default()
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{\"agent_access\": \"restricted\", \"ai_features_enabled\": null, \"grant\": \"operator\", \"mcp_enabled\": true, \"schema_version\": 1}\n"
        );
        assert_eq!(get(&status, "access_recorded"), Some(&Value::Bool(true)));
        // A later switch keeps the recorded choice.
        set(&path, changes(Some(false), None)).unwrap();
        let access = read(&path);
        assert_eq!(
            (access.grant, access.agent_access),
            ("operator", "restricted")
        );
        // Admin is never grantable; unknown values are refused.
        for bad in ["admin", "write", ""] {
            let error = set(
                &path,
                Changes {
                    grant: Some(s(bad)),
                    ..Changes::default()
                },
            )
            .unwrap_err();
            assert_eq!(
                error.hint.as_deref(),
                Some("Admin work is never granted to agents.")
            );
        }
        assert!(set(
            &path,
            Changes {
                agent_access: Some(s("everything")),
                ..Changes::default()
            },
        )
        .is_err());
        // A file with an unknown grant reads as the defaults.
        let odd = file(
            &dir,
            br#"{"schema_version": 1, "mcp_enabled": true, "grant": "admin", "agent_access": "root"}"#,
        );
        let access = read(&odd);
        assert_eq!((access.grant, access.agent_access), ("read", "restricted"));
        assert!(access.access_recorded);
        assert!(grant_allows("accounting_decisions", "operator"));
        assert!(!grant_allows("read", "operator"));
        assert_eq!(narrower_grant("operator", "read"), "read");
    }
}
