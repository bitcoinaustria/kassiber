//! Freshness and maintenance settings for the desktop: `ui.freshness.status`,
//! `ui.freshness.configure`, `ui.maintenance.settings` and
//! `ui.maintenance.configure` (`daemon_freshness.py`).
//!
//! These read or write local policy only. None of them enqueues a job, runs
//! a source or opens a connection; saved policy is configuration, not consent.
use super::{field, or_text, put, ui, Queue};
use crate::publication::{
    row::{first, Result},
    wallet::SystemSources,
};
use kassiber_envelope::{dict, get, s, type_name, AppError};
use kassiber_pyjson::{Key, PyStr, Value};
use kassiber_store::rusqlite::Connection;

pub const KINDS: &[&str] = &[
    "ui.maintenance.settings",
    "ui.maintenance.configure",
    "ui.freshness.status",
    "ui.freshness.configure",
];

const FRESHNESS_ARGS: &[&str] = &[
    "auto_sync_before_report_reads",
    "background_enabled",
    "market_rate_provider",
    "report_read_sync",
    "source_classes",
];
const MAINTENANCE_ARGS: &[&str] = &[
    "auto_sync_before_report_reads",
    "background_enabled",
    "bitcoin_rail_carrying_value",
    "market_rate_provider",
    "report_read_sync",
    "require_coarse_review",
    "source_classes",
];
const PROFILE_FLAGS: &[&str] = &["require_coarse_review", "bitcoin_rail_carrying_value"];

/// Serves one settings kind. `args` is the daemon's coerced argument object;
/// the read kinds ignore it, as Python's handlers do.
pub fn execute(
    conn: &Connection,
    kind: &str,
    args: &Value,
    setup: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    match kind {
        "ui.freshness.status" => status(conn),
        "ui.freshness.configure" => configure(conn, args),
        "ui.maintenance.settings" => maintenance_settings(conn),
        "ui.maintenance.configure" => maintenance_configure(conn, args, setup),
        _ => Err(AppError::new(
            "not_implemented",
            format!("{kind} is not a freshness settings kind"),
        )),
    }
}

/// `_active_profile_row(conn)`.
fn active_profile(conn: &Connection) -> Result<Option<Value>> {
    let context = kassiber_setup::context::snapshot(conn)?;
    let id = or_text(field(&context, "profile_id"));
    if id.is_empty() {
        return Ok(None);
    }
    first(conn, "SELECT * FROM profiles WHERE id=?", [id])
}

fn profile_ref(profile: &Value) -> Value {
    dict([
        ("id", field(profile, "id").clone()),
        ("label", field(profile, "label").clone()),
    ])
}

fn snapshot(conn: &Connection, profile: &str) -> Result<Value> {
    let mut sources = SystemSources::default();
    Ok(ui::payload(
        &Queue::new(conn, &mut sources).snapshot(profile)?,
    ))
}

fn policy(conn: &Connection, profile: &str) -> Result<Value> {
    let mut sources = SystemSources::default();
    Queue::new(conn, &mut sources).policy(profile)
}

/// `core_freshness.default_policy().to_payload()`.
fn default_policy() -> Value {
    dict([
        ("background_enabled", Value::Bool(false)),
        (
            "source_classes",
            dict([
                ("btcpay_provenance", Value::Bool(false)),
                ("btcpay_wallet", Value::Bool(false)),
                ("journals", Value::Bool(true)),
                ("market_rates", Value::Bool(false)),
                ("onchain_wallet", Value::Bool(false)),
            ]),
        ),
        ("report_read_sync", Value::Bool(false)),
    ])
}

/// `_market_rate_provider_settings(conn, profile)`.
fn market_rate_settings(conn: &Connection, profile: Option<&Value>) -> Result<Vec<(Key, Value)>> {
    let pair = profile.and_then(|profile| {
        kassiber_rates::transaction_rate_pair("BTC", &or_text(field(profile, "fiat_currency")))
    });
    Ok(vec![
        (
            "market_rate_provider".into(),
            s(kassiber_rates::market_provider(conn)?),
        ),
        (
            "market_rate_providers".into(),
            Value::List(kassiber_rates::LIVE_SOURCES.iter().map(|v| s(*v)).collect()),
        ),
        ("active_rate_pair".into(), pair.map_or(Value::Null, s)),
    ])
}

fn extend(value: &mut Value, fields: Vec<(Key, Value)>) {
    for (key, item) in fields {
        if let Key::Str(name) = &key {
            if let Some(name) = name.as_str() {
                put(value, name, item);
            }
        }
    }
}

fn setting_key(profile: &str) -> Value {
    s(format!("freshness.policy.profile.{profile}"))
}

/// `sorted(set(raw_args) - allowed)` as the `unsupported arguments` refusal.
fn refuse_unknown(kind: &str, args: &Value, allowed: &[&str]) -> Result<()> {
    let Value::Dict(pairs) = args else {
        return Ok(());
    };
    let mut unknown: Vec<PyStr> = pairs
        .iter()
        .filter_map(|(key, _)| match key {
            Key::Str(key) if !allowed.iter().any(|name| key.as_str() == Some(name)) => {
                Some(key.clone())
            }
            _ => None,
        })
        .collect();
    unknown.sort();
    unknown.dedup();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(AppError::new(
        "validation",
        format!("{kind} received unsupported arguments"),
    )
    .with_details(dict([(
        "unknown",
        Value::List(unknown.into_iter().map(Value::Str).collect()),
    )])))
}

fn type_error(kind: &str, key: &str, expected: &str, value: &Value) -> AppError {
    AppError::new("validation", format!("{kind} {key} must be {expected}"))
        .with_details(dict([("type", s(type_name(value)))]))
}

fn present<'a>(args: &'a Value, key: &str) -> Option<&'a Value> {
    get(args, key).filter(|value| !matches!(value, Value::Null))
}

/// `_freshness_status_payload(conn)`.
pub fn status(conn: &Connection) -> Result<Value> {
    let Some(profile) = active_profile(conn)? else {
        return Ok(dict([
            ("profile", Value::Null),
            ("policy", default_policy()),
            ("sources", Value::List(vec![])),
            ("jobs", Value::List(vec![])),
        ]));
    };
    let mut payload = dict([("profile", profile_ref(&profile))]);
    if let Value::Dict(fields) = snapshot(conn, &or_text(field(&profile, "id")))? {
        extend(&mut payload, fields);
    }
    Ok(payload)
}

/// `_freshness_configure_payload(conn, raw_args)`; commits on success.
pub fn configure(conn: &Connection, args: &Value) -> Result<Value> {
    const KIND: &str = "ui.freshness.configure";
    refuse_unknown(KIND, args, FRESHNESS_ARGS)?;
    let profile = active_profile(conn)?.ok_or_else(|| {
        AppError::new(
            "validation",
            "ui.freshness.configure requires an active profile",
        )
    })?;
    let mut source_classes = present(args, "source_classes").cloned();
    if let Some(value) = source_classes.as_ref() {
        if !matches!(value, Value::Dict(_)) {
            return Err(type_error(KIND, "source_classes", "an object", value));
        }
    }
    let provider = present(args, "market_rate_provider");
    let provider = match provider {
        None => None,
        Some(Value::Str(text)) => Some(text),
        Some(value) => return Err(type_error(KIND, "market_rate_provider", "a string", value)),
    };
    let mut report_read_sync = get(args, "report_read_sync").cloned();
    if let Some(legacy) = present(args, "auto_sync_before_report_reads") {
        let Value::Bool(enabled) = legacy else {
            return Err(type_error(
                KIND,
                "auto_sync_before_report_reads",
                "a boolean",
                legacy,
            ));
        };
        report_read_sync = Some(legacy.clone());
        source_classes = Some(dict([
            ("onchain_wallet", Value::Bool(*enabled)),
            ("btcpay_wallet", Value::Bool(*enabled)),
            ("btcpay_provenance", Value::Bool(*enabled)),
        ]));
    }
    let profile_id = or_text(field(&profile, "id"));
    let tx = conn
        .unchecked_transaction()
        .map_err(crate::database_error)?;
    if let Some(provider) = provider {
        let text = provider
            .as_str()
            .ok_or_else(|| unsupported_provider(provider))?;
        kassiber_rates::set_market_provider(&tx, text)?;
    }
    let mut input = dict::<&str>([]);
    if let Some(value) = get(args, "background_enabled") {
        put(&mut input, "background_enabled", value.clone());
    }
    if let Some(value) = report_read_sync {
        put(&mut input, "report_read_sync", value);
    }
    if let Some(value) = source_classes {
        put(&mut input, "source_classes", value);
    }
    let mut sources = SystemSources::default();
    let policy = Queue::new(&tx, &mut sources).set_policy(&profile_id, &input)?;
    tx.commit().map_err(crate::database_error)?;
    let mut settings = policy.clone();
    extend(&mut settings, market_rate_settings(conn, Some(&profile))?);
    put(
        &mut settings,
        "auto_sync_before_report_reads",
        field(&policy, "report_read_sync").clone(),
    );
    put(&mut settings, "setting_key", setting_key(&profile_id));
    Ok(dict([
        ("profile", profile_ref(&profile)),
        ("settings", settings),
    ]))
}

/// `normalize_market_rate_provider` for text Rust cannot hold as UTF-8: such
/// text is never a supported provider, so only the refusal is reachable.
fn unsupported_provider(provider: &PyStr) -> AppError {
    let message = PyStr::from_code_points(
        "Market-rate provider '"
            .chars()
            .map(u32::from)
            .chain(provider.code_points())
            .chain(
                "' is not supported for automatic refresh"
                    .chars()
                    .map(u32::from),
            ),
    )
    .unwrap_or_else(|_| "Market-rate provider is not supported for automatic refresh".into());
    AppError::new("validation", "")
        .with_python_message(message)
        .with_hint(
            "Supported automatic market-rate providers: coinbase-exchange, coingecko, mempool",
        )
}

/// `_maintenance_settings_payload(conn)`.
pub fn maintenance_settings(conn: &Connection) -> Result<Value> {
    let context = kassiber_setup::context::snapshot(conn)?;
    let workspace = match field(&context, "workspace_label") {
        Value::Str(label) if label.code_points().next().is_some() => Value::Str(label.clone()),
        _ => Value::Null,
    };
    let Some(profile) = active_profile(conn)? else {
        let mut settings = default_policy();
        extend(&mut settings, market_rate_settings(conn, None)?);
        put(&mut settings, "active_rate_pair", Value::Null);
        put(
            &mut settings,
            "auto_sync_before_report_reads",
            Value::Bool(false),
        );
        put(
            &mut settings,
            "bitcoin_rail_carrying_value",
            Value::Bool(true),
        );
        return Ok(dict([
            ("workspace", workspace),
            ("profile", Value::Null),
            ("settings", settings),
            (
                "freshness",
                dict([
                    ("sources", Value::List(vec![])),
                    ("jobs", Value::List(vec![])),
                ]),
            ),
        ]));
    };
    let profile_id = or_text(field(&profile, "id"));
    let policy = policy(conn, &profile_id)?;
    let freshness = snapshot(conn, &profile_id)?;
    let mut settings = policy.clone();
    extend(&mut settings, market_rate_settings(conn, Some(&profile))?);
    put(
        &mut settings,
        "auto_sync_before_report_reads",
        field(&policy, "report_read_sync").clone(),
    );
    for flag in PROFILE_FLAGS {
        put(
            &mut settings,
            flag,
            Value::Bool(kassiber_envelope::truthy(field(&profile, flag))),
        );
    }
    let coarse: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id=? AND excluded=0 \
             AND pricing_quality='coarse_fallback'",
            [&profile_id],
            |row| row.get(0),
        )
        .map_err(crate::database_error)?;
    put(&mut settings, "coarse_priced_count", Value::from(coarse));
    put(&mut settings, "setting_key", setting_key(&profile_id));
    Ok(dict([
        ("workspace", workspace),
        ("profile", profile_ref(&profile)),
        ("settings", settings),
        ("freshness", freshness),
    ]))
}

/// `_maintenance_configure_payload(conn, raw_args)`. Like Python, a profile
/// flag change commits through the profile update before the freshness
/// policy is validated and written.
pub fn maintenance_configure(
    conn: &Connection,
    args: &Value,
    setup: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    const KIND: &str = "ui.maintenance.configure";
    refuse_unknown(KIND, args, MAINTENANCE_ARGS)?;
    let profile = active_profile(conn)?.ok_or_else(|| {
        AppError::new(
            "validation",
            "ui.maintenance.configure requires an active profile",
        )
    })?;
    let mut updates = vec![];
    for flag in PROFILE_FLAGS {
        if let Some(value) = present(args, flag) {
            if !matches!(value, Value::Bool(_)) {
                return Err(type_error(KIND, flag, "a boolean", value));
            }
            updates.push((Key::from(*flag), value.clone()));
        }
    }
    if !updates.is_empty() {
        let mut request = dict([
            ("workspace", field(&profile, "workspace_id").clone()),
            ("profile", field(&profile, "id").clone()),
        ]);
        extend(&mut request, updates);
        kassiber_setup::execute(conn, "profiles.set", &request, setup)?;
    }
    let freshness_args = match args {
        Value::Dict(pairs) => Value::Dict(
            pairs
                .iter()
                .filter(|(key, _)| !PROFILE_FLAGS.iter().any(|flag| *key == Key::from(*flag)))
                .cloned()
                .collect(),
        ),
        other => other.clone(),
    };
    let configured = configure(conn, &freshness_args)?;
    let mut payload = maintenance_settings(conn)?;
    put(
        &mut payload,
        "configured",
        field(&configured, "settings").clone(),
    );
    Ok(payload)
}
