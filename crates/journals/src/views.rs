//! Desktop journal reads: `ui.journals.snapshot`, `.events.list` and
//! `.transfers.list`, ported from `kassiber.core.ui_snapshot`.
//!
//! Each read follows Python's queries and Python's scalar conversions
//! (`int(x or 0)`, `float(x or 0)`, `float(msat_to_btc(...))`) so the ordered
//! payload is byte-identical. They read the active book only and never write.
use kassiber_envelope::{dict, get, s, type_name, AppError};
use kassiber_pydecimal::{Context, Decimal};
use kassiber_pyjson::{BigInt, Key, Value as Py};
use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql, Connection};

pub const KINDS: &[&str] = &[
    "ui.journals.snapshot",
    "ui.journals.events.list",
    "ui.journals.transfers.list",
    "ui.journals.quarantine",
];
const MAX_UI_LIST_LIMIT: i64 = 500;
const MAX_UI_PREVIEW_LIMIT: i64 = 100;

const ENTRY_TYPE: &str = "CASE
    WHEN je.at_category = 'neu_swap' THEN 'neutral_swap'
    ELSE je.entry_type
END";
const GAIN_LOSS: &str = "CASE
    WHEN je.at_category = 'neu_swap' THEN 0
    ELSE COALESCE(je.gain_loss, 0)
END";
const RELATION: &str = "
                    SELECT profile_id, id, kind, policy, swap_fee_msat,
                           swap_fee_kind, out_transaction_id,
                           in_transaction_id, out_amount, in_amount, created_at
                    FROM journal_custody_projection_relations
                    WHERE relation_kind IN ('move', 'conversion')
                      AND in_transaction_id IS NOT NULL
";
const EMPTY_PAIR_JOIN: &str = "
            LEFT JOIN (
                SELECT NULL AS id, NULL AS kind, NULL AS policy,
                       NULL AS swap_fee_msat, NULL AS out_transaction_id,
                       NULL AS in_transaction_id, NULL AS out_amount
                WHERE 0
            ) p_out ON 0
            LEFT JOIN (
                SELECT NULL AS id, NULL AS kind, NULL AS policy,
                       NULL AS swap_fee_msat, NULL AS out_transaction_id,
                       NULL AS in_transaction_id, NULL AS out_amount
                WHERE 0
            ) p_in ON 0
";
const PAIR_ID: &str = "CASE WHEN p_out.id IS NOT NULL THEN p_out.id ELSE p_in.id END";
const PAIR_KIND: &str = "CASE WHEN p_out.id IS NOT NULL THEN p_out.kind ELSE p_in.kind END";
const PAIR_POLICY: &str = "CASE WHEN p_out.id IS NOT NULL THEN p_out.policy ELSE p_in.policy END";
const PAIR_SWAP_FEE: &str =
    "CASE WHEN p_out.id IS NOT NULL THEN p_out.swap_fee_msat ELSE p_in.swap_fee_msat END";
const PAIR_OUT_TRANSACTION: &str =
    "CASE WHEN p_out.id IS NOT NULL THEN p_out.out_transaction_id ELSE p_in.out_transaction_id END";
const PAIR_OUT_AMOUNT: &str =
    "CASE WHEN p_out.id IS NOT NULL THEN p_out.out_amount ELSE p_in.out_amount END";
const PAIR_IN_TRANSACTION: &str =
    "CASE WHEN p_out.id IS NOT NULL THEN p_out.in_transaction_id ELSE p_in.in_transaction_id END";

fn pair_join() -> String {
    let ranked = |column: &str| {
        format!(
            "
            LEFT JOIN (
                SELECT * FROM (
                    SELECT relation.*,
                           ROW_NUMBER() OVER (
                               PARTITION BY profile_id, {column}
                               ORDER BY created_at, id
                           ) AS relation_rank
                    FROM ({RELATION}) relation
                )
                WHERE relation_rank = 1
            )"
        )
    };
    format!(
        "{} p_out
              ON p_out.profile_id = je.profile_id
             AND p_out.out_transaction_id = je.transaction_id{} p_in
              ON p_in.profile_id = je.profile_id
             AND p_in.in_transaction_id = je.transaction_id
",
        ranked("out_transaction_id"),
        ranked("in_transaction_id")
    )
}

/// Pair columns shared by the snapshot and the events list.
fn pair_columns() -> String {
    format!(
        "{PAIR_ID} AS pair_id,
                {PAIR_KIND} AS pair_kind,
                {PAIR_POLICY} AS pair_policy,
                COALESCE({PAIR_SWAP_FEE}, 0) AS pair_swap_fee_msat,
                {PAIR_OUT_TRANSACTION} AS pair_out_transaction_id,
                tout.external_id AS pair_out_external_id,
                wout.label AS pair_out_wallet,
                tout.asset AS pair_out_asset,
                COALESCE({PAIR_OUT_AMOUNT}, tout.amount) AS pair_out_amount,
                {PAIR_IN_TRANSACTION} AS pair_in_transaction_id,
                tin.external_id AS pair_in_external_id,
                win.label AS pair_in_wallet,
                tin.asset AS pair_in_asset,
                tin.amount AS pair_in_amount"
    )
}

fn pair_tables(join: &str) -> String {
    format!(
        "{join}
            LEFT JOIN transactions tout ON tout.id = {PAIR_OUT_TRANSACTION}
            LEFT JOIN transactions tin ON tin.id = {PAIR_IN_TRANSACTION}
            LEFT JOIN wallets wout ON wout.id = tout.wallet_id
            LEFT JOIN wallets win ON win.id = tin.wallet_id"
    )
}

pub(crate) struct Row(Vec<(String, Sql)>);
impl Row {
    pub(crate) fn get(&self, name: &str) -> &Sql {
        self.0
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
            .unwrap_or(&Sql::Null)
    }
    fn has(&self, name: &str) -> bool {
        self.0.iter().any(|(key, _)| key == name)
    }
}

/// A `profiles` row as the report crate's mapping.
fn profile_value(profile: &Row) -> Py {
    Py::Dict(
        profile
            .0
            .iter()
            .map(|(key, value)| (Key::from(key.as_str()), py(value)))
            .collect(),
    )
}

pub(crate) fn sql_error(error: kassiber_store::rusqlite::Error) -> AppError {
    AppError::new("internal_error", error.to_string())
}

pub(crate) fn query(conn: &Connection, sql: &str, params: &[Sql]) -> Result<Vec<Row>, AppError> {
    let mut statement = conn.prepare(sql).map_err(sql_error)?;
    let names: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let rows = statement
        .query_map(params_from_iter(params.iter()), |row| {
            (0..names.len())
                .map(|index| Ok((names[index].clone(), row.get::<_, Sql>(index)?)))
                .collect::<Result<Vec<_>, _>>()
                .map(Row)
        })
        .map_err(sql_error)?;
    rows.collect::<Result<_, _>>().map_err(sql_error)
}

pub(crate) fn py(value: &Sql) -> Py {
    match value {
        Sql::Null => Py::Null,
        Sql::Integer(n) => Py::Int((*n).into()),
        Sql::Real(n) => Py::Float(*n),
        Sql::Text(text) => Py::Str(text.as_str().into()),
        Sql::Blob(bytes) => Py::Bytes(bytes.clone()),
    }
}

pub(crate) fn truthy(value: &Sql) -> bool {
    match value {
        Sql::Null => false,
        Sql::Integer(n) => *n != 0,
        Sql::Real(n) => *n != 0.0,
        Sql::Text(text) => !text.is_empty(),
        Sql::Blob(bytes) => !bytes.is_empty(),
    }
}

pub(crate) fn py_truthy(value: &Py) -> bool {
    match value {
        Py::Null => false,
        Py::Bool(b) => *b,
        Py::Int(n) => *n != BigInt::from(0),
        Py::Float(n) => *n != 0.0,
        Py::Str(text) => text.code_points().next().is_some(),
        Py::Bytes(bytes) => !bytes.is_empty(),
        Py::List(items) => !items.is_empty(),
        Py::Dict(items) => !items.is_empty(),
        _ => true,
    }
}

pub(crate) fn python_error(family: &str, message: String) -> AppError {
    // Python's daemon reports an unexpected exception as internal_error with
    // `str(exc)`; its traceback debug string has no native counterpart.
    let _ = family;
    AppError::new("internal_error", message)
}

/// Python `int(value or 0)`.
pub(crate) fn int_or_zero(value: &Sql) -> Result<BigInt, AppError> {
    if !truthy(value) {
        return Ok(BigInt::from(0));
    }
    let serde = match value {
        Sql::Null => serde_json::Value::Null,
        Sql::Integer(n) => serde_json::json!(n),
        Sql::Real(n) => serde_json::json!(n),
        Sql::Text(text) => serde_json::Value::String(text.clone()),
        Sql::Blob(bytes) => {
            kassiber_custody::codec::to_json(&kassiber_pyjson::Value::Bytes(bytes.clone()))
        }
    };
    let integer = crate::integer::integer_or_zero(&serde)
        .map_err(|refusal| python_error(&refusal.family, refusal.message.clone()))?;
    integer
        .to_string()
        .parse()
        .map_err(|_| AppError::new("internal_error", "invalid integer"))
}

fn float_text(text: &str, shown: String) -> Result<f64, AppError> {
    kassiber_pytext::parse_float(text).map_err(|_| {
        python_error(
            "ValueError",
            format!("could not convert string to float: {shown}"),
        )
    })
}

/// Python `float(value)` for a non-NULL value.
pub(crate) fn float(value: &Sql) -> Result<f64, AppError> {
    match value {
        Sql::Null => Err(python_error(
            "TypeError",
            "float() argument must be a string or a real number, not 'NoneType'".into(),
        )),
        Sql::Integer(n) => Ok(*n as f64),
        Sql::Real(n) => Ok(*n),
        Sql::Text(text) => float_text(text, kassiber_pytext::repr_str(text)),
        Sql::Blob(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) if text.is_ascii() => float_text(text, kassiber_pyjson::bytes_repr(bytes)),
            _ => Err(python_error(
                "ValueError",
                format!(
                    "could not convert string to float: {}",
                    kassiber_pyjson::bytes_repr(bytes)
                ),
            )),
        },
    }
}

/// Python `float(value or 0)`.
fn float_or_zero(value: &Sql) -> Result<f64, AppError> {
    if truthy(value) {
        float(value)
    } else {
        Ok(0.0)
    }
}

/// Python `float(msat_to_btc(value or 0))`.
pub(crate) fn btc(value: &Sql) -> Result<f64, AppError> {
    let msat = int_or_zero(value)?;
    let decimal: Decimal = msat
        .to_string()
        .parse()
        .map_err(|_| AppError::new("internal_error", "invalid msat quantity"))?;
    let quotient = decimal
        .div(&Decimal::from(100_000_000_000_i64), &Context::RP2)
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    quotient
        .to_string()
        .parse::<f64>()
        .map_err(|_| AppError::new("internal_error", "invalid BTC quantity"))
}

pub(crate) fn int_py(value: BigInt) -> Py {
    Py::Int(value)
}

pub(crate) fn or_none(value: &Py) -> Py {
    if py_truthy(value) {
        value.clone()
    } else {
        Py::Null
    }
}

pub(crate) fn or_empty(value: &Sql) -> Py {
    if truthy(value) {
        py(value)
    } else {
        s("")
    }
}

pub(crate) fn args_object(args: Option<&Py>) -> Result<Vec<(Key, Py)>, AppError> {
    match args {
        None | Some(Py::Null) => Ok(vec![]),
        Some(Py::Dict(pairs)) => Ok(pairs.clone()),
        Some(other) => Err(
            AppError::new("validation", "ui snapshot args must be an object")
                .with_details(dict([("type", s(type_name(other)))])),
        ),
    }
}

pub(crate) fn unknown(kind: &str, args: &[(Key, Py)], allowed: &[&str]) -> Result<(), AppError> {
    let mut names: Vec<Py> = Vec::new();
    for (key, _) in args {
        match key {
            Key::Str(name) if name.as_str().is_some_and(|n| allowed.contains(&n)) => {}
            Key::Str(name) => names.push(Py::Str(name.clone())),
            _ => {}
        }
    }
    if names.is_empty() {
        return Ok(());
    }
    names.sort_by(|a, b| match (a, b) {
        (Py::Str(a), Py::Str(b)) => a.code_points().cmp(b.code_points()),
        _ => std::cmp::Ordering::Equal,
    });
    Err(AppError::new(
        "validation",
        format!("{kind} received unsupported arguments"),
    )
    .with_details(dict([("unknown", Py::List(names))])))
}

fn arg<'a>(args: &'a [(Key, Py)], name: &str) -> Option<&'a Py> {
    args.iter()
        .find(|(key, _)| *key == Key::from(name))
        .map(|(_, value)| value)
}

/// `_coerce_limit`: Python `int(raw)`, at least 1, capped at `maximum`.
pub(crate) fn limit(args: &[(Key, Py)], default: i64, maximum: i64) -> Result<i64, AppError> {
    let raw = arg(args, "limit")
        .cloned()
        .unwrap_or(Py::Int(default.into()));
    let invalid = || {
        AppError::new("validation", "limit must be an integer")
            .with_details(dict([("limit", raw.clone())]))
    };
    let value: BigInt = match &raw {
        Py::Bool(b) => BigInt::from(i64::from(*b)),
        Py::Int(n) => n.clone(),
        Py::Float(f) if f.is_nan() => return Err(invalid()),
        Py::Float(f) if f.is_infinite() => {
            return Err(python_error(
                "OverflowError",
                "cannot convert float infinity to integer".into(),
            ))
        }
        Py::Float(f) => format!("{:.0}", f.trunc()).parse().map_err(|_| invalid())?,
        Py::Str(text) => match text.as_str() {
            Some(text) => kassiber_pytext::parse_int(text).map_err(|_| invalid())?,
            None => return Err(invalid()),
        },
        Py::Bytes(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) if text.is_ascii() => {
                kassiber_pytext::parse_int(text).map_err(|_| invalid())?
            }
            _ => return Err(invalid()),
        },
        _ => return Err(invalid()),
    };
    if value < BigInt::from(1) {
        return Err(AppError::new("validation", "limit must be positive")
            .with_details(dict([("limit", raw.clone())])));
    }
    Ok(if value > BigInt::from(maximum) {
        maximum
    } else {
        value.to_string().parse().unwrap_or(maximum)
    })
}

pub(crate) struct Scope {
    pub(crate) context: Py,
    pub(crate) profile: Option<Row>,
}

/// `_active_context_and_profile` for the active book.
pub(crate) fn active(conn: &Connection) -> Result<Scope, AppError> {
    let context = kassiber_setup::context::snapshot(conn)?;
    let id = get(&context, "profile_id").cloned().unwrap_or(Py::Null);
    let workspace = get(&context, "workspace_id").cloned().unwrap_or(Py::Null);
    if !py_truthy(&workspace) || !py_truthy(&id) {
        return Ok(Scope {
            context,
            profile: None,
        });
    }
    let param = match &id {
        Py::Str(text) => Sql::Text(text.as_str().unwrap_or_default().to_owned()),
        Py::Int(n) => Sql::Integer(n.to_string().parse().unwrap_or_default()),
        Py::Float(n) => Sql::Real(*n),
        Py::Bytes(bytes) => Sql::Blob(bytes.clone()),
        _ => Sql::Null,
    };
    let profile = query(conn, "SELECT * FROM profiles WHERE id = ?", &[param])?
        .into_iter()
        .next();
    Ok(Scope { context, profile })
}

pub(crate) fn label(scope: &Scope, key: &str) -> Py {
    or_none(&get(&scope.context, key).cloned().unwrap_or(Py::Null))
}

struct Freshness {
    status: &'static str,
    reason: &'static str,
    needs_processing: bool,
    input_version: BigInt,
    processed_version: BigInt,
    active: i64,
    entries: BigInt,
    quarantines: BigInt,
}

/// `_journal_freshness` over `custody_journal.evaluate_projection_freshness`.
fn freshness(conn: &Connection, profile: &Row) -> Result<Freshness, AppError> {
    let id = profile.get("id").clone();
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id = ? AND excluded = 0",
            [&id],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    let processed_at = truthy(profile.get("last_processed_at"));
    let processed_count = int_or_zero(profile.get("last_processed_tx_count"))?;
    let input_version = int_or_zero(profile.get("journal_input_version"))?;
    let processed_version = int_or_zero(profile.get("last_processed_input_version"))?;
    // A profile row without both semantics columns is never current.
    let semantics_current = profile.has("required_journal_semantics_version")
        && profile.has("journal_semantics_version")
        && {
            let required = int_or_zero(profile.get("required_journal_semantics_version"))?;
            let built = int_or_zero(profile.get("journal_semantics_version"))?;
            required == BigInt::from(0)
                || (built
                    == BigInt::from(kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION)
                    && built >= required)
        };
    let (status, reason) = if active == 0 {
        ("no_transactions", "no active transactions")
    } else if !processed_at {
        ("not_processed", "journals have not been processed")
    } else if processed_count != BigInt::from(active) {
        (
            "stale",
            "active transaction count changed since last processing",
        )
    } else if input_version != processed_version {
        ("stale", "journal inputs changed since last processing")
    } else if !semantics_current {
        (
            "stale",
            "journal calculation semantics changed; rebuild with current Kassiber",
        )
    } else {
        (
            "current",
            "journals match the active transaction count, input version and calculation semantics",
        )
    };
    let count = |table: &str| -> Result<BigInt, AppError> {
        let value: Sql = conn
            .query_row(
                &format!("SELECT COUNT(*) AS count FROM {table} WHERE profile_id = ?"),
                [&id],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        int_or_zero(&value)
    };
    Ok(Freshness {
        status,
        reason,
        needs_processing: matches!(status, "not_processed" | "stale"),
        input_version,
        processed_version,
        active,
        entries: count("journal_entries")?,
        quarantines: count("journal_quarantines")?,
    })
}

fn entry_types(rows: &[Row]) -> Result<Py, AppError> {
    Ok(Py::List(
        rows.iter()
            .map(|row| {
                Ok(dict([
                    ("type", py(row.get("entry_type"))),
                    ("count", int_py(int_or_zero(row.get("count"))?)),
                    (
                        "gainLossEur",
                        Py::Float(float_or_zero(row.get("gain_loss"))?),
                    ),
                ]))
            })
            .collect::<Result<_, AppError>>()?,
    ))
}

/// `_journal_pair_payload`.
fn pair(row: &Row) -> Result<Option<Py>, AppError> {
    if !row.has("pair_id") || !truthy(row.get("pair_id")) {
        return Ok(None);
    }
    let out_asset = row.get("pair_out_asset");
    let in_asset = row.get("pair_in_asset");
    let swap_fee = int_or_zero(row.get("pair_swap_fee_msat"))?;
    let side = |prefix: &str, asset: &Sql| -> Result<Py, AppError> {
        Ok(dict([
            (
                "transactionId",
                py(row.get(&format!("{prefix}_transaction_id"))),
            ),
            (
                "externalId",
                or_empty(row.get(&format!("{prefix}_external_id"))),
            ),
            ("wallet", py(row.get(&format!("{prefix}_wallet")))),
            ("asset", py(asset)),
            (
                "amountMsat",
                int_py(int_or_zero(row.get(&format!("{prefix}_amount")))?),
            ),
            (
                "amount",
                Py::Float(btc(row.get(&format!("{prefix}_amount")))?),
            ),
        ]))
    };
    Ok(Some(dict([
        ("pairId", py(row.get("pair_id"))),
        (
            "pairType",
            s(if py(out_asset) == py(in_asset) {
                "transfer"
            } else {
                "swap"
            }),
        ),
        ("kind", py(row.get("pair_kind"))),
        ("policy", py(row.get("pair_policy"))),
        ("swapFeeMsat", int_py(swap_fee.clone())),
        ("swapFee", Py::Float(btc(&Sql::Text(swap_fee.to_string()))?)),
        ("out", side("pair_out", out_asset)?),
        ("in", side("pair_in", in_asset)?),
    ])))
}

/// `(row["occurred_at"] or "")[:16].replace("T", " ")`.
fn date(value: &Sql) -> Result<Py, AppError> {
    match value {
        value if !truthy(value) => Ok(s("")),
        Sql::Text(text) => Ok(s(text
            .chars()
            .take(16)
            .collect::<String>()
            .replace('T', " "))),
        Sql::Blob(_) => Err(python_error(
            "TypeError",
            "a bytes-like object is required, not 'str'".into(),
        )),
        Sql::Integer(_) => Err(python_error(
            "TypeError",
            "'int' object is not subscriptable".into(),
        )),
        _ => Err(python_error(
            "TypeError",
            "'float' object is not subscriptable".into(),
        )),
    }
}

/// `_journal_recent_row_payload`.
fn recent(row: &Row) -> Result<Py, AppError> {
    let mut payload = vec![
        (Key::from("date"), date(row.get("occurred_at"))?),
        (Key::from("type"), py(row.get("entry_type"))),
        (Key::from("transactionId"), py(row.get("transaction_id"))),
        (
            Key::from("transactionExternalId"),
            py(row.get("transaction_external_id")),
        ),
        (Key::from("wallet"), py(row.get("wallet"))),
        (Key::from("asset"), py(row.get("asset"))),
        (Key::from("quantity"), Py::Float(btc(row.get("quantity"))?)),
        (
            Key::from("fiatValueEur"),
            Py::Float(float_or_zero(row.get("fiat_value"))?),
        ),
        (
            Key::from("gainLossEur"),
            Py::Float(float_or_zero(row.get("gain_loss"))?),
        ),
        (Key::from("atCategory"), py(row.get("at_category"))),
    ];
    if let Some(pair) = pair(row)? {
        payload.push((Key::from("pair"), pair));
    }
    Ok(Py::Dict(payload))
}

fn recent_sql(filtered: bool, join: &str) -> String {
    format!(
        "
        WITH normalized AS (
            SELECT
                je.occurred_at,
                je.created_at,
                je.id,
                je.transaction_id,
                COALESCE(t.external_id, '') AS transaction_external_id,
                {ENTRY_TYPE} AS entry_type,
                je.asset,
                je.quantity,
                je.fiat_value,
                {GAIN_LOSS} AS gain_loss,
                je.at_category,
                w.label AS wallet,
                {}
            FROM journal_entries je
            JOIN wallets w ON w.id = je.wallet_id
            LEFT JOIN transactions t ON t.id = je.transaction_id
            {}
            WHERE je.profile_id = ?
        )
        SELECT *
        FROM normalized
        {}
        ORDER BY occurred_at DESC, created_at DESC, id DESC
        LIMIT 12
        ",
        pair_columns(),
        pair_tables(join),
        if filtered { "WHERE entry_type = ?" } else { "" }
    )
}

fn empty_snapshot() -> Py {
    dict([
        (
            "status",
            dict([
                ("workspace", Py::Null),
                ("profile", Py::Null),
                ("transactionCount", Py::Int(0.into())),
                ("journalEntryCount", Py::Int(0.into())),
                ("needsJournals", Py::Bool(false)),
                ("quarantines", Py::Int(0.into())),
                ("lastProcessedAt", Py::Null),
            ]),
        ),
        ("entryTypes", Py::List(vec![])),
        ("recent", Py::List(vec![])),
        ("recentByType", dict::<&str>([])),
    ])
}

fn dict_key(value: &Sql) -> Key {
    match value {
        Sql::Text(text) => Key::from(text.as_str()),
        Sql::Integer(n) => Key::Int((*n).into()),
        Sql::Real(n) => Key::Float(*n),
        Sql::Null => Key::Null,
        // json.dumps rejects bytes keys; entry types are TEXT in every schema.
        Sql::Blob(bytes) => Key::from(kassiber_pyjson::bytes_repr(bytes).as_str()),
    }
}

/// `build_journals_snapshot`; Python ignores this kind's arguments.
pub fn snapshot(conn: &Connection) -> Result<Py, AppError> {
    let scope = active(conn)?;
    let Some(profile) = &scope.profile else {
        return Ok(empty_snapshot());
    };
    let freshness = freshness(conn, profile)?;
    let join = if freshness.needs_processing {
        EMPTY_PAIR_JOIN.to_owned()
    } else {
        pair_join()
    };
    let id = profile.get("id").clone();
    let entry_rows = query(
        conn,
        &format!(
            "SELECT {ENTRY_TYPE} AS entry_type, COUNT(*) AS count, SUM({GAIN_LOSS}) AS gain_loss
        FROM journal_entries je
        WHERE profile_id = ?
        GROUP BY {ENTRY_TYPE}
        ORDER BY count DESC, entry_type ASC"
        ),
        std::slice::from_ref(&id),
    )?;
    let recent_rows = query(conn, &recent_sql(false, &join), std::slice::from_ref(&id))?;
    let typed_sql = recent_sql(true, &join);
    let mut by_type = Vec::new();
    for entry in &entry_rows {
        let rows = query(
            conn,
            &typed_sql,
            &[id.clone(), entry.get("entry_type").clone()],
        )?;
        let payloads = rows.iter().map(recent).collect::<Result<_, _>>()?;
        let key = dict_key(entry.get("entry_type"));
        if let Some(existing) = by_type.iter_mut().find(|(k, _)| *k == key) {
            existing.1 = Py::List(payloads);
        } else {
            by_type.push((key, Py::List(payloads)));
        }
    }
    Ok(dict([
        (
            "status",
            dict([
                ("workspace", label(&scope, "workspace_label")),
                ("profile", label(&scope, "profile_label")),
                ("transactionCount", Py::Int(freshness.active.into())),
                ("journalEntryCount", int_py(freshness.entries.clone())),
                ("needsJournals", Py::Bool(freshness.needs_processing)),
                ("quarantines", int_py(freshness.quarantines.clone())),
                ("lastProcessedAt", py(profile.get("last_processed_at"))),
                ("freshnessStatus", s(freshness.status)),
                ("freshnessReason", s(freshness.reason)),
                (
                    "journalInputVersion",
                    int_py(freshness.input_version.clone()),
                ),
                (
                    "lastProcessedInputVersion",
                    int_py(freshness.processed_version.clone()),
                ),
            ]),
        ),
        ("entryTypes", entry_types(&entry_rows)?),
        (
            "recent",
            Py::List(recent_rows.iter().map(recent).collect::<Result<_, _>>()?),
        ),
        ("recentByType", Py::Dict(by_type)),
    ]))
}

/// `build_journal_events_list_snapshot`.
pub fn events(conn: &Connection, args: Option<&Py>) -> Result<Py, AppError> {
    const KIND: &str = "ui.journals.events.list";
    let args = args_object(args)?;
    unknown(KIND, &args, &["limit", "transaction"])?;
    let limit = limit(&args, 100, MAX_UI_LIST_LIMIT)?;
    let transaction = match arg(&args, "transaction") {
        None | Some(Py::Null) => None,
        Some(Py::Str(text)) => {
            let stripped = kassiber_pytext::strip_py(text);
            stripped.code_points().next().is_some().then_some(stripped)
        }
        Some(_) => {
            return Err(AppError::new(
                "validation",
                format!("{KIND} transaction must be a string"),
            ))
        }
    };
    let scope = active(conn)?;
    let Some(profile) = &scope.profile else {
        return Ok(dict([
            (
                "summary",
                dict([
                    ("workspace", Py::Null),
                    ("profile", Py::Null),
                    ("count", Py::Int(0.into())),
                    ("reportableCount", Py::Int(0.into())),
                    ("needsJournals", Py::Bool(false)),
                    ("lastProcessedAt", Py::Null),
                    ("freshnessStatus", s("no_profile")),
                    ("freshnessReason", s("no active profile")),
                    ("entryTypes", Py::List(vec![])),
                    ("limit", Py::Int(limit.into())),
                ]),
            ),
            ("events", Py::List(vec![])),
        ]));
    };
    let freshness = freshness(conn, profile)?;
    let join = if freshness.needs_processing {
        EMPTY_PAIR_JOIN.to_owned()
    } else {
        pair_join()
    };
    let mut where_sql = "WHERE je.profile_id = ?".to_owned();
    let mut params = vec![profile.get("id").clone()];
    if let Some(reference) = &transaction {
        let Some(reference) = reference.as_str() else {
            return Err(AppError::new(
                "internal_error",
                "'utf-8' codec can't encode characters: surrogates not allowed",
            ));
        };
        where_sql.push_str(" AND (je.transaction_id = ? OR t.external_id = ?)");
        params.push(Sql::Text(reference.to_owned()));
        params.push(Sql::Text(reference.to_owned()));
    }
    let summary_rows = query(
        conn,
        &format!(
            "SELECT {ENTRY_TYPE} AS entry_type, COUNT(*) AS count, SUM({GAIN_LOSS}) AS gain_loss
        FROM journal_entries je
        LEFT JOIN transactions t ON t.id = je.transaction_id
        {where_sql}
        GROUP BY {ENTRY_TYPE}
        ORDER BY count DESC, entry_type ASC"
        ),
        &params,
    )?;
    let mut total = BigInt::from(0);
    for row in &summary_rows {
        total += int_or_zero(row.get("count"))?;
    }
    let reportable: Sql = query(
        conn,
        &format!(
            "SELECT COUNT(*) AS count
        FROM journal_entries je
        LEFT JOIN transactions t ON t.id = je.transaction_id
        {where_sql}
          AND COALESCE(t.taxability_override, 1) != 0
          AND (
            (je.entry_type = 'disposal' AND COALESCE(je.at_category, '') != 'neu_swap')
            OR je.entry_type = 'income'
            OR (je.entry_type NOT IN ('fee', 'transfer_fee') AND je.at_kennzahl IS NOT NULL)
          )"
        ),
        &params,
    )?
    .into_iter()
    .next()
    .map(|row| row.get("count").clone())
    .unwrap_or(Sql::Null);
    let mut list_params = params.clone();
    list_params.push(Sql::Integer(limit));
    let rows = query(
        conn,
        &format!(
            "SELECT
            je.id, je.transaction_id, je.occurred_at, je.created_at, je.entry_type,
            je.asset, je.quantity, je.fiat_value, je.unit_cost, je.cost_basis,
            je.proceeds, je.gain_loss, je.pricing_source_kind, je.pricing_quality,
            COALESCE(je.description, '') AS description,
            je.at_category, je.at_kennzahl,
            w.label AS wallet,
            COALESCE(a.code, '') AS account,
            COALESCE(a.label, '') AS account_label,
            t.external_id AS transaction_external_id,
            t.direction AS transaction_direction,
            t.taxability_override AS taxability_override,
            {}
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN accounts a ON a.id = je.account_id
        LEFT JOIN transactions t ON t.id = je.transaction_id
        {}
        {where_sql}
        ORDER BY je.occurred_at DESC, je.created_at DESC, je.id DESC
        LIMIT ?",
            pair_columns(),
            pair_tables(&join)
        ),
        &list_params,
    )?;
    let reference_profile = profile_value(profile);
    // The rows ui.reports.explain_capital_gain accepts: realized disposals and
    // income that count for tax, from a current journal.
    let explanation_reference = |row: &Row| -> Result<Py, AppError> {
        let explainable = matches!(row.get("entry_type"), Sql::Text(t) if t == "disposal" || t == "income")
            && !matches!(row.get("at_category"), Sql::Text(c) if c == "neu_swap")
            && !matches!(row.get("taxability_override"), Sql::Integer(0))
            && !matches!(row.get("taxability_override"), Sql::Real(n) if *n == 0.0);
        if freshness.needs_processing || !explainable {
            return Ok(Py::Null);
        }
        kassiber_reports::result_reference(conn, &reference_profile, &py(row.get("id")))
    };
    let events = rows
        .iter()
        .map(|row| {
            let neutral = matches!(row.get("at_category"), Sql::Text(c) if c == "neu_swap");
            let optional = |name: &str| -> Result<Py, AppError> {
                match row.get(name) {
                    Sql::Null => Ok(Py::Null),
                    value => Ok(Py::Float(float(value)?)),
                }
            };
            let proceeds = if neutral && !matches!(row.get("cost_basis"), Sql::Null) {
                Py::Float(float(row.get("cost_basis"))?)
            } else {
                optional("proceeds")?
            };
            let gain_loss = if neutral {
                Py::Float(0.0)
            } else {
                optional("gain_loss")?
            };
            let market = |name: &str| -> Result<Py, AppError> {
                if neutral && !matches!(row.get(name), Sql::Null) {
                    Ok(Py::Float(float(row.get(name))?))
                } else {
                    Ok(Py::Null)
                }
            };
            Ok(dict([
                ("id", py(row.get("id"))),
                ("transactionId", py(row.get("transaction_id"))),
                (
                    "transactionExternalId",
                    or_empty(row.get("transaction_external_id")),
                ),
                (
                    "transactionDirection",
                    or_empty(row.get("transaction_direction")),
                ),
                ("occurredAt", py(row.get("occurred_at"))),
                ("createdAt", py(row.get("created_at"))),
                (
                    "entryType",
                    if neutral {
                        s("neutral_swap")
                    } else {
                        py(row.get("entry_type"))
                    },
                ),
                ("wallet", py(row.get("wallet"))),
                ("account", py(row.get("account"))),
                ("accountLabel", py(row.get("account_label"))),
                ("asset", py(row.get("asset"))),
                ("quantity", Py::Float(btc(row.get("quantity"))?)),
                ("quantityMsat", int_py(int_or_zero(row.get("quantity"))?)),
                (
                    "fiatValueEur",
                    Py::Float(float_or_zero(row.get("fiat_value"))?),
                ),
                (
                    "unitCostEur",
                    Py::Float(float_or_zero(row.get("unit_cost"))?),
                ),
                ("costBasisEur", optional("cost_basis")?),
                ("proceedsEur", proceeds),
                ("gainLossEur", gain_loss),
                ("marketValueEur", market("proceeds")?),
                ("marketDeltaEur", market("gain_loss")?),
                (
                    "pricingSourceKind",
                    or_empty(row.get("pricing_source_kind")),
                ),
                ("pricingQuality", or_empty(row.get("pricing_quality"))),
                ("description", py(row.get("description"))),
                ("atCategory", py(row.get("at_category"))),
                ("atKennzahl", py(row.get("at_kennzahl"))),
                ("pair", pair(row)?.unwrap_or(Py::Null)),
                ("explanationReference", explanation_reference(row)?),
            ]))
        })
        .collect::<Result<_, AppError>>()?;
    Ok(dict([
        (
            "summary",
            dict([
                ("workspace", label(&scope, "workspace_label")),
                ("profile", label(&scope, "profile_label")),
                ("count", int_py(total)),
                ("reportableCount", int_py(int_or_zero(&reportable)?)),
                ("needsJournals", Py::Bool(freshness.needs_processing)),
                ("lastProcessedAt", py(profile.get("last_processed_at"))),
                ("freshnessStatus", s(freshness.status)),
                ("freshnessReason", s(freshness.reason)),
                ("entryTypes", entry_types(&summary_rows)?),
                ("limit", Py::Int(limit.into())),
            ]),
        ),
        ("events", Py::List(events)),
    ]))
}

/// `build_journals_transfers_list_snapshot`.
pub fn transfers(conn: &Connection, args: Option<&Py>) -> Result<Py, AppError> {
    let args = args_object(args)?;
    unknown("ui.journals.transfers.list", &args, &["limit"])?;
    let limit = limit(&args, 20, MAX_UI_PREVIEW_LIMIT)?;
    let empty = |scope: Option<&Scope>, stale: bool| {
        let mut summary = vec![
            (
                Key::from("workspace"),
                scope.map_or(Py::Null, |s| label(s, "workspace_label")),
            ),
            (
                Key::from("profile"),
                scope.map_or(Py::Null, |s| label(s, "profile_label")),
            ),
            (Key::from("manual_pairs"), Py::Int(0.into())),
            (Key::from("same_asset_pairs"), Py::Int(0.into())),
            (Key::from("cross_asset_pairs"), Py::Int(0.into())),
            (Key::from("journal_transfer_entries"), Py::Int(0.into())),
            (Key::from("limit"), Py::Int(limit.into())),
        ];
        if stale {
            summary.push((Key::from("projection_status"), s("stale")));
        }
        dict([("summary", Py::Dict(summary)), ("pairs", Py::List(vec![]))])
    };
    let scope = active(conn)?;
    let Some(profile) = &scope.profile else {
        return Ok(empty(None, false));
    };
    if freshness(conn, profile)?.needs_processing {
        return Ok(empty(Some(&scope), true));
    }
    let id = profile.get("id").clone();
    let summary = query(
        conn,
        "SELECT
            COUNT(*) AS manual_pairs,
            SUM(CASE WHEN source_asset = target_asset THEN 1 ELSE 0 END)
                AS same_asset_pairs,
            SUM(CASE WHEN source_asset <> target_asset THEN 1 ELSE 0 END)
                AS cross_asset_pairs
        FROM (
            SELECT source_asset, target_asset
            FROM journal_custody_decisions
            WHERE profile_id = ?
            UNION ALL
            SELECT source_asset, target_asset
            FROM journal_custody_economic_relations
            WHERE profile_id = ? AND relation_kind = 'conversion'
              AND target_transaction_id IS NOT NULL
        )",
        &[id.clone(), id.clone()],
    )?
    .into_iter()
    .next()
    .unwrap_or(Row(vec![]));
    let entries: Sql = conn
        .query_row(
            "SELECT COUNT(*) AS count FROM journal_entries
        WHERE profile_id = ? AND entry_type IN ('transfer_out', 'transfer_in', 'transfer_fee')",
            [&id],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    let rows = query(
        conn,
        "SELECT
            relation.id, relation.kind, relation.policy, relation.created_at,
            relation.out_transaction_id, relation.in_transaction_id,
            tout.external_id AS out_external_id,
            tout.occurred_at AS out_occurred_at,
            relation.out_asset, relation.out_amount,
            wout.label AS out_wallet,
            tin.external_id AS in_external_id,
            tin.occurred_at AS in_occurred_at,
            relation.in_asset, relation.in_amount,
            win.label AS in_wallet,
            relation.occurred_at AS sort_at
        FROM journal_custody_projection_relations relation
        JOIN transactions tout ON tout.id = relation.out_transaction_id
        JOIN transactions tin ON tin.id = relation.in_transaction_id
        JOIN wallets wout ON wout.id = tout.wallet_id
        JOIN wallets win ON win.id = tin.wallet_id
        WHERE relation.profile_id = ?
          AND relation.relation_kind IN ('move', 'conversion')
          AND relation.in_transaction_id IS NOT NULL
        ORDER BY sort_at DESC, relation.id DESC
        LIMIT ?",
        &[id, Sql::Integer(limit)],
    )?;
    let side = |row: &Row, prefix: &str| -> Result<Py, AppError> {
        let amount = row.get(&format!("{prefix}_amount"));
        Ok(dict([
            (
                "transaction_id",
                py(row.get(&format!("{prefix}_transaction_id"))),
            ),
            (
                "external_id",
                or_empty(row.get(&format!("{prefix}_external_id"))),
            ),
            ("occurred_at", py(row.get(&format!("{prefix}_occurred_at")))),
            ("wallet", py(row.get(&format!("{prefix}_wallet")))),
            ("asset", py(row.get(&format!("{prefix}_asset")))),
            ("amount", Py::Float(btc(amount)?)),
            ("amount_msat", int_py(int_or_zero(amount)?)),
        ]))
    };
    let pairs = rows
        .iter()
        .map(|row| {
            Ok(dict([
                ("id", py(row.get("id"))),
                ("kind", py(row.get("kind"))),
                ("policy", py(row.get("policy"))),
                ("created_at", py(row.get("created_at"))),
                ("out", side(row, "out")?),
                ("in", side(row, "in")?),
            ]))
        })
        .collect::<Result<_, AppError>>()?;
    Ok(dict([
        (
            "summary",
            dict([
                ("workspace", label(&scope, "workspace_label")),
                ("profile", label(&scope, "profile_label")),
                (
                    "manual_pairs",
                    int_py(int_or_zero(summary.get("manual_pairs"))?),
                ),
                (
                    "same_asset_pairs",
                    int_py(int_or_zero(summary.get("same_asset_pairs"))?),
                ),
                (
                    "cross_asset_pairs",
                    int_py(int_or_zero(summary.get("cross_asset_pairs"))?),
                ),
                ("journal_transfer_entries", int_py(int_or_zero(&entries)?)),
                ("limit", Py::Int(limit.into())),
            ]),
        ),
        ("pairs", Py::List(pairs)),
    ]))
}

pub fn execute(conn: &Connection, kind: &str, args: Option<&Py>) -> Result<Py, AppError> {
    match kind {
        "ui.journals.snapshot" => snapshot(conn),
        "ui.journals.events.list" => events(conn, args),
        "ui.journals.transfers.list" => transfers(conn, args),
        "ui.journals.quarantine" => crate::quarantine::snapshot(conn, args),
        _ => Err(AppError::new(
            "validation",
            format!("unknown journal view {kind}"),
        )),
    }
}

/// `quarantine_resolution.update_quarantine_metadata` price arithmetic:
/// derive the missing side at the default decimal context, then require a
/// positive finite rate and a nonnegative finite value.
pub fn override_prices(
    rate: Option<&Py>,
    total: Option<&Py>,
    amount_msat: i64,
) -> Result<
    (
        Option<kassiber_pydecimal::Decimal>,
        Option<kassiber_pydecimal::Decimal>,
    ),
    kassiber_envelope::AppError,
> {
    use kassiber_envelope::{py_str, AppError};
    let context = Context::RP2;
    let arithmetic = |error: kassiber_pydecimal::DecimalError| {
        AppError::new("internal_error", error.to_string())
    };
    enum Parsed {
        Finite(Decimal),
        NonFinite,
    }
    // msat.dec: str(value), with "" as zero; NaN and infinities are valid
    // Decimals that the later finiteness check refuses.
    let parse = |value: &Py| -> Result<Parsed, AppError> {
        if matches!(value, Py::Str(text) if text.code_points().next().is_none()) {
            return Ok(Parsed::Finite(Decimal::zero()));
        }
        let shown = py_str(value);
        let text = kassiber_pytext::strip(&shown);
        let unsigned = text.trim_start_matches(['+', '-']).to_ascii_lowercase();
        if matches!(unsigned.as_str(), "inf" | "infinity")
            || ((unsigned.starts_with("nan") && unsigned[3..].bytes().all(|b| b.is_ascii_digit()))
                || (unsigned.starts_with("snan")
                    && unsigned[4..].bytes().all(|b| b.is_ascii_digit())))
        {
            return Ok(Parsed::NonFinite);
        }
        text.parse::<Decimal>()
            .map(Parsed::Finite)
            .map_err(|_| AppError::new("app_error", format!("Invalid decimal value: {shown}")))
    };
    let rate_error = || AppError::new("validation", "Fiat rate must be positive and finite");
    let value_error = || AppError::new("validation", "Fiat value must be nonnegative and finite");
    let amount = Decimal::from(amount_msat)
        .div(&Decimal::from(100_000_000_000_i64), &context)
        .map_err(arithmetic)?
        .abs(&context)
        .map_err(arithmetic)?;
    let positive = amount.is_positive();
    match (rate, total) {
        (Some(rate), _) => match parse(rate)? {
            Parsed::NonFinite => Err(rate_error()),
            Parsed::Finite(rate) => {
                if !rate.is_positive() {
                    return Err(rate_error());
                }
                let total = positive
                    .then(|| rate.mul(&amount, &context))
                    .transpose()
                    .map_err(arithmetic)?;
                Ok((Some(rate), total))
            }
        },
        (None, Some(total)) => match parse(total)? {
            Parsed::NonFinite if positive => Err(rate_error()),
            Parsed::NonFinite => Err(value_error()),
            Parsed::Finite(total) => {
                let rate = positive
                    .then(|| total.div(&amount, &context))
                    .transpose()
                    .map_err(arithmetic)?;
                if rate.as_ref().is_some_and(|r| !r.is_positive()) {
                    return Err(rate_error());
                }
                if total.is_negative() {
                    return Err(value_error());
                }
                Ok((rate, Some(total)))
            }
        },
        (None, None) => Ok((None, None)),
    }
}

#[cfg(test)]
mod explanation_tests {
    use super::*;

    const BOOK: &str = "
        INSERT INTO workspaces(id, label, created_at) VALUES('ws-explain', 'Main', '2026-01-01T00:00:00Z');
        INSERT INTO profiles(id, workspace_id, label, fiat_currency, tax_country, tax_long_term_days,
            gains_algorithm, last_processed_at, last_processed_tx_count, created_at)
        VALUES('pf-explain', 'ws-explain', 'Default', 'EUR', 'generic', 365, 'fifo',
            '2026-01-01T01:00:00Z', 3, '2026-01-01T00:00:00Z');
        INSERT INTO wallets(id, workspace_id, profile_id, label, kind, config_json, created_at)
        VALUES('wal-explain', 'ws-explain', 'pf-explain', 'Cold', 'address', '{}', '2026-01-01T00:00:00Z');
        INSERT INTO transactions(id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
            occurred_at, confirmed_at, direction, asset, amount, fee, fiat_currency, fiat_rate,
            fiat_value, fiat_price_source, kind, description, excluded, raw_json, created_at)
        SELECT 'tx-' || c, 'ws-explain', 'pf-explain', 'wal-explain', c, 'fp-' || c,
            '2025-06-01T12:00:00Z', '2025-06-01T12:05:00Z', 'outbound', 'BTC', -10000000000, 0,
            'EUR', 1400, 140, 'import', 'sell', c, 0, '{}', '2025-06-01T12:00:00Z'
        FROM (SELECT 'sale' AS c UNION ALL SELECT 'swap' UNION ALL SELECT 'untaxed'
              UNION ALL SELECT 'income');
        UPDATE transactions SET taxability_override = 0 WHERE id = 'tx-untaxed';
        INSERT INTO journal_entries(id, workspace_id, profile_id, transaction_id, wallet_id,
            occurred_at, entry_type, asset, quantity, fiat_value, unit_cost, cost_basis, proceeds,
            gain_loss, description, at_category, at_kennzahl, created_at)
        SELECT 'je-' || c, 'ws-explain', 'pf-explain', 'tx-' || c, 'wal-explain',
            '2025-06-01T12:00:00Z', CASE c WHEN 'income' THEN 'income' ELSE 'disposal' END, 'BTC',
            -10000000000, -140, 1000, 100, 140, 40, c,
            CASE c WHEN 'swap' THEN 'neu_swap' END, NULL, '2025-06-01T12:00:00Z'
        FROM (SELECT 'sale' AS c UNION ALL SELECT 'swap' UNION ALL SELECT 'untaxed'
              UNION ALL SELECT 'income');
        INSERT INTO journal_entries(id, workspace_id, profile_id, transaction_id, wallet_id,
            occurred_at, entry_type, asset, quantity, fiat_value, created_at)
        VALUES('je-acquired', 'ws-explain', 'pf-explain', 'tx-sale', 'wal-explain',
            '2025-01-01T00:00:00Z', 'acquisition', 'BTC', 10000000000, 100, '2025-01-01T00:00:00Z');
        INSERT OR REPLACE INTO settings(key, value) VALUES('context_workspace', 'ws-explain'),
            ('context_profile', 'pf-explain');
        UPDATE profiles SET last_processed_tx_count =
            (SELECT COUNT(*) FROM transactions WHERE excluded = 0);
    ";

    fn references(conn: &Connection) -> Vec<(String, Option<String>)> {
        let snapshot = events(conn, None).unwrap();
        let Some(Py::List(events)) = get(&snapshot, "events") else {
            panic!("{snapshot:?}")
        };
        let mut result: Vec<_> = events
            .iter()
            .map(|event| {
                let reference = get(event, "explanationReference").unwrap();
                (
                    kassiber_envelope::py_str(get(event, "id").unwrap()),
                    match reference {
                        Py::Null => None,
                        reference => Some(kassiber_envelope::py_str(
                            get(reference, "entry_id").unwrap(),
                        )),
                    },
                )
            })
            .collect();
        result.sort();
        result
    }

    // tests/test_review_regressions.py::test_journal_events_reference_only_explainable_gains,
    // ported: only a realized gain or income that counts for tax has a
    // calculation to open, and a stale journal explains nothing.
    #[test]
    fn journal_events_reference_only_explainable_gains() {
        let dir =
            std::env::temp_dir().join(format!("kassiber-journal-events-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let book = kassiber_store::Book::create(&dir.join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(BOOK).unwrap();
        let some = |id: &str| Some(id.to_owned());
        let expected = |sale, income| {
            vec![
                ("je-acquired".to_owned(), None),
                ("je-income".to_owned(), income),
                ("je-sale".to_owned(), sale),
                ("je-swap".to_owned(), None),
                ("je-untaxed".to_owned(), None),
            ]
        };
        assert_eq!(
            references(conn),
            expected(some("je-sale"), some("je-income"))
        );
        let snapshot = events(conn, None).unwrap();
        let Some(Py::List(events)) = get(&snapshot, "events") else {
            unreachable!()
        };
        let sale = events
            .iter()
            .find(|e| get(e, "id") == Some(&s("je-sale")))
            .unwrap();
        let reference = get(sale, "explanationReference").unwrap();
        let keys: Vec<_> = match reference {
            Py::Dict(fields) => fields.iter().map(|(k, _)| format!("{k:?}")).collect(),
            other => panic!("{other:?}"),
        };
        assert_eq!(keys.len(), 6, "{keys:?}");
        assert_eq!(get(reference, "profile_id"), Some(&s("pf-explain")));
        conn.execute_batch("UPDATE profiles SET last_processed_at = NULL")
            .unwrap();
        assert_eq!(references(conn), expected(None, None));
        drop(book);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
