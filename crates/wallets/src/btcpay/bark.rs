//! `kassiber/btcpay/bark.py` and `core/btcpay_bark.py`: a store's Bark (Ark)
//! wallet served by the BTCPay Bark plugin (0.10 and later), booked so that
//! every final row's booked effect equals its `amount` to the msat, under the
//! identity of the invoice payment or payout it settles, without counting a
//! payment twice when a payment ledger already booked it.
use super::ledger;
use super::Result;
use crate::file_import;
use crate::publication::row::{field, or_text, rows};
use kassiber_chain::btcpay::normalize::{parse_labels, unix_to_iso};
use kassiber_chain::btcpay::payment_methods::{self as methods, BARK_PAYMENT_METHOD_ID};
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pydecimal::Decimal;
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{params, Connection};
use std::collections::BTreeMap;

pub const RAW_SOURCE: &str = "btcpay_bark_wallet";
pub const PAYMENT_HASH_SOURCE: &str = "btcpay_bark";
const FALLBACK_PREFIX: &str = "btcpay-bark";
pub const PART_PRINCIPAL: &str = "principal";
pub const PART_FEE: &str = "fee";
/// No row can move more than every bitcoin there will be.
const MAX_MSAT: u128 = 21_000_000 * 100_000_000 * 1000;

const INBOUND_KINDS: [&str; 3] = ["ark-receive", "lightning-receive", "onchain-receive"];
const SEND_KINDS: [&str; 6] = [
    "ark-send",
    "lightning-send",
    "offboard",
    "send-onchain",
    "onchain-send",
    "exit",
];
const FEE_KINDS: [&str; 5] = [
    "refresh",
    "board",
    "exit-fee",
    "exit-claim",
    "onchain-internal",
];
const LIGHTNING_KINDS: [&str; 2] = ["lightning-receive", "lightning-send"];

fn known(kind: &str) -> bool {
    INBOUND_KINDS.contains(&kind) || SEND_KINDS.contains(&kind) || FEE_KINDS.contains(&kind)
}

fn is_hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn unsupported(row_id: &str, kind: &str, reason: &str, message: String) -> AppError {
    AppError::new("bark_row_unsupported", message)
        .with_hint("Kassiber books a Bark wallet only when every row's amount and fee agree. Update Kassiber, or keep this store's Bark payments as a payment ledger until it understands this row.")
        .with_details(dict([
            ("row_id", s(row_id)),
            ("kind", s(kind)),
            ("reason", s(reason)),
        ]))
}

fn malformed(message: String, row_id: Option<&str>) -> AppError {
    let error = AppError::new("protocol_error", message);
    match row_id {
        Some(id) => error.with_details(dict([("row_id", s(id))])),
        None => error,
    }
}

/// `str(value).strip()` or `None` when empty (`_text`).
fn text(value: &Value) -> Option<String> {
    if matches!(value, Value::Null) {
        return None;
    }
    let text = kassiber_pytext::strip(&py_str(value)).to_owned();
    (!text.is_empty()).then_some(text)
}

fn opt(value: Option<String>) -> Value {
    value.map_or(Value::Null, s)
}

/// `btc_msat`: exact msat of a BTC amount, refusing anything not a whole msat.
pub fn btc_msat(value: &Value, name: &str, row_id: &str) -> Result<i64> {
    let invalid = || {
        malformed(
            format!("Bark wallet row '{row_id}' has an invalid {name}"),
            Some(row_id),
        )
    };
    let below = || {
        malformed(
            format!("The {name} of Bark wallet row '{row_id}' is not a whole msat"),
            Some(row_id),
        )
    };
    let raw = match value {
        Value::Int(_) | Value::Float(_) | Value::Str(_) => py_str(value),
        _ => return Err(invalid()),
    };
    let amount: Decimal = kassiber_pytext::strip(&raw)
        .parse()
        .map_err(|_| invalid())?;
    if amount.is_zero() {
        return Ok(0);
    }
    // Exact and independent of the decimal context: digits times a power of ten.
    if amount.adjusted() > 8 {
        return Err(invalid());
    }
    if amount.adjusted() < -11 {
        return Err(below());
    }
    let digits = amount.coefficient().to_string();
    let shift = amount.exponent() + 11;
    let whole = if shift >= 0 {
        format!("{digits}{}", "0".repeat(shift as usize))
    } else {
        let cut = digits.len() - (-shift) as usize;
        if digits[cut..].bytes().any(|b| b != b'0') {
            return Err(below());
        }
        digits[..cut].to_owned()
    };
    let msat: u128 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| invalid())?
    };
    if msat > MAX_MSAT {
        return Err(invalid());
    }
    let msat = msat as i64;
    Ok(if amount.is_negative() { -msat } else { msat })
}

/// `invoice_label_ref`: the BTCPay invoice id of a row's `invoice` label.
pub fn invoice_label_ref(labels: &Value) -> Option<String> {
    let label = get(labels, "invoice").filter(|v| matches!(v, Value::Dict(_)))?;
    let reference = get(label, "ref").filter(|v| truthy(v));
    text(
        reference
            .or_else(|| get(label, "id"))
            .unwrap_or(&Value::Null),
    )
}

/// `row_id`, without lone surrogates.
pub fn row_id(row: &Value) -> Result<String> {
    kassiber_chain::btcpay::bark::row_id(row).map(|id| kassiber_chain::btcpay::url::from_py(&id))
}

/// `row_keys`: what ties a row to a BTCPay invoice payment.
#[derive(Clone, Debug)]
pub struct Keys {
    pub outpoint: Option<String>,
    pub payment_hash: Option<String>,
    /// The row's `movementId` when it is an int.
    pub movement_id: Option<String>,
    pub movement_value: Value,
    pub invoice_id: Option<String>,
}

impl Default for Keys {
    fn default() -> Self {
        Keys {
            outpoint: None,
            payment_hash: None,
            movement_id: None,
            movement_value: Value::Null,
            invoice_id: None,
        }
    }
}

fn row_keys(row: &Value) -> Keys {
    let outpoint = text(field(row, "outpoint")).map(|v| kassiber_pytext::lower(&v));
    let payment_hash = text(field(row, "paymentHash"))
        .map(|v| kassiber_pytext::lower(&v))
        .filter(|v| is_hex64(v));
    let movement = field(row, "movementId");
    let (movement_id, movement_value) = match movement {
        Value::Int(n) => (Some(n.to_string()), movement.clone()),
        _ => (None, Value::Null),
    };
    Keys {
        outpoint,
        payment_hash,
        movement_id,
        movement_value,
        invoice_id: invoice_label_ref(field(row, "labels")),
    }
}

/// What a BTCPay Bark invoice payment id names (`payment_keys`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaymentKey {
    PaymentHash(String),
    Outpoint(String),
    Movement {
        invoice_id: String,
        movement_id: String,
    },
    None,
}

fn ascii_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

/// `int(digits)` as text.
fn int_text(digits: &str) -> String {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".into()
    } else {
        trimmed.into()
    }
}

/// `payment_keys`: `txid-vout` is an Ark VTXO, a bare 64-hex id a Lightning
/// payment hash, `{invoiceId}#ark-movement-N` or `…-N-I` (before plugin 0.10)
/// a movement.
pub fn payment_keys(payment_id: &Value) -> PaymentKey {
    let raw = if truthy(payment_id) {
        py_str(payment_id)
    } else {
        String::new()
    };
    let raw = kassiber_pytext::strip(&raw);
    let lowered = kassiber_pytext::lower(raw);
    if is_hex64(&lowered) {
        return PaymentKey::PaymentHash(lowered);
    }
    if let Some((txid, vout)) = lowered.split_once('-') {
        if is_hex64(txid) && ascii_digits(vout) {
            return PaymentKey::Outpoint(format!("{txid}:{}", int_text(vout)));
        }
    }
    if let Some((invoice_id, movement)) = raw.split_once("#ark-movement-") {
        let (number, index) = match movement.split_once('-') {
            Some((number, index)) => (number, Some(index)),
            None => (movement, None),
        };
        if !invoice_id.is_empty() && ascii_digits(number) && index.is_none_or(ascii_digits) {
            return PaymentKey::Movement {
                invoice_id: invoice_id.to_owned(),
                movement_id: int_text(number),
            };
        }
    }
    PaymentKey::None
}

/// `fallback_identity`.
pub fn fallback_identity(store_id: &str, row_id: &str, part: &str) -> String {
    let base = format!("{FALLBACK_PREFIX}:{store_id}:{row_id}");
    if part == PART_PRINCIPAL {
        base
    } else {
        format!("{base}:{part}")
    }
}

/// `is_fallback_identity`.
pub fn is_fallback_identity(store_id: &str, external_id: &str) -> bool {
    external_id.starts_with(&format!("{FALLBACK_PREFIX}:{store_id}:"))
}

/// One import record of a Bark row (`bark_records`), before its identity.
#[derive(Clone, Debug)]
pub struct Record {
    pub row_id: String,
    pub part: &'static str,
    /// The principal record's payment keys; empty for a fee record.
    pub keys: Keys,
    pub direction: &'static str,
    pub amount_msat: i64,
    pub fee_msat: i64,
    pub payment_hash: Option<String>,
    pub transaction_hash: Option<String>,
    /// The record dict, without `id`/`txid` until an identity is set.
    pub record: Value,
    pub id: String,
}

impl Record {
    /// `record["id"] = record["txid"] = identity`.
    pub fn set_identity(&mut self, identity: &str) {
        self.id = identity.to_owned();
        kassiber_envelope::set(&mut self.record, "id", s(identity));
        kassiber_envelope::set(&mut self.record, "txid", s(identity));
    }
}

/// `bark_records`: import records for one final Bark wallet row.
pub fn records(row: &Value, store_id: &str) -> Result<Vec<Record>> {
    let bark_row_id = row_id(row)?;
    let kind = text(field(row, "kind")).unwrap_or_else(|| "unknown".into());
    if !known(&kind) {
        return Err(unsupported(
            &bark_row_id,
            &kind,
            "unknown_kind",
            format!("Bark wallet row '{bark_row_id}' has an unknown kind '{kind}'"),
        ));
    }
    let amount_value = field(row, "amount");
    if matches!(amount_value, Value::Null) {
        return Err(malformed(
            format!("Bark wallet row '{bark_row_id}' is missing 'amount'"),
            Some(&bark_row_id),
        ));
    }
    let amount = btc_msat(amount_value, "amount", &bark_row_id)?;
    let fee = match field(row, "fee") {
        Value::Null => 0,
        value => btc_msat(value, "fee", &bark_row_id)?,
    };
    if fee < 0 {
        return Err(unsupported(
            &bark_row_id,
            &kind,
            "negative_fee",
            format!("Bark wallet row '{bark_row_id}' has a negative fee"),
        ));
    }
    let timestamp = field(row, "timestamp");
    if matches!(timestamp, Value::Null) {
        return Err(malformed(
            format!("Bark wallet row '{bark_row_id}' is missing 'timestamp'"),
            Some(&bark_row_id),
        ));
    }
    let occurred_at = unix_to_iso(timestamp)?;
    let keys = row_keys(row);
    let lightning = LIGHTNING_KINDS.contains(&kind.as_str());
    let payment_hash = keys.payment_hash.clone().filter(|_| lightning);
    let transaction_hash = text(field(row, "transactionHash"));
    let comment = text(field(row, "comment"));
    let labels = field(row, "labels");

    let record = |part: &'static str,
                  direction: &'static str,
                  principal: i64,
                  fee_part: i64,
                  record_kind: &str|
     -> Result<Record> {
        let principal_part = part == PART_PRINCIPAL;
        let raw = dict([
            ("amount_msat", Value::Int(amount.into())),
            // Not `payment_hash`: the book-open backfill would copy it onto
            // the row's fee record.
            ("bark_payment_hash", opt(keys.payment_hash.clone())),
            ("bark_row_id", s(bark_row_id.as_str())),
            ("fee_msat", Value::Int(fee.into())),
            ("invoice_id", opt(keys.invoice_id.clone())),
            ("kind", s(kind.as_str())),
            ("movement_id", keys.movement_value.clone()),
            ("outpoint", opt(keys.outpoint.clone())),
            ("part", s(part)),
            ("source", s(RAW_SOURCE)),
            ("store_id", s(store_id)),
            ("transaction_hash", opt(transaction_hash.clone())),
        ]);
        let raw = kassiber_pyjson::dumps(&raw, &kassiber_pyjson::Options::new().sort_keys(true))
            .map_err(|e| AppError::new("internal_error", e.to_string()))?;
        let hash = if principal_part {
            payment_hash.clone()
        } else {
            None
        };
        let description = comment
            .clone()
            .unwrap_or_else(|| format!("Bark {kind}{}", if principal_part { "" } else { " fee" }));
        Ok(Record {
            row_id: bark_row_id.clone(),
            part,
            keys: if principal_part {
                keys.clone()
            } else {
                Keys::default()
            },
            direction,
            amount_msat: principal,
            fee_msat: fee_part,
            payment_hash: hash.clone(),
            transaction_hash: transaction_hash.clone(),
            id: String::new(),
            record: dict([
                ("occurred_at", s(occurred_at.as_str())),
                ("confirmed_at", s(occurred_at.as_str())),
                ("direction", s(direction)),
                ("asset", s("BTC")),
                ("amount", s(file_import::msat_to_btc(principal))),
                ("fee", s(file_import::msat_to_btc(fee_part))),
                ("amount_includes_fee", Value::Bool(false)),
                ("kind", s(record_kind)),
                ("description", s(description)),
                ("counterparty", Value::Null),
                ("payment_hash", opt(hash.clone())),
                (
                    "payment_hash_source",
                    if hash.is_some() {
                        s(PAYMENT_HASH_SOURCE)
                    } else {
                        Value::Null
                    },
                ),
                (
                    "_btcpay_comment",
                    if principal_part {
                        opt(comment.clone())
                    } else {
                        Value::Null
                    },
                ),
                (
                    "_btcpay_labels",
                    if principal_part {
                        parse_labels(labels)
                    } else {
                        Value::List(vec![])
                    },
                ),
                ("raw_json", s(raw)),
            ]),
        })
    };

    if amount > 0 {
        if !INBOUND_KINDS.contains(&kind.as_str()) {
            return Err(unsupported(
                &bark_row_id,
                &kind,
                "unexpected_inflow",
                format!("Bark wallet row '{bark_row_id}' ({kind}) adds funds"),
            ));
        }
        let mut out = vec![record(
            PART_PRINCIPAL,
            "inbound",
            amount + fee,
            0,
            "deposit",
        )?];
        if fee != 0 {
            out.push(record(PART_FEE, "outbound", 0, fee, "fee")?);
        }
        return Ok(out);
    }
    if amount < 0 {
        if INBOUND_KINDS.contains(&kind.as_str()) {
            return Err(unsupported(
                &bark_row_id,
                &kind,
                "unexpected_outflow",
                format!("Bark wallet row '{bark_row_id}' ({kind}) removes funds"),
            ));
        }
        if -amount == fee {
            return Ok(vec![record(PART_PRINCIPAL, "outbound", 0, fee, "fee")?]);
        }
        if -amount < fee {
            return Err(unsupported(
                &bark_row_id,
                &kind,
                "fee_exceeds_amount",
                format!("Bark wallet row '{bark_row_id}' has a fee larger than its amount"),
            ));
        }
        if !SEND_KINDS.contains(&kind.as_str()) {
            return Err(unsupported(
                &bark_row_id,
                &kind,
                "unexpected_outflow",
                format!("Bark wallet row '{bark_row_id}' ({kind}) sends funds"),
            ));
        }
        return Ok(vec![record(
            PART_PRINCIPAL,
            "outbound",
            -amount - fee,
            fee,
            "withdrawal",
        )?]);
    }
    if fee != 0 {
        return Err(unsupported(
            &bark_row_id,
            &kind,
            "fee_without_amount",
            format!("Bark wallet row '{bark_row_id}' has a fee but no amount"),
        ));
    }
    Ok(vec![])
}

// ---------------------------------------------------------------------------
// The book: identities, the ledger switch and Lightning rows (`core/btcpay_bark.py`)
// ---------------------------------------------------------------------------

/// Tables whose rows bind a transaction's wallet or quantity; soft-deleted
/// pairs, swap payouts and loan legs bind nothing.
const BINDING_REFERENCES: [(&str, &[&str], &str); 8] = [
    (
        "custody_component_legs",
        &["transaction_id", "anchor_transaction_id"],
        "",
    ),
    (
        "custody_component_transaction_memberships",
        &["transaction_id"],
        "",
    ),
    ("custody_gap_review_transactions", &["transaction_id"], ""),
    (
        "transaction_pairs",
        &["out_transaction_id", "in_transaction_id"],
        " AND deleted_at IS NULL",
    ),
    (
        "direct_swap_payouts",
        &["out_transaction_id"],
        " AND deleted_at IS NULL",
    ),
    ("loan_legs", &["transaction_id"], " AND deleted_at IS NULL"),
    (
        "source_funds_links",
        &["from_transaction_id", "to_transaction_id"],
        "",
    ),
    ("source_funds_cases", &["target_transaction_id"], ""),
];

fn database(error: kassiber_store::rusqlite::Error) -> AppError {
    crate::database_error(error)
}

/// `_raw`: a stored JSON object, or empty.
fn raw(value: &Value) -> Value {
    let parsed = match value {
        Value::Str(_) => kassiber_pytext::loads(&py_str(value)).ok(),
        _ => None,
    };
    match parsed {
        Some(object @ Value::Dict(_)) => object,
        _ => dict::<&str>([]),
    }
}

fn raw_text(object: &Value, key: &str) -> String {
    let value = field(object, key);
    if truthy(value) {
        py_str(value)
    } else {
        String::new()
    }
}

fn int(value: &Value) -> i64 {
    match value {
        Value::Int(n) => i64::try_from(n.clone()).unwrap_or_default(),
        _ => 0,
    }
}

/// Provenance keys and amounts by what a Bark row can match.
type Index = BTreeMap<IndexKey, Vec<(String, Option<i64>)>>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum IndexKey {
    Outpoint(String),
    InboundHash(String),
    Movement(String, String),
    OutboundHash(String),
    TransactionHash(String),
}

/// `_provenance_index`: provenance keys (and amounts) of the store's Bark and
/// Lightning payments and payouts.
fn provenance_index(conn: &Connection, profile_id: &str, store_id: &str) -> Result<Index> {
    let mut index: Index = BTreeMap::new();
    let mut add = |key: IndexKey, stable: &str, amount: Option<i64>| {
        let keys = index.entry(key).or_default();
        if !keys.iter().any(|(k, _)| k == stable) {
            keys.push((stable.to_owned(), amount));
        }
    };
    for row in rows(
        conn,
        "SELECT record_type, stable_key, invoice_id, payment_id, payment_method_id, txid, payment_hash, amount FROM btcpay_provenance_records WHERE profile_id = ? AND store_id = ? AND record_type IN ('payment', 'payout') ORDER BY stable_key ASC",
        [profile_id, store_id],
    )? {
        let method = raw_text(&row, "payment_method_id");
        let bark = methods::canonical(&method) == BARK_PAYMENT_METHOD_ID;
        let lightning = methods::ledger_group(&method).as_deref() == Some("lightning");
        if !(bark || lightning) {
            continue;
        }
        let stable = or_text(field(&row, "stable_key"));
        let amount = match field(&row, "amount") {
            Value::Int(n) => i64::try_from(n.clone()).ok(),
            _ => None,
        };
        let payment_hash = Some(kassiber_pytext::lower(&raw_text(&row, "payment_hash"))).filter(|h| !h.is_empty());
        if or_text(field(&row, "record_type")) == "payout" {
            if let Some(hash) = &payment_hash {
                add(IndexKey::OutboundHash(hash.clone()), &stable, amount);
            }
            let txid = raw_text(&row, "txid");
            if bark && !txid.is_empty() {
                add(IndexKey::TransactionHash(kassiber_pytext::lower(&txid)), &stable, amount);
            }
            continue;
        }
        let mut key = if bark { payment_keys(field(&row, "payment_id")) } else { PaymentKey::None };
        if lightning {
            if let Some(hash) = &payment_hash {
                key = PaymentKey::PaymentHash(hash.clone());
            }
        }
        match key {
            PaymentKey::Outpoint(outpoint) => add(IndexKey::Outpoint(outpoint), &stable, amount),
            PaymentKey::PaymentHash(hash) => add(IndexKey::InboundHash(hash), &stable, amount),
            PaymentKey::Movement { invoice_id, movement_id } => {
                if field(&row, "invoice_id") == &s(invoice_id.as_str()) {
                    add(IndexKey::Movement(invoice_id, movement_id), &stable, amount);
                }
            }
            PaymentKey::None => {}
        }
    }
    Ok(index)
}

/// `_candidates`.
fn candidates(record: &Record, index: &Index) -> Vec<String> {
    let keys = &record.keys;
    let mut lookups = Vec::new();
    if record.direction == "inbound" {
        if let Some(outpoint) = &keys.outpoint {
            lookups.push(IndexKey::Outpoint(outpoint.clone()));
        }
        if let Some(hash) = &keys.payment_hash {
            lookups.push(IndexKey::InboundHash(hash.clone()));
        }
        if let (Some(invoice), Some(movement)) = (&keys.invoice_id, &keys.movement_id) {
            lookups.push(IndexKey::Movement(invoice.clone(), movement.clone()));
        }
    } else {
        if let Some(hash) = &keys.payment_hash {
            lookups.push(IndexKey::OutboundHash(hash.clone()));
        }
        if let Some(txid) = &record.transaction_hash {
            lookups.push(IndexKey::TransactionHash(kassiber_pytext::lower(txid)));
        }
    }
    let Some(found) = lookups
        .into_iter()
        .find_map(|lookup| index.get(&lookup).filter(|found| !found.is_empty()))
    else {
        return vec![];
    };
    // One movement can pay several payments (one per VTXO): the amount tells
    // them apart, or none is taken.
    if found.len() > 1 {
        let same: Vec<&String> = found
            .iter()
            .filter(|(_, amount)| amount.is_some_and(|a| a.abs() == record.amount_msat))
            .map(|(key, _)| key)
            .collect();
        if same.len() == 1 {
            return vec![same[0].clone()];
        }
    }
    found.iter().map(|(key, _)| key.clone()).collect()
}

/// `bark_networks`: `main`/`test` from the Ark addresses the store's Bark
/// payments went to.
fn bark_networks(conn: &Connection, profile_id: &str, store_id: &str) -> Result<Vec<&'static str>> {
    let mut found = std::collections::BTreeSet::new();
    for row in rows(
        conn,
        "SELECT payment_method_id, destination FROM btcpay_provenance_records WHERE profile_id = ? AND store_id = ? AND record_type = 'payment' AND destination IS NOT NULL",
        [profile_id, store_id],
    )? {
        if methods::canonical(&raw_text(&row, "payment_method_id")) != BARK_PAYMENT_METHOD_ID {
            continue;
        }
        let destination = kassiber_pytext::lower(kassiber_pytext::strip(&py_str(field(&row, "destination"))));
        if destination.starts_with("tark1") {
            found.insert("test");
        } else if destination.starts_with("ark1") {
            found.insert("main");
        }
    }
    Ok(found.into_iter().collect())
}

/// `require_book_network`: refuse a Bark wallet whose payments went to
/// another network than the book's.
pub fn require_book_network(conn: &Connection, profile_id: &str, store_id: &str) -> Result<()> {
    let binding = super::setup::book_network(conn, profile_id)?;
    if field(&binding, "state") != &s("bound") {
        return Ok(());
    }
    let environment = field(&binding, "environment").clone();
    let scope = if environment == s("main") {
        "main"
    } else {
        "test"
    };
    let networks = bark_networks(conn, profile_id, store_id)?;
    if !networks.is_empty() && networks != [scope] {
        return Err(AppError::new(
            "book_network_mismatch",
            format!("The Bark wallet of store {store_id} is on a different network than this book"),
        )
        .with_hint("Its payments went to Ark addresses of another network (`ark1` is mainnet, `tark1` a test network). Connect this store in a book bound to that network.")
        .with_details(dict([
            ("store_id", s(store_id)),
            ("bark_networks", Value::List(networks.into_iter().map(s).collect())),
            ("book_environment", environment),
        ])));
    }
    Ok(())
}

/// A stored row booked from a Bark record.
struct Stored {
    id: String,
    external_id: String,
    occurred_at: String,
    direction: String,
    asset: String,
    amount: i64,
    fee: i64,
}

/// `stored_identities`: `(bark_row_id, part)` -> the stored row booked from it.
fn stored_identities(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
) -> Result<BTreeMap<(String, String), Stored>> {
    let mut stored = BTreeMap::new();
    for row in rows(
        conn,
        "SELECT id, external_id, occurred_at, direction, asset, amount, fee, raw_json FROM transactions WHERE profile_id = ? AND wallet_id = ? AND external_id IS NOT NULL ORDER BY created_at ASC, id ASC",
        [profile_id, wallet_id],
    )? {
        let object = raw(field(&row, "raw_json"));
        if raw_text(&object, "source") != RAW_SOURCE || !truthy(field(&object, "bark_row_id")) {
            continue;
        }
        let part = match raw_text(&object, "part") {
            part if part.is_empty() => PART_PRINCIPAL.to_owned(),
            part => part,
        };
        stored
            .entry((py_str(field(&object, "bark_row_id")), part))
            .or_insert_with(|| Stored {
                id: or_text(field(&row, "id")),
                external_id: or_text(field(&row, "external_id")),
                occurred_at: or_text(field(&row, "occurred_at")),
                direction: or_text(field(&row, "direction")),
                asset: or_text(field(&row, "asset")),
                amount: int(field(&row, "amount")),
                fee: int(field(&row, "fee")),
            });
    }
    Ok(stored)
}

fn identity_taken(conn: &Connection, wallet_id: &str, external_id: &str) -> Result<bool> {
    Ok(!rows(
        conn,
        "SELECT 1 FROM transactions WHERE wallet_id = ? AND external_id = ?",
        [wallet_id, external_id],
    )?
    .is_empty())
}

/// What `assign_identities` did.
#[derive(Default, Debug, Clone, Copy)]
pub struct Identities {
    pub linked: i64,
    pub ambiguous: i64,
    pub rekeyed: i64,
}

/// `assign_identities`: each record's identity; a stored fallback id moves to
/// a provenance key that appeared. A key two rows would share is ambiguous.
pub fn assign_identities(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    store_id: &str,
    records: &mut [Record],
) -> Result<Identities> {
    let index = provenance_index(conn, profile_id, store_id)?;
    let mut claims: BTreeMap<String, i64> = BTreeMap::new();
    let mut wanted = Vec::with_capacity(records.len());
    for record in records.iter() {
        let found = if record.part == PART_PRINCIPAL {
            candidates(record, &index)
        } else {
            vec![]
        };
        let key = (found.len() == 1).then(|| found[0].clone());
        if let Some(key) = &key {
            *claims.entry(key.clone()).or_default() += 1;
        }
        wanted.push(key);
    }
    let stored = stored_identities(conn, profile_id, wallet_id)?;
    let mut counts = Identities::default();
    for (record, key) in records.iter_mut().zip(wanted) {
        let mut key = key;
        if key.as_ref().is_some_and(|k| claims[k] > 1) {
            counts.ambiguous += 1;
            key = None;
        }
        let desired = key
            .clone()
            .unwrap_or_else(|| fallback_identity(store_id, &record.row_id, record.part));
        counts.linked += i64::from(key.is_some());
        let Some(existing) = stored.get(&(record.row_id.clone(), record.part.to_owned())) else {
            record.set_identity(&desired);
            continue;
        };
        let mut current = existing.external_id.clone();
        if current != desired
            && key.is_some()
            && is_fallback_identity(store_id, &current)
            && !identity_taken(conn, wallet_id, &desired)?
        {
            let fingerprint = file_import::fingerprint_parts(
                wallet_id,
                &desired,
                &existing.occurred_at,
                &existing.direction,
                &existing.asset,
                &file_import::msat_to_btc(existing.amount),
                &file_import::msat_to_btc(existing.fee),
            );
            conn.execute(
                "UPDATE transactions SET external_id = ?, fingerprint = ? WHERE id = ?",
                params![desired, fingerprint, existing.id],
            )
            .map_err(database)?;
            counts.rekeyed += 1;
            current = desired;
        }
        record.set_identity(&current);
    }
    Ok(counts)
}

/// `binding_references`: tables that bind this transaction's wallet or quantity.
pub fn binding_references(conn: &Connection, transaction_id: &str) -> Result<Vec<&'static str>> {
    let mut found = Vec::new();
    for (table, columns, live) in BINDING_REFERENCES {
        let clause = columns
            .iter()
            .map(|c| format!("{c} = ?"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let sql = format!("SELECT 1 FROM {table} WHERE ({clause}){live} LIMIT 1");
        let bound = conn
            .prepare(&sql)
            .and_then(|mut st| {
                st.exists(kassiber_store::rusqlite::params_from_iter(
                    columns.iter().map(|_| transaction_id),
                ))
            })
            .map_err(database)?;
        if bound {
            found.push(table);
        }
    }
    Ok(found)
}

fn conflict(message: String, reason: &str, hint: &str, details: Vec<(&str, Value)>) -> AppError {
    let mut pairs = vec![("reason", s(reason))];
    pairs.extend(details);
    AppError::new("bark_migration_conflict", message)
        .with_hint(hint)
        .with_details(dict(pairs))
}

/// `_stamp`: keep `row` as `record`: its id, notes and reviews stay; Bark's fee and evidence apply.
fn stamp(conn: &Connection, row: &Value, record: &Record, wallet_id: &str) -> Result<()> {
    let hash = record
        .payment_hash
        .as_ref()
        .map(|h| s(h.as_str()))
        .unwrap_or_else(|| field(row, "payment_hash").clone());
    let hash_source = if record.payment_hash.is_some() {
        s(PAYMENT_HASH_SOURCE)
    } else {
        field(row, "payment_hash_source").clone()
    };
    let fingerprint = file_import::fingerprint_parts(
        wallet_id,
        &or_text(field(row, "external_id")),
        &or_text(field(row, "occurred_at")),
        &or_text(field(row, "direction")),
        &or_text(field(row, "asset")),
        &file_import::msat_to_btc(int(field(row, "amount"))),
        &file_import::msat_to_btc(record.fee_msat),
    );
    let text_or_null = |value: &Value| match value {
        Value::Null => None,
        other => Some(py_str(other)),
    };
    conn.execute(
        "UPDATE transactions SET wallet_id = ?, fee = ?, amount_includes_fee = 0, raw_json = ?, payment_hash = ?, payment_hash_source = ?, fingerprint = ? WHERE id = ?",
        params![
            wallet_id,
            record.fee_msat,
            py_str(field(&record.record, "raw_json")),
            text_or_null(&hash),
            text_or_null(&hash_source),
            fingerprint,
            or_text(field(row, "id")),
        ],
    )
    .map_err(database)?;
    Ok(())
}

/// `_match`: why `record` cannot keep `row`.
fn mismatch(row: &Value, record: Option<&Record>) -> Option<&'static str> {
    let Some(record) = record else {
        return Some("unmatched");
    };
    if or_text(field(row, "direction")) != record.direction || or_text(field(row, "asset")) != "BTC"
    {
        return Some("direction_mismatch");
    }
    (record.amount_msat != int(field(row, "amount"))).then_some("amount_mismatch")
}

/// `migrate_payment_ledger`: keep a `BTC-ARK` ledger's rows as the Bark
/// records they booked; refuses before any write when one cannot be kept.
pub fn migrate_payment_ledger(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    label: &str,
    records: &[Record],
) -> Result<i64> {
    let mut by_id: BTreeMap<&str, Vec<&Record>> = BTreeMap::new();
    for record in records {
        by_id.entry(record.id.as_str()).or_default().push(record);
    }
    let mut plan = Vec::new();
    let mut problems = Vec::new();
    for row in rows(
        conn,
        "SELECT * FROM transactions WHERE profile_id = ? AND wallet_id = ? AND excluded = 0 ORDER BY occurred_at ASC, id ASC",
        [profile_id, wallet_id],
    )? {
        if raw_text(&raw(field(&row, "raw_json")), "source") != ledger::RAW_SOURCE {
            continue;
        }
        let external = or_text(field(&row, "external_id"));
        let matches = by_id.get(external.as_str()).cloned().unwrap_or_default();
        let record = (matches.len() == 1).then(|| matches[0]);
        let mut reason = if matches.len() > 1 { Some("ambiguous") } else { mismatch(&row, record) };
        let mut extra: Vec<(&str, Value)> = vec![];
        if let (None, Some(record)) = (reason, record) {
            if record.fee_msat != int(field(&row, "fee")) {
                // Adding Bark's fee changes the row's quantity; reviewed evidence on it would not follow.
                let bound = binding_references(conn, &or_text(field(&row, "id")))?;
                if !bound.is_empty() {
                    reason = Some("bound_by_review");
                    extra.push(("tables", Value::List(bound.into_iter().map(s).collect())));
                }
            }
        }
        if let Some(reason) = reason {
            let mut problem = vec![
                ("transaction_id", field(&row, "id").clone()),
                ("external_id", field(&row, "external_id").clone()),
                ("reason", s(reason)),
            ];
            problem.extend(extra);
            problems.push(dict(problem));
            continue;
        }
        plan.push((row, record.expect("matched")));
    }
    if !problems.is_empty() {
        return Err(conflict(
            format!("{} payment(s) booked by '{label}' have no single matching row in the Bark wallet", problems.len()),
            "unmatched_ledger_rows",
            "Kassiber keeps every booked payment when it switches this store to Bark's wallet history, and refuses rather than guess. Check the listed payments in BTCPay; excluding a payment that was booked by mistake lets the switch go ahead. Until then the payment ledger stays as it is.",
            vec![("wallet", s(label)), ("problems", Value::List(problems))],
        ));
    }
    for (row, record) in &plan {
        stamp(conn, row, record, wallet_id)?;
    }
    Ok(plan.len() as i64)
}

/// `adopt_lightning_ledger_rows`: move `BTC-LN` ledger rows of payments the
/// Bark wallet books into it; another wallet booking one refuses.
pub fn adopt_lightning_ledger_rows(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    backend_name: &str,
    store_id: &str,
    records: &[Record],
) -> Result<i64> {
    let mut moved = 0;
    for record in records {
        let Some(hash) = record
            .payment_hash
            .as_ref()
            .map(|h| kassiber_pytext::lower(h))
        else {
            continue;
        };
        for row in rows(
            conn,
            "SELECT t.*, w.label AS wallet_label, w.config_json AS wallet_config FROM transactions t JOIN wallets w ON w.id = t.wallet_id WHERE t.profile_id = ? AND t.wallet_id != ? AND t.excluded = 0 AND LOWER(t.payment_hash) = ? AND t.direction = ? ORDER BY t.id ASC",
            [profile_id, wallet_id, hash.as_str(), record.direction],
        )? {
            let config: serde_json::Value = serde_json::from_str(&or_text(field(&row, "wallet_config"))).unwrap_or_default();
            let config = if config.is_object() { config } else { serde_json::json!({}) };
            let object = raw(field(&row, "raw_json"));
            let backend = config["backend"].as_str().map(|v| kassiber_pytext::lower(kassiber_pytext::strip(v))).unwrap_or_default();
            let ledger_row = raw_text(&object, "source") == ledger::RAW_SOURCE
                && raw_text(&object, "store_id") == store_id
                && backend == kassiber_pytext::lower(kassiber_pytext::strip(backend_name))
                && ledger::is_ledger(&config);
            let label = or_text(field(&row, "wallet_label"));
            let mut details: Vec<(&str, Value)> = vec![
                ("payment_hash", s(hash.as_str())),
                ("transaction_id", field(&row, "id").clone()),
                ("wallet", s(label.as_str())),
                ("bark_row_id", s(record.row_id.as_str())),
            ];
            if !ledger_row {
                return Err(conflict(
                    format!("'{label}' already books a Lightning payment the Bark wallet settles"),
                    "booked_by_other_wallet",
                    "A payment can settle in one wallet only. Check which wallet really received it and exclude the other copy, then sync again.",
                    details,
                ));
            }
            let mut reason = if or_text(field(&row, "external_id")) != record.id {
                Some("identity_mismatch")
            } else if identity_taken(conn, wallet_id, &record.id)? {
                Some("already_booked_by_bark")
            } else {
                mismatch(&row, Some(record))
            };
            let bound = binding_references(conn, &or_text(field(&row, "id")))?;
            if reason.is_none() && !bound.is_empty() {
                reason = Some("bound_by_review");
                details.push(("tables", Value::List(bound.into_iter().map(s).collect())));
            }
            if let Some(reason) = reason {
                return Err(conflict(
                    format!("A Lightning payment in '{label}' cannot move to the Bark wallet that settled it"),
                    reason,
                    "The Bark wallet owns Lightning payments that settle into it, so the Lightning ledger's copy moves there with its notes and reviews. Resolve the listed review first, or exclude one copy.",
                    details,
                ));
            }
            stamp(conn, &row, record, wallet_id)?;
            moved += 1;
        }
    }
    Ok(moved)
}

/// `is_bark_ledger`: a payment ledger that books only the Bark rail.
pub fn is_bark_ledger(config: &serde_json::Value) -> bool {
    let methods = if ledger::is_ledger(config) {
        ledger::payment_method_ids(config)
    } else {
        vec![]
    };
    !methods.is_empty()
        && methods
            .iter()
            .all(|m| methods::canonical(m) == BARK_PAYMENT_METHOD_ID)
}

/// `bark_config`: a `BTC-ARK` payment ledger's config as a Bark wallet import.
pub fn bark_config(config: &serde_json::Value) -> serde_json::Value {
    let mut switched = config.as_object().cloned().unwrap_or_default();
    switched.remove("source_mode");
    switched.remove("payment_method_ids");
    switched.insert(
        "payment_method_id".into(),
        serde_json::Value::String(BARK_PAYMENT_METHOD_ID.into()),
    );
    serde_json::Value::Object(switched)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vectors() -> Value {
        kassiber_pytext::loads(include_str!("../../tests/vectors/python_btcpay_bark.json")).unwrap()
    }

    fn section(name: &str) -> Vec<Value> {
        match get(&vectors(), name) {
            Some(Value::List(items)) => items.clone(),
            _ => panic!("{name}"),
        }
    }

    fn outcome<T>(result: Result<T>, ok: impl FnOnce(T) -> Value) -> Value {
        match result {
            Ok(value) => dict([("ok", ok(value))]),
            Err(e) if e.code == "internal_error" => dict([("exception", s("internal"))]),
            Err(e) => dict([(
                "error",
                dict([
                    ("code", s(e.code.as_str())),
                    ("message", s(e.message.to_string())),
                    ("hint", e.hint.as_deref().map_or(Value::Null, s)),
                    ("details", e.details.clone().unwrap_or(Value::Null)),
                ]),
            )]),
        }
    }

    /// The Python record dict: private keys first, as `bark_records` builds it.
    fn python_record(record: &Record) -> Value {
        let keys = if record.part == PART_PRINCIPAL {
            dict([
                ("outpoint", opt(record.keys.outpoint.clone())),
                ("payment_hash", opt(record.keys.payment_hash.clone())),
                ("movement_id", record.keys.movement_value.clone()),
                ("invoice_id", opt(record.keys.invoice_id.clone())),
            ])
        } else {
            dict::<&str>([])
        };
        let mut pairs = vec![
            ("_bark_row_id".to_owned(), s(record.row_id.as_str())),
            ("_bark_part".to_owned(), s(record.part)),
            ("_bark_keys".to_owned(), keys),
        ];
        if let Value::Dict(items) = &record.record {
            for (key, value) in items {
                let kassiber_pyjson::Key::Str(key) = key else {
                    panic!("key")
                };
                pairs.push((kassiber_chain::btcpay::url::from_py(key), value.clone()));
            }
        }
        dict(pairs)
    }

    #[test]
    fn python_bark_row_vectors() {
        for case in section("rows") {
            let actual = outcome(records(field(&case, "input"), "S1"), |records| {
                Value::List(records.iter().map(python_record).collect())
            });
            let expected = field(&case, "records");
            if get(expected, "exception").is_some() {
                assert!(get(&actual, "exception").is_some(), "{case:?}: {actual:?}");
            } else {
                assert_eq!(&actual, expected, "{case:?}");
            }
        }
    }

    #[test]
    fn python_bark_payment_key_vectors() {
        for case in section("payment_keys") {
            let actual = match payment_keys(field(&case, "input")) {
                PaymentKey::PaymentHash(hash) => dict([("payment_hash", s(hash))]),
                PaymentKey::Outpoint(outpoint) => dict([("outpoint", s(outpoint))]),
                PaymentKey::Movement {
                    invoice_id,
                    movement_id,
                } => dict([
                    ("invoice_id", s(invoice_id)),
                    (
                        "movement_id",
                        Value::Int(movement_id.parse::<i64>().unwrap().into()),
                    ),
                ]),
                PaymentKey::None => dict::<&str>([]),
            };
            assert_eq!(&actual, field(&case, "keys"), "{case:?}");
        }
    }

    #[test]
    fn python_bark_amount_vectors() {
        for case in section("amounts") {
            let actual = outcome(btc_msat(field(&case, "input"), "amount", "r"), |n| {
                Value::Int(n.into())
            });
            assert_eq!(&actual, field(&case, "msat"), "{case:?}");
        }
    }
}
