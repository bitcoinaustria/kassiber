//! Ports of the storage classes of `tests/test_ai_core.py`
//! (`ProvidersCrudTest`), `tests/test_ai_core_api_format.py`
//! (`ProviderFormatStorageTest`) and `tests/test_ai_core_provider_settings.py`
//! (`ProviderSettingsStorageTest`, `ProviderKindInferenceTest`). The
//! "older book gains columns" cases belong to the store's migrations.

use super::*;
use crate::pyval::field;

pub(crate) struct Book {
    pub conn: Connection,
    pub root: std::path::PathBuf,
}

impl Drop for Book {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A disposable book file under the test temp directory, with Kassiber's
/// creation schema.
pub(crate) fn book() -> Book {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "kassiber-ai-providers-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let conn = Connection::open(data.join("kassiber.sqlite3")).unwrap();
    // Disposable books: no fsync per statement on a shared build disk.
    conn.execute_batch("PRAGMA synchronous = OFF; PRAGMA journal_mode = MEMORY; BEGIN")
        .unwrap();
    let schema =
        kassiber_pytext::loads(include_str!("../../../store/fixtures/creation.json")).unwrap();
    let Some(Value::List(statements)) = kassiber_envelope::get(&schema, "statements") else {
        panic!()
    };
    for statement in statements {
        conn.execute_batch(&kassiber_envelope::py_str(statement))
            .unwrap();
    }
    conn.execute_batch("COMMIT").unwrap();
    Book { conn, root }
}

pub(crate) fn text(value: &Value, key: &str) -> Option<String> {
    crate::pyval::string(field(value, key))
}

fn create_simple(
    conn: &Connection,
    name: &str,
    base: &str,
    kind: Option<&str>,
) -> Result<Value, AppError> {
    create(
        conn,
        Create {
            name: name.into(),
            base_url: base.into(),
            kind: opt(kind),
            enabled: true,
            ..Default::default()
        },
    )
}

fn update_with(
    conn: &Connection,
    name: &str,
    set: impl FnOnce(&mut Update),
) -> Result<Value, AppError> {
    let mut updates = Update::default();
    set(&mut updates);
    update(conn, name, updates)
}

fn names(conn: &Connection) -> Vec<String> {
    list(conn)
        .unwrap()
        .iter()
        .map(|p| text(p, "name").unwrap())
        .collect()
}

#[test]
fn seed_inserts_local_providers_when_empty() {
    let book = book();
    let providers = list(&book.conn).unwrap();
    let by_name = |name: &str| {
        providers
            .iter()
            .find(|p| text(p, "name").as_deref() == Some(name))
            .unwrap()
            .clone()
    };
    let mut seen = names(&book.conn);
    seen.sort();
    assert_eq!(
        seen,
        ["claude", "codex", "copilot", "ollama", "omlx", "opencode"]
    );
    assert_eq!(text(&by_name("ollama"), "kind").unwrap(), "local");
    assert_eq!(
        text(&by_name("ollama"), "base_url").unwrap(),
        "http://localhost:11434/v1"
    );
    assert_eq!(
        text(&by_name("omlx"), "base_url").unwrap(),
        "http://127.0.0.1:8000/v1"
    );
    assert_eq!(
        text(&by_name("copilot"), "base_url").unwrap(),
        "copilot-cli://default"
    );
    assert_eq!(text(&by_name("copilot"), "kind").unwrap(), "remote");
    assert_eq!(*field(&by_name("codex"), "acknowledged_at"), Value::Null);
    assert!(text(&by_name("ollama"), "acknowledged_at").is_some());
    assert_eq!(default_name(&book.conn).unwrap().as_deref(), Some("ollama"));
}

#[test]
fn acp_agents_are_seeded_once_into_books_that_predate_them() {
    let book = book();
    list(&book.conn).unwrap();
    book.conn
        .execute_batch(
            "DELETE FROM ai_providers WHERE base_url = 'copilot-cli://default'; DELETE FROM settings WHERE key = 'ai_acp_broker_providers_seeded';",
        )
        .unwrap();
    assert!(names(&book.conn).contains(&"copilot".into()));
    book.conn
        .execute("DELETE FROM ai_providers WHERE name = 'copilot'", [])
        .unwrap();
    let after = names(&book.conn);
    assert!(!after.contains(&"copilot".into()));
    assert!(after.contains(&"codex".into()));
}

#[test]
fn acp_agent_can_be_acknowledged_and_chosen_first_in_an_old_book() {
    let book = book();
    list(&book.conn).unwrap();
    book.conn
        .execute_batch(
            "DELETE FROM ai_providers WHERE base_url = 'copilot-cli://default'; DELETE FROM settings WHERE key = 'ai_acp_broker_providers_seeded';",
        )
        .unwrap();
    assert!(text(
        &acknowledge(&book.conn, "copilot").unwrap(),
        "acknowledged_at"
    )
    .is_some());
    set_default(&book.conn, "copilot").unwrap();
    assert_eq!(
        text(&resolve(&book.conn, None).unwrap(), "name").unwrap(),
        "copilot"
    );
}

#[test]
fn acp_agent_is_seeded_when_a_user_row_holds_its_name() {
    let book = book();
    list(&book.conn).unwrap();
    book.conn
        .execute_batch(
            "UPDATE ai_providers SET base_url = 'https://models.example.test/v1' WHERE name = 'copilot'; DELETE FROM settings WHERE key = 'ai_acp_broker_providers_seeded';",
        )
        .unwrap();
    let rows = list(&book.conn).unwrap();
    let name_at = |url: &str| {
        rows.iter()
            .find(|p| text(p, "base_url").as_deref() == Some(url))
            .and_then(|p| text(p, "name"))
    };
    assert_eq!(
        name_at("https://models.example.test/v1").unwrap(),
        "copilot"
    );
    assert_eq!(name_at("copilot-cli://default").unwrap(), "copilot-2");
}

#[test]
fn seed_does_not_recreate_after_delete() {
    let book = book();
    seed_default_if_empty(&book.conn).unwrap();
    clear_default(&book.conn).unwrap();
    delete(&book.conn, "ollama").unwrap();
    seed_default_if_empty(&book.conn).unwrap();
    assert!(!names(&book.conn).contains(&"ollama".into()));
}

#[test]
fn create_and_get_with_remote_kind() {
    let book = book();
    let created = create(
        &book.conn,
        Create {
            name: "OpenAI".into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key: Some("sk-secret".into()),
            default_model: s("gpt-4o-mini"),
            kind: s("remote"),
            notes: s("Cloud OpenAI."),
            enabled: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(text(&created, "name").unwrap(), "openai");
    assert_eq!(text(&created, "display_name").unwrap(), "OpenAI");
    assert_eq!(*field(&created, "acknowledged_at"), Value::Null);
    let fetched = get(&book.conn, "openai").unwrap();
    assert_eq!(text(&fetched, "api_key").unwrap(), "sk-secret");
    let redacted = redact_for_output(&fetched, Some("ollama"));
    assert!(kassiber_envelope::get(&redacted, "api_key").is_none());
    assert_eq!(*field(&redacted, "has_api_key"), Value::Bool(true));
    assert_eq!(
        *field(&redacted, "secret_ref"),
        dict([("store_id", s("sqlcipher_inline")), ("state", s("ok"))])
    );
    assert_eq!(*field(&redacted, "is_default"), Value::Bool(false));
    assert_eq!(
        *field(&redacted, "supports_reasoning_effort"),
        Value::Bool(false)
    );
    assert!(!kassiber_pyjson::dumps(&redacted, &Default::default())
        .unwrap()
        .contains("sk-secret"));
    assert_eq!(
        require_acknowledged(&fetched).unwrap_err().code,
        "ai_remote_ack_required"
    );
    let acknowledged = update_with(&book.conn, "openai", |u| u.acknowledged = Some(true)).unwrap();
    require_acknowledged(&acknowledged).unwrap();
}

#[test]
fn update_clears_and_sets_fields_and_retargeting_drops_the_key() {
    let book = book();
    create(
        &book.conn,
        Create {
            name: "openrouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            api_key: Some("sk-old".into()),
            kind: s("remote"),
            enabled: true,
            ..Default::default()
        },
    )
    .unwrap();
    let updated = update_with(&book.conn, "openrouter", |u| {
        u.api_key = s("sk-new");
        u.default_model = s("anthropic/claude-3.5-sonnet");
    })
    .unwrap();
    assert_eq!(text(&updated, "api_key").unwrap(), "sk-new");
    assert_eq!(text(field(&updated, "secret_ref"), "state").unwrap(), "ok");
    let cleared = update_with(&book.conn, "openrouter", |u| {
        u.clear = vec!["api_key".into(), "default_model".into()];
    })
    .unwrap();
    assert_eq!(*field(&cleared, "api_key"), Value::Null);
    assert_eq!(
        text(field(&cleared, "secret_ref"), "state").unwrap(),
        "missing"
    );
    assert_eq!(*field(&cleared, "default_model"), Value::Null);

    update_with(&book.conn, "openrouter", |u| u.api_key = s("sk-old")).unwrap();
    // A path change on the same origin keeps the key.
    let same = update_with(&book.conn, "openrouter", |u| {
        u.base_url = s("https://OPENROUTER.ai:443/other/v1")
    })
    .unwrap();
    assert_eq!(text(&same, "api_key").unwrap(), "sk-old");
    let moved = update_with(&book.conn, "openrouter", |u| {
        u.base_url = s("https://example.invalid/v1")
    })
    .unwrap();
    assert_eq!(*field(&moved, "api_key"), Value::Null);
    assert_eq!(
        text(field(&moved, "secret_ref"), "state").unwrap(),
        "missing"
    );
    assert_eq!(
        text(field(&moved, "secret_ref"), "account").unwrap(),
        "openrouter"
    );
    let rotated = update_with(&book.conn, "openrouter", |u| {
        u.base_url = s("https://api.openai.com/v1");
        u.api_key = s("sk-new");
    })
    .unwrap();
    assert_eq!(text(&rotated, "api_key").unwrap(), "sk-new");
}

#[test]
fn narrow_set_api_key_and_native_refs() {
    let book = book();
    create_simple(
        &book.conn,
        "openrouter",
        "https://openrouter.ai/api/v1",
        Some("remote"),
    )
    .unwrap();
    let updated = set_api_key(&book.conn, "openrouter", Some("sk-rotated")).unwrap();
    assert_eq!(
        api_key_for_use(&updated, None, None).unwrap().as_deref(),
        Some("sk-rotated")
    );
    let redacted = redact_for_output(&updated, None);
    assert!(kassiber_envelope::get(&redacted, "is_default").is_none());
    assert_eq!(
        *field(&redacted, "secret_ref"),
        dict([("store_id", s("sqlcipher_inline")), ("state", s("ok"))])
    );
    let cleared = set_api_key(&book.conn, "openrouter", None).unwrap();
    assert_eq!(api_key_for_use(&cleared, None, None).unwrap(), None);

    set_api_key(&book.conn, "openrouter", Some("sk-inline-before-move")).unwrap();
    let (service, account) = secret_ref_namespace(&book.conn, "openrouter").unwrap();
    assert_eq!(
        service,
        secret_service_id(
            &std::fs::canonicalize(book.root.join("data"))
                .unwrap()
                .to_string_lossy()
        )
    );
    let native = set_native_secret_ref(
        &book.conn,
        "openrouter",
        "macos_keychain",
        &service,
        &account,
        "ok",
    )
    .unwrap();
    assert_eq!(*field(&native, "api_key"), Value::Null);
    assert_eq!(
        text(field(&native, "secret_ref"), "store_id").unwrap(),
        "macos_keychain"
    );
    let redacted = redact_for_output(&native, None);
    assert_eq!(*field(&redacted, "has_api_key"), Value::Bool(true));
    assert_eq!(
        set_native_secret_ref(
            &book.conn,
            "openrouter",
            "macos_keychain",
            "elsewhere",
            &account,
            "ok"
        )
        .unwrap_err()
        .code,
        "validation"
    );

    // Resolved through the desktop, or persisted unavailable without it.
    let mut resolver = |reference: &Value| -> Result<Option<String>, AppError> {
        assert_eq!(text(reference, "provider_name").unwrap(), "openrouter");
        Ok(Some("sk-native".into()))
    };
    assert_eq!(
        api_key_for_use(&native, Some(&book.conn), Some(&mut resolver))
            .unwrap()
            .as_deref(),
        Some("sk-native")
    );
    let error = api_key_for_use(&native, Some(&book.conn), None).unwrap_err();
    assert_eq!(error.code, "secret_ref_unavailable");
    let refs = field(error.details.as_ref().unwrap(), "refs");
    let Value::List(refs) = refs else { panic!() };
    assert_eq!(text(&refs[0], "state").unwrap(), "unavailable");
    let state: String = book
        .conn
        .query_row(
            "SELECT state FROM ai_provider_secret_refs WHERE provider_name = 'openrouter'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "unavailable");
    let error = api_key_for_use(&get(&book.conn, "openrouter").unwrap(), None, None).unwrap_err();
    assert_eq!(
        error.message,
        "AI provider 'openrouter' secret is not available"
    );

    // A ref outside this data root's namespace never resolves.
    book.conn
        .execute(
            "UPDATE ai_provider_secret_refs SET service = 'foreign', state = 'ok'",
            [],
        )
        .unwrap();
    let foreign = get(&book.conn, "openrouter").unwrap();
    assert_eq!(
        text(field(&foreign, "secret_ref"), "state").unwrap(),
        "unavailable"
    );
    assert_eq!(
        text(field(&foreign, "secret_ref"), "service").unwrap(),
        service
    );
}

#[test]
fn default_pointer_roundtrip_and_deletion_guards() {
    let book = book();
    assert_eq!(delete(&book.conn, "ollama").unwrap_err().code, "conflict");
    create_simple(
        &book.conn,
        "remote-1",
        "https://example.test/v1",
        Some("remote"),
    )
    .unwrap();
    set_default(&book.conn, "remote-1").unwrap();
    assert_eq!(
        text(&resolve(&book.conn, None).unwrap(), "name").unwrap(),
        "remote-1"
    );
    assert_eq!(
        clear_default(&book.conn).unwrap(),
        dict([("default", Value::Null), ("cleared", Value::Bool(true))])
    );
    assert_eq!(
        resolve(&book.conn, None).unwrap_err().code,
        "ai_provider_not_configured"
    );
    let payload = list_with_default(&book.conn).unwrap();
    assert_eq!(*field(&payload, "default"), Value::Null);
    assert_eq!(
        delete(&book.conn, "remote-1").unwrap(),
        dict([("name", s("remote-1")), ("deleted", Value::Bool(true))])
    );
    assert_eq!(get(&book.conn, "remote-1").unwrap_err().code, "not_found");
}

#[test]
fn list_with_default_payload_shape() {
    let book = book();
    let payload = list_with_default(&book.conn).unwrap();
    assert_eq!(text(&payload, "default").unwrap(), "ollama");
    let Value::List(rows) = field(&payload, "providers") else {
        panic!()
    };
    for row in rows {
        assert!(kassiber_envelope::get(row, "api_key").is_none());
        let keys: Vec<String> = crate::pyval::entries(row).map(|(k, _)| k).collect();
        assert_eq!(
            keys,
            [
                "name",
                "display_name",
                "base_url",
                "default_model",
                "kind",
                "notes",
                "acknowledged_at",
                "enabled",
                "default_reasoning_effort",
                "api_format",
                "detected_api_format",
                "omit_store_field",
                "agent_access",
                "created_at",
                "updated_at",
                "has_api_key",
                "secret_ref",
                "supports_reasoning_effort",
                "effective_api_format",
                "is_default",
                "last_checked_at",
                "set_up"
            ]
        );
        let is_default = text(row, "name").as_deref() == Some("ollama");
        assert_eq!(*field(row, "is_default"), Value::Bool(is_default));
        // Untouched built-ins fold away until they are the default.
        assert_eq!(*field(row, "set_up"), Value::Bool(is_default));
        assert_eq!(*field(row, "agent_access"), s("full"));
    }
}

fn snapshot(value: Value, stale: bool) -> crate::discovery::Snapshot {
    crate::discovery::Snapshot {
        value,
        checked_at: "2026-11-05T12:00:00Z".into(),
        stale,
        error: stale.then(|| dict([("code", s("ai_unavailable")), ("message", s("down"))])),
    }
}

fn row<'a>(payload: &'a Value, name: &str) -> &'a Value {
    let Value::List(rows) = field(payload, "providers") else {
        panic!()
    };
    rows.iter()
        .find(|row| text(row, "name").as_deref() == Some(name))
        .unwrap()
}

#[test]
fn checks_are_bound_to_the_endpoint_and_capped() {
    let book = book();
    let conn = &book.conn;
    // Checks belong to provider rows (a foreign key): seed them first.
    ensure_seeded(conn).unwrap();
    let url = "http://localhost:11434/v1";
    let models = Value::List((0..1005).map(Value::from).collect());
    checks::save(
        conn,
        "omlx",
        checks::CHECK_KIND_MODELS,
        url,
        &snapshot(models, true),
    )
    .unwrap();
    let loaded = checks::load(conn, "omlx", checks::CHECK_KIND_MODELS, url)
        .unwrap()
        .unwrap();
    assert!(matches!(&loaded.value, Value::List(items) if items.len() == checks::MAX_SAVED_MODELS));
    assert!(loaded.stale);
    assert_eq!(loaded.error, snapshot(Value::Null, true).error);
    // Another address, another kind, or another provider: nothing.
    assert!(checks::load(
        conn,
        "omlx",
        checks::CHECK_KIND_MODELS,
        "http://127.0.0.1:8000/v1"
    )
    .unwrap()
    .is_none());
    assert!(checks::load(conn, "omlx", checks::CHECK_KIND_RUNTIME, url)
        .unwrap()
        .is_none());
    // A runtime check caps each status's models.
    let status = dict([
        ("id", s("codex")),
        ("models", Value::List((0..1001).map(Value::from).collect())),
    ]);
    checks::save(
        conn,
        "codex",
        checks::CHECK_KIND_RUNTIME,
        "codex-cli://default",
        &snapshot(Value::List(vec![status, s("other")]), false),
    )
    .unwrap();
    let runtime = checks::load(
        conn,
        "codex",
        checks::CHECK_KIND_RUNTIME,
        "codex-cli://default",
    )
    .unwrap()
    .unwrap();
    let Value::List(statuses) = &runtime.value else {
        panic!()
    };
    assert!(matches!(field(&statuses[0], "models"), Value::List(m) if m.len() == 1000));
    assert_eq!(statuses[1], s("other"));
    assert_eq!(runtime.error, None);
    // A malformed stored value reads as nothing.
    conn.execute(
        "UPDATE ai_provider_checks SET value_json = 'nope' WHERE provider_name = 'codex'",
        [],
    )
    .unwrap();
    assert!(checks::load(
        conn,
        "codex",
        checks::CHECK_KIND_RUNTIME,
        "codex-cli://default"
    )
    .unwrap()
    .is_none());
}

#[test]
fn a_check_marks_a_built_in_provider_set_up_until_its_address_changes() {
    let book = book();
    let conn = &book.conn;
    let listed = list_with_default(conn).unwrap();
    assert_eq!(*field(row(&listed, "omlx"), "last_checked_at"), Value::Null);
    assert_eq!(*field(row(&listed, "omlx"), "set_up"), Value::Bool(false));
    checks::save(
        conn,
        "omlx",
        checks::CHECK_KIND_MODELS,
        "http://127.0.0.1:8000/v1",
        &snapshot(Value::List(Vec::new()), false),
    )
    .unwrap();
    let listed = list_with_default(conn).unwrap();
    assert_eq!(
        *field(row(&listed, "omlx"), "last_checked_at"),
        s("2026-11-05T12:00:00Z")
    );
    assert_eq!(*field(row(&listed, "omlx"), "set_up"), Value::Bool(true));
    // Preference edits keep the check; a new address clears it.
    update_with(conn, "omlx", |u| u.agent_access = s("restricted")).unwrap();
    assert!(checks::load(
        conn,
        "omlx",
        checks::CHECK_KIND_MODELS,
        "http://127.0.0.1:8000/v1"
    )
    .unwrap()
    .is_some());
    // Same origin (the key stays), another path: the check still goes.
    update_with(conn, "omlx", |u| u.base_url = s("http://127.0.0.1:8000/v2")).unwrap();
    assert!(checks::load(
        conn,
        "omlx",
        checks::CHECK_KIND_MODELS,
        "http://127.0.0.1:8000/v1"
    )
    .unwrap()
    .is_none());
    let listed = list_with_default(conn).unwrap();
    assert_eq!(*field(row(&listed, "omlx"), "last_checked_at"), Value::Null);
    // Edited away from its built-in address, it counts as the user's own.
    assert_eq!(*field(row(&listed, "omlx"), "set_up"), Value::Bool(true));
    // An acknowledged off-device built-in counts; a local one needs more.
    acknowledge(conn, "codex").unwrap();
    let listed = list_with_default(conn).unwrap();
    assert_eq!(*field(row(&listed, "codex"), "set_up"), Value::Bool(true));
    assert_eq!(*field(row(&listed, "claude"), "set_up"), Value::Bool(false));
    // A new API key clears the check.
    checks::save(
        conn,
        "codex",
        checks::CHECK_KIND_RUNTIME,
        "codex-cli://default",
        &snapshot(Value::List(Vec::new()), false),
    )
    .unwrap();
    let created = create_simple(conn, "mine", "https://api.example/v1", None).unwrap();
    assert_eq!(*field(&created, "agent_access"), s("full"));
    checks::save(
        conn,
        "mine",
        checks::CHECK_KIND_MODELS,
        "https://api.example/v1",
        &snapshot(Value::List(Vec::new()), false),
    )
    .unwrap();
    update_with(conn, "mine", |u| u.api_key = s("sk-new")).unwrap();
    assert!(checks::load(
        conn,
        "mine",
        checks::CHECK_KIND_MODELS,
        "https://api.example/v1"
    )
    .unwrap()
    .is_none());
}

#[test]
fn agent_access_is_full_or_restricted_and_unknown_values_restrict() {
    let book = book();
    let conn = &book.conn;
    let restricted = create(
        conn,
        Create {
            name: "remote-r".into(),
            base_url: "https://api.example/v1".into(),
            agent_access: s("restricted"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(*field(&restricted, "agent_access"), s("restricted"));
    for bad in [s("everything"), Value::Null, Value::Bool(true)] {
        let error = create(
            conn,
            Create {
                name: "remote-x".into(),
                base_url: "https://api.example/v1".into(),
                agent_access: bad,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(
            error.message,
            "AI provider agent_access must be full or restricted"
        );
    }
    let error = update_with(conn, "remote-r", |u| u.agent_access = s("all")).unwrap_err();
    assert_eq!(error.code, "validation");
    // `agent_access` alone is a change.
    let full = update_with(conn, "remote-r", |u| u.agent_access = s("full")).unwrap();
    assert_eq!(*field(&full, "agent_access"), s("full"));
    conn.execute(
        "UPDATE ai_providers SET agent_access = 'root' WHERE name = 'remote-r'",
        [],
    )
    .unwrap();
    assert_eq!(
        *field(&get(conn, "remote-r").unwrap(), "agent_access"),
        s("restricted")
    );
    // An update that leaves it alone keeps the stored (restricted) reading.
    let kept = update_with(conn, "remote-r", |u| u.notes = s("n")).unwrap();
    assert_eq!(*field(&kept, "agent_access"), s("restricted"));
}

#[test]
fn kinds_and_locators_are_validated() {
    let book = book();
    for (name, base, kind) in [
        ("off-device", "https://api.example/v1", Some("local")),
        ("claude-cli", "claude-cli://default", Some("local")),
        ("x", "http://x/v1", Some("invalid")),
        ("y", "no-scheme", Some("local")),
        ("z", "ftp://host/v1", None),
    ] {
        assert_eq!(
            create_simple(&book.conn, name, base, kind)
                .unwrap_err()
                .code,
            "validation",
            "{base}"
        );
    }
    let created = create_simple(
        &book.conn,
        "codex-cli",
        "codex-cli://default",
        Some("remote"),
    )
    .unwrap();
    assert_eq!(*field(&created, "acknowledged_at"), Value::Null);
    assert_eq!(
        *field(
            &redact_for_output(&created, None),
            "supports_reasoning_effort"
        ),
        Value::Bool(true)
    );
    assert!(text(
        &create_simple(
            &book.conn,
            "local-ollama",
            "http://127.0.0.1:11434/v1",
            Some("local")
        )
        .unwrap(),
        "acknowledged_at"
    )
    .is_some());
    assert_eq!(
        create_simple(&book.conn, "codex-cli", "https://a.test/v1", None)
            .unwrap_err()
            .code,
        "conflict"
    );
}

#[test]
fn create_infers_local_or_remote_never_tee() {
    let book = book();
    for (index, (base, kind)) in [
        ("http://localhost:1234/v1", "local"),
        ("http://127.0.0.1:8080/v1", "local"),
        ("http://[::1]:8000/v1", "local"),
        ("http://192.168.1.20:11434/v1", "remote"),
        ("https://openrouter.ai/api/v1", "remote"),
        ("claude-cli://default", "remote"),
    ]
    .into_iter()
    .enumerate()
    {
        let row = create_simple(&book.conn, &format!("p{index}"), base, None).unwrap();
        assert_eq!(text(&row, "kind").unwrap(), kind, "{base}");
        assert_eq!(
            text(&row, "acknowledged_at").is_some(),
            kind == "local",
            "{base}"
        );
    }
}

#[test]
fn update_moves_local_off_device_only_with_a_new_url() {
    let book = book();
    create_simple(&book.conn, "lan", "http://127.0.0.1:1234/v1", None).unwrap();
    let row = update_with(&book.conn, "lan", |u| {
        u.base_url = s("http://localhost:4321/v1")
    })
    .unwrap();
    assert_eq!(text(&row, "kind").unwrap(), "local");
    let row = update_with(&book.conn, "lan", |u| {
        u.base_url = s("https://llm.example.test/v1")
    })
    .unwrap();
    assert_eq!(text(&row, "kind").unwrap(), "remote");
    assert_eq!(*field(&row, "acknowledged_at"), Value::Null);
    let row = update_with(&book.conn, "lan", |u| {
        u.base_url = s("http://127.0.0.1:1234/v1");
        u.acknowledged = Some(true);
    })
    .unwrap();
    assert_eq!(text(&row, "kind").unwrap(), "remote");
    assert!(text(&row, "acknowledged_at").is_some());

    create_simple(&book.conn, "lan2", "http://127.0.0.1:1235/v1", None).unwrap();
    let row = update_with(&book.conn, "lan2", |u| u.kind = s("tee")).unwrap();
    assert_eq!(*field(&row, "acknowledged_at"), Value::Null);
    create_simple(&book.conn, "lan3", "http://127.0.0.1:1236/v1", None).unwrap();
    let row = update_with(&book.conn, "lan3", |u| {
        u.kind = s("remote");
        u.acknowledged = Some(true);
    })
    .unwrap();
    assert!(text(&row, "acknowledged_at").is_some());
    let row = update_with(&book.conn, "lan3", |u| u.acknowledge_clear = Some(true)).unwrap();
    assert_eq!(*field(&row, "acknowledged_at"), Value::Null);
}

#[test]
fn update_requires_a_change_and_known_clear_fields() {
    let book = book();
    assert_eq!(
        update(&book.conn, "codex", Update::default())
            .unwrap_err()
            .code,
        "validation"
    );
    // The daemon and the CLI always pass `acknowledged`: a no-op update.
    update_with(&book.conn, "codex", |u| u.acknowledged = Some(false)).unwrap();
    let error = update_with(&book.conn, "codex", |u| {
        u.clear = vec!["kind".into(), "base_url".into()]
    })
    .unwrap_err();
    assert_eq!(
        error.message,
        "Unsupported AI provider clear field(s): base_url, kind"
    );
    assert_eq!(
        error.hint.as_deref(),
        Some("Clearable fields: api_key, default_model, default_reasoning_effort, detected_api_format, notes")
    );
    assert_eq!(
        update_with(&book.conn, "missing", |u| u.notes = s("x"))
            .unwrap_err()
            .code,
        "not_found"
    );
    assert_eq!(
        update_with(&book.conn, "codex", |u| u.enabled = s("yes"))
            .unwrap_err()
            .code,
        "validation"
    );
}

#[test]
fn enable_switch_and_default_effort_persist() {
    let book = book();
    for row in list(&book.conn).unwrap() {
        assert_eq!(*field(&row, "enabled"), Value::Bool(true));
        assert_eq!(*field(&row, "default_reasoning_effort"), Value::Null);
    }
    update_with(&book.conn, "claude", |u| {
        u.enabled = Value::Bool(false);
        u.default_reasoning_effort = s("HIGH");
    })
    .unwrap();
    let row = get(&book.conn, "claude").unwrap();
    assert_eq!(*field(&row, "enabled"), Value::Bool(false));
    assert_eq!(text(&row, "default_reasoning_effort").unwrap(), "high");
    update_with(&book.conn, "claude", |u| u.notes = s("edited")).unwrap();
    let row = get(&book.conn, "claude").unwrap();
    assert_eq!(*field(&row, "enabled"), Value::Bool(false));
    let row = update_with(&book.conn, "claude", |u| {
        u.enabled = Value::Bool(true);
        u.clear = vec!["default_reasoning_effort".into()];
    })
    .unwrap();
    assert_eq!(*field(&row, "enabled"), Value::Bool(true));
    assert_eq!(*field(&row, "default_reasoning_effort"), Value::Null);
    assert_eq!(
        update_with(&book.conn, "codex", |u| u.default_reasoning_effort =
            s("turbo"))
        .unwrap_err()
        .code,
        "validation"
    );
    set_default(&book.conn, "ollama").unwrap();
    assert_eq!(
        update_with(&book.conn, "ollama", |u| u.enabled = Value::Bool(false))
            .unwrap_err()
            .code,
        "conflict"
    );
    update_with(&book.conn, "codex", |u| u.enabled = Value::Bool(false)).unwrap();
    assert_eq!(
        set_default(&book.conn, "codex").unwrap_err().code,
        "conflict"
    );
    assert_eq!(
        require_contactable(&get(&book.conn, "codex").unwrap())
            .unwrap_err()
            .code,
        "ai_provider_disabled"
    );
}

#[test]
fn formats_start_auto_and_an_explicit_format_wins() {
    let book = book();
    create_simple(&book.conn, "lan", "http://127.0.0.1:1234/v1", None).unwrap();
    let payload = list_with_default(&book.conn).unwrap();
    let Value::List(rows) = field(&payload, "providers") else {
        panic!()
    };
    for row in rows {
        assert_eq!(text(row, "api_format").unwrap(), "auto");
        assert_eq!(*field(row, "detected_api_format"), Value::Null);
        let expected = if text(row, "base_url").unwrap().contains("-cli://") {
            Value::Null
        } else {
            s("responses")
        };
        assert_eq!(*field(row, "effective_api_format"), expected);
    }
    create(
        &book.conn,
        Create {
            name: "router".into(),
            base_url: "http://127.0.0.1:1234/v1".into(),
            api_format: s("chat-completions"),
            enabled: true,
            ..Default::default()
        },
    )
    .unwrap();
    let redacted = |name: &str| redact_for_output(&get(&book.conn, name).unwrap(), None);
    assert_eq!(
        text(&redacted("router"), "api_format").unwrap(),
        "chat_completions"
    );
    update_with(&book.conn, "router", |u| {
        u.api_format = s("auto");
        u.detected_api_format = s("chat_completions");
    })
    .unwrap();
    assert_eq!(
        text(&redacted("router"), "effective_api_format").unwrap(),
        "chat_completions"
    );
    update_with(&book.conn, "router", |u| u.api_format = s("responses")).unwrap();
    assert_eq!(
        text(&redacted("router"), "detected_api_format").unwrap(),
        "chat_completions"
    );
    assert_eq!(
        text(&redacted("router"), "effective_api_format").unwrap(),
        "responses"
    );
    for (format, detected) in [(s("completions"), Value::Null), (Value::Null, s("auto"))] {
        let error = create(
            &book.conn,
            Create {
                name: "x".into(),
                base_url: "http://127.0.0.1:9/v1".into(),
                api_format: format,
                detected_api_format: detected,
                enabled: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "validation");
    }
}

#[test]
fn cli_agents_take_no_format_and_drop_a_pinned_one_when_moved() {
    let book = book();
    let error = create(
        &book.conn,
        Create {
            name: "claude-work".into(),
            base_url: "claude-cli://default".into(),
            kind: s("remote"),
            api_format: s("responses"),
            enabled: true,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "validation");
    assert_eq!(
        update_with(&book.conn, "codex", |u| u.api_format =
            s("chat_completions"))
        .unwrap_err()
        .code,
        "validation"
    );
    assert_eq!(
        update_with(&book.conn, "codex", |u| u.detected_api_format =
            s("responses"))
        .unwrap_err()
        .code,
        "validation"
    );
    assert_eq!(
        update_with(&book.conn, "codex", |u| u.omit_store_field =
            Value::Bool(true))
        .unwrap_err()
        .code,
        "validation"
    );
    update_with(&book.conn, "codex", |u| u.api_format = s("auto")).unwrap();

    create(
        &book.conn,
        Create {
            name: "router".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            kind: s("remote"),
            api_format: s("responses"),
            detected_api_format: s("responses"),
            omit_store_field: true,
            enabled: true,
            ..Default::default()
        },
    )
    .unwrap();
    let row = update_with(&book.conn, "router", |u| {
        u.base_url = s("codex-cli://default")
    })
    .unwrap();
    assert_eq!(text(&row, "api_format").unwrap(), "auto");
    assert_eq!(*field(&row, "detected_api_format"), Value::Null);
    assert_eq!(*field(&row, "omit_store_field"), Value::Bool(false));
    update_with(&book.conn, "router", |u| {
        u.base_url = s("https://openrouter.ai/api/v1")
    })
    .unwrap();
    update_with(&book.conn, "router", |u| {
        u.api_format = s("chat_completions")
    })
    .unwrap();
    let error = update_with(&book.conn, "router", |u| {
        u.base_url = s("claude-cli://default");
        u.api_format = s("chat_completions");
    })
    .unwrap_err();
    assert_eq!(error.code, "validation");
    assert_eq!(
        text(&get(&book.conn, "router").unwrap(), "base_url").unwrap(),
        "https://openrouter.ai/api/v1"
    );
}

#[test]
fn a_new_endpoint_forgets_the_old_detection_and_records_only_its_own() {
    let book = book();
    create(
        &book.conn,
        Create {
            name: "lan".into(),
            base_url: "http://127.0.0.1:1234/v1".into(),
            detected_api_format: s("chat_completions"),
            enabled: true,
            ..Default::default()
        },
    )
    .unwrap();
    update_with(&book.conn, "lan", |u| u.notes = s("same endpoint")).unwrap();
    assert_eq!(
        text(&get(&book.conn, "lan").unwrap(), "detected_api_format").unwrap(),
        "chat_completions"
    );
    update_with(&book.conn, "lan", |u| {
        u.base_url = s("http://127.0.0.1:5678/v1")
    })
    .unwrap();
    assert_eq!(
        *field(&get(&book.conn, "lan").unwrap(), "detected_api_format"),
        Value::Null
    );
    update_with(&book.conn, "lan", |u| {
        u.base_url = s("http://127.0.0.1:9999/v1");
        u.detected_api_format = s("responses");
    })
    .unwrap();
    assert_eq!(
        text(&get(&book.conn, "lan").unwrap(), "detected_api_format").unwrap(),
        "responses"
    );
    update_with(&book.conn, "lan", |u| {
        u.clear = vec!["detected_api_format".into()]
    })
    .unwrap();
    let before = text(&get(&book.conn, "lan").unwrap(), "updated_at");
    assert!(!record_detected_api_format(
        &book.conn,
        "lan",
        "http://127.0.0.1:9/v1",
        "chat_completions"
    )
    .unwrap());
    assert!(record_detected_api_format(
        &book.conn,
        "lan",
        "http://127.0.0.1:9999/v1",
        "chat_completions"
    )
    .unwrap());
    assert!(record_detected_api_format(
        &book.conn,
        "lan",
        "http://127.0.0.1:9999/v1",
        "chat_completions"
    )
    .unwrap());
    let row = get(&book.conn, "lan").unwrap();
    assert_eq!(
        text(&row, "detected_api_format").unwrap(),
        "chat_completions"
    );
    assert_eq!(text(&row, "updated_at"), before);
    assert!(
        !record_detected_api_format(&book.conn, "codex", "codex-cli://default", "responses")
            .unwrap()
    );
}

#[test]
fn gates_refuse_in_python_order() {
    let book = book();
    let codex = get(&book.conn, "codex").unwrap();
    assert_eq!(
        require_contactable(&codex).unwrap_err().code,
        "ai_remote_ack_required"
    );
    require_endpoint(&codex, &Value::Null).unwrap();
    require_endpoint(&codex, &s("CODEX-CLI://default/")).unwrap();
    assert_eq!(
        require_endpoint(&codex, &s("claude-cli://default"))
            .unwrap_err()
            .code,
        "ai_provider_changed"
    );
    assert_eq!(
        require_endpoint(&codex, &Value::from(1)).unwrap_err().code,
        "validation"
    );
    require_test_target_acknowledged("http://127.0.0.1:1/v1", Some("local"), false, None).unwrap();
    let error = require_test_target_acknowledged("http://10.0.0.1/v1", Some("local"), false, None)
        .unwrap_err();
    assert_eq!(
        *error.details.as_ref().unwrap(),
        dict([("kind", s("local"))])
    );
    require_test_target_acknowledged("https://a.test/v1", None, true, None).unwrap();
    let error = require_test_target_acknowledged("https://a.test/v1", None, false, Some(&codex))
        .unwrap_err();
    assert_eq!(
        error.hint.as_deref(),
        Some("Tick the acknowledgement in the provider form, or acknowledge the saved provider with `kassiber ai providers update codex --acknowledge`.")
    );
    // Acknowledging a local row never covers an off-device test.
    let ollama = get(&book.conn, "ollama").unwrap();
    assert!(require_test_target_acknowledged(
        "https://a.test/v1",
        Some("remote"),
        false,
        Some(&ollama)
    )
    .is_err());
    acknowledge(&book.conn, "codex").unwrap();
    require_test_target_acknowledged(
        "codex-cli://default",
        Some("remote"),
        false,
        Some(&get(&book.conn, "codex").unwrap()),
    )
    .unwrap();
    assert_eq!(contactable_cli_runtimes(&book.conn).unwrap(), ["codex"]);
    acknowledge(&book.conn, "claude").unwrap();
    assert_eq!(
        contactable_cli_runtimes(&book.conn).unwrap(),
        ["claude", "codex"]
    );
}

#[test]
fn origins_and_url_helpers_match_python() {
    for (url, origin) in [
        ("https://Host:443/v1", "https://host"),
        ("https://host/openai/v1", "https://host"),
        ("http://host:80", "http://host"),
        ("http://user:pw@Host:8080/x", "http://host:8080"),
        ("http://[::1]:11434/v1", "http://[::1]:11434"),
        ("CODEX-CLI://default", "codex-cli://default"),
        ("http://host:99999/v1", "http://host:99999/v1"),
        ("http://host:abc/v1", "http://host:abc/v1"),
        ("", "://"),
    ] {
        assert_eq!(base_url_origin(&s(url)), origin, "{url}");
    }
    assert!(is_loopback_http_url("http://[::1]:8000/v1"));
    assert!(is_loopback_http_url("https://LOCALHOST/v1"));
    assert!(!is_loopback_http_url("http://localhost.example/v1"));
    assert!(!is_loopback_http_url("ftp://127.0.0.1/"));
    assert_eq!(
        normalize_base_url(&s("  http://h/v1///  ")).unwrap(),
        "http://h/v1"
    );
    assert_eq!(
        normalize_base_url(&s("Claude-CLI://default/")).unwrap(),
        "claude-cli://default"
    );
    for (effort, wire) in [
        ("LOW", Some("low")),
        (" ultra ", Some("high")),
        ("auto", None),
    ] {
        assert_eq!(http_reasoning_effort(&s(effort)), wire);
    }
}

#[test]
fn a_cli_proxy_is_off_device_and_takes_no_format_store_choice_or_locator() {
    let book = book();
    let conn = &book.conn;
    let proxy =
        create_simple(conn, "proxy", "http://127.0.0.1:8317/v1", Some("cli_proxy")).unwrap();
    assert_eq!(text(&proxy, "kind").as_deref(), Some("cli_proxy"));
    // Never acknowledged implicitly, even at a loopback address.
    assert_eq!(*field(&proxy, "acknowledged_at"), Value::Null);
    assert_eq!(
        require_contactable(&proxy).unwrap_err().code,
        "ai_remote_ack_required"
    );
    let shown = redact_for_output(&proxy, None);
    assert_eq!(
        *field(&shown, "supports_reasoning_effort"),
        Value::Bool(true)
    );
    assert_eq!(*field(&shown, "effective_api_format"), Value::Null);
    let refused = |request: Create| create(conn, request).unwrap_err().message;
    let base = || Create {
        name: "other".into(),
        base_url: "http://127.0.0.1:8317/v1".into(),
        kind: s("cli_proxy"),
        enabled: true,
        ..Default::default()
    };
    assert_eq!(
        refused(Create {
            api_format: s("responses"),
            ..base()
        }),
        "CLI proxy providers do not take an API format"
    );
    assert_eq!(
        refused(Create {
            detected_api_format: s("chat_completions"),
            ..base()
        }),
        "CLI proxy providers do not take an API format"
    );
    assert_eq!(
        refused(Create {
            omit_store_field: true,
            ..base()
        }),
        "CLI proxy providers do not take omit_store_field"
    );
    assert_eq!(
        refused(Create {
            base_url: "claude-cli://default".into(),
            ..base()
        }),
        "A CLI proxy provider needs the proxy's HTTP address"
    );
    // The same rules hold for an edit, against the merged row.
    let pinned = update_with(conn, "proxy", |u| u.api_format = s("responses")).unwrap_err();
    assert_eq!(
        pinned.message,
        "CLI proxy providers do not take an API format"
    );
    // A detection is never recorded onto a CLI proxy.
    assert!(!record_detected_api_format(
        conn,
        "proxy",
        "http://127.0.0.1:8317/v1",
        "chat_completions"
    )
    .unwrap());
    assert_eq!(
        *field(&get(conn, "proxy").unwrap(), "detected_api_format"),
        Value::Null
    );
}

#[test]
fn a_local_provider_becoming_a_cli_proxy_must_be_acknowledged_again() {
    let book = book();
    let conn = &book.conn;
    create_simple(conn, "box", "http://127.0.0.1:8317/v1", None).unwrap();
    assert!(truthy(field(&get(conn, "box").unwrap(), "acknowledged_at")));
    let moved = update_with(conn, "box", |u| u.kind = s("cli_proxy")).unwrap();
    assert_eq!(text(&moved, "kind").as_deref(), Some("cli_proxy"));
    assert_eq!(*field(&moved, "acknowledged_at"), Value::Null);
    let acknowledged = update_with(conn, "box", |u| u.acknowledged = Some(true)).unwrap();
    assert!(require_contactable(&acknowledged).is_ok());
    // A connection test of a saved, acknowledged CLI proxy needs no tick.
    assert!(require_test_target_acknowledged(
        "http://127.0.0.1:8317/v1",
        Some("cli_proxy"),
        false,
        Some(&acknowledged)
    )
    .is_ok());
    assert!(require_test_target_acknowledged(
        "http://127.0.0.1:8317/v1",
        Some("cli_proxy"),
        false,
        None
    )
    .is_err());
}
