//! Automatic saved-export completion (`register_saved_report_export`). The
//! caller holds the admitted write transaction from preparation through this
//! completion, so hashes, summaries and dependencies describe one book state.
//! A saved marker never files a report; only explicit user action does.
use super::*;
use kassiber_custody::confirmation;
use kassiber_pyjson::BigInt;
use kassiber_setup::Sources;
use kassiber_store::rusqlite::{params, types::ValueRef};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const NOTE: &str = "Automatically registered after a completed Kassiber export.";
const HOLDINGS: &[&str] = &[
    "full-report.csv",
    "full-report.xlsx",
    "full-report.pdf",
    "summary.pdf",
    "year-handoff.xlsx",
];

/// What a prepared artifact registers once it is published.
#[derive(Clone, Debug)]
pub struct Completion {
    pub kind: String,
    /// The report's resolved tax year, or `None` for the journal year range.
    pub year: Option<Value>,
    /// The last year of a multi-year period starting at `year`.
    pub end_year: Option<Value>,
    pub scope: Value,
}

fn invalid(message: &str, field: &str) -> AppError {
    AppError::new("filed_report_snapshot_validation", message)
        .with_details(dict([("field", s(field))]))
}
/// `str(value or "")`.
fn or_text(v: &Value) -> String {
    if truthy(v) {
        py_str(v)
    } else {
        String::new()
    }
}
fn decimal_digits(text: &str) -> Option<i64> {
    let mut year = 0;
    for c in text.chars() {
        year = year * 10 + i64::from(kassiber_pytext::decimal_value(u32::from(c))?);
    }
    Some(year)
}
/// `_entry_year`; `at` is the exact Austrian policy comparison of the caller.
fn entry_year(occurred_at: &Value, at: bool) -> Option<i64> {
    let text = or_text(occurred_at);
    if at {
        return calendar::vienna(&text).ok().map(|(year, _)| year);
    }
    let head: String = text.chars().take(4).collect();
    if head.chars().count() < 4 {
        return None;
    }
    decimal_digits(&head)
}
fn country(profile: &Value) -> String {
    kassiber_pytext::strip(&or_text(value(profile, "tax_country"))).to_lowercase()
}
fn profile_row(conn: &Connection, id: &str) -> Result<Value> {
    rows(
        conn,
        "SELECT fiat_currency,tax_country FROM profiles WHERE id=?",
        [id],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::new("not_found", "profile was not found"))
}

/// `report_period_years`: the explicit year, else the journal's year range.
pub(crate) fn period_years(
    conn: &Connection,
    profile_id: &str,
    year: Option<&Value>,
    fallback_at: &str,
) -> Result<(i64, i64)> {
    if let Some(year) = year.filter(|v| !matches!(v, Value::Null)) {
        let start = snapshot_write::year(year, "period_start_year")?;
        let end = snapshot_write::year(year, "period_end_year")?;
        return Ok((start, end));
    }
    let at = country(&profile_row(conn, profile_id)?) == "at";
    let years: Vec<i64> = rows(
        conn,
        "SELECT occurred_at FROM journal_entries WHERE profile_id = ?",
        [profile_id],
    )?
    .iter()
    .filter_map(|row| entry_year(value(row, "occurred_at"), at))
    .collect();
    if let (Some(start), Some(end)) = (years.iter().min(), years.iter().max()) {
        return Ok((*start, *end));
    }
    let head: String = fallback_at.chars().take(4).collect();
    if head.chars().count() < 4 || decimal_digits(&head).is_none() {
        return Err(invalid(
            "fallback_at must contain a calendar year",
            "fallback_at",
        ));
    }
    let year = snapshot_write::year(&s(head), "fallback_at")?;
    Ok((year, year))
}

/// Python `int(value or 0)` for a stored journal quantity.
pub(super) fn integer(v: &Value) -> Result<BigInt> {
    let error = || AppError::new("internal_error", "invalid stored journal quantity");
    match v {
        _ if !truthy(v) => Ok(0.into()),
        Value::Int(n) => Ok(n.clone()),
        Value::Bool(b) => Ok(i64::from(*b).into()),
        Value::Float(f) if f.is_finite() => {
            kassiber_pytext::parse_int(&format!("{:.0}", f.trunc())).map_err(|_| error())
        }
        Value::Str(t) => t
            .as_str()
            .and_then(|t| kassiber_pytext::parse_int(t).ok())
            .ok_or_else(error),
        _ => Err(error()),
    }
}
/// `Decimal(str(raw or 0))`, preferring the exact column.
fn exact(row: &Value, exact: &str, fallback: &str) -> Result<Decimal> {
    let mut raw = value(row, exact);
    if matches!(raw, Value::Null) || *raw == s("") {
        raw = value(row, fallback);
    }
    let text = if truthy(raw) { py_str(raw) } else { "0".into() };
    kassiber_pytext::strip(&text)
        .parse()
        .map_err(|_| AppError::new("internal_error", "[<class 'decimal.ConversionSyntax'>]"))
}

/// `current_report_summaries`: bounded exact summaries of the current journals.
pub fn summaries(
    conn: &Connection,
    profile_id: &str,
    start: i64,
    end: i64,
    scope: &Value,
) -> Result<(Value, Value)> {
    let profile = profile_row(conn, profile_id)?;
    let at = country(&profile) == "at";
    let wallets: Vec<String> = artifact_csv::list(value(scope, "wallet_ids"))
        .iter()
        .map(py_str)
        .collect();
    let first = or_text(value(scope, "occurred_at_start"));
    let last = or_text(value(scope, "occurred_at_end"));
    let entries = rows(
        conn,
        "SELECT je.occurred_at, je.wallet_id, je.entry_type, je.quantity,
               je.proceeds, je.proceeds_exact, je.cost_basis, je.cost_basis_exact,
               je.gain_loss, je.gain_loss_exact, je.at_category,
               COALESCE(t.taxability_override, 1) AS taxability
        FROM journal_entries je
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.profile_id = ?
        ORDER BY je.occurred_at, je.created_at, je.id",
        [profile_id],
    )?;
    let mut buckets: Vec<(String, BigInt, BigInt)> = Vec::new();
    let zero = Decimal::zero();
    let (mut proceeds, mut basis, mut gain) = (zero.clone(), zero.clone(), zero);
    for row in &entries {
        let occurred = or_text(value(row, "occurred_at"));
        if !wallets.is_empty() && !wallets.contains(&py_str(value(row, "wallet_id"))) {
            continue;
        }
        if !first.is_empty() && occurred < first {
            continue;
        }
        if !last.is_empty() && occurred > last {
            continue;
        }
        match entry_year(value(row, "occurred_at"), at) {
            Some(year) if (start..=end).contains(&year) => (),
            _ => continue,
        }
        let kind = snapshot_write::token(value(row, "entry_type"), "journal_entry.entry_type")?;
        let quantity = integer(value(row, "quantity"))?;
        let quantity = if quantity < 0.into() {
            -quantity
        } else {
            quantity
        };
        match buckets.iter_mut().find(|(k, _, _)| *k == kind) {
            Some((_, count, amount)) => {
                *count += 1;
                *amount += quantity;
            }
            None => buckets.push((kind.clone(), 1.into(), quantity)),
        }
        if !["disposal", "income"].contains(&kind.as_str())
            || integer(value(row, "taxability"))? == 0.into()
            || or_text(value(row, "at_category")) == "neu_swap"
        {
            continue;
        }
        for (total, field) in [
            (&mut proceeds, "proceeds"),
            (&mut basis, "cost_basis"),
            (&mut gain, "gain_loss"),
        ] {
            *total = total
                .add(&exact(row, &format!("{field}_exact"), field)?, &DECIMAL)
                .map_err(arithmetic)?;
        }
    }
    let classifications = dict(buckets.into_iter().map(|(kind, count, amount)| {
        (
            kind,
            dict([
                ("count", Value::Int(count)),
                ("amount_msat", Value::Int(amount)),
            ]),
        )
    }));
    let fixed = |d: &Decimal| d.to_fixed_string().map(s).map_err(arithmetic);
    let gains = dict([
        (
            "fiat_currency",
            // The validator strips and upper-cases exactly as `.upper()` did.
            s(or_text(value(&profile, "fiat_currency"))),
        ),
        ("proceeds_exact", fixed(&proceeds)?),
        ("cost_basis_exact", fixed(&basis)?),
        ("gain_loss_exact", fixed(&gain)?),
        ("status", s("final")),
    ]);
    Ok((
        snapshot_write::classifications(&classifications)?,
        snapshot_write::gains(&gains)?,
    ))
}

fn file_sha256(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|e| AppError::new("internal_error", e.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
/// `artifact_content_sha256`: one file's digest, or a bundle manifest digest.
pub fn content_sha256(paths: &[PathBuf]) -> Result<String> {
    if paths.is_empty() {
        return Err(invalid(
            "at least one exported artifact is required",
            "paths",
        ));
    }
    if paths.iter().any(|path| !path.is_file()) {
        return Err(invalid("exported artifact is not a regular file", "paths"));
    }
    if let [path] = paths {
        return file_sha256(path);
    }
    let mut manifest = BTreeMap::new();
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if manifest.contains_key(&name) {
            return Err(invalid("bundle artifact basenames must be unique", "paths"));
        }
        let size = path
            .metadata()
            .map_err(|e| AppError::new("internal_error", e.to_string()))?
            .len();
        manifest.insert(
            name.clone(),
            dict([
                ("bytes", Value::Int(size.into())),
                ("name", s(name)),
                ("sha256", s(file_sha256(path)?)),
            ]),
        );
    }
    let encoded = kassiber_pyjson::dumps(
        &Value::List(manifest.into_values().collect()),
        &kassiber_pyjson::Options {
            ensure_ascii: false,
            sort_keys: true,
            separators: Some((",".into(), ":".into())),
            ..Default::default()
        },
    )
    .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(encoded.as_bytes())))
}

fn json_rows(conn: &Connection, query: &str, profile_id: &str) -> Result<Vec<serde_json::Value>> {
    let mut stmt = conn.prepare(query).map_err(sql_error)?;
    let names: Vec<String> = stmt.column_names().iter().map(|n| n.to_string()).collect();
    let result = stmt
        .query_map([profile_id], |row| {
            let mut fields = serde_json::Map::new();
            for (i, name) in names.iter().enumerate() {
                let v = match row.get_ref(i)? {
                    ValueRef::Null | ValueRef::Blob(_) => serde_json::Value::Null,
                    ValueRef::Integer(n) => n.into(),
                    ValueRef::Real(n) => serde_json::Number::from_f64(n)
                        .map(serde_json::Value::Number)
                        .unwrap_or(serde_json::Value::Null),
                    ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned().into(),
                };
                // sqlite3.Row lookups return the first column of a name.
                fields.entry(name.clone()).or_insert(v);
            }
            Ok(serde_json::Value::Object(fields))
        })
        .map_err(sql_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    Ok(result)
}

/// `OccurrenceResolver(conn, profile).source(row, chain, network).domain_id`.
struct Domains(BTreeMap<(String, String), Value>);
impl Domains {
    fn load(conn: &Connection, profile_id: &str) -> Result<Self> {
        let bound: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='book_network_bindings')",
                [],
                |r| r.get(0),
            )
            .map_err(sql_error)?;
        let mut domains = BTreeMap::new();
        if bound {
            for row in rows(
                conn,
                "SELECT domains_json FROM book_network_bindings WHERE profile_id=?",
                [profile_id],
            )? {
                let parsed = kassiber_pytext::loads(&text(&row, "domains_json"))
                    .map_err(|e| AppError::new("internal_error", e.to_string()))?;
                for item in artifact_csv::list(&parsed) {
                    domains.insert((text(item, "chain"), text(item, "network")), item.clone());
                }
            }
        }
        Ok(Self(domains))
    }
    fn domain(&self, row: &serde_json::Value, chain: &str, network: &str) -> Result<Value> {
        let mapping = |raw: &serde_json::Value| -> Value {
            confirmation::stored_mapping(raw, false)
                .map(|v| kassiber_custody::codec::from_json(&v).unwrap_or(Value::Null))
                .unwrap_or(Value::Null)
        };
        let raw = mapping(&row["raw_json"]);
        let config = mapping(&row["wallet_config_json"]);
        let instance = [
            value(&raw, "chain_instance_id"),
            value(&config, "chain_instance_id"),
        ]
        .into_iter()
        .find(|v| truthy(v))
        .cloned();
        let digest = |instance: Value| -> Result<String> {
            let text = kassiber_pyjson::dumps(
                &Value::List(vec![s(chain), s(network), instance]),
                &kassiber_pyjson::Options {
                    sort_keys: true,
                    separators: Some((",".into(), ":".into())),
                    ..Default::default()
                },
            )
            .map_err(|e| AppError::new("internal_error", e.to_string()))?;
            Ok(format!("{:x}", Sha256::digest(text.as_bytes())))
        };
        let bound = self
            .0
            .get(&(chain.to_owned(), network.to_owned()))
            .map(|item| value(item, "domain_id").clone())
            .unwrap_or(Value::Null);
        let domain = match instance {
            Some(instance) => s(digest(instance)?),
            None => bound,
        };
        if truthy(&domain) {
            Ok(domain)
        } else {
            digest(Value::Null).map(s)
        }
    }
}

/// `capture_export_dependencies`: freeze booked authoritative observations.
pub(crate) fn capture_dependencies(
    conn: &Connection,
    snapshot: &Value,
    captured_at: &str,
) -> Result<()> {
    let profile_id = text(snapshot, "profile_id");
    let snapshot_id = text(snapshot, "id");
    // Unlike the summaries, capture compares the stored policy literally.
    let at = rows(
        conn,
        "SELECT tax_country FROM profiles WHERE id=?",
        [&profile_id],
    )?
    .first()
    .is_some_and(|row| *value(row, "tax_country") == s("at"));
    let scope = value(snapshot, "report_scope");
    let wallets: BTreeSet<String> = artifact_csv::list(value(scope, "wallet_ids"))
        .iter()
        .map(py_str)
        .collect();
    let first = or_text(value(scope, "occurred_at_start"));
    let last = or_text(value(scope, "occurred_at_end"));
    let (start, end) = (
        int(snapshot, "period_start_year"),
        int(snapshot, "period_end_year"),
    );
    let holdings = HOLDINGS.contains(&text(snapshot, "report_kind").as_str());
    let entries = rows(
        conn,
        "SELECT transaction_id,wallet_id,asset,occurred_at FROM journal_entries WHERE profile_id=?",
        [&profile_id],
    )?;
    let occurred = |row: &Value| py_str(value(row, "occurred_at"));
    let before_end = |row: &Value| last.is_empty() || occurred(row) <= last;
    let mut assets = BTreeSet::new();
    for row in &entries {
        let Some(year) = entry_year(value(row, "occurred_at"), at) else {
            continue;
        };
        if year <= end
            && (holdings || start <= year)
            && (wallets.is_empty() || wallets.contains(&py_str(value(row, "wallet_id"))))
            && (holdings || first.is_empty() || occurred(row) >= first)
            && before_end(row)
        {
            assets
                .insert(kassiber_pyjson::dumps(value(row, "asset"), &Default::default()).unwrap());
        }
    }
    let asset_key =
        |row: &Value| kassiber_pyjson::dumps(value(row, "asset"), &Default::default()).unwrap();
    let id_key = |v: &Value| kassiber_pyjson::dumps(v, &Default::default()).unwrap();
    let mut included = BTreeSet::new();
    for row in &entries {
        if assets.contains(&asset_key(row))
            && entry_year(value(row, "occurred_at"), at).is_some_and(|year| year <= end)
            && before_end(row)
        {
            included.insert(id_key(value(row, "transaction_id")));
        }
    }
    // Follow eligible carrying relations backwards into their source pools.
    let relations = rows(
        conn,
        "SELECT out_transaction_id,in_transaction_id,out_asset,occurred_at
        FROM journal_custody_projection_relations WHERE profile_id=?
        AND policy='carrying-value' AND basis_state='eligible'
        AND in_transaction_id IS NOT NULL AND out_asset != in_asset
        ORDER BY occurred_at DESC,id",
        [&profile_id],
    )?;
    let booked: BTreeSet<String> = entries
        .iter()
        .map(|row| id_key(value(row, "transaction_id")))
        .collect();
    let mut changed = true;
    while changed {
        changed = false;
        for relation in &relations {
            let out = id_key(value(relation, "out_transaction_id"));
            if !included.contains(&id_key(value(relation, "in_transaction_id")))
                || !booked.contains(&out)
                || included.contains(&out)
            {
                continue;
            }
            let cutoff = value(relation, "occurred_at");
            if !truthy(cutoff) {
                continue;
            }
            let cutoff = py_str(cutoff);
            let out_asset = id_key(value(relation, "out_asset"));
            for entry in &entries {
                if asset_key(entry) == out_asset
                    && occurred(entry) <= cutoff
                    && included.insert(id_key(value(entry, "transaction_id")))
                {
                    changed = true;
                }
            }
        }
    }
    let domains = Domains::load(conn, &profile_id)?;
    let observations = json_rows(
        conn,
        "SELECT t.*, w.kind AS wallet_kind, w.config_json AS wallet_config_json,
         o.authority_version AS observation_authority_version,
         o.graph_hash AS observation_graph_hash, o.quantity_hash AS observation_quantity_hash,
         o.observed_at AS observation_observed_at, o.observer_ids_json AS observation_observer_ids,
         o.observer_kinds_json AS observation_observer_kinds, o.fee_attribution AS observation_fee_attribution,
         o.chain AS observation_chain, o.network AS observation_network
         FROM transactions t JOIN wallets w ON w.id=t.wallet_id
         LEFT JOIN chain_observation_provenance o ON o.transaction_id=t.id
         WHERE t.profile_id=? AND t.excluded=0",
        &profile_id,
    )?;
    for row in &observations {
        let id = kassiber_custody::codec::from_json(&row["id"]).unwrap_or(Value::Null);
        if !included.contains(&id_key(&id)) {
            continue;
        }
        let observed = observed_instant(&row["observation_observed_at"]);
        let closed = confirmation::closed(row, observed).map_err(|refusal| match refusal {
            confirmation::Refusal::Evidence(message) => AppError::new("internal_error", message),
        })?;
        let Some(closed) = closed else { continue };
        let status = confirmation::stored_mapping(&row["raw_json"], true)
            .map(|payload| payload["status"].clone())
            .filter(kassiber_custody::codec::truth)
            .unwrap_or(serde_json::json!({}));
        let status = kassiber_custody::codec::from_json(&status).unwrap_or(Value::Null);
        let block_hash = match value(&status, "block_hash") {
            Value::Str(h) if closed.confirmed => h
                .as_str()
                .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
                .map(|h| s(h.to_ascii_lowercase()))
                .unwrap_or(Value::Null),
            _ => Value::Null,
        };
        let block_height = match value(&status, "block_height") {
            Value::Int(n) if closed.confirmed => Value::Int(n.clone()),
            _ => Value::Null,
        };
        let observation = dict([
            ("status", s("observed")),
            ("confirmed", Value::Bool(closed.confirmed)),
            ("block_hash", block_hash),
            ("block_height", block_height),
            (
                "quantity_hash",
                kassiber_custody::codec::from_json(&row["observation_quantity_hash"])
                    .unwrap_or(Value::Null),
            ),
            (
                "domain_id",
                domains.domain(row, &closed.chain, &closed.network)?,
            ),
        ]);
        conn.execute(
            "INSERT INTO filed_report_chain_dependencies
            (snapshot_id,profile_id,transaction_id,wallet_id,chain,network,txid,observation_json)
            VALUES(?,?,?,?,?,?,?,?)",
            params![
                snapshot_id,
                profile_id,
                row["id"].as_str(),
                row["wallet_id"].as_str(),
                closed.chain,
                closed.network,
                closed.txid,
                snapshot_write::json(&observation)?,
            ],
        )
        .map_err(sql_error)?;
    }
    conn.execute(
        "INSERT INTO filed_report_chain_captures(snapshot_id,profile_id,captured_at) VALUES(?,?,?)",
        params![snapshot_id, profile_id, captured_at],
    )
    .map_err(sql_error)?;
    Ok(())
}

/// `parse_iso_datetime_or_none(...) is not None`.
fn observed_instant(raw: &serde_json::Value) -> bool {
    let text = match raw {
        serde_json::Value::Null => return false,
        serde_json::Value::String(t) if t.is_empty() => return false,
        v => kassiber_custody::codec::pystr(v),
    };
    let text = kassiber_pytext::strip(&text);
    !text.is_empty() && kassiber_rates::time::parse(text).is_some()
}

/// `register_saved_report_export` inside the caller's admitted transaction.
pub(crate) fn register(
    conn: &Connection,
    workspace_id: &str,
    profile_id: &str,
    completion: &Completion,
    paths: &[PathBuf],
    sources: &dyn Sources,
) -> Result<Value> {
    let timestamp = sources.now(conn)?;
    let (start, end) = period_years(conn, profile_id, completion.year.as_ref(), &timestamp)?;
    let end = match &completion.end_year {
        Some(year) => {
            let end = snapshot_write::year(year, "period_end_year")?;
            if end < start {
                return Err(invalid(
                    "period_end_year must not precede period_start_year",
                    "period_end_year",
                ));
            }
            end
        }
        None => end,
    };
    let digest = content_sha256(paths)?;
    let scope = snapshot_write::scope(&completion.scope)?;
    let (classifications, gains) = summaries(conn, profile_id, start, end, &scope)?;
    let id = sources.uuid(conn)?;
    let args = dict([
        ("report_kind", s(completion.kind.as_str())),
        ("report_state", s("saved")),
        ("period_start_year", Value::Int(start.into())),
        ("period_end_year", Value::Int(end.into())),
        ("content_sha256", s(digest)),
        ("classification_summary", classifications),
        ("gain_summary", gains),
        ("report_scope", scope),
        ("authored_source", s("user")),
        ("notes", s(NOTE)),
    ]);
    let snapshot = snapshot_write::create_with(
        conn,
        workspace_id,
        profile_id,
        &args,
        &id,
        &timestamp,
        sources,
    )?;
    capture_dependencies(conn, &snapshot, &sources.now(conn)?)?;
    snapshots::get_snapshot(conn, &id, profile_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    struct Fixed(Cell<usize>);
    impl Sources for Fixed {
        fn uuid(&self, _: &Connection) -> Result<String> {
            Ok("new".into())
        }
        fn now(&self, _: &Connection) -> Result<String> {
            let calls = self.0.get();
            self.0.set(calls + 1);
            Ok(if calls == 0 {
                "2026-01-02T03:04:05Z"
            } else {
                "captured"
            }
            .into())
        }
    }
    fn encode(v: &Value) -> String {
        kassiber_pyjson::dumps(
            v,
            &kassiber_pyjson::Options {
                sort_keys: true,
                separators: Some((",".into(), ":".into())),
                ..Default::default()
            },
        )
        .unwrap()
    }
    fn error_value(e: AppError) -> Value {
        dict([(
            "error",
            dict([
                ("code", s(e.code)),
                ("message", s(e.message)),
                ("hint", e.hint.map(s).unwrap_or(Value::Null)),
                ("details", e.details.unwrap_or(Value::Null)),
                ("retryable", Value::Bool(e.retryable)),
            ]),
        )])
    }

    /// Malformed bytes produce no physical dependency, matching Python.
    #[test]
    fn graphless_liquid_raw_hex_invalid_has_no_dependency() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        let raw = r#"{"chain":"liquid","network":"liquidv1","raw_hex":"0200","status":{"confirmed":true}}"#;
        let txid = "b".repeat(64);
        let row = serde_json::json!({"wallet_id":"hot","external_id":txid,"direction":"inbound","asset":"LBTC","amount":5,"fee":0,"amount_includes_fee":0,"raw_json":raw});
        let (graph, _) = kassiber_custody::evidence::graph_hash(&row["raw_json"]).unwrap();
        let (quantity, _) = kassiber_custody::evidence::observed_hash(&row).unwrap();
        conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,excluded,raw_json,created_at) VALUES('hex','ws','p','hot',?1,'fp-hex','2025-01-01T00:00:00Z','inbound','LBTC',5,0,0,?2,'fixed')", params![txid, raw]).unwrap();
        conn.execute("INSERT INTO chain_observation_provenance(transaction_id,workspace_id,profile_id,wallet_id,authority_version,observer_ids_json,observer_kinds_json,chain,network,application_revision,graph_hash,quantity_hash,fee_attribution,observed_at,updated_at) VALUES('hex','ws','p','hot',1,'[]','[]','liquid','liquidv1','r',?1,?2,'unknown','2025-01-01T00:00:00Z','fixed')", params![graph, quantity]).unwrap();
        conn.execute_batch("INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,account_id,occurred_at,entry_type,asset,quantity,created_at) VALUES('je-hex','ws','p','hex','hot','acct','2025-01-01T00:00:00Z','acquisition','LBTC',5,'fixed')").unwrap();
        let file = root.path().join("report.csv");
        std::fs::write(&file, "complete").unwrap();
        let completion = Completion {
            kind: "full-report.csv".into(),
            year: None,
            end_year: None,
            scope: Value::Null,
        };
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        register(conn, "ws", "p", &completion, &[file], &Fixed(Cell::new(0))).unwrap();
        let dependencies: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM filed_report_chain_dependencies",
                [],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute_batch("ROLLBACK").unwrap();
        assert_eq!(dependencies, 0);
    }

    /// Python-generated registrations, including authoritative dependencies.
    #[test]
    fn export_completion_python_vectors() {
        let Value::List(cases) =
            kassiber_pytext::loads(include_str!("../tests/data/completion.json")).unwrap()
        else {
            panic!()
        };
        assert!(cases.len() >= 30);
        for case in cases {
            let name = text(&case, "name");
            let root = tempfile::tempdir().unwrap();
            let artifacts = root.path().join("artifacts");
            std::fs::create_dir_all(artifacts.join("folder")).unwrap();
            if let Value::Dict(files) = value(&case, "files") {
                for (file, content) in files {
                    let kassiber_pyjson::Key::Str(file) = file else {
                        panic!()
                    };
                    let path = artifacts.join(file.as_str().unwrap());
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(path, py_str(content)).unwrap();
                }
            }
            let paths: Vec<PathBuf> = artifact_csv::list(value(&case, "paths"))
                .iter()
                .map(|p| artifacts.join(py_str(p)))
                .collect();
            let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
            let conn = book.connection();
            conn.execute_batch(include_str!("../tests/data/book.sql"))
                .unwrap();
            conn.execute_batch(&text(&case, "sql")).unwrap();
            let completion = Completion {
                kind: text(&case, "kind"),
                end_year: None,
                year: Some(value(&case, "year").clone()).filter(|v| !matches!(v, Value::Null)),
                scope: value(&case, "scope").clone(),
            };
            conn.execute_batch("BEGIN IMMEDIATE").unwrap();
            let actual = match register(conn, "ws", "p", &completion, &paths, &Fixed(Cell::new(0)))
            {
                Ok(v) => {
                    conn.execute_batch("COMMIT").unwrap();
                    dict([("ok", v)])
                }
                Err(e) => {
                    conn.execute_batch("ROLLBACK").unwrap();
                    error_value(e)
                }
            };
            assert_eq!(encode(&actual), encode(value(&case, "expected")), "{name}");
            let Value::Dict(tables) = value(&case, "tables") else {
                panic!()
            };
            for (table, expected) in tables {
                let kassiber_pyjson::Key::Str(table) = table else {
                    panic!()
                };
                let table = table.as_str().unwrap();
                let actual = Value::List(
                    rows(conn, &format!("SELECT * FROM {table} ORDER BY 1,2,3"), []).unwrap(),
                );
                assert_eq!(encode(&actual), encode(expected), "{name}: {table}");
            }
        }
    }
}
