//! Desktop transaction reads over persisted observations and journal projections.
//! These reads never process journals, infer ownership or contact a provider.
mod audit;
mod dashboard;
mod lineage;
mod render;
mod resolve;
mod sql;

pub use dashboard::Zones;

use std::collections::{BTreeMap, BTreeSet};

use kassiber_envelope::{dict, get, py_str, s, truthy, type_name, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_pytext::{lower, strip, upper};
use kassiber_store::rusqlite::{params_from_iter, types::Value as SqlValue, Connection};

use crate::metadata::{cursor, time};
use crate::sql::{rows, Result, Scope};

pub const KINDS: &[&str] = &[
    "ui.transactions.list",
    "ui.transactions.dashboard",
    "ui.transactions.extremes",
    "ui.transactions.resolve",
    "ui.transactions.search",
];

/// Audit reads served by the same projection boundary, advertised separately.
pub const AUDIT_KINDS: &[&str] = &["ui.audit.changes_since_last_answer"];

/// The complete desktop projection allowlist; there is no generic kind dispatch.
#[derive(Clone, Copy, Debug)]
pub enum Projection {
    List,
    Search,
    Extremes,
    Resolve,
    AuditChanges,
    CustodyLineage,
}
impl Projection {
    pub fn from_kind(kind: &str) -> Option<Self> {
        match kind {
            "ui.transactions.list" => Some(Self::List),
            "ui.transactions.search" => Some(Self::Search),
            "ui.transactions.extremes" => Some(Self::Extremes),
            "ui.transactions.resolve" => Some(Self::Resolve),
            "ui.audit.changes_since_last_answer" => Some(Self::AuditChanges),
            "ui.custody.lineage.snapshot" => Some(Self::CustodyLineage),
            _ => None,
        }
    }
    pub fn kind(self) -> &'static str {
        match self {
            Self::List => "ui.transactions.list",
            Self::Search => "ui.transactions.search",
            Self::Extremes => "ui.transactions.extremes",
            Self::Resolve => "ui.transactions.resolve",
            Self::AuditChanges => "ui.audit.changes_since_last_answer",
            Self::CustodyLineage => "ui.custody.lineage.snapshot",
        }
    }
}

const FROM: &str = "FROM transactions t JOIN wallets w ON w.id=t.wallet_id LEFT JOIN journal_quarantines jq ON jq.transaction_id=t.id";
const COLUMNS: &str = "t.id,t.external_id,t.occurred_at,t.confirmed_at,t.created_at AS _created_at,w.label AS wallet,w.kind AS wallet_kind,w.config_json AS wallet_config_json,t.direction,t.asset,t.amount,t.fee,t.fiat_currency,t.fiat_value,t.fiat_rate,t.pricing_source_kind,t.pricing_quality,t.pricing_external_ref,t.pricing_provider,t.pricing_pair,t.pricing_timestamp,t.pricing_fetched_at,t.pricing_granularity,t.pricing_method,t.review_status,t.taxability_override,t.at_regime_override,t.at_category_override,t.kind_override,COALESCE(t.kind,'') AS kind,COALESCE(t.description,'') AS description,COALESCE(t.counterparty,'') AS counterparty,COALESCE(t.note,'') AS note,t.excluded,jq.reason AS quarantine_reason";
const SWAPS: &str = "'chain-swap','peg-in','peg-out','reverse-submarine-swap','submarine-swap','swap','swap-refund'";
const LAYERS: &str =
    "'chain-swap','peg-in','peg-out','reverse-submarine-swap','submarine-swap','swap-refund'";

fn val(row: &Value, key: &str) -> Value {
    get(row, key).cloned().unwrap_or(Value::Null)
}
fn text<'a>(row: &'a Value, key: &str) -> &'a str {
    get(row, key)
        .and_then(crate::sql::string)
        .unwrap_or_default()
}
fn num(row: &Value, key: &str) -> i64 {
    match get(row, key) {
        Some(Value::Int(n)) => i64::try_from(n.clone()).unwrap_or_default(),
        _ => 0,
    }
}
fn set(row: &mut Value, key: &str, value: Value) {
    if let Value::Dict(pairs) = row {
        if let Some((_, old)) = pairs.iter_mut().find(|(k, _)| *k == Key::from(key)) {
            *old = value;
        } else {
            pairs.push((Key::from(key), value));
        }
    }
}
fn nullable(value: &str) -> Value {
    if value.is_empty() {
        Value::Null
    } else {
        s(value)
    }
}
fn invalid(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}
fn optional(args: &Value, key: &str) -> Result<Option<String>> {
    let raw = val(args, key);
    if raw == Value::Null {
        return Ok(None);
    }
    match crate::sql::string(&raw)
        .map(strip)
        .filter(|t| !t.is_empty())
    {
        Some(t) => Ok(Some(t.to_owned())),
        None => {
            Err(invalid(format!("{key} must be a non-empty string"))
                .with_details(dict([(key, raw)])))
        }
    }
}
fn strings(args: &Value, key: &str, maximum: usize) -> Result<Vec<String>> {
    let raw = val(args, key);
    let list = match &raw {
        Value::Null => return Ok(Vec::new()),
        Value::Str(t) if t.as_str().is_some() => t.as_str().unwrap().split(',').map(s).collect(),
        Value::List(list) => list.clone(),
        _ => {
            return Err(
                invalid(format!("{key} must be a string or a list of strings"))
                    .with_details(dict([(key, raw)])),
            )
        }
    };
    let mut output = Vec::new();
    let mut seen = BTreeSet::new();
    for item in list {
        let Some(t) = crate::sql::string(&item) else {
            return Err(invalid(format!("{key} must contain only strings"))
                .with_details(dict([(key, raw)])));
        };
        let t = strip(t);
        if !t.is_empty() && seen.insert(lower(t)) {
            output.push(t.to_owned());
            if output.len() > maximum {
                return Err(invalid(format!("{key} accepts at most {maximum} values"))
                    .with_details(dict([
                        ("key", s(key)),
                        ("maximum", Value::Int(maximum.into())),
                    ])));
            }
        }
    }
    Ok(output)
}
fn flag(args: &Value, keys: &[&str]) -> Result<bool> {
    for key in keys {
        let Some(raw) = get(args, key) else {
            continue;
        };
        if cursor::equivalent(raw, &Value::Int(0.into())) {
            return Ok(false);
        }
        if cursor::equivalent(raw, &Value::Int(1.into())) {
            return Ok(true);
        }
        if let Some(t) = crate::sql::string(raw) {
            match lower(strip(t)).as_str() {
                "1" | "true" | "yes" | "on" | "with-fees" => return Ok(true),
                "0" | "false" | "no" | "off" | "all" | "" => return Ok(false),
                _ => {}
            }
        }
        return Err(
            invalid(format!("{key} must be a boolean")).with_details(dict([(*key, raw.clone())]))
        );
    }
    Ok(false)
}
fn limit(args: &Value, default: i64, maximum: i64) -> Result<i64> {
    let raw = get(args, "limit")
        .cloned()
        .unwrap_or(Value::Int(default.into()));
    let n = match &raw {
        Value::Int(n) => Some(n.clone()),
        Value::Bool(b) => Some(i64::from(*b).into()),
        Value::Float(n) if n.is_finite() => {
            kassiber_pytext::parse_int(&format!("{:.0}", n.trunc())).ok()
        }
        Value::Float(n) if n.is_infinite() => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        Value::Str(t) => t.as_str().and_then(|t| kassiber_pytext::parse_int(t).ok()),
        _ => None,
    }
    .ok_or_else(|| {
        invalid("limit must be an integer").with_details(dict([("limit", raw.clone())]))
    })?;
    if n < 1.into() {
        return Err(invalid("limit must be positive").with_details(dict([("limit", raw)])));
    }
    Ok(i64::try_from(n).unwrap_or(i64::MAX).min(maximum))
}
fn unknown(args: &Value, kind: &str, allowed: &[&str]) -> Result<()> {
    let Value::Dict(pairs) = args else {
        unreachable!()
    };
    let mut unknown: Vec<_> = pairs
        .iter()
        .filter_map(|(key, _)| match key {
            Key::Str(t) if !allowed.contains(&t.as_str().unwrap_or_default()) => {
                Some(Value::Str(t.clone()))
            }
            _ => None,
        })
        .collect();
    unknown.sort_by_key(py_str);
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(invalid(format!("{kind} received unsupported filters"))
            .with_details(dict([("unknown", Value::List(unknown))])))
    }
}
fn normalized(
    args: &Value,
    key: &str,
    accepted: &[&str],
    message: &str,
    convert: impl FnOnce(&str) -> String,
) -> Result<Option<String>> {
    let Some(raw) = optional(args, key)? else {
        return Ok(None);
    };
    let normalized = convert(&raw);
    if accepted.contains(&normalized.as_str()) {
        Ok(Some(normalized))
    } else {
        Err(invalid(message).with_details(dict([(key, s(raw))])))
    }
}
fn payment(t: &str) -> String {
    match lower(t).as_str() {
        "on-chain" | "onchain" | "on chain" => "On-chain".into(),
        "exchange" => "Exchange".into(),
        "lightning" => "Lightning".into(),
        "liquid" => "Liquid".into(),
        _ => String::new(),
    }
}
fn wallet_sql(wallet: &str) -> String {
    if wallet.is_empty() {
        "1".into()
    } else {
        "t.wallet_id IN (SELECT id FROM wallets WHERE profile_id=t.profile_id AND (id=? OR lower(label)=lower(?)))".into()
    }
}
fn wallet_args(args: &mut Vec<SqlValue>, wallet: &str) {
    if !wallet.is_empty() {
        args.extend([SqlValue::Text(wallet.into()), SqlValue::Text(wallet.into())]);
    }
}
fn current(conn: &Connection, scope: &Scope<'_>) -> Result<bool> {
    let result = rows(conn, &format!("SELECT CASE WHEN (SELECT COUNT(*) FROM transactions WHERE profile_id=p.id AND excluded=0)=0 THEN 1 ELSE COALESCE(p.last_processed_at,'')<>'' AND p.last_processed_tx_count=(SELECT COUNT(*) FROM transactions WHERE profile_id=p.id AND excluded=0) AND p.journal_input_version=p.last_processed_input_version AND {} END AS current FROM profiles p WHERE p.id=?",kassiber_store::semantics::current_sql("p")), [scope.profile_id])?;
    Ok(result.first().is_some_and(|r| truthy(&val(r, "current"))))
}
/// `transaction_review_queue_counts(conn, profile_id)` over the whole book.
pub fn review_queue_counts(conn: &Connection, scope: &Scope<'_>) -> Result<Value> {
    let current = current(conn, scope)?;
    let counts = rows(
        conn,
        &format!(
            "SELECT queue_leg.item_rank AS rank,COUNT(DISTINCT queue_leg.item_key) AS count FROM ({}) queue_leg GROUP BY queue_leg.item_rank",
            queue_items(conn, "", "", "", current)?
        ),
        params_from_iter(queue_args(scope, "", "", "")),
    )?;
    let count = |rank: i64| {
        counts
            .iter()
            .find(|row| num(row, "rank") == rank)
            .map_or(0, |row| num(row, "count"))
    };
    let (review, pending, failed) = (count(3), count(1), count(2));
    Ok(dict([
        ("review", review.into()),
        ("pending", pending.into()),
        ("failed", failed.into()),
        ("total", (review + pending + failed).into()),
        ("pairs_grouped", Value::Bool(current)),
        ("scope", s(if current {
            "all transactions in the active book, any date; the two legs of a reviewed transfer or swap count as one item"
        } else {
            "all transactions in the active book, any date; journals are not current, so each leg of a transfer or swap counts separately until they are processed"
        })),
        ("definitions", dict([
            ("review", s("Marked for review (review/needs_review/blocked) or held in journal quarantine; needs a decision before reports are trustworthy.")),
            ("pending", s("Marked pending by the user, or, without a status the user set, no confirmation timestamp is recorded: unconfirmed on-chain, or imported from a ledger/CSV without a confirmation time. It is not a quarantine and does not by itself block reports.")),
            ("failed", s("Marked failed by its import or source.")),
        ])),
    ]))
}
/// Overview rows (`_transactions`, `_activity_transactions`): newest-first
/// recent rows with tags, or the uncapped oldest-first BTC/L-BTC activity
/// with each leg's running balance and cost basis.
pub enum OverviewRows<'a> {
    Recent,
    /// Running `(balance_btc, cost_basis)` per transaction id.
    Activity(&'a dyn Fn(&str) -> (f64, f64)),
}
pub fn overview_rows(
    conn: &Connection,
    scope: &Scope<'_>,
    shape: OverviewRows<'_>,
) -> Result<Vec<Value>> {
    let activity = matches!(shape, OverviewRows::Activity(_));
    let sql = if activity {
        format!("SELECT {COLUMNS},{} AS payment_method {FROM} WHERE t.profile_id=? AND t.asset IN ('BTC','LBTC') AND COALESCE(t.excluded,0)=0 ORDER BY t.occurred_at ASC,t.created_at ASC,t.id ASC", sql::PAYMENT)
    } else {
        format!("SELECT {COLUMNS},{} AS payment_method {FROM} WHERE t.profile_id=? ORDER BY t.occurred_at DESC,t.created_at DESC,t.id DESC LIMIT 40", sql::PAYMENT)
    };
    let mut raw = rows(conn, &sql, [scope.profile_id])?;
    let current = current(conn, scope)?;
    let mut pairs = BTreeMap::new();
    if activity {
        // Python's 400-row chunks let a later chunk replace a leg's pair.
        for chunk in raw.chunks(400) {
            pairs.extend(render::pairs(conn, chunk, current)?);
        }
    } else {
        pairs = render::pairs(conn, &raw, current)?;
    }
    if let OverviewRows::Activity(running) = &shape {
        for row in &mut raw {
            let (balance, basis) = running(text(row, "id"));
            set(row, "running_balance_btc", Value::Float(balance));
            set(row, "running_cost_basis_eur", Value::Float(basis));
        }
    }
    let mut last_leg: BTreeMap<&str, &Value> = BTreeMap::new();
    if activity {
        for row in &raw {
            if let Some(pair) = pairs.get(text(row, "id")) {
                last_leg.insert(text(pair, "pair_id"), row);
            }
        }
    }
    let ranks = rows(
        conn,
        &queue_items(conn, "", "", "", current)?,
        params_from_iter(queue_args(scope, "", "", "")),
    )?;
    let ranks: BTreeMap<_, _> = ranks
        .iter()
        .map(|r| (text(r, "id").to_owned(), num(r, "item_rank")))
        .collect();
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    for row in &raw {
        let pair = pairs.get(text(row, "id"));
        let mut shown = row;
        if let Some(pair) = pair {
            if !seen.insert(text(pair, "pair_id")) {
                continue;
            }
            if activity {
                shown = last_leg.get(text(pair, "pair_id")).copied().unwrap_or(row);
            }
        }
        let mut payload = render::row_with(conn, shown, pair, !activity)?;
        if pair.is_some_and(|p| text(p, "kind") != "journal-derived") {
            if let Some(rank) = ranks.get(text(&payload, "id")) {
                set(
                    &mut payload,
                    "queueStatus",
                    s(match rank {
                        3 => "review",
                        2 => "failed",
                        1 => "pending",
                        _ => "completed",
                    }),
                );
            }
        }
        output.push(payload);
    }
    if !activity {
        output.truncate(20);
    }
    Ok(output)
}
fn group(current: bool) -> &'static str {
    if current {
        sql::GROUP
    } else {
        "'tx:' || t.id"
    }
}
fn queue_items(
    conn: &Connection,
    wallet: &str,
    since: &str,
    until: &str,
    current: bool,
) -> Result<String> {
    let mut conditions = vec!["t.profile_id=?".to_owned(), wallet_sql(wallet)];
    // This subquery is embedded with its own bindings; callers bind queue_args first.
    for (op, bound) in [(">=", since), ("<=", until)] {
        if !bound.is_empty() {
            conditions.push(time::predicate(conn, "t.occurred_at", op)?);
        }
    }
    Ok(format!("SELECT leg.id,leg.item_key,MAX(leg.rank) OVER (PARTITION BY leg.item_key) AS item_rank FROM (SELECT t.id,{} AS item_key,CASE ({}) WHEN 'review' THEN 3 WHEN 'failed' THEN 2 WHEN 'pending' THEN 1 ELSE 0 END AS rank {FROM} WHERE {}) leg", group(current),sql::STATUS,conditions.join(" AND ")))
}
fn queue_args(scope: &Scope<'_>, wallet: &str, since: &str, until: &str) -> Vec<SqlValue> {
    let mut args = vec![SqlValue::Text(scope.profile_id.into())];
    wallet_args(&mut args, wallet);
    for bound in [since, until] {
        if !bound.is_empty() {
            args.push(SqlValue::Text(bound.into()));
        }
    }
    args
}
fn cursor_error() -> AppError {
    invalid("Invalid cursor").with_hint("Pass the exact nextCursor value from the previous response; do not modify it or change filters.")
}
const CURSOR_KEYS: &[&str] = &[
    "sort",
    "order",
    "value",
    "value_is_null",
    "occurred_at",
    "created_at",
    "id",
];
fn decode(args: &Value, filters: &Value, sort: &str, order: &str) -> Result<Option<Value>> {
    let raw = val(args, "cursor");
    if raw == Value::Null || raw == s("") {
        return Ok(None);
    }
    let Some(raw) = crate::sql::string(&raw) else {
        return Err(invalid("cursor must be a string"));
    };
    let row = cursor::decode(Some(raw), filters, CURSOR_KEYS, false)
        .map_err(|_| cursor_error())?
        .unwrap();
    if val(&row, "sort") != s(sort)
        || val(&row, "order") != s(order)
        || !matches!(val(&row, "value_is_null"), Value::Bool(_))
        || val(&row, "value_is_null") != Value::Bool(val(&row, "value") == Value::Null)
        || get(&row, "skip_pairs").is_some_and(|v| !matches!(v, Value::List(_)))
    {
        return Err(cursor_error());
    }
    Ok(Some(row))
}
fn year(conn: &Connection, page: &[Value]) -> Result<Value> {
    for row in page {
        let t = text(row, "occurred_at");
        if let Some(year) = t.get(..4).filter(|t| t.chars().all(|c| c.is_ascii_digit())) {
            return Ok(Value::Int(kassiber_pytext::parse_int(year).unwrap()));
        }
    }
    Ok(Value::Int(
        crate::sql::now(conn)?[..4].parse::<i64>().unwrap().into(),
    ))
}

/// `ui.transactions.dashboard` over suggestion candidates the caller computed.
pub fn dashboard(
    conn: &Connection,
    scope: &Scope<'_>,
    args: &Value,
    candidates: &[Value],
    zones: &Zones<'_>,
) -> Result<Value> {
    dashboard::execute(conn, scope, args, candidates, zones)
}

/// Existing desktop kinds share one projection, independently of the CLI renderer.
pub fn execute(
    conn: &Connection,
    scope: &Scope<'_>,
    projection: Projection,
    args: Option<&Value>,
    pair_reading: &dyn Fn(&str, &str) -> Result<Value>,
) -> Result<Value> {
    if matches!(projection, Projection::Resolve) {
        return resolve::execute(conn, scope, args, pair_reading);
    }
    if matches!(projection, Projection::AuditChanges) {
        return audit::changes(conn, scope, args);
    }
    if matches!(projection, Projection::CustodyLineage) {
        return lineage::snapshot(conn, scope, args);
    }
    let kind = projection.kind();
    let empty = dict::<&str>([]);
    let args = args.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    // List/search deliberately short circuit before argument validation without a book.
    if kind != "ui.transactions.extremes"
        && (scope.workspace_id.is_empty() || scope.profile_id.is_empty())
    {
        let mut filters = dict([("limit", Value::Int(0.into()))]);
        if kind == "ui.transactions.search" {
            set(&mut filters, "query", s(""));
        }
        return Ok(dict([
            ("txs", Value::List(vec![])),
            ("year", year(conn, &[])?),
            ("filters", filters),
            ("nextCursor", Value::Null),
            ("hasMore", Value::Bool(false)),
        ]));
    }
    if !matches!(args, Value::Dict(_)) {
        return Err(invalid("ui snapshot args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    }
    if kind == "ui.transactions.extremes" {
        unknown(
            args,
            kind,
            &["limit", "direction", "asset", "wallet", "since"],
        )?;
        let limit = limit(args, 3, 20)?;
        let mut base = args.clone();
        set(&mut base, "limit", Value::Int(limit.into()));
        set(&mut base, "sort", s("amount"));
        set(&mut base, "order", s("desc"));
        let largest = execute(conn, scope, Projection::List, Some(&base), pair_reading)?;
        set(&mut base, "order", s("asc"));
        let smallest = execute(conn, scope, Projection::List, Some(&base), pair_reading)?;
        let filters = val(&largest, "filters");
        return Ok(dict([
            ("largest", val(&largest, "txs")),
            ("smallest", val(&smallest, "txs")),
            (
                "filters",
                dict([
                    ("limit", Value::Int(limit.into())),
                    ("direction", val(&filters, "direction")),
                    ("asset", val(&filters, "asset")),
                    ("wallet", val(&filters, "wallet")),
                    ("since", val(&filters, "since")),
                    ("sort", s("amount")),
                    ("scope", s("all_time_before_limit")),
                ]),
            ),
        ]));
    }
    page(conn, scope, kind, args)
}

fn page(conn: &Connection, scope: &Scope<'_>, kind: &str, args: &Value) -> Result<Value> {
    unknown(
        args,
        kind,
        &[
            "limit",
            "cursor",
            "query",
            "txids",
            "candidate_txids",
            "direction",
            "asset",
            "wallet",
            "since",
            "start",
            "until",
            "end",
            "period",
            "status",
            "flow",
            "payment_method",
            "paymentMethod",
            "network",
            "with_fees",
            "withFees",
            "quick",
            "sort",
            "order",
        ],
    )?;
    let query = optional(args, "query")?.unwrap_or_default();
    if query.is_empty() && kind == "ui.transactions.search" {
        return Err(invalid(format!("{kind} query must be a non-empty string")));
    }
    let limit = limit(
        args,
        if kind == "ui.transactions.search" {
            25
        } else {
            100
        },
        if kind == "ui.transactions.search" {
            100
        } else {
            500
        },
    )?;
    let current = current(conn, scope)?;
    let direction = normalized(
        args,
        "direction",
        &["inbound", "outbound"],
        "direction must be inbound or outbound",
        str::to_owned,
    )?
    .unwrap_or_default();
    let asset = optional(args, "asset")?
        .map(|s| upper(&s))
        .unwrap_or_default();
    let wallet = optional(args, "wallet")?.unwrap_or_default();
    let period = normalized(args,"period",&["all","ytd","30days","30day","30d","3months","3month","3m","6months","6month","6m","1year","1years","1y","5years","5year","5y","10years","10year","10y","15years","15year","15y"],"period must be one of: 30days, 3months, 6months, ytd, 1year, 5years, 10years, 15years, all",|t| { let t=lower(t).replace(['_','-',' '],""); if t=="max" {"all".into()} else {t} })?.unwrap_or_default();
    let since = optional(args, "since")?.or(optional_if_absent(args, "since", "start")?);
    let since = match since {
        Some(t) => time::timestamp(Some(&t), "since")?,
        None => period_since(conn, &period)?,
    };
    let until = optional(args, "until")?.or(optional_if_absent(args, "until", "end")?);
    let until = time::timestamp(until.as_deref(), "until")?;
    let ids = strings(args, "txids", 128)?;
    let candidates = strings(args, "candidate_txids", 512)?;
    let status = normalized(
        args,
        "status",
        &["completed", "pending", "failed", "review"],
        "status must be one of: completed, pending, failed, review",
        |t| match lower(t).replace('-', "_").as_str() {
            "complete" => "completed".into(),
            "error" => "failed".into(),
            "needs_review" | "blocked" | "quarantined" => "review".into(),
            t => t.into(),
        },
    )?
    .unwrap_or_default();
    let flow = normalized(
        args,
        "flow",
        &[
            "incoming",
            "outgoing",
            "transfer",
            "swap",
            "layer-transition",
        ],
        "flow must be one of: incoming, outgoing, transfer, swap, layer-transition",
        |t| lower(t).replace('_', "-"),
    )?
    .unwrap_or_default();
    let method_raw = optional(args, "payment_method")?.or(optional_if_absent(
        args,
        "payment_method",
        "paymentMethod",
    )?);
    let method = if let Some(raw) = method_raw {
        let method = payment(&raw);
        if method.is_empty() {
            return Err(invalid(
                "payment_method must be one of: On-chain, Exchange, Lightning, Liquid",
            )
            .with_details(dict([("payment_method", s(raw))])));
        }
        method
    } else {
        String::new()
    };
    let network = optional(args, "network")?.unwrap_or_default();
    let fees = flag(args, &["with_fees", "withFees"])?;
    let quick = normalized(args,"quick",&["external_flow","review_queue","no_explorer_id","missing_price","failed_import"],"quick must be one of: external_flow, review_queue, no_explorer_id, missing_price, failed_import",|t|lower(t).replace('-',"_"))?.unwrap_or_default();
    let sort = get(args, "sort").cloned().unwrap_or(s("occurred-at"));
    let column = match crate::sql::string(&sort) {
        Some("occurred-at") => "t.occurred_at",
        Some("amount") => sql::AMOUNT,
        Some("fee") => sql::FEE,
        Some("fiat-value") => sql::FIAT_VALUE,
        _ => {
            return Err(
                invalid("sort must be one of: occurred-at, amount, fiat-value, fee")
                    .with_details(dict([("sort", sort)])),
            )
        }
    };
    let sort = crate::sql::string(&sort).unwrap();
    let order = get(args, "order").cloned().unwrap_or(s("desc"));
    let order = match crate::sql::string(&order) {
        Some("asc") => "asc",
        Some("desc") => "desc",
        _ => {
            return Err(invalid("order must be asc or desc").with_details(dict([("order", order)])))
        }
    };
    let queue_sql = queue_items(conn, &wallet, &since, &until, current)?;
    let queue_bind = queue_args(scope, &wallet, &since, &until);
    let mut conditions = vec!["t.profile_id=?".to_owned(), wallet_sql(&wallet)];
    let mut bind = vec![SqlValue::Text(scope.profile_id.into())];
    wallet_args(&mut bind, &wallet);
    for (clause, t) in [
        ("t.direction=?", direction.as_str()),
        ("upper(t.asset)=?", asset.as_str()),
    ] {
        if !t.is_empty() {
            conditions.push(clause.into());
            bind.push(SqlValue::Text(t.into()));
        }
    }
    for (op, t) in [(">=", since.as_str()), ("<=", until.as_str())] {
        if !t.is_empty() {
            conditions.push(time::predicate(conn, "t.occurred_at", op)?);
            bind.push(SqlValue::Text(t.into()));
        }
    }
    if !ids.is_empty() {
        let terms = vec!["?"; ids.len()].join(",");
        conditions.push(format!(
            "(lower(t.id) IN ({terms}) OR lower(COALESCE(t.external_id,'')) IN ({terms}))"
        ));
        for _ in 0..2 {
            bind.extend(ids.iter().map(|t| SqlValue::Text(lower(t))));
        }
    }
    let rank = |t| match t {
        "review" => 3,
        "failed" => 2,
        "pending" => 1,
        _ => 0,
    };
    if !status.is_empty() {
        conditions.push(format!(
            "t.id IN (SELECT id FROM ({queue_sql}) WHERE item_rank=?)"
        ));
        bind.extend(queue_bind.clone());
        bind.push(SqlValue::Integer(rank(&status)));
    }
    let transfer = if current { sql::TRANSFER } else { "0" };
    let swap = if current { sql::SWAP } else { "0" };
    match flow.as_str() {
        "incoming"|"outgoing" => conditions.push(format!("(t.direction='{}' AND lower(COALESCE(t.kind,'')) NOT IN ({SWAPS},'transfer') AND NOT ({transfer} OR {swap}))",if flow=="incoming" {"inbound"} else {"outbound"})),
        "transfer"|"swap"|"layer-transition" => {
            let base=match flow.as_str() {"transfer"=>format!("lower(COALESCE(t.kind,''))='transfer' OR {transfer}"),"swap"=>format!("lower(COALESCE(t.kind,'')) IN ({SWAPS}) OR {swap}"),_=>format!("lower(COALESCE(t.kind,'')) IN ({LAYERS})")};
            let candidate=if candidates.is_empty() {String::new()} else {format!(" OR lower(t.id) IN ({})",vec!["?";candidates.len()].join(","))};
            conditions.push(format!("({base}{candidate})")); bind.extend(candidates.iter().map(|t|SqlValue::Text(lower(t))));
        }
        _=>{}
    }
    if !method.is_empty() {
        conditions.push(format!("({})=?", sql::PAYMENT));
        bind.push(SqlValue::Text(method.clone()));
    }
    if !network.is_empty() {
        let normalized = lower(&network);
        let payment = payment(&normalized);
        if !payment.is_empty() {
            conditions.push(format!("({})=?", sql::PAYMENT));
            bind.push(SqlValue::Text(payment));
        } else {
            conditions.push("(lower(w.kind)=? OR CASE WHEN json_valid(w.config_json) THEN lower(json_extract(w.config_json,'$.chain')) END=? OR CASE WHEN json_valid(w.config_json) THEN lower(json_extract(w.config_json,'$.network')) END=? OR upper(t.asset)=?)".into());
            bind.extend(std::iter::repeat_n(SqlValue::Text(normalized.clone()), 3));
            bind.push(SqlValue::Text(upper(&normalized)));
        }
    }
    if fees {
        conditions.push("COALESCE(t.fee,0)<>0".into());
    }
    match quick.as_str() {
        "external_flow"=>conditions.push("t.direction IN ('inbound','outbound') AND lower(COALESCE(t.kind,''))<>'transfer'".into()),
        "review_queue"|"failed_import"=>{conditions.push(format!("t.id IN (SELECT id FROM ({queue_sql}) WHERE item_rank{})",if quick=="review_queue" {"<>0"} else {"=2"}));bind.extend(queue_bind.clone());}
        "no_explorer_id"=>conditions.push("(t.external_id IS NULL OR length(trim(t.external_id))<>64 OR lower(trim(t.external_id)) GLOB '*[^0-9a-f]*')".into()),
        "missing_price"=>conditions.push("(t.fiat_rate IS NULL OR t.fiat_rate<=0 OR lower(COALESCE(t.pricing_quality,''))='missing' OR lower(COALESCE(t.pricing_source_kind,''))='missing')".into()),
        _=>{}
    }
    if !query.is_empty() {
        conditions.push("(lower(t.id) LIKE ? OR lower(COALESCE(t.external_id,'')) LIKE ? OR lower(COALESCE(t.kind,'')) LIKE ? OR lower(COALESCE(t.description,'')) LIKE ? OR lower(COALESCE(t.counterparty,'')) LIKE ? OR lower(COALESCE(t.note,'')) LIKE ? OR lower(w.label) LIKE ? OR EXISTS(SELECT 1 FROM transaction_tags tt JOIN tags ON tags.id=tt.tag_id WHERE tt.transaction_id=t.id AND lower(tags.label) LIKE ?))".into());
        bind.extend(std::iter::repeat_n(
            SqlValue::Text(format!("%{}%", lower(&query))),
            8,
        ));
    }
    let filters = dict([
        ("workspace_id", s(scope.workspace_id)),
        ("profile_id", s(scope.profile_id)),
        ("direction", s(direction.as_str())),
        ("asset", s(asset.as_str())),
        ("wallet", s(wallet.as_str())),
        ("since", s(since.as_str())),
        ("until", s(until.as_str())),
        ("query", s(query.as_str())),
        ("txids", s(ids.join(","))),
        ("candidate_txids", s(candidates.join(","))),
        ("status", s(status.as_str())),
        ("flow", s(flow.as_str())),
        ("payment_method", s(method.as_str())),
        ("network", s(network.as_str())),
        ("with_fees", s(if fees { "1" } else { "" })),
        ("quick", s(quick.as_str())),
    ]);
    let count = rows(
        conn,
        &format!(
            "SELECT COUNT(DISTINCT {}) AS count {FROM} WHERE {}",
            group(current),
            conditions.join(" AND ")
        ),
        params_from_iter(&bind),
    )?;
    let decoded = decode(args, &filters, sort, order)?;
    if let Some(row) = &decoded {
        let op = if order == "asc" { ">" } else { "<" };
        let tie_op = if sort == "occurred-at" { op } else { "<" };
        let tie = format!("(t.occurred_at,t.created_at,t.id){tie_op}(?,?,?)");
        let mut keys = Vec::new();
        if sort == "occurred-at" {
            conditions.push(tie);
        } else if truthy(&val(row, "value_is_null")) {
            conditions.push(format!("({column} IS NULL AND {tie})"));
        } else {
            conditions.push(format!("(({column} IS NOT NULL AND ({column}{op}? OR ({column}=? AND {tie}))) OR {column} IS NULL)"));
            keys.extend(["value", "value"]);
        }
        keys.extend(["occurred_at", "created_at", "id"]);
        for key in keys {
            bind.push(cursor::bind(&val(row, key))?);
        }
    }
    let ordering = if sort == "occurred-at" {
        format!("t.occurred_at {order},t.created_at {order},t.id {order}")
    } else {
        format!("({column}) IS NULL ASC,{column} {order},t.occurred_at DESC,t.created_at DESC,t.id DESC")
    };
    let raw_limit = (limit * 6).max(limit + 20);
    bind.push(SqlValue::Integer(raw_limit + 1));
    let mut raw=rows(conn,&format!("SELECT {COLUMNS},{} AS payment_method,{column} AS _sort_value {FROM} WHERE {} ORDER BY {ordering} LIMIT ?",sql::PAYMENT,conditions.join(" AND ")),params_from_iter(bind))?;
    let raw_more = raw.len() > raw_limit as usize;
    raw.truncate(raw_limit as usize);
    let pairs = render::pairs(conn, &raw, current)?;
    let mut pending: BTreeSet<String> = match decoded.as_ref().and_then(|r| get(r, "skip_pairs")) {
        Some(Value::List(items)) => items.iter().map(py_str).collect(),
        _ => BTreeSet::new(),
    };
    let mut seen = BTreeMap::new();
    let mut consumed = BTreeSet::new();
    let mut last = None;
    let mut output = Vec::new();
    let mut more = raw_more;
    let ranks = rows(conn, &queue_sql, params_from_iter(queue_bind))?;
    let ranks: BTreeMap<_, _> = ranks
        .iter()
        .map(|r| (text(r, "id").to_owned(), num(r, "item_rank")))
        .collect();
    for row in &raw {
        let pair = pairs.get(text(row, "id"));
        let pair_id = pair.map(|p| text(p, "pair_id"));
        let would_render = pair_id.is_none_or(|id| !pending.contains(id) && !seen.contains_key(id));
        if output.len() >= limit as usize && would_render {
            more = true;
            break;
        }
        last = Some(row);
        consumed.insert(text(row, "id"));
        if let Some(id) = pair_id {
            if pending.remove(id) {
                continue;
            }
            if seen.contains_key(id) {
                continue;
            }
            seen.insert(id, pair.unwrap());
        }
        let mut payload = render::row(conn, row, pair)?;
        if pair.is_some_and(|p| text(p, "kind") != "journal-derived") {
            if let Some(rank) = ranks.get(text(row, "id")) {
                set(
                    &mut payload,
                    "queueStatus",
                    s(match rank {
                        3 => "review",
                        2 => "failed",
                        1 => "pending",
                        _ => "completed",
                    }),
                );
            }
        }
        output.push(payload);
    }
    for (id, pair) in seen {
        if !consumed.contains(text(pair, "out_transaction_id"))
            || !consumed.contains(text(pair, "in_transaction_id"))
        {
            pending.insert(id.to_owned());
        }
    }
    let next = if more {
        if let Some(row) = last {
            let value = match sort {
                "fiat-value" if val(row, "_sort_value") != Value::Null => Value::Float(
                    kassiber_pytext::parse_float(&py_str(&val(row, "_sort_value"))).unwrap(),
                ),
                _ => val(row, "_sort_value"),
            };
            let payload = dict([
                ("sort", s(sort)),
                ("order", s(order)),
                ("value_is_null", Value::Bool(value == Value::Null)),
                ("value", value),
                ("occurred_at", val(row, "occurred_at")),
                ("created_at", val(row, "_created_at")),
                ("id", val(row, "id")),
                (
                    "skip_pairs",
                    Value::List(pending.iter().map(|t| s(t.as_str())).collect()),
                ),
            ]);
            let mut keys = CURSOR_KEYS.to_vec();
            keys.push("skip_pairs");
            // The cursor encoder sorts all keys, including the filter object.
            cursor::encode(&payload, &filters, &keys)?
        } else {
            Value::Null
        }
    } else {
        Value::Null
    };
    Ok(dict([
        ("txs", Value::List(output)),
        ("year", year(conn, &raw)?),
        (
            "filters",
            dict([
                ("query", nullable(&query)),
                ("limit", Value::Int(limit.into())),
                ("period", nullable(&period)),
                ("direction", nullable(&direction)),
                ("asset", nullable(&asset)),
                ("wallet", nullable(&wallet)),
                ("since", nullable(&since)),
                ("until", nullable(&until)),
                (
                    "txids",
                    Value::List(ids.iter().map(|t| s(t.as_str())).collect()),
                ),
                (
                    "candidateTxids",
                    Value::List(candidates.iter().map(|t| s(t.as_str())).collect()),
                ),
                ("status", nullable(&status)),
                ("flow", nullable(&flow)),
                ("paymentMethod", nullable(&method)),
                ("network", nullable(&network)),
                ("withFees", Value::Bool(fees)),
                ("quick", nullable(&quick)),
                ("sort", s(sort)),
                ("order", s(order)),
            ]),
        ),
        ("count", val(&count[0], "count")),
        ("total", val(&count[0], "count")),
        ("nextCursor", next),
        ("hasMore", Value::Bool(more)),
    ]))
}
fn optional_if_absent(args: &Value, primary: &str, alias: &str) -> Result<Option<String>> {
    if get(args, primary).is_none_or(|v| *v == Value::Null) {
        optional(args, alias)
    } else {
        Ok(None)
    }
}
fn period_since(conn: &Connection, period: &str) -> Result<String> {
    if period.is_empty() || period == "all" {
        return Ok(String::new());
    }
    let now = crate::sql::now(conn)?;
    if period == "ytd" {
        return Ok(format!("{}-01-01T00:00:00Z", &now[..4]));
    }
    let days = match period {
        "30days" | "30day" | "30d" => 29,
        "3months" | "3month" | "3m" => 92,
        "6months" | "6month" | "6m" => 183,
        "1year" | "1years" | "1y" => 365,
        "5years" | "5year" | "5y" => 365 * 5,
        "10years" | "10year" | "10y" => 365 * 10,
        _ => 365 * 15,
    };
    let day = time::timestamp(Some(&now[..10]), "since")?;
    let instant = time::instant(&day).ok_or_else(|| invalid("invalid clock"))?;
    time::format_timestamp(instant - days * 86_400_000_000)
}
