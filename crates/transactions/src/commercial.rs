//! Stored commercial evidence and deterministic, propose-only matching.
use crate::{
    documents::{choice, decimal, error, or_empty, reference_failure, sql, text, value},
    sql::{atomic, now, rows, uuid, Result, Scope},
};
use kassiber_envelope::{dict, s, truthy, AppError};
use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_pyjson::Value;
use kassiber_pytext::{lower, strip, upper};
use kassiber_store::rusqlite::{params, Connection};
use std::collections::BTreeSet;

pub enum Command {
    Suggest { limit: i64 },
    Links { state: Value, limit: i64 },
    Records { record_type: Value, limit: i64 },
    Context { transaction: String },
    Review(Review),
}
mod review;
pub use review::Review;
const LINK_SELECT: &str = "SELECT cl.*, p.stable_key AS btcpay_stable_key, p.invoice_id, p.payment_id, p.txid, p.payment_request_id, p.origin_kind, p.origin_label, d.label AS document_label, t.external_id AS transaction_external_id FROM commercial_links cl LEFT JOIN btcpay_provenance_records p ON p.id=cl.btcpay_record_id LEFT JOIN external_documents d ON d.id=cl.document_id LEFT JOIN transactions t ON t.id=cl.transaction_id";
fn limit(n: i64, default: i64, max: i64) -> i64 {
    if n == 0 {
        default
    } else {
        n.clamp(1, max)
    }
}
pub fn execute(conn: &Connection, scope: &Scope<'_>, command: &Command) -> Result<Value> {
    let result = (|| match command {
        Command::Suggest { limit: n } => atomic(conn, || suggest(conn, scope, limit(*n, 500, 500))),
        Command::Links { state, limit: n } => {
            let state = if truthy(state) {
                Some(choice(
                    state,
                    &["suggested", "reviewed", "rejected"],
                    "link state",
                )?)
            } else {
                None
            };
            Ok(Value::List(rows(conn,&format!("{LINK_SELECT} WHERE cl.profile_id=? AND (? IS NULL OR cl.state=?) ORDER BY cl.created_at DESC, cl.id DESC LIMIT ?"),params![scope.profile_id,state,state,limit(*n,100,1000)])?.iter().map(link_payload).collect::<Result<_>>()?))
        }
        Command::Records {
            record_type,
            limit: n,
        } => {
            let kind = if truthy(record_type) {
                Some(choice(
                    record_type,
                    &["invoice", "payment", "payout"],
                    "record type",
                )?)
            } else {
                None
            };
            Ok(Value::List(rows(conn,"SELECT * FROM btcpay_provenance_records WHERE profile_id=? AND (? IS NULL OR record_type=?) ORDER BY COALESCE(occurred_at,created_at) DESC,id DESC LIMIT ?",params![scope.profile_id,kind,kind,limit(*n,100,1000)])?.iter().map(record_payload).collect::<Result<_>>()?))
        }
        Command::Context { transaction } => context(conn, scope, transaction),
        Command::Review(request) => atomic(conn, || review::review(conn, scope, request)),
    })();
    result.map_err(reference_failure)
}
fn integer(v: &Value) -> Result<i128> {
    let number = match v {
        Value::Null => Some(0),
        Value::Int(n) => i128::try_from(n.clone()).ok(),
        Value::Bool(b) => Some(i128::from(*b)),
        Value::Float(n) if n.is_finite() && n.abs() < i128::MAX as f64 => Some(*n as i128),
        Value::Str(t) => t
            .as_str()
            .and_then(|s| kassiber_pytext::parse_int(s).ok())
            .and_then(|n| i128::try_from(n).ok()),
        _ => None,
    };
    number.ok_or_else(|| AppError::new("internal_error", "Invalid stored commercial amount"))
}
fn btc(v: &Value) -> Result<Value> {
    if matches!(v, Value::Null) {
        return Ok(Value::Null);
    }
    let n = integer(v)?;
    let negative = if n < 0 { "-" } else { "" };
    let n = n.unsigned_abs();
    let number = format!(
        "{negative}{}.{:011}",
        n / 100_000_000_000,
        n % 100_000_000_000
    )
    .parse::<f64>()
    .map_err(|_| AppError::new("validation", "Invalid stored amount"))?;
    Ok(Value::Float(number))
}
fn link_payload(row: &Value) -> Result<Value> {
    let mut pairs = Vec::new();
    for key in [
        "id",
        "btcpay_record_id",
        "btcpay_stable_key",
        "invoice_id",
        "payment_id",
        "payment_request_id",
        "origin_kind",
        "origin_label",
        "txid",
        "document_id",
        "document_label",
        "transaction_id",
        "transaction_external_id",
        "link_type",
        "state",
        "confidence",
        "method",
        "allocation_amount_msat",
        "allocation_amount",
        "allocation_fiat_exact",
        "reconciliation_state",
        "commercial_kind",
        "reviewed_record_snapshot_sha256",
        "has_applied_transaction_snapshot",
        "notes",
        "reviewed_at",
        "created_at",
        "updated_at",
    ] {
        let v = match key {
            "allocation_amount_msat" => value(row, "allocation_amount"),
            "allocation_amount" => btc(&value(row, "allocation_amount"))?,
            "has_applied_transaction_snapshot" => {
                Value::Bool(truthy(&value(row, "applied_transaction_snapshot_json")))
            }
            "id"
            | "link_type"
            | "state"
            | "confidence"
            | "method"
            | "reconciliation_state"
            | "created_at"
            | "updated_at" => value(row, key),
            _ => or_empty(row, key),
        };
        pairs.push((key, v));
    }
    Ok(dict(pairs))
}
fn record_payload(row: &Value) -> Result<Value> {
    let mut pairs = Vec::new();
    for key in [
        "id",
        "record_type",
        "stable_key",
        "backend",
        "store_id",
        "payment_method_id",
        "invoice_id",
        "payment_id",
        "order_id",
        "status",
        "occurred_at",
        "asset",
        "amount_msat",
        "amount",
        "txid",
        "payment_hash",
        "payment_request_id",
        "origin_kind",
        "origin_app_id",
        "origin_label",
        "origin_url",
        "origin_source",
        "fiat_currency",
        "fiat_value_exact",
        "fiat_rate_exact",
        "pricing_timestamp",
        "updated_at",
    ] {
        let v = match key {
            "amount_msat" => value(row, "amount"),
            "amount" => btc(&value(row, "amount"))?,
            "backend" => or_empty(row, "backend_name"),
            "id" | "record_type" | "stable_key" | "store_id" | "updated_at" => value(row, key),
            _ => or_empty(row, key),
        };
        pairs.push((key, v));
    }
    Ok(dict(pairs))
}
fn matching(conn: &Connection, profile: &str, record: &Value, stable: bool) -> Result<Vec<Value>> {
    let txid = value(record, "txid");
    let hash = value(record, "payment_hash");
    let key = if stable {
        value(record, "stable_key")
    } else {
        Value::Null
    };
    rows(conn,"SELECT id,direction,amount,fee,amount_includes_fee,fiat_value_exact,fiat_value FROM transactions WHERE profile_id=? AND excluded=0 AND ((? AND external_id=?) OR (? AND payment_hash=?) OR (? AND external_id=?)) AND (? != 'payout' OR direction='outbound') ORDER BY occurred_at ASC,id ASC",params![profile,truthy(&txid),sql(&txid)?,truthy(&hash),sql(&hash)?,truthy(&key),sql(&key)?,text(record,"record_type")])
}
struct Suggestion<'a> {
    record: &'a str,
    document: Option<&'a str>,
    transaction: Option<&'a str>,
    kind: &'a str,
    confidence: &'a str,
    method: &'a str,
    amount: Value,
    fiat: Value,
}
fn upsert(
    conn: &Connection,
    scope: &Scope<'_>,
    a: Suggestion<'_>,
    now: &str,
) -> Result<(bool, Value)> {
    let existing=rows(conn,"SELECT id,document_id FROM commercial_links WHERE profile_id=? AND COALESCE(btcpay_record_id,'')=COALESCE(?,'') AND (?='btcpay_payment_transaction' OR COALESCE(document_id,'')=COALESCE(?,'')) AND COALESCE(transaction_id,'')=COALESCE(?,'') AND link_type=? AND state!='rejected'",params![scope.profile_id,a.record,a.kind,a.document,a.transaction,a.kind]);
    let existing = existing?;
    let (created, id) = if let Some(row) = existing.first() {
        if a.kind == "btcpay_payment_transaction"
            && a.document.is_some_and(|s| !s.is_empty())
            && !truthy(&value(row, "document_id"))
        {
            conn.execute("UPDATE commercial_links SET document_id=?,allocation_fiat_exact=COALESCE(?,allocation_fiat_exact),method=?,updated_at=? WHERE id=?",params![a.document,sql(&a.fiat)?,a.method,now,text(row,"id")]).map_err(error)?;
        }
        (false, text(row, "id").to_owned())
    } else {
        let id = uuid(conn)?;
        conn.execute("INSERT INTO commercial_links(id,workspace_id,profile_id,btcpay_record_id,document_id,transaction_id,link_type,state,confidence,method,allocation_amount,allocation_fiat_exact,reconciliation_state,commercial_kind,notes,created_at,updated_at) VALUES(?,?,?,?,?,?,?,'suggested',?,?,?,?,'unreviewed',NULL,NULL,?,?)",params![id,scope.workspace_id,scope.profile_id,a.record,a.document,a.transaction,a.kind,a.confidence,a.method,sql(&a.amount)?,sql(&a.fiat)?,now,now]).map_err(error)?;
        (true, id)
    };
    let row = rows(
        conn,
        &format!("{LINK_SELECT} WHERE cl.profile_id=? AND cl.id=?"),
        [scope.profile_id, &id],
    )?;
    Ok((created, link_payload(&row[0])?))
}
fn suggest(conn: &Connection, scope: &Scope<'_>, limit: i64) -> Result<Value> {
    let now = now(conn)?;
    let mut created = 0;
    let mut suggestions = Vec::new();
    let records=rows(conn,"SELECT * FROM btcpay_provenance_records WHERE profile_id=? AND (record_type='payment' OR (record_type='payout' AND LOWER(COALESCE(status,'')) IN ('inprogress','completed'))) ORDER BY COALESCE(occurred_at,created_at) DESC LIMIT ?",params![scope.profile_id,limit])?;
    for record in &records {
        let matches = matching(conn, scope.profile_id, record, true)?;
        if matches.len() != 1 {
            continue;
        }
        let tx = &matches[0];
        let amount = value(tx, "amount");
        let mut principal = integer(&amount)?;
        if lower(strip(text(tx, "direction"))) == "outbound"
            && truthy(&value(tx, "amount_includes_fee"))
        {
            principal -= integer(&value(tx, "fee"))?;
        }
        let exact = !matches!(value(record, "amount"), Value::Null)
            && integer(&value(record, "amount"))? == principal.abs();
        let (new, link) = upsert(
            conn,
            scope,
            Suggestion {
                record: text(record, "id"),
                document: None,
                transaction: Some(text(tx, "id")),
                kind: "btcpay_payment_transaction",
                confidence: if exact { "exact" } else { "strong" },
                method: if text(record, "record_type") == "payout" {
                    "payout_proof"
                } else {
                    "txid_or_payment_hash"
                },
                amount: value(record, "amount"),
                fiat: value(record, "fiat_value_exact"),
            },
            &now,
        )?;
        created += usize::from(new);
        suggestions.push(link);
    }
    for record_type in ["invoice", "payment"] {
        let matches=rows(conn,"SELECT d.id AS document_id,p.id AS btcpay_record_id,p.txid,p.payment_hash,d.fiat_value_exact AS doc_value,p.fiat_value_exact AS btcpay_value,p.amount AS payment_amount FROM external_documents d JOIN btcpay_provenance_records p ON p.profile_id=d.profile_id AND p.record_type=? AND ((d.external_ref IS NOT NULL AND d.external_ref!='' AND (d.external_ref=p.invoice_id OR d.external_ref=p.order_id OR d.external_ref=p.payment_request_id)) OR (d.fiat_currency IS NOT NULL AND p.fiat_currency=d.fiat_currency AND d.fiat_value_exact IS NOT NULL AND p.fiat_value_exact=d.fiat_value_exact)) WHERE d.profile_id=? LIMIT 500",[record_type,scope.profile_id])?;
        for row in &matches {
            let tx = if record_type == "payment" {
                // This reference query deliberately does not project stable_key.
                let tx = matching(conn, scope.profile_id, row, false)?;
                if tx.len() != 1 {
                    continue;
                }
                Some(text(&tx[0], "id").to_owned())
            } else {
                None
            };
            let fiat = if truthy(&value(row, "doc_value")) {
                value(row, "doc_value")
            } else {
                value(row, "btcpay_value")
            };
            let (new, link) = upsert(
                conn,
                scope,
                Suggestion {
                    record: text(row, "btcpay_record_id"),
                    document: Some(text(row, "document_id")),
                    transaction: tx.as_deref(),
                    kind: if tx.is_some() {
                        "btcpay_payment_transaction"
                    } else {
                        "document_btcpay"
                    },
                    confidence: "strong",
                    method: if tx.is_some() {
                        "document_btcpay_txid_or_payment_hash"
                    } else {
                        "document_reference_or_amount"
                    },
                    amount: if tx.is_some() {
                        value(row, "payment_amount")
                    } else {
                        Value::Null
                    },
                    fiat,
                },
                &now,
            )?;
            created += usize::from(new);
            suggestions.push(link);
        }
    }
    // Dict assignment replaces a duplicate's value without moving its position.
    let mut unique: Vec<Value> = Vec::new();
    for link in suggestions {
        if let Some(old) = unique
            .iter_mut()
            .find(|old| text(old, "id") == text(&link, "id"))
        {
            *old = link;
        } else {
            unique.push(link);
        }
    }
    Ok(dict([
        ("created", Value::Int(created.into())),
        ("suggestions", Value::List(unique)),
    ]))
}

fn link_context(row: &Value) -> Value {
    dict(
        [
            "id",
            "invoice_id",
            "payment_id",
            "document_id",
            "document_label",
            "link_type",
            "state",
            "confidence",
            "reconciliation_state",
            "commercial_kind",
            "reviewed_at",
        ]
        .map(|key| {
            (
                key,
                if matches!(
                    key,
                    "id" | "link_type" | "state" | "confidence" | "reconciliation_state"
                ) {
                    value(row, key)
                } else {
                    or_empty(row, key)
                },
            )
        }),
    )
}
fn fiat_currency(v: &Value) -> String {
    let code = if truthy(v) {
        upper(strip(&kassiber_envelope::py_str(v)))
    } else {
        String::new()
    };
    if ["BTC", "SATS", "SAT", "MSAT", "LBTC"].contains(&code.as_str()) {
        String::new()
    } else {
        code
    }
}
fn record_context(row: &Value, prefix: &str) -> Result<Value> {
    let id = value(row, &format!("{prefix}_record_id"));
    if !truthy(&id) {
        return Ok(Value::Null);
    }
    let currency = fiat_currency(&value(row, &format!("{prefix}_fiat_currency")));
    let mut pairs = vec![("id", id)];
    for key in [
        "record_type",
        "invoice_id",
        "payment_id",
        "order_id",
        "status",
        "occurred_at",
        "asset",
        "amount_msat",
        "amount",
        "payment_request_id",
        "origin_kind",
        "origin_app_id",
        "origin_label",
        "origin_url",
        "fiat_currency",
        "fiat_value_exact",
        "fiat_rate_exact",
        "pricing_timestamp",
        "updated_at",
    ] {
        let name = format!(
            "{prefix}_{}",
            if key == "amount_msat" { "amount" } else { key }
        );
        let v = match key {
            "amount_msat" | "record_type" | "updated_at" => value(row, &name),
            "amount" => btc(&value(row, &name))?,
            "fiat_currency" => s(currency.as_str()),
            "fiat_value_exact" | "fiat_rate_exact" if currency.is_empty() => s(""),
            _ => or_empty(row, &name),
        };
        pairs.push((key, v));
    }
    Ok(dict(pairs))
}
fn context(conn: &Connection, scope: &Scope<'_>, reference: &str) -> Result<Value> {
    let id = crate::attachments::resolve_transaction(conn, scope.profile_id, reference)?;
    let tx = rows(
        conn,
        "SELECT external_id FROM transactions WHERE profile_id=? AND id=?",
        [scope.profile_id, &id],
    )?;
    let selected = rows(conn, CONTEXT_SQL, [scope.profile_id, &id])?;
    let mut links = Vec::new();
    let mut btcpay = Vec::new();
    let mut documents = Vec::new();
    let mut stores = BTreeSet::new();
    let mut payouts = Vec::new();
    for row in &selected {
        let link = link_context(row);
        links.push(link.clone());
        if truthy(&value(row, "payment_store_id")) && truthy(&value(row, "payment_backend_name")) {
            stores.insert((
                lower(strip(text(row, "payment_backend_name"))),
                text(row, "payment_store_id").to_owned(),
            ));
        }
        if text(row, "payment_record_type") == "payout"
            && text(row, "link_type") == "btcpay_payment_transaction"
        {
            payouts.push(row);
        }
        if truthy(&value(row, "ctx_document_id")) {
            let doc = dict([
                ("id", value(row, "ctx_document_id")),
                ("document_type", value(row, "ctx_document_type")),
                ("label", value(row, "ctx_document_label")),
                ("external_ref", or_empty(row, "ctx_document_external_ref")),
                ("review_state", value(row, "ctx_document_review_state")),
            ]);
            if let Some(old) = documents
                .iter_mut()
                .find(|d| text(d, "id") == text(&doc, "id"))
            {
                *old = doc;
            } else {
                documents.push(doc);
            }
        }
        let payment = record_context(row, "payment")?;
        let invoice = record_context(row, "invoice")?;
        let source = if truthy(&invoice) { &invoice } else { &payment };
        if truthy(&payment) || truthy(&invoice) {
            let request = if truthy(&value(source, "payment_request_id")) {
                dict([
                    ("id", value(source, "payment_request_id")),
                    (
                        "label",
                        if truthy(&value(source, "origin_label")) {
                            value(source, "origin_label")
                        } else {
                            value(source, "payment_request_id")
                        },
                    ),
                    ("status", or_empty(source, "status")),
                    ("url", or_empty(source, "origin_url")),
                ])
            } else {
                Value::Null
            };
            let origin = if truthy(&value(source, "origin_kind"))
                && text(source, "origin_kind") != "unknown"
            {
                dict([
                    ("kind", value(source, "origin_kind")),
                    ("app_id", or_empty(source, "origin_app_id")),
                    ("label", or_empty(source, "origin_label")),
                    ("url", or_empty(source, "origin_url")),
                ])
            } else {
                Value::Null
            };
            btcpay.push(dict([
                ("link", link),
                ("payment", payment),
                ("invoice", invoice),
                ("payment_request", request),
                ("origin", origin),
            ]));
        }
    }
    let batch = if payouts.len() > 1 {
        let currencies: BTreeSet<_> = payouts
            .iter()
            .map(|r| text(r, "payment_fiat_currency"))
            .filter(|s| !s.is_empty())
            .collect();
        let priced = currencies.len() == 1
            && payouts.iter().all(|r| {
                truthy(&value(r, "payment_fiat_value_exact"))
                    && truthy(&value(r, "payment_fiat_currency"))
            });
        let total = if priced {
            let mut total = Decimal::zero();
            let context = Context::new(32, Rounding::HalfEven);
            for r in &payouts {
                total = total
                    .add(&decimal(&value(r, "payment_fiat_value_exact"))?, &context)
                    .map_err(|e| AppError::new("app_error", e.to_string()))?;
            }
            s(total
                .to_fixed_string()
                .map_err(|e| AppError::new("app_error", e.to_string()))?)
        } else {
            Value::Null
        };
        dict([
            ("size", Value::Int(payouts.len().into())),
            (
                "link_ids",
                Value::List(payouts.iter().map(|r| value(r, "id")).collect()),
            ),
            (
                "amount_msat",
                Value::Int(
                    payouts
                        .iter()
                        .try_fold(0i128, |sum, r| {
                            integer(&value(r, "payment_amount")).map(|n| sum + n.abs())
                        })?
                        .into(),
                ),
            ),
            (
                "fiat_currency",
                if currencies.len() == 1 {
                    s(*currencies.first().expect("one currency"))
                } else {
                    Value::Null
                },
            ),
            ("fiat_value_exact", total),
            ("mixed_currencies", Value::Bool(currencies.len() > 1)),
        ])
    } else {
        Value::Null
    };
    let now = now(conn)?;
    let mut freshness = Vec::new();
    for (backend, store) in stores {
        let state=rows(conn,"SELECT * FROM btcpay_store_sync_states WHERE profile_id=? AND backend_name=? AND store_id=?",[scope.profile_id,&backend,&store])?;
        let empty = dict::<&str>([]);
        let state = state.first().unwrap_or(&empty);
        let success = value(state, "last_success_at");
        let age = if truthy(&success) {
            crate::metadata::time::instant(text(state, "last_success_at")).map(|t| {
                ((crate::metadata::time::instant(&now).expect("current instant") / 1_000_000)
                    - (t / 1_000_000))
                    .max(0)
            })
        } else {
            None
        };
        freshness.push(dict([
            ("backend", s(backend)),
            ("store_id", s(store)),
            ("last_attempt_at", value(state, "last_attempt_at")),
            ("last_success_at", success.clone()),
            ("last_error_code", value(state, "last_error_code")),
            (
                "age_seconds",
                age.map_or(Value::Null, |n| Value::Int(n.into())),
            ),
            (
                "stale",
                Value::Bool(
                    age.is_none_or(|n| n > 86400) || truthy(&value(state, "last_error_code")),
                ),
            ),
            ("never_synced", Value::Bool(!truthy(&success))),
        ]));
    }
    Ok(dict([
        ("transaction_id", s(id)),
        ("transaction_external_id", or_empty(&tx[0], "external_id")),
        ("links", Value::List(links)),
        ("btcpay", Value::List(btcpay)),
        ("documents", Value::List(documents)),
        ("payout_batch", batch),
        ("btcpay_freshness", Value::List(freshness)),
    ]))
}

const CONTEXT_SQL: &str = r#"SELECT cl.*, p.invoice_id, p.payment_id,
               d.label AS document_label,
               p.id AS payment_record_id, p.record_type AS payment_record_type,
               p.invoice_id AS payment_invoice_id,
               p.payment_id AS payment_payment_id, p.order_id AS payment_order_id,
               p.status AS payment_status, p.occurred_at AS payment_occurred_at,
               p.asset AS payment_asset, p.amount AS payment_amount,
               p.payment_request_id AS payment_payment_request_id,
               p.origin_kind AS payment_origin_kind,
               p.origin_app_id AS payment_origin_app_id,
               p.origin_label AS payment_origin_label,
               p.origin_url AS payment_origin_url,
               p.fiat_currency AS payment_fiat_currency,
               p.fiat_value_exact AS payment_fiat_value_exact,
               p.fiat_rate_exact AS payment_fiat_rate_exact,
               p.pricing_timestamp AS payment_pricing_timestamp,
               p.updated_at AS payment_updated_at,
               p.backend_name AS payment_backend_name,
               p.store_id AS payment_store_id,
               inv.id AS invoice_record_id, inv.record_type AS invoice_record_type,
               inv.invoice_id AS invoice_invoice_id,
               inv.payment_id AS invoice_payment_id,
               inv.order_id AS invoice_order_id,
               inv.status AS invoice_status,
               inv.occurred_at AS invoice_occurred_at,
               inv.asset AS invoice_asset,
               inv.amount AS invoice_amount,
               inv.payment_request_id AS invoice_payment_request_id,
               inv.origin_kind AS invoice_origin_kind,
               inv.origin_app_id AS invoice_origin_app_id,
               inv.origin_label AS invoice_origin_label,
               inv.origin_url AS invoice_origin_url,
               inv.fiat_currency AS invoice_fiat_currency,
               inv.fiat_value_exact AS invoice_fiat_value_exact,
               inv.fiat_rate_exact AS invoice_fiat_rate_exact,
               inv.pricing_timestamp AS invoice_pricing_timestamp,
               inv.updated_at AS invoice_updated_at,
               d.id AS ctx_document_id,
               d.document_type AS ctx_document_type,
               d.label AS ctx_document_label,
               d.external_ref AS ctx_document_external_ref,
               d.review_state AS ctx_document_review_state
        FROM commercial_links cl
        LEFT JOIN btcpay_provenance_records p ON p.id = cl.btcpay_record_id
        LEFT JOIN btcpay_provenance_records inv
          ON inv.profile_id = cl.profile_id
         AND inv.store_id = p.store_id
         AND inv.invoice_id = p.invoice_id
         AND inv.record_type = 'invoice'
        LEFT JOIN external_documents d ON d.id = cl.document_id
        WHERE cl.profile_id = ? AND cl.transaction_id = ?
          AND cl.state IN ('suggested', 'reviewed')
        ORDER BY cl.reviewed_at DESC, cl.created_at DESC, cl.id DESC"#;
