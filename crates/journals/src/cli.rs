//! The journal CLI leaves Python serves from `cli/handlers.py`:
//! `journals list`, `journals events list|get`, `journals transfers list`,
//! `journals quarantined` and `journals quarantine show|clear|resolve`.
//!
//! Each follows Python's query, ordering and scalar conversions after the
//! caller resolved the scope (`resolve_scope` comes first in every handler).
//! The reads never process journals. `clear` and `resolve` write one
//! transaction behind a reserved writer, with Python's journal invalidation.
use std::collections::HashMap;

use kassiber_envelope::{dict, py_str, s, AppError};
use kassiber_pyjson::{BigInt, Key, Value as Py};
use kassiber_store::rusqlite::{types::Value as Sql, Connection};

use crate::views::{btc, float_or_zero, int_or_zero, py, query, sql_error, truthy, Row};

type Result<T> = std::result::Result<T, AppError>;

/// `DEFAULT_EVENTS_LIMIT` and `MAX_EVENTS_LIMIT`.
pub const DEFAULT_EVENTS_LIMIT: i64 = 100;
const MAX_EVENTS_LIMIT: i64 = 1000;
const CURSOR_KEYS: [&str; 3] = ["occurred_at", "created_at", "id"];
const RESOLVE_NOTE: &str = "Run `kassiber journals process` to regenerate entries.";

/// The resolved `(workspace_id, profile_id)` of a command.
#[derive(Clone, Copy)]
pub struct Scope<'a> {
    pub workspace_id: &'a str,
    pub profile_id: &'a str,
}

/// Python `int(value)` for a stored value that must be present.
fn int_of(value: &Sql) -> Result<BigInt> {
    if matches!(value, Sql::Null) {
        return Err(AppError::new(
            "internal_error",
            "int() argument must be a string, a bytes-like object or a real number, not 'NoneType'",
        ));
    }
    int_or_zero(value)
}

/// `float(msat_to_btc(value))` for a Python integer.
fn btc_of(value: &BigInt) -> Result<f64> {
    btc(&Sql::Text(value.to_string()))
}

fn text_param(text: &str) -> Sql {
    Sql::Text(text.to_owned())
}

/// `_journal_cursor_filters(...)`.
fn cursor_filters(scope: Scope<'_>, filters: [&str; 6]) -> Py {
    let [wallet_id, account_id, asset, entry_type, start, end] = filters;
    dict([
        ("workspace_id", s(scope.workspace_id)),
        ("profile_id", s(scope.profile_id)),
        ("wallet_id", s(wallet_id)),
        ("account_id", s(account_id)),
        ("asset", s(asset)),
        ("entry_type", s(entry_type)),
        ("start", s(start)),
        ("end", s(end)),
    ])
}

/// `_decode_event_cursor(cursor, filters)` as the extra predicate and its
/// parameters.
fn cursor_clause(cursor: Option<&str>, filters: &Py) -> Result<Option<(String, Vec<Sql>)>> {
    let Some(row) =
        kassiber_transactions::metadata::decode_cursor(cursor, filters, &CURSOR_KEYS, false)?
    else {
        return Ok(None);
    };
    let field = |key: &str| {
        kassiber_transactions::metadata::cursor_bind(
            kassiber_envelope::get(&row, key).unwrap_or(&Py::Null),
        )
    };
    let (occurred, created, id) = (field("occurred_at")?, field("created_at")?, field("id")?);
    Ok(Some((
        "(je.occurred_at < ? OR \
         (je.occurred_at = ? AND je.created_at < ?) OR \
         (je.occurred_at = ? AND je.created_at = ? AND je.id < ?))"
            .to_owned(),
        vec![
            occurred.clone(),
            occurred.clone(),
            created.clone(),
            occurred,
            created,
            id,
        ],
    )))
}

/// `_encode_event_cursor(row, filters)` for the last row of a page.
fn next_cursor(row: &Row, created_at: &str, filters: &Py) -> Result<Py> {
    let fields = dict([
        ("occurred_at", py(row.get("occurred_at"))),
        ("created_at", py(row.get(created_at))),
        ("id", py(row.get("id"))),
    ]);
    kassiber_transactions::metadata::encode_cursor(&fields, filters, &CURSOR_KEYS)
}

/// `limit + 1` as SQLite binds it.
fn page_size(limit: &BigInt) -> Result<Sql> {
    i64::try_from(limit.clone() + BigInt::from(1))
        .map(Sql::Integer)
        .map_err(|_| {
            AppError::new(
                "internal_error",
                "Python int too large to convert to SQLite INTEGER",
            )
        })
}

/// One listed entry or event: Python's `dict(row)`, then `quantity_msat`
/// appended, `quantity` in BTC, and absent Austrian markers dropped.
fn entry(row: &Row, skip: &str) -> Result<Py> {
    let quantity = int_of(row.get("quantity"))?;
    let mut fields: Vec<(Key, Py)> = Vec::new();
    for (name, value) in row_fields(row) {
        if name == skip
            || (matches!(value, Sql::Null) && (name == "at_category" || name == "at_kennzahl"))
        {
            continue;
        }
        let value = if name == "quantity" {
            Py::Float(btc_of(&quantity)?)
        } else {
            py(value)
        };
        fields.push((Key::from(name), value));
    }
    fields.push((Key::from("quantity_msat"), Py::Int(quantity)));
    Ok(Py::Dict(fields))
}

/// A row's columns as `dict(row)` keeps them: `sqlite3.Row` looks a name up
/// by its first column, so a repeated name keeps its first position and value.
fn row_fields(row: &Row) -> Vec<(&str, &Sql)> {
    let mut fields: Vec<(&str, &Sql)> = Vec::new();
    for (name, value) in row.columns() {
        if !fields.iter().any(|(existing, _)| *existing == name) {
            fields.push((name, value));
        }
    }
    fields
}

/// `list_journal_entries(..., return_meta=True)`: the page and the envelope
/// meta (`next_cursor`, `has_more`, `limit`).
pub fn entries(
    conn: &Connection,
    scope: Scope<'_>,
    limit: Option<&BigInt>,
    cursor: Option<&str>,
) -> Result<(Py, Vec<(String, Py)>)> {
    if limit.is_some_and(|limit| *limit <= BigInt::from(0)) {
        return Err(AppError::new("validation", "--limit must be positive"));
    }
    let filters = cursor_filters(scope, [""; 6]);
    let mut params = vec![text_param(scope.profile_id)];
    let mut cursor_sql = String::new();
    if let Some((clause, values)) = cursor_clause(cursor, &filters)? {
        cursor_sql = format!("AND {clause}");
        params.extend(values);
    }
    let mut limit_sql = "";
    if let Some(limit) = limit {
        limit_sql = "LIMIT ?";
        params.push(page_size(limit)?);
    }
    let rows = query(
        conn,
        &format!(
            "SELECT
                je.id,
                je.occurred_at,
                je.created_at AS _created_at,
                w.label AS wallet,
                COALESCE(a.code, '') AS account,
                je.entry_type,
                je.asset,
                je.quantity,
                je.fiat_value,
                COALESCE(je.cost_basis, 0) AS cost_basis,
                COALESCE(je.proceeds, 0) AS proceeds,
                COALESCE(je.gain_loss, 0) AS gain_loss,
                COALESCE(je.description, '') AS description,
                je.at_category,
                je.at_kennzahl
            FROM journal_entries je
            JOIN wallets w ON w.id = je.wallet_id
            LEFT JOIN accounts a ON a.id = je.account_id
            WHERE je.profile_id = ? {cursor_sql}
            ORDER BY je.occurred_at DESC, je.created_at DESC, je.id DESC
            {limit_sql}"
        ),
        &params,
    )?;
    let page_len = match limit {
        Some(limit) => usize::try_from(limit.clone())
            .unwrap_or(usize::MAX)
            .min(rows.len()),
        None => rows.len(),
    };
    let has_more = limit.is_some() && rows.len() > page_len;
    let page = &rows[..page_len];
    let results = page
        .iter()
        .map(|row| entry(row, "_created_at"))
        .collect::<Result<Vec<_>>>()?;
    let next = match page.last() {
        Some(last) if has_more => next_cursor(last, "_created_at", &filters)?,
        _ => Py::Null,
    };
    let meta = vec![
        ("next_cursor".to_owned(), next),
        ("has_more".to_owned(), Py::Bool(has_more)),
        (
            "limit".to_owned(),
            limit.map_or(Py::Null, |limit| Py::Int(limit.clone())),
        ),
    ];
    Ok((Py::List(results), meta))
}

/// `journals events list` filters, as parsed.
#[derive(Default)]
pub struct EventFilters<'a> {
    pub wallet: Option<&'a str>,
    pub account: Option<&'a str>,
    pub asset: Option<&'a str>,
    pub entry_type: Option<&'a str>,
    pub start: Option<&'a str>,
    pub end: Option<&'a str>,
    pub cursor: Option<&'a str>,
    pub limit: Option<&'a BigInt>,
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.filter(|text| !text.is_empty())
}

/// `list_journal_events(...)`.
pub fn events(conn: &Connection, scope: Scope<'_>, filters: &EventFilters<'_>) -> Result<Py> {
    let limit = filters
        .limit
        .cloned()
        .unwrap_or_else(|| BigInt::from(DEFAULT_EVENTS_LIMIT));
    if limit <= BigInt::from(0) {
        return Err(AppError::new("validation", "--limit must be positive"));
    }
    if limit > BigInt::from(MAX_EVENTS_LIMIT) {
        return Err(AppError::new(
            "validation",
            format!("--limit cannot exceed {MAX_EVENTS_LIMIT}"),
        )
        .with_hint(format!(
            "Use cursor-based pagination instead of larger limits; max page size is {MAX_EVENTS_LIMIT}."
        )));
    }
    let mut conditions = vec!["je.profile_id = ?".to_owned()];
    let mut params = vec![text_param(scope.profile_id)];
    let bound = |raw: Option<&str>, field: &str| {
        kassiber_transactions::metadata::timestamp_bound(nonempty(raw), field)
    };
    let start = bound(filters.start, "start")?;
    let end = bound(filters.end, "end")?;
    let mut wallet_id = String::new();
    let mut account_id = String::new();
    if let Some(wallet) = nonempty(filters.wallet) {
        wallet_id = kassiber_wallets::resolve(conn, scope.profile_id, wallet)?.id;
        conditions.push("je.wallet_id = ?".into());
        params.push(text_param(&wallet_id));
    }
    if let Some(account) = nonempty(filters.account) {
        account_id = kassiber_wallets::resolve_account(conn, scope.profile_id, account)?;
        conditions.push("je.account_id = ?".into());
        params.push(text_param(&account_id));
    }
    let asset = nonempty(filters.asset).map_or_else(String::new, kassiber_pytext::upper);
    if !asset.is_empty() {
        conditions.push("upper(je.asset) = ?".into());
        params.push(text_param(&asset));
    }
    let entry_type = nonempty(filters.entry_type).map_or_else(String::new, kassiber_pytext::lower);
    if !entry_type.is_empty() {
        conditions.push("lower(je.entry_type) = ?".into());
        params.push(text_param(&entry_type));
    }
    for (bound, op) in [(&start, ">="), (&end, "<=")] {
        if !bound.is_empty() {
            conditions.push(kassiber_transactions::metadata::timestamp_predicate(
                conn,
                "je.occurred_at",
                op,
            )?);
            params.push(text_param(bound));
        }
    }
    let cursor_filters = cursor_filters(
        scope,
        [&wallet_id, &account_id, &asset, &entry_type, &start, &end],
    );
    if let Some((clause, values)) = cursor_clause(filters.cursor, &cursor_filters)? {
        conditions.push(clause);
        params.extend(values);
    }
    params.push(page_size(&limit)?);
    let rows = query(
        conn,
        &format!(
            "SELECT
                je.id,
                je.occurred_at,
                je.created_at,
                je.transaction_id,
                je.wallet_id,
                w.label AS wallet,
                je.account_id,
                COALESCE(a.code, '') AS account,
                COALESCE(a.label, '') AS account_label,
                je.entry_type,
                je.asset,
                je.quantity,
                je.fiat_value,
                je.unit_cost,
                COALESCE(je.cost_basis, 0) AS cost_basis,
                COALESCE(je.proceeds, 0) AS proceeds,
                COALESCE(je.gain_loss, 0) AS gain_loss,
                je.fiat_value_exact,
                je.unit_cost_exact,
                je.cost_basis_exact,
                je.proceeds_exact,
                je.gain_loss_exact,
                je.pricing_source_kind,
                je.pricing_quality,
                COALESCE(je.description, '') AS description,
                je.at_category,
                je.at_kennzahl
            FROM journal_entries je
            JOIN wallets w ON w.id = je.wallet_id
            LEFT JOIN accounts a ON a.id = je.account_id
            WHERE {}
            ORDER BY je.occurred_at DESC, je.created_at DESC, je.id DESC
            LIMIT ?",
            conditions.join(" AND ")
        ),
        &params,
    )?;
    let page_len = usize::try_from(limit.clone())
        .unwrap_or(usize::MAX)
        .min(rows.len());
    let has_more = rows.len() > page_len;
    let page = &rows[..page_len];
    let events = page
        .iter()
        .map(|row| entry(row, ""))
        .collect::<Result<Vec<_>>>()?;
    let next = match page.last() {
        Some(last) if has_more => next_cursor(last, "created_at", &cursor_filters)?,
        _ => Py::Null,
    };
    Ok(dict([
        ("events", Py::List(events)),
        ("next_cursor", next),
        ("has_more", Py::Bool(has_more)),
        ("limit", Py::Int(limit)),
    ]))
}

/// `core_journal_calculation.legacy_text(compact)`: the sorted legacy JSON,
/// or the compact value unchanged when it does not decode.
fn legacy_text(compact: &Sql) -> Result<Py> {
    let text = match compact {
        Sql::Text(text) => Some(text.clone()),
        Sql::Blob(bytes) => String::from_utf8(bytes.clone()).ok(),
        _ => None,
    };
    let Some(decoded) = text
        .as_deref()
        .and_then(kassiber_reports::decode_compact_calculation)
    else {
        return Ok(py(compact));
    };
    kassiber_pyjson::dumps(
        &decoded,
        &kassiber_pyjson::Options {
            sort_keys: true,
            ..Default::default()
        },
    )
    .map(s)
    .map_err(|error| AppError::new("internal_error", error.to_string()))
}

/// `get_journal_event(...)`.
pub fn event(conn: &Connection, scope: Scope<'_>, event_id: &str) -> Result<Py> {
    let row = query(
        conn,
        "SELECT
            je.*,
            w.label AS wallet,
            COALESCE(a.code, '') AS account,
            COALESCE(a.label, '') AS account_label,
            t.external_id AS transaction_external_id,
            t.direction AS transaction_direction,
            t.counterparty AS transaction_counterparty,
            t.note AS transaction_note
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN accounts a ON a.id = je.account_id
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.profile_id = ? AND je.id = ?",
        &[text_param(scope.profile_id), text_param(event_id)],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| {
        AppError::new("not_found", format!("Journal event '{event_id}' not found"))
            .with_hint("Run `kassiber journals events list` to find valid event ids.")
    })?;
    let compact = row_fields(&row)
        .into_iter()
        .find(|(name, _)| *name == "calculation_compact")
        .map(|(_, value)| value.clone())
        .unwrap_or(Sql::Null);
    let legacy = if truthy(&compact) {
        Some(legacy_text(&compact)?)
    } else {
        None
    };
    let quantity = int_of(row.get("quantity"))?;
    let mut fields: Vec<(Key, Py)> = Vec::new();
    for (name, value) in row_fields(&row) {
        let value = match name {
            "calculation_compact" => continue,
            "calculation_json" if legacy.is_some() => legacy.clone().expect("legacy text"),
            "quantity" => Py::Float(btc_of(&quantity)?),
            _ => py(value),
        };
        fields.push((Key::from(name), value));
    }
    if !fields
        .iter()
        .any(|(key, _)| *key == Key::from("calculation_json"))
    {
        if let Some(legacy) = legacy {
            fields.push((Key::from("calculation_json"), legacy));
        }
    }
    fields.push((Key::from("quantity_msat"), Py::Int(quantity)));
    Ok(Py::Dict(fields))
}

/// The scope's `profiles` row (`resolve_scope`'s profile).
fn profile_row(conn: &Connection, scope: Scope<'_>) -> Result<Row> {
    query(
        conn,
        "SELECT * FROM profiles WHERE id = ?",
        &[text_param(scope.profile_id)],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::new("not_found", "Profile not found"))
}

/// `_journal_processing_status(conn, profile)`.
fn processing_status(conn: &Connection, profile: &Row) -> Result<Py> {
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id = ? AND excluded = 0",
            [profile.get("id")],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    let freshness = kassiber_workspace::readiness::evaluate_freshness(&profile.to_py(), active)?;
    let field = |key: &str| {
        kassiber_envelope::get(&freshness, key)
            .cloned()
            .unwrap_or(Py::Null)
    };
    let stored = |key: &str| -> Result<Py> {
        Ok(if profile.has(key) {
            Py::Int(int_or_zero(profile.get(key))?)
        } else {
            Py::Int(0.into())
        })
    };
    Ok(dict([
        ("last_processed_at", py(profile.get("last_processed_at"))),
        (
            "last_processed_tx_count",
            stored("last_processed_tx_count")?,
        ),
        ("journal_input_version", stored("journal_input_version")?),
        (
            "last_processed_input_version",
            stored("last_processed_input_version")?,
        ),
        ("current_active_tx_count", field("active_transaction_count")),
        ("processed_journals_current", field("is_current")),
    ]))
}

/// `require_tax_processing_supported(profile)`, through the same gate the
/// journal rebuild uses.
fn require_tax_processing(profile: &Row) -> Result<()> {
    let country = kassiber_custody::codec::to_json(&py(profile.get("tax_country")));
    kassiber_custody::journal_support::require_tax_processing(
        &serde_json::json!({ "tax_country": country }),
    )
    .map_err(|(_, message)| AppError::new("internal_error", message))
}

/// `core_custody_journal.load_stored_transfer_audit(conn, profile_id)`.
fn stored_transfer_audit(conn: &Connection, profile_id: &str) -> Result<[Py; 5]> {
    let decisions = query(
        conn,
        "SELECT d.*, d.source_end_msat - d.source_start_msat AS amount_msat,
               source.external_id AS source_external_id,
               source.fee AS source_fee_msat,
               source.fiat_rate AS source_fiat_rate,
               source_wallet.label AS source_wallet_label,
               target.external_id AS target_external_id,
               target_wallet.label AS target_wallet_label
        FROM journal_custody_decisions d
        JOIN transactions source ON source.id = d.source_transaction_id
        JOIN transactions target ON target.id = d.target_transaction_id
        LEFT JOIN wallets source_wallet ON source_wallet.id = d.source_wallet_id
        LEFT JOIN wallets target_wallet ON target_wallet.id = d.target_wallet_id
        WHERE d.profile_id = ?
        ORDER BY d.occurred_at ASC, d.decision_id ASC",
        &[text_param(profile_id)],
    )?;
    let mut fee_emitted = std::collections::HashSet::new();
    let mut same_asset = Vec::new();
    let mut custody = Vec::new();
    for row in &decisions {
        let amount = int_of(row.get("amount_msat"))?;
        let source_id = py_str(&py(row.get("source_transaction_id")));
        let mut fee = BigInt::from(0);
        if fee_emitted.insert(source_id.clone()) {
            fee = int_or_zero(row.get("source_fee_msat"))?;
        }
        if row.get("source_asset") == row.get("target_asset") {
            let sent = amount.clone() + fee.clone();
            let mut transfer = vec![
                ("out_id", s(source_id.as_str())),
                ("in_id", py(row.get("target_transaction_id"))),
                ("external_id", py(row.get("source_external_id"))),
                ("occurred_at", py(row.get("occurred_at"))),
                ("asset", py(row.get("source_asset"))),
                ("from_wallet", py(row.get("source_wallet_label"))),
                ("to_wallet", py(row.get("target_wallet_label"))),
                ("sent", Py::Float(btc_of(&sent)?)),
                ("sent_msat", Py::Int(sent)),
                ("received", Py::Float(btc_of(&amount)?)),
                ("received_msat", Py::Int(amount.clone())),
                ("fee", Py::Float(btc_of(&fee)?)),
                ("fee_msat", Py::Int(fee)),
                (
                    "spot_price",
                    Py::Float(float_or_zero(row.get("source_fiat_rate"))?),
                ),
                ("pairing_source", py(row.get("reason"))),
            ];
            if truthy(row.get("atomic_group_id")) {
                transfer.push(("transfer_group_id", py(row.get("atomic_group_id"))));
            }
            same_asset.push(dict(transfer));
        }
        custody.push(dict([
            ("decision_id", py(row.get("decision_id"))),
            ("out_id", s(source_id.as_str())),
            ("in_id", py(row.get("target_transaction_id"))),
            ("from_wallet", py(row.get("source_wallet_label"))),
            ("to_wallet", py(row.get("target_wallet_label"))),
            ("source_asset", py(row.get("source_asset"))),
            ("target_asset", py(row.get("target_asset"))),
            ("amount", Py::Float(btc_of(&amount)?)),
            ("amount_msat", Py::Int(amount)),
            ("custody_state", py(row.get("state"))),
            ("basis_state", py(row.get("basis_state"))),
            ("evidence_reason", py(row.get("reason"))),
            ("component_id", py(row.get("component_id"))),
        ]));
    }
    let relations = query(
        conn,
        "SELECT r.id AS relation_id, r.relation_kind,
               r.kind AS review_kind, r.policy, r.swap_fee_msat,
               r.swap_fee_kind,
               r.out_transaction_id AS source_transaction_id,
               r.in_transaction_id AS target_transaction_id,
               r.out_asset AS source_asset, r.in_asset AS target_asset,
               r.out_amount AS source_amount_msat,
               r.in_amount AS target_amount_msat, r.basis_state,
               r.component_id, r.occurred_at, r.target_occurred_at,
               r.counterparty,
               source.external_id AS source_external_id,
               source.occurred_at AS source_occurred_at,
               source_wallet.label AS source_wallet_label,
               COALESCE(target.external_id, r.target_external_id)
                   AS target_external_id,
               target.occurred_at AS target_transaction_occurred_at,
               target_wallet.label AS target_wallet_label
        FROM journal_custody_projection_relations r
        JOIN transactions source ON source.id = r.out_transaction_id
        LEFT JOIN wallets source_wallet ON source_wallet.id = source.wallet_id
        LEFT JOIN transactions target ON target.id = r.in_transaction_id
        LEFT JOIN wallets target_wallet ON target_wallet.id = target.wallet_id
        WHERE r.profile_id = ?
          AND r.relation_kind IN ('conversion', 'direct_payout')
        ORDER BY COALESCE(r.target_occurred_at, r.occurred_at) ASC,
                 r.id ASC",
        &[text_param(profile_id)],
    )?;
    let mut pairs = Vec::new();
    let mut payouts = Vec::new();
    for row in &relations {
        if row.get("relation_kind") == &Sql::Text("conversion".into()) {
            pairs.push(dict([
                ("pair_id", py(row.get("relation_id"))),
                ("kind", py(row.get("review_kind"))),
                ("policy", py(row.get("policy"))),
                ("out_id", py(row.get("source_transaction_id"))),
                ("out_asset", py(row.get("source_asset"))),
                ("out_wallet", py(row.get("source_wallet_label"))),
                ("out_external_id", py(row.get("source_external_id"))),
                ("out_occurred_at", py(row.get("source_occurred_at"))),
                ("in_id", py(row.get("target_transaction_id"))),
                ("in_asset", py(row.get("target_asset"))),
                ("in_wallet", py(row.get("target_wallet_label"))),
                ("in_external_id", py(row.get("target_external_id"))),
                (
                    "in_occurred_at",
                    py(row.get("target_transaction_occurred_at")),
                ),
            ]));
            continue;
        }
        let out_amount = int_of(row.get("source_amount_msat"))?;
        let payout_amount = int_of(row.get("target_amount_msat"))?;
        let swap_fee = int_or_zero(row.get("swap_fee_msat"))?;
        payouts.push(dict([
            ("payout_id", py(row.get("relation_id"))),
            ("kind", py(row.get("review_kind"))),
            ("policy", py(row.get("policy"))),
            ("out_id", py(row.get("source_transaction_id"))),
            ("out_asset", py(row.get("source_asset"))),
            ("out_wallet", py(row.get("source_wallet_label"))),
            ("out_external_id", py(row.get("source_external_id"))),
            ("out_occurred_at", py(row.get("source_occurred_at"))),
            ("out_amount", Py::Float(btc_of(&out_amount)?)),
            ("out_amount_msat", Py::Int(out_amount)),
            ("payout_asset", py(row.get("target_asset"))),
            ("payout_amount", Py::Float(btc_of(&payout_amount)?)),
            ("payout_amount_msat", Py::Int(payout_amount)),
            ("payout_occurred_at", py(row.get("target_occurred_at"))),
            ("payout_external_id", py(row.get("target_external_id"))),
            ("counterparty", py(row.get("counterparty"))),
            ("swap_fee", Py::Float(btc_of(&swap_fee)?)),
            ("swap_fee_msat", Py::Int(swap_fee)),
            ("swap_fee_kind", py(row.get("swap_fee_kind"))),
        ]));
    }
    let quarantines: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM journal_quarantines WHERE profile_id = ?",
            [profile_id],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    Ok([
        Py::List(same_asset),
        Py::List(custody),
        Py::List(pairs),
        Py::List(payouts),
        Py::Int(quarantines.into()),
    ])
}

/// `inspect_transfer_audit(...)`: the stored custody projection's transfers,
/// pairs and payouts, with the journal processing status.
pub fn transfer_audit(conn: &Connection, scope: Scope<'_>) -> Result<Py> {
    let profile = profile_row(conn, scope)?;
    require_tax_processing(&profile)?;
    let [same_asset, custody, pairs, payouts, quarantines] =
        stored_transfer_audit(conn, scope.profile_id)?;
    let count = |list: &Py| match list {
        Py::List(items) => Py::Int(items.len().into()),
        _ => Py::Int(0.into()),
    };
    Ok(dict([
        ("profile", py(profile.get("label"))),
        ("processing", processing_status(conn, &profile)?),
        (
            "summary",
            dict([
                ("same_asset_transfers", count(&same_asset)),
                ("custody_transfers", count(&custody)),
                ("cross_asset_pairs", count(&pairs)),
                ("direct_swap_payouts", count(&payouts)),
                ("quarantines", quarantines),
            ]),
        ),
        ("same_asset_transfers", same_asset),
        ("custody_transfers", custody),
        ("cross_asset_pairs", pairs),
        ("direct_swap_payouts", payouts),
    ]))
}

/// `json.loads(row["detail_json"] or "{}")`, failing as Python does.
fn stored_detail(value: &Sql) -> Result<Py> {
    let text = match value {
        _ if !truthy(value) => "{}".to_owned(),
        Sql::Text(text) => text.clone(),
        Sql::Blob(bytes) => String::from_utf8(bytes.clone()).map_err(|_| {
            AppError::new(
                "internal_error",
                "'utf-8' codec can't decode the stored detail",
            )
        })?,
        Sql::Integer(_) => {
            return Err(AppError::new(
                "internal_error",
                "the JSON object must be str, bytes or bytearray, not int",
            ))
        }
        _ => {
            return Err(AppError::new(
                "internal_error",
                "the JSON object must be str, bytes or bytearray, not float",
            ))
        }
    };
    kassiber_pytext::loads(&text)
        .map_err(|error| AppError::new("internal_error", error.to_string()))
}

/// `_explained_quarantines(conn, profile, limit)`: the review's cause, root,
/// blocking state and actions per held transaction.
fn explained(
    conn: &Connection,
    profile: &Row,
    count: i64,
) -> Result<HashMap<String, Vec<(Key, Py)>>> {
    crate::quarantine::explained(conn, profile, count.max(1))
}

fn with_explanation(mut fields: Vec<(Key, Py)>, explanation: Option<&Vec<(Key, Py)>>) -> Py {
    for (key, value) in explanation.into_iter().flatten() {
        match fields.iter_mut().find(|(existing, _)| existing == key) {
            Some(slot) => slot.1 = value.clone(),
            None => fields.push((key.clone(), value.clone())),
        }
    }
    Py::Dict(fields)
}

/// `list_quarantines(...)`.
pub fn quarantines(conn: &Connection, scope: Scope<'_>) -> Result<Py> {
    let profile = profile_row(conn, scope)?;
    let rows = query(
        conn,
        "SELECT
            q.transaction_id,
            t.external_id,
            t.occurred_at,
            t.confirmed_at,
            w.label AS wallet,
            t.asset,
            t.amount,
            t.fee,
            q.reason,
            q.detail_json
        FROM journal_quarantines q
        JOIN transactions t ON t.id = q.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        WHERE q.profile_id = ?
        ORDER BY t.occurred_at DESC",
        &[text_param(scope.profile_id)],
    )?;
    let explained = explained(conn, &profile, rows.len() as i64)?;
    let mut output = Vec::new();
    for row in &rows {
        let detail = stored_detail(row.get("detail_json"))?;
        let fields = vec![
            (Key::from("transaction_id"), py(row.get("transaction_id"))),
            (Key::from("external_id"), or_empty(row.get("external_id"))),
            (Key::from("occurred_at"), py(row.get("occurred_at"))),
            (Key::from("confirmed_at"), py(row.get("confirmed_at"))),
            (Key::from("wallet"), py(row.get("wallet"))),
            (Key::from("asset"), py(row.get("asset"))),
            (
                Key::from("amount"),
                Py::Float(btc_of(&int_of(row.get("amount"))?)?),
            ),
            (
                Key::from("amount_msat"),
                Py::Int(int_of(row.get("amount"))?),
            ),
            (
                Key::from("fee"),
                Py::Float(btc_of(&int_of(row.get("fee"))?)?),
            ),
            (Key::from("fee_msat"), Py::Int(int_of(row.get("fee"))?)),
            (Key::from("reason"), py(row.get("reason"))),
            (Key::from("detail"), detail),
        ];
        let id = py_str(&py(row.get("transaction_id")));
        output.push(with_explanation(fields, explained.get(&id)));
    }
    Ok(Py::List(output))
}

fn or_empty(value: &Sql) -> Py {
    if truthy(value) {
        py(value)
    } else {
        s("")
    }
}

/// `resolve_transaction(conn, profile_id, tx_ref)["id"]`.
fn transaction_id(conn: &Connection, scope: Scope<'_>, reference: &str) -> Result<(String, Py)> {
    let row = kassiber_transactions::lookup::resolve_transaction_str(
        conn,
        scope.profile_id,
        reference,
        None,
    )?;
    Ok((kassiber_transactions::lookup::row_id(&row), row))
}

/// `show_quarantine(...)`.
pub fn quarantine_show(conn: &Connection, scope: Scope<'_>, reference: &str) -> Result<Py> {
    let profile = profile_row(conn, scope)?;
    let (id, _) = transaction_id(conn, scope, reference)?;
    let row = query(
        conn,
        "SELECT q.transaction_id, q.reason, q.detail_json, q.created_at,
               w.label AS wallet, t.external_id, t.occurred_at, t.confirmed_at, t.asset,
               t.amount, t.fee, t.fiat_rate, t.fiat_value, t.fiat_rate_exact,
               t.fiat_value_exact, t.pricing_source_kind, t.pricing_provider,
               t.pricing_pair, t.pricing_timestamp, t.pricing_granularity,
               t.pricing_method, t.pricing_quality, t.direction, t.excluded
        FROM journal_quarantines q
        JOIN transactions t ON t.id = q.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        WHERE q.profile_id = ? AND q.transaction_id = ?",
        &[text_param(scope.profile_id), text_param(&id)],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| {
        AppError::new(
            "not_found",
            format!("Transaction '{reference}' has no active quarantine"),
        )
        .with_hint("Only transactions flagged during `journals process` appear here.")
    })?;
    let amount = int_of(row.get("amount"))?;
    let fee = int_of(row.get("fee"))?;
    let fields = vec![
        (Key::from("transaction_id"), py(row.get("transaction_id"))),
        (Key::from("external_id"), or_empty(row.get("external_id"))),
        (Key::from("wallet"), py(row.get("wallet"))),
        (Key::from("occurred_at"), py(row.get("occurred_at"))),
        (Key::from("confirmed_at"), py(row.get("confirmed_at"))),
        (Key::from("direction"), py(row.get("direction"))),
        (Key::from("asset"), py(row.get("asset"))),
        (Key::from("amount"), Py::Float(btc_of(&amount)?)),
        (Key::from("amount_msat"), Py::Int(amount)),
        (Key::from("fee"), Py::Float(btc_of(&fee)?)),
        (Key::from("fee_msat"), Py::Int(fee)),
        (Key::from("fiat_rate"), py(row.get("fiat_rate"))),
        (Key::from("fiat_value"), py(row.get("fiat_value"))),
        (Key::from("fiat_rate_exact"), py(row.get("fiat_rate_exact"))),
        (
            Key::from("fiat_value_exact"),
            py(row.get("fiat_value_exact")),
        ),
        (
            Key::from("pricing"),
            dict([
                ("source_kind", py(row.get("pricing_source_kind"))),
                ("provider", py(row.get("pricing_provider"))),
                ("pair", py(row.get("pricing_pair"))),
                ("timestamp", py(row.get("pricing_timestamp"))),
                ("granularity", py(row.get("pricing_granularity"))),
                ("method", py(row.get("pricing_method"))),
                ("quality", py(row.get("pricing_quality"))),
            ]),
        ),
        (Key::from("excluded"), Py::Bool(truthy(row.get("excluded")))),
        (Key::from("reason"), py(row.get("reason"))),
        (Key::from("detail"), stored_detail(row.get("detail_json"))?),
        (Key::from("quarantined_at"), py(row.get("created_at"))),
    ];
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM journal_quarantines WHERE profile_id = ?",
            [scope.profile_id],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    let explained = explained(conn, &profile, count)?;
    let id = py_str(&py(row.get("transaction_id")));
    Ok(with_explanation(fields, explained.get(&id)))
}

/// `_ensure_quarantined(conn, profile_id, transaction_id)`.
fn ensure_quarantined(conn: &Connection, profile_id: &str, transaction_id: &str) -> Result<()> {
    let held: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?)",
            [profile_id, transaction_id],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    if held {
        Ok(())
    } else {
        Err(AppError::new("not_found", "Transaction is not quarantined")
            .with_hint("Run `kassiber journals quarantined` to see active entries."))
    }
}

/// Run `write` in one `BEGIN IMMEDIATE` transaction: committed on success,
/// rolled back on failure.
fn reserved_write<T>(conn: &Connection, write: impl FnOnce() -> Result<T>) -> Result<T> {
    let transaction = kassiber_store::write_transaction(conn).map_err(sql_error)?;
    let result = write()?;
    transaction.commit().map_err(sql_error)?;
    Ok(result)
}

/// `clear_quarantine(...)`: drop the stored quarantine and invalidate the
/// journals, so the next `journals process` re-evaluates the row.
pub fn quarantine_clear(conn: &Connection, scope: Scope<'_>, reference: &str) -> Result<Py> {
    reserved_write(conn, || {
        let (id, _) = transaction_id(conn, scope, reference)?;
        ensure_quarantined(conn, scope.profile_id, &id)?;
        conn.execute(
            "DELETE FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
            [scope.profile_id, id.as_str()],
        )
        .map_err(sql_error)?;
        kassiber_wallets::invalidate_journals(conn, scope.profile_id)?;
        Ok(dict([
            ("transaction_id", s(id.as_str())),
            ("resolution", s("clear")),
            ("note", s("Run `kassiber journals process` to re-evaluate.")),
        ]))
    })
}

/// The two audited quarantine resolutions.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    PriceOverride,
    Exclude,
}

/// One quarantine resolution as `resolve_quarantine_price_override` /
/// `resolve_quarantine_exclude` receive it.
pub struct Resolution<'a> {
    pub action: Action,
    pub transaction: &'a str,
    pub fiat_rate: Option<&'a Py>,
    pub fiat_value: Option<&'a Py>,
    /// `authored_source`: `"cli"`, or the daemon's `default_source`.
    pub source: &'a str,
    pub reason: &'a str,
}

/// `resolve_quarantine_price_override` / `resolve_quarantine_exclude`: the
/// held transaction, the audited metadata edit, the dropped quarantine and,
/// when the edit changed nothing, invalidated journals, in one reserved
/// write. Returns Python's resolution record.
pub fn resolve(conn: &Connection, scope: Scope<'_>, request: &Resolution<'_>) -> Result<Py> {
    let (transaction_id, row) = transaction_id(conn, scope, request.transaction)?;
    ensure_quarantined(conn, scope.profile_id, &transaction_id)?;
    let mut update = kassiber_transactions::metadata::GroupedUpdate {
        transaction: transaction_id.clone(),
        source: if request.source == "user" {
            "gui".into()
        } else {
            request.source.into()
        },
        reason: Some(request.reason.into()),
        ..Default::default()
    };
    if request.action == Action::Exclude {
        update.excluded = Some(Py::Bool(true));
    } else {
        if request.fiat_rate.is_none() && request.fiat_value.is_none() {
            return Err(AppError::new("validation", "Provide a fiat rate or value"));
        }
        let amount: i64 = match kassiber_envelope::get(&row, "amount") {
            Some(Py::Int(n)) => i64::try_from(n.clone()).ok(),
            _ => None,
        }
        .ok_or_else(|| {
            AppError::new(
                "internal_error",
                "stored transaction amount is not an integer",
            )
        })?;
        let (rate, total) =
            crate::views::override_prices(request.fiat_rate, request.fiat_value, amount)?;
        // A reviewed price is in the book's currency, which the journals read
        // it in, whatever currency an unpriced row was imported with.
        let currency = query(
            conn,
            "SELECT fiat_currency FROM profiles WHERE id = ?",
            &[text_param(scope.profile_id)],
        )?
        .into_iter()
        .next()
        .map_or(Py::Null, |row| py(row.get("fiat_currency")));
        update.pricing = Some(kassiber_transactions::metadata::PricingUpdate {
            fiat_currency: currency,
            fiat_rate: rate.map_or(Py::Null, |v| s(v.to_string())),
            fiat_value: total.map_or(Py::Null, |v| s(v.to_string())),
            source_kind: s("manual_override"),
            quality: s("exact"),
            method: s("quarantine_price_override"),
            ..Default::default()
        });
    }
    let record = reserved_write(conn, || {
        let record = kassiber_transactions::metadata::execute(
            conn,
            &kassiber_transactions::Scope {
                workspace_id: scope.workspace_id,
                profile_id: scope.profile_id,
            },
            &kassiber_transactions::metadata::Command::Update(Box::new(update)),
        )?;
        conn.execute(
            "DELETE FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
            [scope.profile_id, transaction_id.as_str()],
        )
        .map_err(sql_error)?;
        if kassiber_envelope::get(&record, "updated") != Some(&Py::Bool(true)) {
            kassiber_wallets::invalidate_journals(conn, scope.profile_id)?;
        }
        Ok(record)
    })?;
    let field = |key: &str| {
        kassiber_envelope::get(&record, key)
            .cloned()
            .unwrap_or(Py::Null)
    };
    let float = |key: &str| match kassiber_envelope::get(&record, key) {
        Some(Py::Str(text)) => text
            .as_str()
            .and_then(|t| kassiber_pytext::parse_float(t).ok())
            .map_or(Py::Null, Py::Float),
        _ => Py::Null,
    };
    Ok(match request.action {
        Action::Exclude => dict([
            ("transaction_id", s(transaction_id.as_str())),
            ("resolution", s("exclude")),
            ("excluded", Py::Bool(true)),
            ("history_event_id", field("history_event_id")),
            ("note", s(RESOLVE_NOTE)),
        ]),
        Action::PriceOverride => dict([
            ("transaction_id", s(transaction_id.as_str())),
            ("resolution", s("price-override")),
            ("fiat_rate", float("fiat_rate_exact")),
            ("fiat_value", float("fiat_value_exact")),
            ("fiat_rate_exact", field("fiat_rate_exact")),
            ("fiat_value_exact", field("fiat_value_exact")),
            ("pricing_source_kind", field("pricing_source_kind")),
            ("history_event_id", field("history_event_id")),
            ("note", s(RESOLVE_NOTE)),
        ]),
    })
}
