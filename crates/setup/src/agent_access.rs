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

/// The two recorded choices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Access {
    pub mcp_enabled: bool,
    /// The desktop's AI master switch; `None` when it never recorded one.
    pub ai_features_enabled: Option<bool>,
}

/// `_read(path)`: a regular file of at most 1,024 bytes holding a JSON
/// object whose `schema_version` is exactly the integer 1 (not `true`, not
/// `1.0`); `mcp_enabled` counts only as the literal `true`.
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
        ])
    }
}

/// `agent_access_status(path)`.
pub fn status(path: &Path) -> Value {
    read(path).status()
}

/// `set_agent_access(...)`: update either field, keeping the other, and
/// write `json.dumps(document, sort_keys=True)` with a final newline.
pub fn set(
    path: &Path,
    mcp_enabled: Option<bool>,
    ai_features_enabled: Option<bool>,
) -> Result<Value> {
    let current = read(path);
    let next = Access {
        mcp_enabled: mcp_enabled.unwrap_or(current.mcp_enabled),
        ai_features_enabled: ai_features_enabled.or(current.ai_features_enabled),
    };
    let flag = |value: Option<bool>| match value {
        None => "null",
        Some(true) => "true",
        Some(false) => "false",
    };
    let document = format!(
        "{{\"ai_features_enabled\": {}, \"mcp_enabled\": {}, \"schema_version\": {SCHEMA_VERSION}}}\n",
        flag(next.ai_features_enabled),
        flag(Some(next.mcp_enabled)),
    );
    kassiber_store::layout::write_private_preference(path, document.as_bytes())
        .map_err(|e| projects::io_error(e, path))?;
    Ok(status(path))
}

/// `require_mcp_access(path)`: the typed refusal while agents are off.
pub fn require_mcp_access(path: &Path) -> Result<()> {
    match read(path).reason() {
        None => Ok(()),
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

    fn on(mcp: bool, ai: Option<bool>) -> Access {
        Access {
            mcp_enabled: mcp,
            ai_features_enabled: ai,
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
        set(&path, Some(true), Some(false)).unwrap();
        let refused = require_mcp_access(&path).unwrap_err();
        assert_eq!(refused.code, "ai_features_disabled");
        assert_eq!(
            refused.hint.as_deref(),
            Some("Ask the user to turn on AI features in Kassiber Settings > AI.")
        );
        set(&path, None, Some(true)).unwrap();
        require_mcp_access(&path).unwrap();
    }

    #[test]
    fn agent_access_writes_sorted_keys_and_keeps_the_other_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(FILENAME);
        let first = set(&path, Some(true), None).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{\"ai_features_enabled\": null, \"mcp_enabled\": true, \"schema_version\": 1}\n"
        );
        assert_eq!(
            kassiber_envelope::daemon_line(&first).unwrap(),
            "{\"mcp_enabled\":true,\"ai_features_enabled\":null,\"mcp_available\":true,\"reason\":null}\n"
        );
        set(&path, None, Some(false)).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{\"ai_features_enabled\": false, \"mcp_enabled\": true, \"schema_version\": 1}\n"
        );
        set(&path, Some(false), None).unwrap();
        assert_eq!(read(&path), on(false, Some(false)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
