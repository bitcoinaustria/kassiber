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
