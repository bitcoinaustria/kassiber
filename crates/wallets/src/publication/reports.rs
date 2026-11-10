//! Saved-report impacts of a wallet publication, from
//! `filed_report_chain.record_wallet_changes`. It only appends inbox evidence;
//! report bytes, filing state and lot authority are never changed here.
use super::physical::{confirmation_observation, stored_tx_mapping};
use super::row::{field, first, or_text, py_eq, rows, Result};
use crate::database_error;
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql, Connection};
use sha2::{Digest, Sha256};

const OBSERVATION_SELECT: &str = "
 SELECT t.*, w.kind AS wallet_kind, w.config_json AS wallet_config_json,
 o.authority_version AS observation_authority_version,
 o.graph_hash AS observation_graph_hash, o.quantity_hash AS observation_quantity_hash,
 o.observed_at AS observation_observed_at, o.observer_ids_json AS observation_observer_ids,
 o.observer_kinds_json AS observation_observer_kinds, o.fee_attribution AS observation_fee_attribution,
 o.chain AS observation_chain, o.network AS observation_network
 FROM transactions t JOIN wallets w ON w.id=t.wallet_id
 LEFT JOIN chain_observation_provenance o ON o.transaction_id=t.id
";

fn compact(value: &Value) -> Result<String> {
    kassiber_pyjson::dumps(
        value,
        &kassiber_pyjson::Options::new().sort_keys(true).compact(),
    )
    .map_err(|_| AppError::new("app_error", "Report impact cannot be encoded"))
}

/// `chain_analysis.index.digest`.
fn digest(value: &Value) -> Result<String> {
    let text = kassiber_pyjson::dumps(
        value,
        &kassiber_pyjson::Options::new()
            .sort_keys(true)
            .compact()
            .default_str(true),
    )
    .map_err(|_| AppError::new("app_error", "Report domain cannot be encoded"))?;
    Ok(Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn sql(value: &Value) -> Sql {
    match value {
        Value::Null => Sql::Null,
        Value::Int(n) => i64::try_from(n.clone()).map_or(Sql::Null, Sql::Integer),
        Value::Float(f) => Sql::Real(*f),
        other => Sql::Text(py_str(other)),
    }
}

/// `OccurrenceResolver(conn, profile).source(row, chain, network).domain_id`.
fn domain_id(
    conn: &Connection,
    profile_id: &str,
    row: &Value,
    chain: &Value,
    network: &Value,
) -> Result<String> {
    let raw = stored_tx_mapping(field(row, "raw_json"), false).unwrap_or(Value::Dict(vec![]));
    let config =
        stored_tx_mapping(field(row, "wallet_config_json"), false).unwrap_or(Value::Dict(vec![]));
    let null = Value::Null;
    let mut instance = field(row, "chain_instance_id");
    for candidate in [
        get(&raw, "chain_instance_id"),
        get(&config, "chain_instance_id"),
    ] {
        if !truthy(instance) {
            instance = candidate.unwrap_or(&null);
        }
    }
    let mut domain = field(row, "_index_domain_id").clone();
    if !truthy(&domain) {
        domain = if truthy(instance) {
            Value::from(digest(&Value::List(vec![
                chain.clone(),
                network.clone(),
                instance.clone(),
            ]))?)
        } else {
            let binding = first(
                conn,
                "SELECT domains_json FROM book_network_bindings WHERE profile_id=?",
                [profile_id],
            )?;
            let mut bound = Value::Null;
            if let Some(binding) = binding {
                let domains = kassiber_pytext::loads(&or_text(field(&binding, "domains_json")))
                    .map_err(|_| {
                        AppError::new("app_error", "Stored book network domains are invalid")
                    })?;
                if let Value::List(items) = domains {
                    for item in items {
                        let key_chain = get(&item, "chain").unwrap_or(&null);
                        let key_network = get(&item, "network").unwrap_or(&null);
                        if py_eq(key_chain, chain) && py_eq(key_network, network) {
                            bound = get(&item, "domain_id").cloned().unwrap_or(Value::Null);
                        }
                    }
                }
            }
            bound
        };
    }
    if !truthy(&domain) {
        domain = Value::from(digest(&Value::List(vec![
            chain.clone(),
            network.clone(),
            Value::Null,
        ]))?);
    }
    Ok(py_str(&domain))
}

/// `filed_report_chain._observation(row)`.
fn observation(row: &Value) -> Result<Option<Value>> {
    let Some((_, _, confirmed)) = confirmation_observation(row)? else {
        return Ok(None);
    };
    let payload = stored_tx_mapping(field(row, "raw_json"), true).unwrap_or(Value::Dict(vec![]));
    let status = get(&payload, "status")
        .filter(|v| truthy(v))
        .cloned()
        .unwrap_or(Value::Dict(vec![]));
    let block_hash = match get(&status, "block_hash") {
        Some(Value::Str(text))
            if text.code_points().count() == 64
                && text
                    .code_points()
                    .all(|c| char::from_u32(c).is_some_and(|c| c.is_ascii_hexdigit())) =>
        {
            Value::from(py_str(&Value::Str(text.clone())).to_ascii_lowercase())
        }
        _ => Value::Null,
    };
    let height = match get(&status, "block_height") {
        Some(height @ Value::Int(_)) if confirmed => height.clone(),
        _ => Value::Null,
    };
    Ok(Some(dict([
        ("status", s("observed")),
        ("confirmed", Value::Bool(confirmed)),
        (
            "block_hash",
            if confirmed { block_hash } else { Value::Null },
        ),
        ("block_height", height),
        (
            "quantity_hash",
            field(row, "observation_quantity_hash").clone(),
        ),
    ])))
}

fn unavailable(reason: &str) -> Value {
    dict([("status", s("unavailable")), ("reason", s(reason))])
}

/// `filed_report_chain._current(conn, dependency)`.
pub fn current(conn: &Connection, dependency: &Value) -> Result<Value> {
    let profile = py_str(field(dependency, "profile_id"));
    let guarded = dict([
        ("chain", field(dependency, "chain").clone()),
        ("network", field(dependency, "network").clone()),
    ]);
    if kassiber_setup::networks::guard_observation(conn, &profile, &guarded, "report_dependency")
        .is_err()
    {
        return Ok(unavailable("domain_changed"));
    }
    let found = rows(
        conn,
        &format!("{OBSERVATION_SELECT} WHERE t.profile_id=? AND t.wallet_id=? AND LOWER(t.external_id)=? AND t.excluded=0"),
        params_from_iter([
            sql(field(dependency, "profile_id")),
            sql(field(dependency, "wallet_id")),
            sql(field(dependency, "txid")),
        ]),
    )?;
    let original: Vec<&Value> = found
        .iter()
        .filter(|row| py_eq(field(row, "id"), field(dependency, "transaction_id")))
        .collect();
    let candidates: Vec<&Value> = if original.is_empty() {
        found.iter().collect()
    } else {
        original
    };
    let [row] = candidates.as_slice() else {
        return Ok(unavailable("observation_missing"));
    };
    let Some((scope, _, _)) = confirmation_observation(row)? else {
        return Ok(unavailable("authority_unavailable"));
    };
    let expected = (
        or_text(field(dependency, "chain")),
        or_text(field(dependency, "network")),
        or_text(field(dependency, "txid")),
    );
    if scope != expected
        || !matches!(field(dependency, "chain"), Value::Str(_))
        || !matches!(field(dependency, "network"), Value::Str(_))
        || !matches!(field(dependency, "txid"), Value::Str(_))
    {
        return Ok(unavailable("authority_unavailable"));
    }
    match observation(row)? {
        Some(mut observed) => {
            let domain = domain_id(
                conn,
                &profile,
                row,
                field(dependency, "chain"),
                field(dependency, "network"),
            )?;
            kassiber_envelope::set(&mut observed, "domain_id", Value::from(domain));
            Ok(observed)
        }
        None => Ok(unavailable("authority_unavailable")),
    }
}

pub fn last_observation(conn: &Connection, dependency: &Value) -> Result<Value> {
    let row = first(
        conn,
        "SELECT inbox.observation_json FROM filed_report_chain_impacts impact
        JOIN chain_analysis_watch_inbox inbox ON inbox.id=impact.inbox_id
        WHERE impact.snapshot_id=? AND impact.transaction_id=? ORDER BY inbox.rowid DESC LIMIT 1",
        params_from_iter([
            sql(field(dependency, "snapshot_id")),
            sql(field(dependency, "transaction_id")),
        ]),
    )?;
    let invalid = || AppError::new("app_error", "Stored report observation is invalid");
    Ok(match row {
        Some(row) => {
            let decoded = kassiber_pytext::loads(&or_text(field(&row, "observation_json")))
                .map_err(|_| invalid())?;
            get(&decoded, "after").cloned().ok_or_else(invalid)?
        }
        None => kassiber_pytext::loads(&or_text(field(dependency, "observation_json")))
            .map_err(|_| invalid())?,
    })
}

fn status(value: &Value) -> Result<Value> {
    get(value, "status")
        .cloned()
        .ok_or_else(|| AppError::new("app_error", "Stored report observation has no status"))
}

/// `record_wallet_changes(conn, profile_id, wallet_id, retracted_txids=...)`.
pub fn record_wallet_changes(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    retracted: &[String],
    new_id: &mut dyn FnMut() -> String,
    now: &mut dyn FnMut() -> String,
) -> Result<i64> {
    super::row::require_transaction(conn)?;
    let retracted: Vec<String> = retracted
        .iter()
        .map(|v| kassiber_pytext::lower(v))
        .collect();
    let mut count = 0;
    let mut observations: Vec<([Value; 4], Value)> = Vec::new();
    for dependency in rows(
        conn,
        "SELECT * FROM filed_report_chain_dependencies WHERE profile_id=? AND wallet_id=?",
        [profile_id, wallet_id],
    )? {
        let before = last_observation(conn, &dependency)?;
        let key = [
            field(&dependency, "transaction_id").clone(),
            field(&dependency, "chain").clone(),
            field(&dependency, "network").clone(),
            field(&dependency, "txid").clone(),
        ];
        let cached = observations
            .iter()
            .find(|(k, _)| k.iter().zip(&key).all(|(a, b)| py_eq(a, b)))
            .map(|(_, v)| v.clone());
        let mut after = match cached {
            Some(value) => value,
            None => {
                let value = current(conn, &dependency)?;
                observations.push((key, value.clone()));
                value
            }
        };
        let unavailable_after = status(&after)? == Value::from("unavailable");
        let txid_retracted = matches!(field(&dependency, "txid"), Value::Str(_))
            && retracted.contains(&py_str(field(&dependency, "txid")));
        if unavailable_after && txid_retracted {
            after = dict([
                ("status", s("retracted")),
                ("reason", s("authoritative_wallet_retraction")),
            ]);
        } else if unavailable_after && py_eq(&status(&before)?, &Value::from("retracted")) {
            continue;
        }
        let after_status = status(&after)?;
        let before_confirmed = get(&before, "confirmed").map(truthy).unwrap_or(false);
        let after_confirmed = get(&after, "confirmed").map(truthy).unwrap_or(false);
        let code = if after_status == Value::from("retracted") {
            "report_input_retracted"
        } else if after_status == Value::from("unavailable") {
            "coverage_lost"
        } else if before_confirmed && !after_confirmed {
            "threshold_reversed"
        } else {
            "evidence_changed"
        };
        if py_eq(&before, &after) {
            continue;
        }
        append(conn, &dependency, before, after, code, new_id, now)?;
        count += 1;
    }
    Ok(count)
}

/// `filed_report_chain._append` after its equality check.
fn append(
    conn: &Connection,
    dependency: &Value,
    before: Value,
    after: Value,
    code: &str,
    new_id: &mut dyn FnMut() -> String,
    now: &mut dyn FnMut() -> String,
) -> Result<()> {
    let ident = new_id();
    let event = new_id();
    let timestamp = now();
    conn.execute(
        "INSERT INTO chain_analysis_watch_inbox
        (id,watch_id,profile_id,sequence,code,observation_json,created_at)
        VALUES(?,NULL,?,0,?,?,?)",
        params_from_iter([
            Sql::Text(event.clone()),
            sql(field(dependency, "profile_id")),
            Sql::Text(code.into()),
            Sql::Text(compact(&dict([("before", before), ("after", after)]))?),
            Sql::Text(timestamp.clone()),
        ]),
    )
    .map_err(database_error)?;
    conn.execute(
        "INSERT INTO filed_report_chain_impacts
        (id,snapshot_id,profile_id,transaction_id,inbox_id,created_at) VALUES(?,?,?,?,?,?)",
        params_from_iter([
            Sql::Text(ident),
            sql(field(dependency, "snapshot_id")),
            sql(field(dependency, "profile_id")),
            sql(field(dependency, "transaction_id")),
            Sql::Text(event),
            Sql::Text(timestamp),
        ]),
    )
    .map_err(database_error)?;
    Ok(())
}

/// `resolve_protocol_scope({"chain": chain, "network": network})` for the
/// Bitcoin/Liquid physical subjects a confirmation watch can name.
fn protocol_scope(chain: &str, network: &str) -> Option<(String, String)> {
    let chain = crate::descriptors::normalize_chain(chain).ok()?;
    let network = crate::descriptors::normalize_network(&chain, network).ok()?;
    Some((chain, network))
}

/// `record_watch_change(conn, profile_id, definition, after, code)`: only an
/// existing physical confirmation watch attaches uncertainty to saved reports.
pub fn record_watch_change(
    conn: &Connection,
    profile_id: &str,
    definition: &Value,
    _after: &Value,
    code: &str,
    new_id: &mut dyn FnMut() -> String,
    now: &mut dyn FnMut() -> String,
) -> Result<()> {
    if field(definition, "rule") != &s("confirmations")
        || !["coverage_lost", "threshold_reversed"].contains(&code)
    {
        return Ok(());
    }
    let query = field(definition, "query");
    let subject = kassiber_pytext::lower(kassiber_pytext::strip(&py_str(
        get(query, "subject").unwrap_or(&s("")),
    )));
    let text = |key: &str| get(query, key).map(py_str);
    let Some(identity) = crate::analysis_index::occurrences::parse_transaction_identity(
        &subject,
        text("chain").as_deref(),
        text("network").as_deref(),
    ) else {
        return Ok(());
    };
    let part = |n: usize| match &identity {
        Value::List(items) => items[n].clone(),
        _ => Value::Null,
    };
    let (chain, network, txid) = (py_str(&part(0)), py_str(&part(1)), py_str(&part(2)));
    let (domain, occurrence) = (part(3), part(4));
    let physical = protocol_scope(&chain, &network);
    let filtered = protocol_scope(
        &text("chain").unwrap_or_else(|| chain.clone()),
        &text("network").unwrap_or_else(|| network.clone()),
    );
    let (Some(physical), Some(filtered)) = (physical, filtered) else {
        return Ok(());
    };
    if physical != filtered {
        return Ok(());
    }
    let network = physical.1;
    let mut resolver =
        crate::analysis_index::occurrences::OccurrenceResolver::read(conn, profile_id)?;
    for dependency in rows(
        conn,
        "SELECT * FROM filed_report_chain_dependencies WHERE profile_id=? AND chain=? AND network=? AND txid=?",
        [profile_id, chain.as_str(), network.as_str(), txid.as_str()],
    )? {
        let captured = kassiber_pytext::loads(&or_text(field(&dependency, "observation_json")))
            .map_err(|error| AppError::new("internal_error", error.to_string()))?;
        let captured_domain = field(&captured, "domain_id").clone();
        let candidate = dict([
            ("chain", s(chain.as_str())),
            ("network", s(network.as_str())),
            ("domain_id", captured_domain.clone()),
        ]);
        if !resolver.accepts(&candidate, false) {
            continue;
        }
        if domain != Value::Null {
            if !py_eq(&domain, &captured_domain) {
                continue;
            }
            let source = crate::analysis_index::occurrences::SourceIdentity {
                domain_id: domain.clone(),
                namespace: format!("domain:{}:", py_str(&domain)),
                occurrence_id: None,
                trusted_reference: false,
            };
            if resolver.transaction_id(&chain, &network, &txid, &source, &occurrence, false)?
                != subject
            {
                continue;
            }
        }
        if let Value::Str(text) = &occurrence {
            let block = py_str(&Value::Str(text.clone()));
            let block = block.split(':').next().unwrap_or("");
            if !py_eq(&s(block), field(&captured, "block_hash")) {
                continue;
            }
        }
        let uncertain = dict([
            ("status", s("unavailable")),
            (
                "reason",
                s(if code == "coverage_lost" {
                    "watch_coverage_lost"
                } else {
                    "watch_confirmation_reversed"
                }),
            ),
        ]);
        let before = last_observation(conn, &dependency)?;
        if py_eq(&before, &uncertain) {
            continue;
        }
        append(conn, &dependency, before, uncertain, code, new_id, now)?;
    }
    Ok(())
}

/// `custody_filed_reports._json_object`.
fn json_object(value: &Value) -> Value {
    kassiber_pytext::loads(&if truthy(value) {
        py_str(value)
    } else {
        "{}".into()
    })
    .ok()
    .filter(|v| matches!(v, Value::Dict(_)))
    .unwrap_or_else(|| Value::Dict(vec![]))
}

/// `inbox_report_impact(conn, profile_id, inbox_id)`.
pub fn inbox_report_impact(
    conn: &Connection,
    profile_id: &str,
    inbox_id: &Value,
) -> Result<Option<Value>> {
    let Some(row) = first(
        conn,
        "SELECT impact.*, resolution.rebuilt_at, resolution.summary_json
        FROM filed_report_chain_impacts impact LEFT JOIN filed_report_chain_resolutions resolution ON resolution.impact_id=impact.id
        WHERE impact.profile_id=? AND impact.inbox_id=?",
        params_from_iter([Sql::Text(profile_id.into()), sql(inbox_id)]),
    )?
    else {
        return Ok(None);
    };
    let snapshot = first(
        conn,
        "SELECT * FROM filed_report_snapshots WHERE id = ?",
        params_from_iter([sql(field(&row, "snapshot_id"))]),
    )?
    .filter(|snapshot| field(snapshot, "profile_id") == &s(profile_id))
    .ok_or_else(|| AppError::new("not_found", "filed report snapshot was not found"))?;
    let available = first(
        conn,
        "SELECT 1 FROM transactions WHERE profile_id=? AND id=?",
        params_from_iter([
            Sql::Text(profile_id.into()),
            sql(field(&row, "transaction_id")),
        ]),
    )?
    .is_some();
    let resolution = if truthy(field(&row, "summary_json")) {
        kassiber_pytext::loads(&py_str(field(&row, "summary_json")))
            .map_err(|error| AppError::new("internal_error", error.to_string()))?
    } else {
        Value::Null
    };
    Ok(Some(dict([
        ("id", field(&row, "id").clone()),
        ("snapshot_id", field(&snapshot, "id").clone()),
        ("report_kind", field(&snapshot, "report_kind").clone()),
        ("report_state", field(&snapshot, "report_state").clone()),
        ("content_sha256", field(&snapshot, "content_sha256").clone()),
        (
            "period_start_year",
            field(&snapshot, "period_start_year").clone(),
        ),
        (
            "period_end_year",
            field(&snapshot, "period_end_year").clone(),
        ),
        (
            "report_scope",
            json_object(field(&snapshot, "report_scope_json")),
        ),
        ("transaction_id", field(&row, "transaction_id").clone()),
        ("transaction_available", Value::Bool(available)),
        (
            "before_gain_summary",
            json_object(field(&snapshot, "gain_summary_json")),
        ),
        ("resolution", resolution),
        ("rebuilt_at", field(&row, "rebuilt_at").clone()),
        (
            "filing_review_required",
            Value::Bool(field(&snapshot, "report_state") == &s("filed")),
        ),
    ])))
}

#[cfg(test)]
mod tests;
