//! The `status` payload, in `build_status_payload`'s key order.

use kassiber_envelope::{dict, s, set, SCHEMA_VERSION};
use kassiber_pyjson::Value;

use crate::store::{RuntimeConfig, StatusFacts};

fn text(value: &Option<String>) -> Value {
    value.as_deref().map_or(Value::Null, s)
}

/// `build_status_payload(conn, data_root)` plus the `default_backend` and
/// `env_file` both `cmd_status` and the daemon's `status` add from the
/// runtime configuration (`args.runtime_config` after `merge_db_backends`).
pub fn status_payload(version: &str, facts: &StatusFacts, config: &RuntimeConfig) -> Value {
    let paths = &facts.paths;
    let counts = facts.counts;
    dict([
        ("version", s(version)),
        ("schema_version", Value::from(SCHEMA_VERSION)),
        (
            "auth",
            dict([("mode", s("local")), ("authenticated", Value::Bool(true))]),
        ),
        ("database_encrypted", Value::Bool(facts.database_encrypted)),
        ("state_root", s(paths.state_root.as_str())),
        ("data_root", s(paths.data_root.as_str())),
        ("database", s(paths.database.as_str())),
        ("project_id", text(&facts.project_id)),
        ("project_name", text(&facts.project_name)),
        ("project_root", text(&facts.project_root)),
        ("config_root", s(paths.config_root.as_str())),
        ("settings_file", s(paths.settings_file.as_str())),
        ("exports_root", s(paths.exports_root.as_str())),
        ("attachments_root", s(paths.attachments_root.as_str())),
        (
            "current_workspace",
            s(facts.context.workspace_label.as_str()),
        ),
        ("current_profile", s(facts.context.profile_label.as_str())),
        ("workspaces", Value::from(counts.workspaces)),
        ("profiles", Value::from(counts.profiles)),
        ("accounts", Value::from(counts.accounts)),
        ("wallets", Value::from(counts.wallets)),
        ("transactions", Value::from(counts.transactions)),
        ("journal_entries", Value::from(counts.journal_entries)),
        ("quarantines", Value::from(counts.quarantines)),
        ("default_backend", text(&config.default_backend)),
        ("env_file", s(config.env_file.as_str())),
    ])
}

/// What the open book occupies on disk, for the daemon's `status` with
/// `args.storage`: the database file and its write-ahead log, in bytes, each
/// `null` when the file is absent or unreadable. Local metadata reads only.
/// Opt-in so the default `status` payload, which recorded sessions pin byte
/// for byte, stays unchanged.
pub fn storage_payload(database: &str) -> Value {
    let size = |path: &str| {
        std::fs::metadata(path)
            .ok()
            .filter(std::fs::Metadata::is_file)
            .map_or(Value::Null, |meta| Value::from(meta.len()))
    };
    dict([
        ("database_bytes", size(database)),
        ("wal_bytes", size(&format!("{database}-wal"))),
    ])
}

/// The daemon's `_status_payload`: a daemon started for a catalog project
/// reports that project's id and root over the data root's metadata.
pub fn daemon_status_payload(
    version: &str,
    facts: &StatusFacts,
    config: &RuntimeConfig,
    project: Option<(&str, Option<&str>)>,
) -> Value {
    let mut payload = status_payload(version, facts, config);
    if let Some((project_id, project_root)) = project {
        set(&mut payload, "project_id", s(project_id));
        set(
            &mut payload,
            "project_root",
            project_root.map_or(Value::Null, s),
        );
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_envelope::get;

    #[test]
    fn storage_reports_file_sizes_and_null_for_missing_files() {
        let dir = std::env::temp_dir().join(format!(
            "kassiber-status-storage-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let database = dir.join("kassiber.sqlite3");
        std::fs::write(&database, vec![0u8; 4096]).unwrap();
        let database = database.to_str().unwrap();

        let payload = storage_payload(database);
        assert_eq!(get(&payload, "database_bytes"), Some(&Value::from(4096u64)));
        assert_eq!(get(&payload, "wal_bytes"), Some(&Value::Null));

        std::fs::write(format!("{database}-wal"), vec![0u8; 32]).unwrap();
        let payload = storage_payload(database);
        assert_eq!(get(&payload, "wal_bytes"), Some(&Value::from(32u64)));

        // A directory where the database should be is not a size.
        let missing = storage_payload(dir.to_str().unwrap());
        assert_eq!(get(&missing, "database_bytes"), Some(&Value::Null));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
