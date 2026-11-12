//! Atomic wallet lifecycle. Private policy history stays in the local book.
use crate::{config, database_error, descriptors, invalidate_journals, network, resolve};
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value as Py;
use kassiber_store::rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::{json, Value};
#[cfg(test)]
mod tests;

pub enum Operation {
    List {
        profile_id: String,
    },
    /// `wallets list`: the list with each wallet's completeness class and
    /// scan bounds (`annotate_wallet_rows`).
    ListWithCompleteness {
        profile_id: String,
    },
    Get {
        profile_id: String,
        wallet: String,
    },
    Create {
        workspace_id: String,
        profile_id: String,
        label: String,
        kind: String,
        account: Option<String>,
        config: Value,
    },
    Update {
        profile_id: String,
        wallet: String,
        label: Option<String>,
        account: Option<String>,
        config: Value,
        clear: Vec<String>,
    },
    Delete {
        profile_id: String,
        wallet: String,
        cascade: bool,
    },
}
struct Wallet {
    id: String,
    workspace_id: String,
    profile_id: String,
    account_id: Option<String>,
    account_code: Option<String>,
    account_label: Option<String>,
    label: String,
    kind: String,
    config: Value,
    created_at: String,
}
fn row(r: &Row<'_>) -> kassiber_store::rusqlite::Result<Wallet> {
    let raw: Option<String> = r.get("config_json")?;
    Ok(Wallet {
        id: r.get("id")?,
        workspace_id: r.get("workspace_id")?,
        profile_id: r.get("profile_id")?,
        account_id: r.get("account_id")?,
        account_code: r.get("account_code")?,
        account_label: r.get("account_label")?,
        label: r.get("label")?,
        kind: r.get("kind")?,
        created_at: r.get("created_at")?,
        config: serde_json::from_str(raw.as_deref().filter(|raw| !raw.is_empty()).unwrap_or("{}"))
            .map_err(|e| {
                kassiber_store::rusqlite::Error::FromSqlConversionFailure(
                    0,
                    kassiber_store::rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
    })
}
const SELECT: &str = "SELECT w.*,a.code AS account_code,a.label AS account_label FROM wallets w LEFT JOIN accounts a ON a.id=w.account_id";
fn load(conn: &Connection, profile: &str, reference: &str) -> Result<Wallet, AppError> {
    let resolved = resolve(conn, profile, reference)?;
    conn.query_row(&format!("{SELECT} WHERE w.id=?"), [resolved.id], row)
        .map_err(database_error)
}
pub(crate) fn now(conn: &Connection) -> Result<String, AppError> {
    conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ','now')", [], |r| {
        r.get(0)
    })
    .map_err(database_error)
}
pub(crate) fn uuid(conn: &Connection) -> Result<String, AppError> {
    let mut bytes: [u8; 16] = conn
        .query_row("SELECT randomblob(16)", [], |r| r.get::<_, Vec<u8>>(0))
        .map_err(database_error)?
        .try_into()
        .expect("randomblob length");
    bytes[6] = bytes[6] & 15 | 64;
    bytes[8] = bytes[8] & 63 | 128;
    let hex = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}
pub fn projection(v: &Value) -> Py {
    kassiber_pytext::loads(&v.to_string()).expect("JSON value")
}
fn state(config: &Value) -> Result<(String, String, String), AppError> {
    let scope = config::live_chain(config)?.unwrap_or_default();
    if config.get("descriptor").is_some_and(descriptors::truthy)
        || config.get("xpub").is_some_and(descriptors::truthy)
    {
        match descriptors::load(config) {
            Ok(Some(plan)) => {
                return Ok((
                    format!("{}:{}", plan.chain, plan.network),
                    plan.chain,
                    plan.network,
                ))
            }
            Err(descriptors::Error::App(error)) if error.code != "dependency_missing" => {
                return Err(error)
            }
            Err(_) => return Ok(("invalid".into(), scope.0, scope.1)),
            _ => (),
        }
    }
    Ok((String::new(), scope.0, scope.1))
}
fn get(c: &Value, k: &str, default: Value) -> Py {
    projection(c.get(k).unwrap_or(&default))
}
fn safe_projection(c: &Value) -> Result<Py, AppError> {
    let safe = config::safe(c)?;
    // Python's allowlist insertion order is distinct from sorted storage.
    let mut result = Vec::new();
    for key in config::safe_fields().iter().copied().chain([
        "descriptor",
        "change_descriptor",
        "xpub",
        "sp_descriptor",
    ]) {
        if let Some(value) = safe.get(key) {
            result.push((key, projection(value)));
        }
    }
    Ok(dict(result))
}
fn details(w: &Wallet) -> Result<Py, AppError> {
    let c = &w.config;
    let (state, chain, network) = state(c)?;
    Ok(dict([
        ("id", s(w.id.as_str())),
        ("workspace_id", s(w.workspace_id.as_str())),
        ("profile_id", s(w.profile_id.as_str())),
        ("account_id", w.account_id.as_deref().map_or(Py::Null, s)),
        (
            "account_code",
            w.account_code.as_deref().map_or(Py::Null, s),
        ),
        (
            "account_label",
            w.account_label.as_deref().map_or(Py::Null, s),
        ),
        ("label", s(w.label.as_str())),
        ("kind", s(w.kind.as_str())),
        ("chain", s(chain)),
        ("network", s(network)),
        ("backend", get(c, "backend", json!(""))),
        (
            "addresses",
            projection(&json!(config::addresses(c.get("addresses"))?)),
        ),
        (
            "descriptor",
            Py::Bool(c.get("descriptor").is_some_and(descriptors::truthy)),
        ),
        ("descriptor_state", s(state)),
        (
            "change_descriptor",
            Py::Bool(c.get("change_descriptor").is_some_and(descriptors::truthy)),
        ),
        (
            "silent_payment",
            dict([
                (
                    "configured",
                    Py::Bool(config::optional_text(c.get("sp_descriptor")).is_some()),
                ),
                ("material_format", get(c, "sp_material_format", json!(""))),
                ("scan_mode", get(c, "sp_scan_mode", json!(""))),
                (
                    "scan_start_height",
                    get(c, "sp_scan_start_height", Value::Null),
                ),
                ("scan_start_date", get(c, "sp_scan_start_date", json!(""))),
                (
                    "full_history",
                    Py::Bool(c.get("sp_full_history").is_some_and(descriptors::truthy)),
                ),
            ]),
        ),
        ("gap_limit", get(c, "gap_limit", Value::Null)),
        ("policy_asset", get(c, "policy_asset", Value::Null)),
        ("source_file", get(c, "source_file", json!(""))),
        ("source_format", get(c, "source_format", json!(""))),
        (
            "deprecated",
            Py::Bool(config::boolean(c.get("deprecated"))?),
        ),
        ("config", safe_projection(c)?),
        ("created_at", s(w.created_at.as_str())),
    ]))
}
/// The exception `json.loads(config_json or "{}").pop(...)` raises for a
/// stored configuration that is not a JSON object.
fn config_failure(raw: Option<&str>) -> Option<AppError> {
    let raw = raw.filter(|raw| !raw.is_empty()).unwrap_or("{}");
    match kassiber_pytext::loads(raw) {
        Err(error) => Some(AppError::new("internal_error", error.to_string())),
        Ok(Py::Dict(_)) => None,
        Ok(Py::List(_)) => Some(AppError::new(
            "internal_error",
            "pop expected at most 1 argument, got 2",
        )),
        Ok(other) => Some(AppError::new(
            "internal_error",
            format!(
                "'{}' object has no attribute 'pop'",
                kassiber_envelope::type_name(&other)
            ),
        )),
    }
}
fn list(conn: &Connection, profile: &str) -> Result<Py, AppError> {
    let mut stmt = conn
        .prepare(&format!(
            "{SELECT} WHERE w.profile_id=? ORDER BY w.label ASC"
        ))
        .map_err(database_error)?;
    let rows = stmt
        .query_map([profile], |r| {
            Ok((r.get::<_, Option<String>>("config_json")?, row(r)))
        })
        .map_err(database_error)?;
    let mut out = Vec::new();
    for found in rows {
        let (raw, w) = found.map_err(database_error)?;
        if let Some(failure) = config_failure(raw.as_deref()) {
            return Err(failure);
        }
        let w = w.map_err(database_error)?;
        let c = &w.config;
        let (state, chain, network) = state(c)?;
        let gap = if state.is_empty() {
            json!("")
        } else {
            json!(100)
        };
        out.push(dict([
            ("id", s(w.id)),
            ("label", s(w.label)),
            ("kind", s(w.kind)),
            (
                "account",
                s(w.account_code
                    .filter(|s| !s.is_empty())
                    .or(w.account_label)
                    .unwrap_or_default()),
            ),
            ("chain", s(chain)),
            ("network", s(network)),
            ("backend", get(c, "backend", json!(""))),
            (
                "addresses",
                s(config::addresses(c.get("addresses"))?.join(",")),
            ),
            ("descriptor", s(state)),
            (
                "silent_payment",
                s(if config::optional_text(c.get("sp_descriptor")).is_some() {
                    "configured"
                } else {
                    ""
                }),
            ),
            ("gap_limit", get(c, "gap_limit", gap)),
            ("source_format", get(c, "source_format", json!(""))),
            ("source_file", get(c, "source_file", json!(""))),
            (
                "deprecated",
                Py::Bool(config::boolean(c.get("deprecated"))?),
            ),
            ("created_at", s(w.created_at)),
        ]));
    }
    Ok(Py::List(out))
}
/// `repo.accounts.resolve_account(...)["id"]`: exact id, then code, then a
/// unique case-insensitive label, within the profile.
pub fn account(conn: &Connection, profile: &str, reference: &str) -> Result<String, AppError> {
    let normalized = kassiber_pytext::strip(reference);
    for predicate in ["id=?", "lower(code)=lower(?)"] {
        if let Some(id) = conn
            .query_row(
                &format!("SELECT id FROM accounts WHERE profile_id=? AND {predicate} LIMIT 1"),
                [profile, normalized],
                |r| r.get(0),
            )
            .optional()
            .map_err(database_error)?
        {
            return Ok(id);
        }
    }
    let mut stmt=conn.prepare("SELECT id,code,label FROM accounts WHERE profile_id=? AND lower(label)=lower(?) ORDER BY code ASC").map_err(database_error)?;
    let matches = stmt
        .query_map([profile, normalized], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    match matches.as_slice() {
        [(id, _, _)] => Ok(id.clone()),
        [] => Err(AppError::new(
            "not_found",
            format!("Account '{reference}' not found"),
        )),
        _ => Err(AppError::new(
            "validation",
            format!("Account label '{reference}' is ambiguous"),
        )
        .with_hint("Use the account code or id instead of the non-unique label.")
        .with_details(dict([(
            "matches",
            Py::List(
                matches
                    .into_iter()
                    .map(|(id, code, label)| {
                        dict([("id", s(id)), ("code", s(code)), ("label", s(label))])
                    })
                    .collect(),
            ),
        )]))),
    }
}
fn conflict(conn: &Connection, profile: &str, label: &str, update: bool) -> AppError {
    let profile_label: String = conn
        .query_row("SELECT label FROM profiles WHERE id=?", [profile], |r| {
            r.get(0)
        })
        .unwrap_or_default();
    AppError::new(
        "conflict",
        format!("Wallet '{label}' already exists in profile '{profile_label}'"),
    )
    .with_hint(if update {
        "Choose a different wallet label."
    } else {
        "Choose a different wallet label or update the existing wallet."
    })
}
fn ensure_epoch(conn: &Connection, w: &Wallet, material: &Value) -> Result<String, AppError> {
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM wallet_policy_epochs WHERE wallet_id=? AND status='active'",
            [&w.id],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?
    {
        return Ok(id);
    }
    let id = uuid(conn)?;
    let chain = descriptors::normalize_chain(&descriptors::field(material, "chain"))
        .map_err(descriptors::Error::into_app)?;
    let network = descriptors::normalize_network(&chain, &descriptors::field(material, "network"))
        .map_err(descriptors::Error::into_app)?;
    conn.execute("INSERT INTO wallet_policy_epochs(id,workspace_id,profile_id,wallet_id,chain,network,status,private_material_json,created_at,retired_at) VALUES(?,?,?,?,?,?,'active',?,?,NULL)",params![id,w.workspace_id,w.profile_id,w.id,chain,network,config::stored_json(material),now(conn)?]).map_err(database_error)?;
    Ok(id)
}
fn roll_epoch(conn: &Connection, w: &Wallet, new: &Value) -> Result<(), AppError> {
    let old = ensure_epoch(conn, w, &config::private_material(&w.config))?;
    conn.execute("UPDATE wallet_policy_epochs SET status='retired',retired_at=? WHERE id=? AND status='active'",params![now(conn)?,old]).map_err(database_error)?;
    ensure_epoch(conn, w, &config::private_material(new))?;
    Ok(())
}
fn delete_freshness(
    conn: &Connection,
    profile: &str,
    wallet: &str,
    source: &str,
) -> Result<(), AppError> {
    let key = format!("{source}:{wallet}");
    for table in ["freshness_jobs", "freshness_source_states"] {
        conn.execute(
            &format!("DELETE FROM {table} WHERE profile_id=? AND source_key=?"),
            [profile, &key],
        )
        .map_err(database_error)?;
    }
    Ok(())
}
fn refresh(conn: &Connection, w: &Wallet, new: &Value) -> Result<(), AppError> {
    if config::policy_identity(new) != config::policy_identity(&w.config) {
        roll_epoch(conn, w, new)?;
    }
    if config::sync_identity(new) != config::sync_identity(&w.config) {
        conn.execute(
            "DELETE FROM chain_observer_instances WHERE logical_wallet_id=? OR source_wallet_id=?",
            [&w.id, &w.id],
        )
        .map_err(database_error)?;
        for table in ["wallet_utxos", "wallet_utxo_refreshes"] {
            conn.execute(&format!("DELETE FROM {table} WHERE wallet_id=?"), [&w.id])
                .map_err(database_error)?;
        }
        conn.execute("UPDATE freshness_source_states SET status=CASE WHEN paused THEN 'paused' WHEN blocking_reports THEN 'blocking_reports' ELSE 'partially_stale' END,state=CASE WHEN paused THEN 'paused' WHEN blocking_reports THEN 'blocking_reports' ELSE 'partially_stale' END,stale_reason='wallet_config_changed',progress_json='{}',checkpoint_json='{}',rate_limited_until=NULL,cooldown_reason=NULL,last_error_at=NULL,last_error_code=NULL,last_error_message=NULL,last_phase=NULL,updated_at=? WHERE profile_id=? AND source_key=?",params![now(conn)?,w.profile_id,format!("onchain_wallet:{}",w.id)]).map_err(database_error)?;
    }
    let active_onchain = !config::boolean(new.get("deprecated"))?
        && w.kind != "coreln"
        && !(new.get("source_file").is_some_and(descriptors::truthy)
            && new.get("source_format").is_some_and(descriptors::truthy))
        && (config::optional_text(new.get("descriptor")).is_some()
            || config::optional_text(new.get("xpub")).is_some()
                && new.get("script_types").is_some_and(descriptors::truthy)
            || !config::addresses(new.get("addresses"))?.is_empty());
    let deprecated = config::boolean(new.get("deprecated"))?;
    for (source, active) in [
        ("onchain_wallet", active_onchain),
        (
            "btcpay_wallet",
            !deprecated && config::optional_text(new.get("sync_source")).is_some(),
        ),
        (
            "btcpay_provenance",
            !deprecated
                && new
                    .get("btcpay_provenance")
                    .is_some_and(descriptors::truthy),
        ),
    ] {
        if !active {
            delete_freshness(conn, &w.profile_id, &w.id, source)?;
        }
    }
    Ok(())
}
pub struct CreationIdentity {
    pub id: String,
    pub created_at: String,
}

pub fn execute(conn: &Connection, operation: Operation) -> Result<Py, AppError> {
    execute_with_creation_identity(conn, operation, None)
}

/// Explicit caller identity, as Python's create_wallet(wallet_id=...) permits.
/// The shipping shell supplies None; test builds can compare wire bytes.
pub fn execute_with_creation_identity(
    conn: &Connection,
    operation: Operation,
    identity: Option<&CreationIdentity>,
) -> Result<Py, AppError> {
    execute_with_creation_source(conn, operation, &mut |conn| {
        Ok(CreationIdentity {
            id: identity
                .map(|v| v.id.clone())
                .map_or_else(|| uuid(conn), Ok)?,
            created_at: identity
                .map(|v| v.created_at.clone())
                .map_or_else(|| now(conn), Ok)?,
        })
    })
}

// Allocate identity only after account, configuration and network admission,
// exactly as the Python coordinator does for an automatically created wallet.
pub(crate) fn execute_with_creation_source(
    conn: &Connection,
    operation: Operation,
    identity: &mut dyn FnMut(&Connection) -> Result<CreationIdentity, AppError>,
) -> Result<Py, AppError> {
    match operation {
        Operation::List { profile_id } => list(conn, &profile_id),
        Operation::ListWithCompleteness { profile_id } => {
            let Py::List(rows) = list(conn, &profile_id)? else {
                unreachable!("wallet list")
            };
            Ok(Py::List(crate::completeness::annotate(
                conn,
                &profile_id,
                rows,
            )?))
        }
        Operation::Get { profile_id, wallet } => details(&load(conn, &profile_id, &wallet)?),
        operation => {
            kassiber_store::begin_write_savepoint(conn, "wallet_lifecycle")
                .map_err(database_error)?;
            let result = write(conn, operation, identity);
            if result.is_err() {
                conn.execute_batch("ROLLBACK TO wallet_lifecycle")
                    .map_err(database_error)?;
            }
            conn.execute_batch("RELEASE wallet_lifecycle")
                .map_err(database_error)?;
            result
        }
    }
}
fn write(
    conn: &Connection,
    operation: Operation,
    identity: &mut dyn FnMut(&Connection) -> Result<CreationIdentity, AppError>,
) -> Result<Py, AppError> {
    match operation {
        Operation::Create {
            workspace_id,
            profile_id,
            label,
            kind,
            account: reference,
            config: input,
        } => {
            let account = account(
                conn,
                &profile_id,
                reference
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("treasury"),
            )?;
            let kind = config::kind(&kind)?;
            let config = network::new_config(conn, &profile_id, &kind, input)?;
            network::guard(conn, &profile_id, &config, None)?;
            let config = config::validate(&kind, &config)?;
            network::guard(conn, &profile_id, &config, None)?;
            let identity = identity(conn)?;
            let id = identity.id;
            let created_at = identity.created_at;
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,account_id,label,kind,config_json,created_at) VALUES(?,?,?,?,?,?,?,?)",params![id,workspace_id,profile_id,account,label,kind,config::stored_json(&config),created_at]).map_err(|e|if matches!(e,kassiber_store::rusqlite::Error::SqliteFailure(ref e,_) if e.code==kassiber_store::rusqlite::ErrorCode::ConstraintViolation){conflict(conn,&profile_id,&label,false)}else{database_error(e)})?;
            invalidate_journals(conn, &profile_id)?;
            details(&load(conn, &profile_id, &id)?)
        }
        Operation::Update {
            profile_id,
            wallet,
            label,
            account: reference,
            config: updates,
            clear,
        } => {
            let w = load(conn, &profile_id, &wallet)?;
            if label.is_none()
                && reference.is_none()
                && updates.as_object().is_none_or(|o| o.is_empty())
                && clear.is_empty()
            {
                return Err(AppError::new(
                    "validation",
                    "wallets update requires at least one field to change",
                )
                .with_hint("Pass --label, --account, --config/--config-file, or --clear <field>"));
            }
            let account = reference
                .map(|r| account(conn, &profile_id, &r))
                .transpose()?
                .or(w.account_id.clone());
            let mut config = w.config.clone();
            for field in clear {
                if config.as_object_mut().unwrap().remove(&field).is_none() {
                    return Err(AppError::new(
                        "validation",
                        format!("Wallet config field '{field}' is not set"),
                    )
                    .with_hint(
                        "Use `wallets get` to inspect clearable config fields before clearing.",
                    ));
                }
            }
            for (key, value) in updates
                .as_object()
                .ok_or_else(|| AppError::new("validation", "Wallet config must be an object"))?
            {
                if value.is_null() {
                    config.as_object_mut().unwrap().remove(key);
                } else {
                    config[key] = value.clone();
                }
            }
            let config = network::new_config(conn, &profile_id, &w.kind, config)?;
            let config = config::validate(&w.kind, &config)?;
            let has_history: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM transactions WHERE wallet_id=?)",
                    [&w.id],
                    |r| r.get(0),
                )
                .map_err(database_error)?;
            network::guard(conn, &profile_id, &config, has_history.then_some(&w.config))?;
            if has_history {
                network::guard_history(conn, &profile_id, &w.id, &config)?;
            }
            let label = label.unwrap_or_else(|| w.label.clone());
            conn.execute("UPDATE wallets SET label=?,account_id=?,config_json=? WHERE id=?",params![label,account,config::stored_json(&config),w.id]).map_err(|e|if matches!(e,kassiber_store::rusqlite::Error::SqliteFailure(ref e,_) if e.code==kassiber_store::rusqlite::ErrorCode::ConstraintViolation){conflict(conn,&profile_id,&label,true)}else{database_error(e)})?;
            refresh(conn, &w, &config)?;
            invalidate_journals(conn, &profile_id)?;
            details(&load(conn, &profile_id, &w.id)?)
        }
        Operation::Delete {
            profile_id,
            wallet,
            cascade,
        } => {
            let w = load(conn, &profile_id, &wallet)?;
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM transactions WHERE wallet_id=?",
                    [&w.id],
                    |r| r.get(0),
                )
                .map_err(database_error)?;
            if count != 0 && !cascade {
                return Err(AppError::new("conflict",format!("Wallet '{}' has {count} transaction(s); pass --cascade to delete them too",w.label)).with_hint("Use --cascade to remove the wallet and all associated transactions/journal entries.").with_details(dict([("transactions",Py::from(count))])));
            }
            conn.execute("DELETE FROM wallets WHERE id=?", [&w.id])
                .map_err(database_error)?;
            for source in ["onchain_wallet", "btcpay_wallet", "btcpay_provenance"] {
                delete_freshness(conn, &profile_id, &w.id, source)?;
            }
            invalidate_journals(conn, &profile_id)?;
            Ok(dict([
                ("id", s(w.id)),
                ("label", s(w.label)),
                ("deleted", Py::Bool(true)),
                (
                    "cascaded_transactions",
                    Py::from(if cascade { count } else { 0 }),
                ),
            ]))
        }
        _ => unreachable!("reads dispatched separately"),
    }
}
