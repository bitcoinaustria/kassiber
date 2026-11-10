//! Observed transaction pages. Reading a custody projection never authors facts.
use std::time::{SystemTime, UNIX_EPOCH};

use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_pytext::{lower, strip, upper};
use kassiber_store::rusqlite::{params_from_iter, types::Value as SqlValue, Connection};

use crate::metadata::{cursor, records::wallet, time};
use crate::sql::{rows, string, Result, Scope};

#[derive(Default)]
pub struct Options {
    pub wallet: Option<String>,
    pub limit: Option<i64>,
    pub direction: Option<String>,
    pub asset: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub txids: Vec<String>,
    pub period: Option<String>,
    pub status: Option<String>,
    pub flow: Option<String>,
    pub payment_method: Option<String>,
    pub network: Option<String>,
    pub with_fees: bool,
    pub quick: Option<String>,
    pub cursor: Option<String>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

const SWAPS: &str = "'chain-swap','peg-in','peg-out','reverse-submarine-swap','submarine-swap','swap','swap-refund'";
const LAYER_TRANSITIONS: &str =
    "'chain-swap','peg-in','peg-out','reverse-submarine-swap','submarine-swap','swap-refund'";
const STATUS: &str = "CASE
 WHEN lower(COALESCE(t.review_status,'')) IN ('review','needs_review','needs-review','blocked','quarantined') OR jq.reason IS NOT NULL THEN 'review'
 WHEN lower(COALESCE(t.review_status,'')) IN ('failed','error') THEN 'failed'
 WHEN lower(COALESCE(t.review_status,'')) IN ('completed','complete') THEN 'completed'
 WHEN lower(COALESCE(t.review_status,''))='pending' THEN 'pending'
 WHEN t.confirmed_at IS NULL THEN 'pending' ELSE 'completed' END";
const PAYMENT: &str = "CASE
 WHEN lower(t.asset)='lbtc' OR lower(w.kind) IN ('liquid') OR lower(w.config_json) LIKE '%\"chain\"%liquid%' OR lower(w.label) LIKE '%liquid%' OR lower(w.label) LIKE '%lbtc%' THEN 'Liquid'
 WHEN lower(w.kind) IN ('lnd','core-ln','coreln','nwc','phoenix') OR lower(w.label) LIKE '%lightning%' OR lower(w.label) LIKE '%phoenix%' OR lower(w.label) LIKE '% ln%' OR lower(w.label) LIKE 'ln %' OR lower(w.label) LIKE '%(ln)%' THEN 'Lightning'
 WHEN lower(w.kind) IN ('kraken','bitstamp','coinbase','bitpanda','river','bullbitcoin','coinfinity','strike','exchange') OR lower(w.label) LIKE '%exchange%' THEN 'Exchange'
 ELSE 'On-chain' END";
const MISSING_PRICE: &str = "(t.fiat_rate IS NULL OR t.fiat_rate<=0)
 AND (t.fiat_rate_exact IS NULL OR CAST(t.fiat_rate_exact AS REAL)<=0)
 AND (t.fiat_value IS NULL OR t.fiat_value<=0)
 AND (t.fiat_value_exact IS NULL OR CAST(t.fiat_value_exact AS REAL)<=0)";
const FROM: &str = "FROM transactions t JOIN wallets w ON w.id=t.wallet_id LEFT JOIN journal_quarantines jq ON jq.transaction_id=t.id";
const COLUMNS: &str = "t.id,COALESCE(t.external_id,'') AS external_id,t.occurred_at,t.confirmed_at,t.created_at AS _created_at,w.label AS wallet,t.direction,t.asset,t.amount,t.fee,t.fiat_rate,t.fiat_value,t.fiat_rate_exact,t.fiat_value_exact,t.fiat_price_source,t.pricing_source_kind,t.pricing_provider,t.pricing_pair,t.pricing_timestamp,t.pricing_fetched_at,t.pricing_granularity,t.pricing_method,t.pricing_external_ref,t.pricing_quality,COALESCE(t.kind,'') AS kind,COALESCE(t.description,'') AS description,COALESCE(t.note,'') AS note,t.excluded";

fn projection(relation: Option<&str>) -> String {
    let moves = "EXISTS(SELECT 1 FROM journal_custody_decisions decision WHERE decision.profile_id=t.profile_id AND (decision.source_transaction_id=t.id OR decision.target_transaction_id=t.id))";
    let swaps = "EXISTS(SELECT 1 FROM journal_custody_economic_relations relation WHERE relation.profile_id=t.profile_id AND (relation.source_transaction_id=t.id OR relation.target_transaction_id=t.id))";
    let relation = match relation {
        Some("transfer") => moves.into(),
        Some("swap") => swaps.into(),
        _ => format!("({moves} OR {swaps})"),
    };
    format!("(EXISTS(SELECT 1 FROM profiles custody_profile WHERE custody_profile.id=t.profile_id AND custody_profile.last_processed_at IS NOT NULL AND custody_profile.journal_input_version=custody_profile.last_processed_input_version AND custody_profile.last_processed_tx_count=(SELECT COUNT(*) FROM transactions freshness_tx WHERE freshness_tx.profile_id=custody_profile.id AND freshness_tx.excluded=0) AND {}) AND {relation})", kassiber_store::semantics::current_sql("custody_profile"))
}

fn payment(value: &str) -> Option<&'static str> {
    match lower(strip(value)).as_str() {
        "exchange" => Some("Exchange"),
        "lightning" => Some("Lightning"),
        "liquid" => Some("Liquid"),
        "on-chain" | "onchain" | "on chain" => Some("On-chain"),
        _ => None,
    }
}

fn since(period: &str) -> Result<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    let micros = i64::try_from(now.as_micros())
        .map_err(|_| AppError::new("internal_error", "date value out of range"))?
        + 62_135_596_800_000_000;
    let days = match period {
        "all" => return Ok(String::new()),
        "ytd" => return Ok(format!("{}-01-01T00:00:00Z", &time::format_timestamp(micros)?[..4])),
        "30days" | "30day" | "30d" => 29,
        "3months" | "3month" | "3m" => 92,
        "6months" | "6month" | "6m" => 183,
        "1year" | "1years" | "1y" => 365,
        "5years" | "5year" | "5y" => 365 * 5,
        "10years" | "10year" | "10y" => 365 * 10,
        "15years" | "15year" | "15y" => 365 * 15,
        _ => return Err(AppError::new("validation", "--period must be one of: 30days, 3months, 6months, ytd, 1year, 5years, 10years, 15years, all")),
    };
    time::format_timestamp(micros - days * 86_400_000_000)
}

fn text<'a>(row: &'a Value, key: &str) -> &'a str {
    get(row, key).and_then(string).unwrap_or_default()
}
fn value(row: &Value, key: &str) -> Value {
    get(row, key).cloned().unwrap_or(Value::Null)
}
fn opt(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or_default()
}
fn set(row: &mut Value, key: &str, v: Value) {
    if let Value::Dict(pairs) = row {
        if let Some((_, old)) = pairs.iter_mut().find(|(k, _)| *k == Key::from(key)) {
            *old = v;
        } else {
            pairs.push((Key::from(key), v));
        }
    }
}

/// Convert the exact decimal to float once, as float(msat_to_btc(n)) does.
/// Casting the integer first loses msat above 2^53 before the division.
fn btc(msat: &Value) -> Result<Value> {
    let decimal = format!("{}e-11", py_str(msat));
    kassiber_pytext::parse_float(&decimal)
        .map(Value::Float)
        .map_err(|_| AppError::new("internal_error", "invalid stored msat amount"))
}

/// Returns records and pagination metadata for either transport adapter.
pub fn list(conn: &Connection, scope: &Scope<'_>, options: &Options) -> Result<Value> {
    let limit = cursor::limit(options.limit, 100)?;
    let direction = opt(&options.direction);
    if !direction.is_empty() && !["inbound", "outbound"].contains(&direction) {
        return Err(AppError::new(
            "validation",
            "--direction must be inbound or outbound",
        ));
    }
    let sort = options.sort.as_deref().unwrap_or("occurred-at");
    let column = match sort {
        "occurred-at" => "t.occurred_at",
        "amount" => "t.amount",
        "fee" => "t.fee",
        "fiat-value" => "COALESCE(t.fiat_value, 0)",
        _ => {
            return Err(AppError::new(
                "validation",
                format!("Unsupported transaction sort: {sort}"),
            )
            .with_hint("Use one of: occurred-at, amount, fiat-value, fee."))
        }
    };
    let order = options.order.as_deref().unwrap_or("desc");
    if !["asc", "desc"].contains(&order) {
        return Err(AppError::new("validation", "--order must be asc or desc"));
    }
    let ordering = if sort == "occurred-at" {
        format!("t.occurred_at {order},t.created_at {order},t.id {order}")
    } else {
        format!("{column} {order},t.occurred_at DESC,t.created_at DESC,t.id DESC")
    };
    let period = lower(strip(opt(&options.period)));
    let mut start = time::timestamp(options.start.as_deref(), "start")?;
    if start.is_empty() && !period.is_empty() {
        start = since(&period)?;
    }
    let end = time::timestamp(options.end.as_deref(), "end")?;
    let mut txids = Vec::new();
    for term in &options.txids {
        for part in term.split(',') {
            let term = lower(strip(part));
            if !term.is_empty() && !txids.contains(&term) {
                txids.push(term);
            }
        }
    }
    let wallet = wallet(conn, scope.profile_id, options.wallet.as_deref())?;
    let mut conditions = vec!["t.profile_id=?".to_owned()];
    let mut args = vec![SqlValue::Text(scope.profile_id.into())];
    let asset = upper(opt(&options.asset));
    for (clause, val) in [
        ("t.wallet_id=?", wallet.as_str()),
        ("t.direction=?", direction),
        ("upper(t.asset)=?", asset.as_str()),
    ] {
        if !val.is_empty() {
            conditions.push(clause.into());
            args.push(SqlValue::Text(val.into()));
        }
    }
    for (op, val) in [(">=", start.as_str()), ("<=", end.as_str())] {
        if !val.is_empty() {
            conditions.push(time::predicate(conn, "t.occurred_at", op)?);
            args.push(SqlValue::Text(val.into()));
        }
    }
    if !txids.is_empty() {
        let placeholders = vec!["?"; txids.len()].join(",");
        conditions.push(format!("(lower(t.id) IN ({placeholders}) OR lower(COALESCE(t.external_id,'')) IN ({placeholders}))"));
        for _ in 0..2 {
            args.extend(txids.iter().cloned().map(SqlValue::Text));
        }
    }
    let status = opt(&options.status);
    if !status.is_empty() {
        conditions.push(format!("({STATUS})=?"));
        args.push(SqlValue::Text(status.into()));
    }
    let flow = opt(&options.flow);
    match flow {
        "incoming" | "outgoing" => {
            conditions.push(format!("(t.direction=? AND lower(COALESCE(t.kind,'')) NOT IN ({SWAPS},'transfer') AND NOT {})",projection(None)));
            args.push(SqlValue::Text(
                if flow == "incoming" {
                    "inbound"
                } else {
                    "outbound"
                }
                .into(),
            ));
        }
        "transfer" => conditions.push(format!(
            "(lower(COALESCE(t.kind,''))='transfer' OR {})",
            projection(Some("transfer"))
        )),
        "swap" => conditions.push(format!(
            "(lower(COALESCE(t.kind,'')) IN ({SWAPS}) OR {})",
            projection(Some("swap"))
        )),
        "layer-transition" => conditions.push(format!(
            "lower(COALESCE(t.kind,'')) IN ({LAYER_TRANSITIONS})"
        )),
        _ => {}
    }
    let payment_method = opt(&options.payment_method);
    if !payment_method.is_empty() {
        let method = payment(payment_method).ok_or_else(|| {
            AppError::new(
                "validation",
                "--payment-method must be one of: On-chain, Exchange, Lightning, Liquid",
            )
        })?;
        conditions.push(format!("({PAYMENT})=?"));
        args.push(SqlValue::Text(method.into()));
    }
    let network = opt(&options.network);
    if !network.is_empty() {
        let normalized = lower(strip(network));
        if let Some(method) = payment(&normalized) {
            conditions.push(format!("({PAYMENT})=?"));
            args.push(SqlValue::Text(method.into()));
        } else {
            conditions.push("(lower(w.kind)=? OR CASE WHEN json_valid(w.config_json) THEN lower(json_extract(w.config_json,'$.chain')) END=? OR CASE WHEN json_valid(w.config_json) THEN lower(json_extract(w.config_json,'$.network')) END=? OR upper(t.asset)=?)".into());
            args.extend(std::iter::repeat_n(SqlValue::Text(normalized.clone()), 3));
            args.push(SqlValue::Text(upper(&normalized)));
        }
    }
    if options.with_fees {
        conditions.push("COALESCE(t.fee,0)<>0".into());
    }
    match opt(&options.quick) {
        "external_flow" => conditions.push("t.direction IN ('inbound','outbound') AND lower(COALESCE(t.kind,''))<>'transfer'".into()),
        "review_queue" => conditions.push(format!("({STATUS})<>'completed'")),
        "no_explorer_id" => conditions.push("(t.external_id IS NULL OR length(trim(t.external_id))<>64 OR lower(trim(t.external_id)) GLOB '*[^0-9a-f]*')".into()),
        "missing_price" => conditions.push(MISSING_PRICE.into()),
        "failed_import" => conditions.push(format!("({STATUS})='failed'")),
        _ => {}
    }
    let filters = dict([
        ("workspace_id", s(scope.workspace_id)),
        ("profile_id", s(scope.profile_id)),
        ("wallet_id", s(wallet)),
        ("direction", s(direction)),
        ("asset", s(asset)),
        (
            "start",
            s(if !period.is_empty() && opt(&options.start).is_empty() {
                ""
            } else {
                &start
            }),
        ),
        ("end", s(end)),
        ("txids", s(txids.join(","))),
        ("period", s(period)),
        ("status", s(status)),
        ("flow", s(flow)),
        ("payment_method", s(payment_method)),
        ("network", s(network)),
        ("with_fees", s(if options.with_fees { "1" } else { "" })),
        ("quick", s(opt(&options.quick))),
    ]);
    const CURSOR_KEYS: &[&str] = &["sort", "order", "value", "occurred_at", "created_at", "id"];
    let decoded = cursor::decode(options.cursor.as_deref(), &filters, CURSOR_KEYS, false)?;
    if decoded
        .as_ref()
        .is_some_and(|row| value(row, "sort") != s(sort) || value(row, "order") != s(order))
    {
        return Err(AppError::new("validation",format!("Invalid cursor: {}",opt(&options.cursor))).with_hint("Pass the exact next_cursor value from the previous response; do not modify it or change filters."));
    }
    let count = rows(
        conn,
        &format!(
            "SELECT COUNT(*) AS count {FROM} WHERE {}",
            conditions.join(" AND ")
        ),
        params_from_iter(&args),
    )?;
    if let Some(row) = decoded {
        let op = if order == "asc" { ">" } else { "<" };
        if sort == "occurred-at" {
            conditions.push(format!("(t.occurred_at,t.created_at,t.id){op}(?,?,?)"));
            for key in ["occurred_at", "created_at", "id"] {
                args.push(cursor::bind(&value(&row, key))?);
            }
        } else {
            conditions.push(format!(
                "({column}{op}? OR ({column}=? AND (t.occurred_at,t.created_at,t.id)<(?,?,?)))"
            ));
            for key in ["value", "value", "occurred_at", "created_at", "id"] {
                args.push(cursor::bind(&value(&row, key))?);
            }
        }
    }
    args.push(SqlValue::Integer(limit + 1));
    let mut page = rows(
        conn,
        &format!(
            // Select narrow identities before loading the wide presentation.
            // Without a matching sort index, SQLite's top-N sorter otherwise
            // copies pricing/description fields for every candidate row.
            "WITH page AS MATERIALIZED (SELECT t.rowid AS row_id {FROM} WHERE {} ORDER BY {ordering} LIMIT ?) SELECT {COLUMNS} FROM page JOIN transactions t ON t.rowid=page.row_id JOIN wallets w ON w.id=t.wallet_id ORDER BY {ordering}",
            conditions.join(" AND ")
        ),
        params_from_iter(args),
    )?;
    let more = page.len() > limit as usize;
    page.truncate(limit as usize);
    let next = if more {
        let row = page.last().unwrap();
        let primary = value(
            row,
            match sort {
                "occurred-at" => "occurred_at",
                "amount" => "amount",
                "fee" => "fee",
                _ => "fiat_value",
            },
        );
        let primary = if sort == "fiat-value" {
            Value::Float(if truthy(&primary) {
                kassiber_pytext::parse_float(&py_str(&primary))
                    .map_err(|_| AppError::new("internal_error", "invalid stored fiat value"))?
            } else {
                0.0
            })
        } else {
            primary
        };
        cursor::encode(
            &dict([
                ("sort", s(sort)),
                ("order", s(order)),
                ("value", primary),
                ("occurred_at", value(row, "occurred_at")),
                ("created_at", value(row, "_created_at")),
                ("id", value(row, "id")),
            ]),
            &filters,
            CURSOR_KEYS,
        )?
    } else {
        Value::Null
    };
    for row in &mut page {
        let tags = rows(conn,"SELECT tags.code,tags.label FROM transaction_tags tt JOIN tags ON tags.id=tt.tag_id WHERE tt.transaction_id=? ORDER BY tags.code ASC",[text(row,"id")])?;
        for key in ["amount", "fee"] {
            let msat = value(row, key);
            set(row, &format!("{key}_msat"), msat.clone());
            set(row, key, btc(&msat)?);
        }
        for key in ["fiat_rate", "fiat_value"] {
            let number = value(row, key);
            if !matches!(number, Value::Null)
                && kassiber_pytext::parse_float(&py_str(&number)).is_ok_and(|n| n <= 0.0)
            {
                set(row, key, Value::Null);
            }
        }
        let excluded = truthy(&value(row, "excluded"));
        set(row, "excluded", Value::Bool(excluded));
        set(row, "tags", Value::List(tags));
        if let Value::Dict(pairs) = row {
            pairs.retain(|(k, _)| *k != Key::from("_created_at"));
        }
    }
    Ok(dict([
        ("records", Value::List(page)),
        ("next_cursor", next),
        ("has_more", Value::Bool(more)),
        ("count", value(&count[0], "count")),
        ("total", value(&count[0], "count")),
        ("limit", Value::Int(limit.into())),
        ("sort", s(sort)),
        ("order", s(order)),
    ]))
}
