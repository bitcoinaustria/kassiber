//! Revocable recurring acquisition grants (`chain_analysis_backfill`). A grant
//! pins the book, source configuration, domain and lifetime quota; planning,
//! authorizing, listing, revoking and queueing never contact the source.
use super::routing::{get_db_backend, lower, py_text, routing_identity};
use super::*;

pub(super) const MAX_RESPONSE_BYTES: i64 = 8 * 1024 * 1024;

fn int(value: i64) -> Value {
    Value::Int(value.into())
}

fn small_int(value: &Value) -> Option<i64> {
    match value {
        Value::Int(n) => i64::try_from(n.clone()).ok(),
        _ => None,
    }
}

pub(super) fn is_txid(value: &Value) -> bool {
    matches!(value, Value::Str(text) if text.as_str().is_some_and(|t| t.len() == 64 && t.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))))
}

/// Python set membership: unhashable JSON containers raise `TypeError`.
fn member(value: &Value, choices: &[&str]) -> Result<bool> {
    match value {
        Value::List(_) | Value::Dict(_) => Err(AppError::new(
            "internal_error",
            format!("unhashable type: '{}'", kassiber_envelope::type_name(value)),
        )),
        Value::Str(text) => Ok(text.as_str().is_some_and(|t| choices.contains(&t))),
        _ => Ok(false),
    }
}

fn domain(conn: &Connection, profile: &str, spec: &Value) -> Result<Value> {
    let domain = kassiber_setup::networks::require_chain_domain(
        conn,
        profile,
        &py_text(field(spec, "chain")),
        field(spec, "network"),
        None,
        "recurring_acquisition",
    )?;
    let instance = get(spec, "chain_instance_id").unwrap_or(&Value::Null);
    if instance != &Value::Null && !py_eq(instance, field(&domain, "chain_instance_id")) {
        return Err(AppError::new(
            "book_network_mismatch",
            "Source belongs to a different book network",
        )
        .with_details(dict([
            ("operation", s("recurring_acquisition")),
            ("profile_id", s(profile)),
        ]))
        .with_hint(
            "Review the active book's network in Settings. Existing history cannot be relabeled.",
        ));
    }
    Ok(domain)
}

/// The saved Bitcoin Core row and its configuration fingerprint.
fn source(conn: &Connection, spec: &Value) -> Result<(Value, String)> {
    let backend = match get_db_backend(conn, &py_text(field(spec, "backend"))) {
        Ok(backend) => backend,
        Err(error) if error.code == "not_found" => {
            return Err(AppError::new(
                "acquisition_source_changed",
                "Source was removed; authorize a new connection",
            ))
        }
        Err(error) => return Err(error),
    };
    if field(&backend, "kind") != &s("bitcoinrpc")
        || field(&backend, "chain") != &s("bitcoin")
        || !py_eq(field(&backend, "network"), field(spec, "network"))
    {
        return Err(AppError::new(
            "acquisition_source_changed",
            "Recurring acquisition requires a matching Bitcoin Core connection",
        ));
    }
    let instance = field(&backend, "chain_instance_id");
    if truthy(instance) && !py_eq(instance, field(spec, "chain_instance_id")) {
        return Err(AppError::new(
            "book_network_mismatch",
            "Source belongs to another local chain instance",
        ));
    }
    let revision = first(
        conn,
        "SELECT updated_at FROM backends WHERE name=?",
        params![sql(field(&backend, "name"), 1)?],
    )?
    .map(|row| field(&row, "updated_at").clone())
    .unwrap_or(Value::Null);
    let fingerprint = digest(&Value::List(vec![
        revision,
        routing_identity(&backend)?,
        field(&backend, "kind").clone(),
        get(&backend, "chain").cloned().unwrap_or(Value::Null),
        get(&backend, "network").cloned().unwrap_or(Value::Null),
    ]))?;
    Ok((backend, fingerprint))
}

pub(super) fn plan(conn: &Connection, profile: &str, args: &Value) -> Result<Value> {
    arguments(
        args,
        &[
            "backend",
            "chain",
            "network",
            "chain_instance_id",
            "mode",
            "subject",
            "start_height",
            "end_height",
            "interval_seconds",
            "duration_days",
            "max_requests",
            "max_bytes",
            "blocks_per_run",
        ],
        &["backend", "network", "mode", "max_requests", "max_bytes"],
    )?;
    let mut spec = dict([
        ("chain", s("bitcoin")),
        ("interval_seconds", int(300)),
        ("duration_days", int(30)),
        ("blocks_per_run", int(4)),
    ]);
    if let Value::Dict(pairs) = args {
        for (key, value) in pairs {
            if let kassiber_pyjson::Key::Str(key) = key {
                set(&mut spec, key.as_str().unwrap(), value.clone());
            }
        }
    }
    let backend = text_value(field(&spec, "backend"), "backend", 128)?;
    set(
        &mut spec,
        "backend",
        s(lower(&py_text(&Value::Str(backend))).as_str()),
    );
    if field(&spec, "chain") != &s("bitcoin")
        || !member(
            field(&spec, "network"),
            &["main", "test", "signet", "regtest"],
        )?
        || !member(field(&spec, "mode"), &["subject", "blocks"])?
    {
        return Err(invalid(
            "Choose a Bitcoin network and subject or blocks acquisition",
        ));
    }
    for (name, low, high) in [
        ("interval_seconds", 30, 86400),
        ("duration_days", 1, 365),
        ("max_requests", 4, 100_000_000),
        ("max_bytes", MAX_RESPONSE_BYTES, 10_000_000_000_000),
        ("blocks_per_run", 1, 16),
    ] {
        if !small_int(field(&spec, name)).is_some_and(|n| (low..=high).contains(&n)) {
            return Err(invalid(&format!("{name} must be between {low} and {high}")));
        }
    }
    if field(&spec, "mode") == &s("subject") {
        if !is_txid(field(&spec, "subject"))
            || get(&spec, "start_height").is_some()
            || get(&spec, "end_height").is_some()
        {
            return Err(invalid(
                "Subject acquisition requires one canonical txid and no block range",
            ));
        }
    } else {
        let start = field(&spec, "start_height");
        if get(&spec, "subject").is_some() || !matches!(start, Value::Int(n) if n >= &0.into()) {
            return Err(invalid(
                "Block acquisition requires an explicit nonnegative start height",
            ));
        }
        if let Some(end) = get(&spec, "end_height") {
            let (Value::Int(end), Value::Int(start)) = (end, start) else {
                return Err(invalid("End height must be at or above the start height"));
            };
            if end < start {
                return Err(invalid("End height must be at or above the start height"));
            }
        }
    }
    let binding = domain(conn, profile, &spec)?;
    let (backend, fingerprint) = source(conn, &spec)?;
    let effects = dict([
        ("recurring_egress", Value::Bool(true)),
        ("max_requests", field(&spec, "max_requests").clone()),
        ("max_response_bytes", int(MAX_RESPONSE_BYTES)),
        ("max_total_bytes", field(&spec, "max_bytes").clone()),
        ("duration_days", field(&spec, "duration_days").clone()),
        ("interval_seconds", field(&spec, "interval_seconds").clone()),
        ("requires_running_unlocked_daemon", Value::Bool(true)),
    ]);
    let mut result = dict([
        ("spec", spec),
        ("binding", binding),
        (
            "backend",
            dict(
                ["name", "kind", "chain", "network"]
                    .map(|key| (key, get(&backend, key).cloned().unwrap_or(Value::Null))),
            ),
        ),
        ("effects", effects),
        (
            "limitations",
            Value::List(
                [
                    "Only the selected source is contacted. Pruned history remains incomplete.",
                    "Source, book or network changes invalidate this authorization.",
                    "Observations do not prove ownership or authorize accounting.",
                ]
                .map(s)
                .to_vec(),
            ),
        ),
    ]);
    let plan_id = digest(&Value::List(vec![
        s(profile),
        watches::database_instance_id(conn)?,
        s(fingerprint.as_str()),
        result.clone(),
    ]))?;
    set(&mut result, "plan_id", s(plan_id.as_str()));
    Ok(result)
}

pub(super) fn authorize(
    conn: &Connection,
    profile: &str,
    args: &Value,
    sources: &mut dyn Sources,
    runtime: &mut dyn Runtime,
) -> Result<Value> {
    watches::require_encrypted(conn)?;
    arguments(args, &["plan"], &["plan"])?;
    let submitted = field(args, "plan");
    if !matches!(submitted, Value::Dict(_)) {
        return Err(invalid("An unchanged source plan is required"));
    }
    let id = atomic(conn, || {
        let expected = plan(
            conn,
            profile,
            get(submitted, "spec").unwrap_or(&dict::<&str>([])),
        )?;
        if canonical(submitted)? != canonical(&expected)? {
            return Err(stale("Source authorization changed; review a new plan"));
        }
        let spec = field(&expected, "spec");
        let (_, fingerprint) = source(conn, spec)?;
        let id = sources.id().replace('-', "");
        let now = runtime.epoch();
        let Value::Int(days) = field(spec, "duration_days") else {
            unreachable!("validated duration")
        };
        let expires = now + i64::try_from(days.clone()).unwrap() * 86400;
        conn.execute(
            "INSERT INTO chain_analysis_acquisition_grants
            (id,profile_id,spec_json,binding_json,source_fingerprint,database_identity,status,next_run_at,expires_at,created_at)
            VALUES(?,?,?,?,?,?,'active',?,?,?)",
            params![
                id,
                profile,
                canonical(spec)?,
                canonical(field(&expected, "binding"))?,
                fingerprint,
                sql(&watches::database_instance_id(conn)?, 6)?,
                now,
                expires,
                sources.now()
            ],
        )
        .map_err(database_error)?;
        Ok(id)
    })?;
    grant(conn, profile, &s(id.as_str()))
}

pub(super) fn row(conn: &Connection, profile: &str, id: &Value) -> Result<Value> {
    text_value(id, "id", 64)?;
    first(
        conn,
        "SELECT * FROM chain_analysis_acquisition_grants WHERE profile_id=? AND id=?",
        params![profile, sql(id, 2)?],
    )?
    .ok_or_else(|| AppError::new("not_found", "Source authorization not found in this book"))
}

/// Never exposes endpoint fingerprints or internal lease identifiers.
pub(super) fn grant(conn: &Connection, profile: &str, id: &Value) -> Result<Value> {
    let row = row(conn, profile, id)?;
    let mut value = dict(
        [
            "id",
            "status",
            "revision",
            "requests_used",
            "bytes_used",
            "next_run_at",
            "expires_at",
            "cursor_height",
            "verified_tip_height",
            "verified_tip_at",
            "last_code",
            "created_at",
        ]
        .map(|key| (key, field(&row, key).clone())),
    );
    for (key, column) in [("spec", "spec_json"), ("binding", "binding_json")] {
        let parsed = kassiber_pytext::loads(&py_text(field(&row, column)))
            .map_err(|error| AppError::new("internal_error", error.to_string()))?;
        set(&mut value, key, parsed);
    }
    Ok(value)
}

fn list(conn: &Connection, profile: &str, args: &Value) -> Result<Value> {
    arguments(args, &["limit", "after_id"], &[])?;
    let limit = small_int(get(args, "limit").unwrap_or(&int(50)))
        .filter(|n| (1..=100).contains(n))
        .ok_or_else(|| invalid("limit must be between 1 and 100"))?;
    let after = get(args, "after_id").cloned().unwrap_or(s(""));
    if !matches!(&after, Value::Str(text) if text.code_points().count() <= 64) {
        return Err(invalid("Invalid source cursor"));
    }
    let ids: Vec<Value> = crate::publication::row::rows(
        conn,
        "SELECT id FROM chain_analysis_acquisition_grants WHERE profile_id=? AND id>? ORDER BY id LIMIT ?",
        params![profile, sql(&after, 2)?, limit + 1],
    )?
    .iter()
    .map(|row| field(row, "id").clone())
    .collect();
    let items = ids
        .iter()
        .take(limit as usize)
        .map(|id| grant(conn, profile, id))
        .collect::<Result<Vec<_>>>()?;
    let next = if ids.len() as i64 > limit {
        ids[limit as usize - 1].clone()
    } else {
        Value::Null
    };
    Ok(dict([("items", Value::List(items)), ("next_cursor", next)]))
}

fn revoke(conn: &Connection, profile: &str, args: &Value) -> Result<Value> {
    arguments(
        args,
        &["id", "expected_revision"],
        &["id", "expected_revision"],
    )?;
    let id = atomic(conn, || {
        let row = row(conn, profile, field(args, "id"))?;
        if !matches!(field(args, "expected_revision"), Value::Int(n) if py_eq(field(&row, "revision"), &Value::Int(n.clone())))
        {
            return Err(stale("Source authorization changed"));
        }
        conn.execute(
            "UPDATE chain_analysis_acquisition_grants SET status='revoked',revision=revision+1,lease_token=NULL,lease_until=NULL WHERE id=?",
            params![sql(field(&row, "id"), 1)?],
        )
        .map_err(database_error)?;
        Ok(field(&row, "id").clone())
    })?;
    grant(conn, profile, &id)
}

/// `_validate` without a lease: the grant, book, domain and source still match.
pub(super) fn validate(
    conn: &Connection,
    profile: &str,
    original: &Value,
    lease: Option<&str>,
    runtime: &dyn Runtime,
) -> Result<(Value, Value)> {
    if runtime.cancelled() {
        return Err(AppError::new("cancelled", "Acquisition cancelled"));
    }
    watches::require_encrypted(conn)?;
    let live = row(conn, profile, field(original, "id"))?;
    if field(&live, "status") != &s("active")
        || !py_eq(field(&live, "revision"), field(original, "revision"))
        || lease.is_some_and(|lease| field(&live, "lease_token") != &s(lease))
    {
        return Err(AppError::new(
            "acquisition_revoked",
            "Source authorization is no longer active",
        ));
    }
    if small_int(field(&live, "expires_at")).is_none_or(|expires| expires <= runtime.epoch()) {
        return Err(AppError::new(
            "acquisition_expired",
            "Source authorization expired",
        ));
    }
    if !py_eq(
        field(&live, "database_identity"),
        &watches::database_instance_id(conn)?,
    ) {
        return Err(AppError::new(
            "scope_changed",
            "Source authorization belongs to a different database",
        ));
    }
    let spec = kassiber_pytext::loads(&py_text(field(&live, "spec_json")))
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    if field(&live, "binding_json") != &s(canonical(&domain(conn, profile, &spec)?)?.as_str()) {
        return Err(AppError::new("scope_changed", "Book environment changed"));
    }
    let (backend, fingerprint) = source(conn, &spec)?;
    if field(&live, "source_fingerprint") != &s(fingerprint.as_str()) {
        return Err(AppError::new(
            "acquisition_source_changed",
            "Source configuration changed; authorize it again",
        ));
    }
    Ok((live, backend))
}

/// Queue the next run for the unlocked daemon; this never contacts the source.
fn run(conn: &Connection, profile: &str, args: &Value, runtime: &mut dyn Runtime) -> Result<Value> {
    arguments(args, &["id"], &["id"])?;
    // The worker pauses acquisition while offline; say so instead of queueing.
    if runtime.offline() {
        return Err(AppError::new(
            "network_egress_disabled",
            "Outbound chain acquisition is disabled: Kassiber is in offline mode",
        )
        .with_hint("Turn off offline mode to connect."));
    }
    let row = row(conn, profile, field(args, "id"))?;
    validate(conn, profile, &row, None, runtime)?;
    // Python's driver opens a transaction before this bare UPDATE; this one
    // holds the writer from its start, as every write transaction here does.
    if conn.is_autocommit() {
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(database_error)?;
    }
    conn.execute(
        "UPDATE chain_analysis_acquisition_grants SET next_run_at=? WHERE id=?",
        params![runtime.epoch(), sql(field(&row, "id"), 2)?],
    )
    .map_err(database_error)?;
    let mut value = grant(conn, profile, field(&row, "id"))?;
    set(&mut value, "queued", Value::Bool(true));
    set(
        &mut value,
        "requires_running_unlocked_daemon",
        Value::Bool(true),
    );
    Ok(value)
}

pub(super) fn dispatch(
    conn: &Connection,
    profile: &str,
    operation: &str,
    args: &Value,
    sources: &mut dyn Sources,
    runtime: &mut dyn Runtime,
) -> Result<Value> {
    match operation {
        "sources.plan" => plan(conn, profile, args),
        "sources.list" => list(conn, profile, args),
        "sources.authorize" => authorize(conn, profile, args, sources, runtime),
        "sources.revoke" => revoke(conn, profile, args),
        _ => run(conn, profile, args, runtime),
    }
}
