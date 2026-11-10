//! `build_journal_events_list_snapshot(conn, {"transaction": ref, "limit": n})`
//! for the transaction review context: stored journal entries with their
//! display pair, read without processing journals.
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql, Connection};

use crate::report_blockers::{rows, val};

type Result<T> = std::result::Result<T, AppError>;

const ENTRY_TYPE: &str =
    "CASE WHEN je.at_category = 'neu_swap' THEN 'neutral_swap' ELSE je.entry_type END";
const GAIN_LOSS: &str =
    "CASE WHEN je.at_category = 'neu_swap' THEN 0 ELSE COALESCE(je.gain_loss, 0) END";
const RELATION: &str = "SELECT profile_id, id, kind, policy, swap_fee_msat, swap_fee_kind, out_transaction_id, in_transaction_id, out_amount, in_amount, created_at FROM journal_custody_projection_relations WHERE relation_kind IN ('move', 'conversion') AND in_transaction_id IS NOT NULL";
const EMPTY_PAIR_JOIN: &str = "LEFT JOIN (SELECT NULL AS id, NULL AS kind, NULL AS policy, NULL AS swap_fee_msat, NULL AS out_transaction_id, NULL AS in_transaction_id, NULL AS out_amount WHERE 0) p_out ON 0 LEFT JOIN (SELECT NULL AS id, NULL AS kind, NULL AS policy, NULL AS swap_fee_msat, NULL AS out_transaction_id, NULL AS in_transaction_id, NULL AS out_amount WHERE 0) p_in ON 0";

fn pair_join() -> String {
    format!(
        "LEFT JOIN (SELECT * FROM (SELECT relation.*, ROW_NUMBER() OVER (PARTITION BY profile_id, out_transaction_id ORDER BY created_at, id) AS relation_rank FROM ({RELATION}) relation) WHERE relation_rank = 1) p_out ON p_out.profile_id = je.profile_id AND p_out.out_transaction_id = je.transaction_id \
         LEFT JOIN (SELECT * FROM (SELECT relation.*, ROW_NUMBER() OVER (PARTITION BY profile_id, in_transaction_id ORDER BY created_at, id) AS relation_rank FROM ({RELATION}) relation) WHERE relation_rank = 1) p_in ON p_in.profile_id = je.profile_id AND p_in.in_transaction_id = je.transaction_id"
    )
}

fn pick(column: &str) -> String {
    format!("CASE WHEN p_out.id IS NOT NULL THEN p_out.{column} ELSE p_in.{column} END")
}

fn float(value: &Value) -> Value {
    match value {
        Value::Int(n) => Value::Float(n.to_string().parse().unwrap_or(0.0)),
        Value::Float(f) => Value::Float(*f),
        Value::Str(t) => t
            .as_str()
            .and_then(|t| kassiber_pytext::parse_float(t).ok())
            .map_or(Value::Float(0.0), Value::Float),
        _ => Value::Float(0.0),
    }
}

/// `float(value or 0)`.
fn float_or_zero(value: &Value) -> Value {
    if truthy(value) {
        float(value)
    } else {
        Value::Float(0.0)
    }
}

fn optional_float(value: &Value) -> Value {
    if *value == Value::Null {
        Value::Null
    } else {
        float(value)
    }
}

fn msat(value: &Value) -> i128 {
    match value {
        Value::Int(n) => i128::try_from(n.clone()).unwrap_or(0),
        Value::Float(f) => f.trunc() as i128,
        _ => 0,
    }
}

fn btc(value: i128) -> Value {
    Value::Float(format!("{value}e-11").parse().unwrap_or(0.0))
}

fn or_empty(row: &Value, key: &str) -> Value {
    match get(row, key) {
        Some(v) if truthy(v) => v.clone(),
        _ => s(""),
    }
}

fn pair(row: &Value) -> Value {
    if !truthy(&val(row, "pair_id")) {
        return Value::Null;
    }
    let out_asset = val(row, "pair_out_asset");
    let in_asset = val(row, "pair_in_asset");
    let fee = msat(&val(row, "pair_swap_fee_msat"));
    let leg = |prefix: &str, asset: Value| {
        let amount = msat(&val(row, &format!("pair_{prefix}_amount")));
        dict([
            (
                "transactionId",
                val(row, &format!("pair_{prefix}_transaction_id")),
            ),
            (
                "externalId",
                or_empty(row, &format!("pair_{prefix}_external_id")),
            ),
            ("wallet", val(row, &format!("pair_{prefix}_wallet"))),
            ("asset", asset),
            ("amountMsat", Value::Int(amount.into())),
            ("amount", btc(amount)),
        ])
    };
    dict([
        ("pairId", val(row, "pair_id")),
        (
            "pairType",
            s(if out_asset == in_asset {
                "transfer"
            } else {
                "swap"
            }),
        ),
        ("kind", val(row, "pair_kind")),
        ("policy", val(row, "pair_policy")),
        ("swapFeeMsat", Value::Int(fee.into())),
        ("swapFee", btc(fee)),
        ("out", leg("out", out_asset.clone())),
        ("in", leg("in", in_asset.clone())),
    ])
}

fn event(row: &Value) -> Value {
    let neutral = val(row, "at_category") == s("neu_swap");
    let quantity = msat(&val(row, "quantity"));
    let cost = val(row, "cost_basis");
    let proceeds = val(row, "proceeds");
    let gain = val(row, "gain_loss");
    dict([
        ("id", val(row, "id")),
        ("transactionId", val(row, "transaction_id")),
        (
            "transactionExternalId",
            or_empty(row, "transaction_external_id"),
        ),
        (
            "transactionDirection",
            or_empty(row, "transaction_direction"),
        ),
        ("occurredAt", val(row, "occurred_at")),
        ("createdAt", val(row, "created_at")),
        (
            "entryType",
            if neutral {
                s("neutral_swap")
            } else {
                val(row, "entry_type")
            },
        ),
        ("wallet", val(row, "wallet")),
        ("account", val(row, "account")),
        ("accountLabel", val(row, "account_label")),
        ("asset", val(row, "asset")),
        ("quantity", btc(quantity)),
        ("quantityMsat", Value::Int(quantity.into())),
        ("fiatValueEur", float_or_zero(&val(row, "fiat_value"))),
        ("unitCostEur", float_or_zero(&val(row, "unit_cost"))),
        ("costBasisEur", optional_float(&cost)),
        (
            "proceedsEur",
            if neutral && cost != Value::Null {
                float(&cost)
            } else {
                optional_float(&proceeds)
            },
        ),
        (
            "gainLossEur",
            if neutral {
                Value::Float(0.0)
            } else {
                optional_float(&gain)
            },
        ),
        (
            "marketValueEur",
            if neutral {
                optional_float(&proceeds)
            } else {
                Value::Null
            },
        ),
        (
            "marketDeltaEur",
            if neutral {
                optional_float(&gain)
            } else {
                Value::Null
            },
        ),
        ("pricingSourceKind", or_empty(row, "pricing_source_kind")),
        ("pricingQuality", or_empty(row, "pricing_quality")),
        ("description", val(row, "description")),
        ("atCategory", val(row, "at_category")),
        ("atKennzahl", val(row, "at_kennzahl")),
        ("pair", pair(row)),
    ])
}

/// The review context's journal section for one transaction reference.
pub fn for_transaction(conn: &Connection, reference: &str, limit: i64) -> Result<Value> {
    let reference = kassiber_pytext::strip(reference).to_owned();
    let (context, profile) = kassiber_workspace::readiness::scope(conn, None)?;
    let Some(profile) = profile else {
        return Ok(dict([
            (
                "summary",
                dict([
                    ("workspace", Value::Null),
                    ("profile", Value::Null),
                    ("count", 0.into()),
                    ("reportableCount", 0.into()),
                    ("needsJournals", Value::Bool(false)),
                    ("lastProcessedAt", Value::Null),
                    ("freshnessStatus", s("no_profile")),
                    ("freshnessReason", s("no active profile")),
                    ("entryTypes", Value::List(vec![])),
                    ("limit", limit.into()),
                ]),
            ),
            ("events", Value::List(vec![])),
        ]));
    };
    let freshness = kassiber_workspace::readiness::journal_freshness(conn, Some(&profile))?;
    let join = if truthy(&val(&freshness, "needs_processing")) {
        EMPTY_PAIR_JOIN.to_owned()
    } else {
        pair_join()
    };
    let mut condition = "WHERE je.profile_id = ?".to_owned();
    let mut bound = vec![Sql::Text(py_str(&val(&profile, "id")))];
    if !reference.is_empty() {
        condition.push_str(" AND (je.transaction_id = ? OR t.external_id = ?)");
        bound.push(Sql::Text(reference.clone()));
        bound.push(Sql::Text(reference));
    }
    let summary = rows(
        conn,
        &format!("SELECT {ENTRY_TYPE} AS entry_type, COUNT(*) AS count, SUM({GAIN_LOSS}) AS gain_loss FROM journal_entries je LEFT JOIN transactions t ON t.id = je.transaction_id {condition} GROUP BY {ENTRY_TYPE} ORDER BY count DESC, entry_type ASC"),
        params_from_iter(bound.clone()),
    )?;
    let total: i128 = summary.iter().map(|r| msat(&val(r, "count"))).sum();
    let reportable = rows(
        conn,
        &format!("SELECT COUNT(*) AS count FROM journal_entries je LEFT JOIN transactions t ON t.id = je.transaction_id {condition} AND COALESCE(t.taxability_override, 1) != 0 AND ((je.entry_type = 'disposal' AND COALESCE(je.at_category, '') != 'neu_swap') OR je.entry_type = 'income' OR (je.entry_type NOT IN ('fee', 'transfer_fee') AND je.at_kennzahl IS NOT NULL))"),
        params_from_iter(bound.clone()),
    )?;
    let mut page_bind = bound;
    page_bind.push(Sql::Integer(limit));
    let found = rows(
        conn,
        &format!(
            "SELECT je.id, je.transaction_id, je.occurred_at, je.created_at, je.entry_type, je.asset, je.quantity, je.fiat_value, je.unit_cost, je.cost_basis, je.proceeds, je.gain_loss, je.pricing_source_kind, je.pricing_quality, COALESCE(je.description, '') AS description, je.at_category, je.at_kennzahl, w.label AS wallet, COALESCE(a.code, '') AS account, COALESCE(a.label, '') AS account_label, t.external_id AS transaction_external_id, t.direction AS transaction_direction, {id} AS pair_id, {kind} AS pair_kind, {policy} AS pair_policy, COALESCE({fee}, 0) AS pair_swap_fee_msat, {out_tx} AS pair_out_transaction_id, tout.external_id AS pair_out_external_id, wout.label AS pair_out_wallet, tout.asset AS pair_out_asset, COALESCE({out_amount}, tout.amount) AS pair_out_amount, {in_tx} AS pair_in_transaction_id, tin.external_id AS pair_in_external_id, win.label AS pair_in_wallet, tin.asset AS pair_in_asset, tin.amount AS pair_in_amount FROM journal_entries je JOIN wallets w ON w.id = je.wallet_id LEFT JOIN accounts a ON a.id = je.account_id LEFT JOIN transactions t ON t.id = je.transaction_id {join} LEFT JOIN transactions tout ON tout.id = {out_tx} LEFT JOIN transactions tin ON tin.id = {in_tx} LEFT JOIN wallets wout ON wout.id = tout.wallet_id LEFT JOIN wallets win ON win.id = tin.wallet_id {condition} ORDER BY je.occurred_at DESC, je.created_at DESC, je.id DESC LIMIT ?",
            id = pick("id"),
            kind = pick("kind"),
            policy = pick("policy"),
            fee = pick("swap_fee_msat"),
            out_tx = pick("out_transaction_id"),
            out_amount = pick("out_amount"),
            in_tx = pick("in_transaction_id"),
        ),
        params_from_iter(page_bind),
    )?;
    let label = |key: &str| {
        let value = val(&context, key);
        if truthy(&value) {
            value
        } else {
            Value::Null
        }
    };
    Ok(dict([
        (
            "summary",
            dict([
                ("workspace", label("workspace_label")),
                ("profile", label("profile_label")),
                ("count", Value::Int(total.into())),
                (
                    "reportableCount",
                    Value::Int(
                        reportable
                            .first()
                            .map_or(0, |r| msat(&val(r, "count")))
                            .into(),
                    ),
                ),
                ("needsJournals", val(&freshness, "needs_processing")),
                ("lastProcessedAt", val(&profile, "last_processed_at")),
                ("freshnessStatus", val(&freshness, "status")),
                ("freshnessReason", val(&freshness, "reason")),
                (
                    "entryTypes",
                    Value::List(
                        summary
                            .iter()
                            .map(|r| {
                                dict([
                                    ("type", val(r, "entry_type")),
                                    ("count", Value::Int(msat(&val(r, "count")).into())),
                                    ("gainLossEur", float_or_zero(&val(r, "gain_loss"))),
                                ])
                            })
                            .collect(),
                    ),
                ),
                ("limit", limit.into()),
            ]),
        ),
        ("events", Value::List(found.iter().map(event).collect())),
    ]))
}
